//! core/script.js: steps run against a XenoNet Dev instance, one after
//! another: the steps xenonet console takes on its command line
//! ("select:atlas-api"), and scripts, JSON lists of the same steps as
//! strings or as { op, args } (a driver method and its arguments, as the
//! console logs what you do). Steps wait for the view to settle, so a
//! script never sleeps for a fixed time unless it says so.
//!
//! Each step waits on the instance, so it's a state machine: `begin` and
//! `resume` say what to call next (the driver, or the control client's
//! launch, input, window…), what to say, how long to sleep, and when the
//! step is done or has failed. Whoever runs it makes the calls.

use crate::jsv::{raw, truthy};
use serde_json::{json, Map, Value};
use xenonet_base::text::{js_number, js_string};

pub const STEPS: &str = r#"  launch[:inactive|front|hidden]  start it unless it's running, and wait until it's ready
  launch:restart[:…]              quit it if it's running, or end a copy stuck holding its
                                  lock, then start it afresh (launch:restart:hidden, say)
  quit                            quit it
  status / state                  the instance and its window / the page's state
  find:<words>                    nodes matching the words, with their ids
  select:<id or words>            select a node (a project gathers its sessions)
  deselect, back, next, prev, reset
  command:<id>                    a palette command; commands lists them
  stats:<id or words>             a session's stats panel
  simulate:<kind>[,a[,b]]         needs-you, collision, stuck, wiki-write (a session writing to
                                  the shared context), turn-end, doctor, clear;
                                  or an actor acting: actor-spawn, actor-edit, actor-message,
                                  actor-online, actor-offline (a: the actor, b: what it acts on);
                                  or actor-request[,name]: a made-up program asking to be an
                                  actor, waiting for an Allow, and actor-needs-you[,actor]: an
                                  agent waiting on you with a made-up question until clear,
                                  actor-tunnel[,actor] and actor-tunnel-down[,actor]: a
                                  made-up connection through a tunnel, with an adapter
                                  connected or not, and actor-read[,actor[,node]]: a read
                                  through it (all demo and test instances only);
                                  with XENONET_HOSTS=1 also host-add,<id>,<port> (another XenoNet
                                  here, shown as a host), host-offline and host-online (a: the
                                  host, the first if left out) and host-skew[,host[,ms]];
                                  and a made-up network across machines: net-none,
                                  net-founder, net-member, net-gateway-down, net-ticket,
                                  net-mismatch and net-remote-waiting (a: the machine you're at;
                                  b: titles-only, the others share titles only)
  actors                          the actors beside the map: presence, status, beams; and the
                                  programs waiting for an Allow
  feed[:<n>]                      the activity feed's last lines, sessions' and actors'
  identities[:<keys>|all]         the identity chips; with chip keys (comma-separated), only their work
  mask:on|off                     hide identities (GitHub A, Claude B…) or show them
  hosts[:<ids>|all]               other machines on the map (over SSH with XENONET_HOSTS=1,
                                  or in a network) and how each
                                  stands; with host ids (comma-separated, @here for this
                                  machine), only their work
  network[:open|start|join|add]   the network across machines (Network settings): this
                                  machine's, its gateway and the other machines; add opens
                                  Add a machine
  root[:<folder>|reset]           the projects folder; with a folder (~/… or a full path), read
                                  that one instead, or reset to go back to ~/Projects
  key:<combo>                     Mod+K, Escape, ArrowDown, W… (Mod is ⌘ on a Mac, Ctrl elsewhere)
  type:<text>                     typed into whatever has focus
  click:<x,y | @id>               a real click; click2: double-clicks
  hover:<x,y | @id>               moves the pointer there
  drag:<x1,y1,x2,y2>              alt-drag: holds Alt, which moves a node
  scroll:<x,y,dy>                 a wheel turn
  size:<1600x900[@2] | window>    the page's size
  show / hide / minimize          the window (shown without taking focus)
  still:on|off                    hold ambient animation still
  settle                          wait for the view to settle
  shot:<name>                     .xenonet/<name>.png, held still and settled
  eval:<js>                       an expression, or a body with return
  logs                            recent warnings and errors
  listen:<ms>                     new warnings and errors over that long
  theme:<name>                    switch the theme, live, and wait until it's drawn
  themes                          the themes, and the one in use
  environment:<id>                choose nebula, aurora, trench or vault independently of the theme
  environments                    the environments and the one in use
  environmentPicker               preview environment cards; keepEnvironment or revertEnvironment
  mcp:<node-id>                    open a shared system's MCP details; mcpTab and mcpSearch browse it
  previewTheme:<name>             show a theme in this window alone; keepTheme
                                  or revertTheme after
  themeSwitcher[:off]             "Theme…": the switcher; stepTheme[:-1] moves
                                  through it, showing each theme (keepTheme or
                                  revertTheme after, as Enter and Esc do)
  themeGallery[:<name>|off]       the gallery of themes; focusTheme:<name> looks
                                  at a card, makeTheme makes your own from it
  film[:<seconds>]                play a film of the day (the day plays for
                                  <seconds>, 12 unless said) and wait for its end
  sleep:<ms>
  <method>[:<arg>]                any other driver method (src/driver.js), such as ops or clearOps"#;

/// An argument to a call: a JSON value, undefined, or an object of them
/// (which JSON can't hold when a field is undefined).
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    V(Value),
    Undef,
    Obj(Vec<(&'static str, Arg)>),
}

fn v(x: Value) -> Arg {
    Arg::V(x)
}

/// What the runner does next, after saying `say`.
#[derive(Clone, Debug, PartialEq)]
pub enum Then {
    /// Calls `target` (the driver, or one of the control client's own
    /// methods) with these arguments, and resumes with what it answers.
    Call {
        target: &'static str,
        args: Vec<Arg>,
    },
    /// Waits this many ms and resumes.
    Sleep(f64),
    Done,
    /// Throws the error the last call threw.
    Rethrow,
    /// Throws an Error (a TypeError when `type_error`) with this message.
    Fail {
        message: String,
        type_error: bool,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct Out {
    pub say: Vec<Arg>,
    pub then: Then,
}

/// How a call came back.
pub enum Answer {
    Ok(Value),
    /// The call answered undefined.
    Undefined,
    Threw(String),
}

/// What the runner knows that a step may need: the instance's name, and
/// how long a film plays when the step doesn't say.
pub struct Context {
    pub name: String,
    pub film_seconds: f64,
}

/// A step under way, as JSON for whoever runs it to keep.
pub type State = Value;

fn call(target: &'static str, args: Vec<Arg>) -> Then {
    Then::Call { target, args }
}

fn driver(method: impl Into<Value>, args: Vec<Arg>) -> Then {
    let mut a = vec![v(method.into())];
    a.extend(args);
    call("driver", a)
}

fn fail(message: impl Into<String>) -> Then {
    Then::Fail { message: message.into(), type_error: false }
}

fn type_error(message: impl Into<String>) -> Then {
    Then::Fail { message: message.into(), type_error: true }
}

// A value in a template literal: String(v), "undefined" when missing.
fn t(x: Option<&Value>) -> String {
    x.map_or_else(|| "undefined".into(), js_string)
}

fn read<'a>(x: Option<&'a Value>, k: &str) -> Result<Option<&'a Value>, String> {
    match x {
        None | Some(Value::Null) => Err(format!("Cannot read properties of {} (reading '{k}')", if x.is_none() { "undefined" } else { "null" })),
        Some(o) => Ok(o.get(k)),
    }
}

fn length(x: Option<&Value>) -> Result<Option<Value>, String> {
    // read throws for undefined and null, as JS does reading .length.
    read(x, "length")?;
    Ok(match x {
        Some(Value::Array(a)) => Some(json!(a.len())),
        Some(Value::String(s)) => Some(json!(crate::jstext::utf16_len(s))),
        _ => None,
    })
}

fn keys_on(x: Option<&Value>) -> Result<Vec<Value>, String> {
    match x {
        None | Some(Value::Null) => Err("Cannot convert undefined or null to object".into()),
        Some(Value::Object(m)) => Ok(m.iter().filter(|(_, x)| truthy(Some(x))).map(|(k, _)| json!(k)).collect()),
        Some(_) => Ok(Vec::new()),
    }
}

/// The state, briefly: what's selected and gathered, which overlays are
/// up, how many alerts, and whether the view has settled.
pub fn brief(st: &Value) -> Result<Value, String> {
    let s = Some(st);
    let selection = raw(read(s, "selection")?, "id").filter(|x| !x.is_null()).cloned().unwrap_or(Value::Null);
    let gathered = match read(s, "gathered")? {
        g if truthy(g) => {
            let members = read(g, "members")?;
            Value::String(format!("{} around {}", t(length(members)?.as_ref()), t(read(g, "root")?)))
        }
        _ => Value::Null,
    };
    let mut out = Map::new();
    out.insert("selection".into(), selection);
    out.insert("gathered".into(), gathered);
    out.insert("overlays".into(), Value::Array(keys_on(read(s, "overlays")?)?));
    if let Some(n) = length(read(s, "alerts")?)? {
        out.insert("alerts".into(), n);
    }
    if let Some(settled) = read(read(s, "motion")?, "settled")? {
        out.insert("settled".into(), settled.clone());
    }
    Ok(Value::Object(out))
}

fn number(s: &str) -> f64 {
    js_number(Some(&Value::String(s.into())))
}

fn num(n: f64) -> Value {
    xenonet_base::json::number(n)
}

// `arg.split(",").map(Number)`, with a part that isn't there undefined.
fn numbers<const N: usize>(arg: &str) -> [Arg; N] {
    let parts: Vec<&str> = arg.split(',').collect();
    std::array::from_fn(|i| parts.get(i).map_or(Arg::Undef, |p| v(num(number(p)))))
}

fn point(arg: &str) -> Arg {
    if let Some(id) = arg.strip_prefix('@') {
        return Arg::Obj(vec![("node", v(json!(id)))]);
    }
    let [x, y] = numbers(arg);
    Arg::Obj(vec![("x", x), ("y", y)])
}

fn split_list(arg: &str) -> Value {
    if arg == "all" {
        json!([])
    } else {
        json!(arg.split(',').collect::<Vec<_>>())
    }
}

struct Run {
    st: Map<String, Value>,
    say: Vec<Arg>,
}

impl Run {
    fn kind(&self) -> String {
        self.st.get("kind").and_then(Value::as_str).unwrap_or("").into()
    }
    // A step given as { op, args }: "op", "wait" or "film".
    fn object(&self) -> Option<String> {
        self.st.get("object").and_then(Value::as_str).map(String::from)
    }
    fn arg(&self) -> String {
        self.st.get("arg").and_then(Value::as_str).unwrap_or("").into()
    }
    fn stage(&self) -> u64 {
        self.st.get("stage").and_then(Value::as_u64).unwrap_or(0)
    }
    fn next(&mut self, stage: u64, then: Then) -> Then {
        self.st.insert("stage".into(), json!(stage));
        then
    }
    fn say(&mut self, x: Value) {
        self.say.push(Arg::V(x));
    }
    fn say_brief(&mut self, st: &Value) -> Result<(), String> {
        let b = brief(st)?;
        self.say(b);
        Ok(())
    }
    fn out(self, then: Then) -> (State, Out) {
        (Value::Object(self.st), Out { say: self.say, then })
    }
}

/// Starts a step: a string ("select:atlas") or { op, args }.
pub fn begin(step: &Value, ctx: &Context) -> (State, Out) {
    let mut r = Run { st: Map::new(), say: Vec::new() };
    r.st.insert("name".into(), json!(ctx.name));
    let then = match step {
        Value::Object(_) | Value::Array(_) => {
            let op = raw(Some(step), "op").cloned();
            let args: Vec<Value> = match raw(Some(step), "args") {
                None => Vec::new(),
                Some(Value::Array(a)) => a.clone(),
                Some(other) => vec![other.clone()],
            };
            let first = args.first();
            match op.as_ref().and_then(Value::as_str) {
                Some("wait") => {
                    let ms = js_number(first);
                    r.st.insert("object".into(), json!("wait"));
                    return r.out(Then::Sleep(if ms.is_nan() || ms == 0.0 { 0.0 } else { ms }));
                }
                Some("shot") => return begin(&json!(format!("shot:{}", first.filter(|x| !x.is_null()).map_or_else(|| "shot".into(), js_string))), ctx),
                Some("theme") => return begin(&json!(format!("theme:{}", t(first))), ctx),
                Some("film") => {
                    let seconds = raw(first, "seconds").filter(|x| !x.is_null()).or(first.filter(|x| !x.is_null())).cloned().unwrap_or_else(|| num(ctx.film_seconds));
                    r.st.insert("object".into(), json!("film"));
                    return start_film(r, seconds);
                }
                _ => {
                    r.st.insert("object".into(), json!("op"));
                    let mut a = vec![op.map_or(Arg::Undef, Arg::V)];
                    a.extend(args.into_iter().map(Arg::V));
                    r.next(1, call("driver", a))
                }
            }
        }
        _ => {
            let text = js_string(step);
            let (cmd, arg) = match text.find(':') {
                None => (text.clone(), String::new()),
                Some(i) => (text[..i].to_string(), text[i + 1..].to_string()),
            };
            r.st.insert("kind".into(), json!(cmd));
            r.st.insert("arg".into(), json!(arg));
            r.st.insert("filmSeconds".into(), num(ctx.film_seconds));
            first_call(&mut r, &cmd, &arg)
        }
    };
    r.out(then)
}

fn start_film(mut r: Run, seconds: Value) -> (State, Out) {
    let then = r.next(50, driver("film", vec![Arg::Obj(vec![("seconds", v(seconds))])]));
    r.out(then)
}

fn first_call(r: &mut Run, cmd: &str, arg: &str) -> Then {
    let input = |o: Arg| call("input", vec![o]);
    let window = |o: Arg| call("window", vec![o]);
    match cmd {
        "launch" => {
            let mut opts: Vec<(&'static str, Arg)> = Vec::new();
            let set = |opts: &mut Vec<(&'static str, Arg)>, k: &'static str, x: Value| match opts.iter_mut().find(|(n, _)| *n == k) {
                Some(slot) => slot.1 = v(x),
                None => opts.push((k, v(x))),
            };
            if !arg.is_empty() {
                for word in arg.split(':') {
                    match word {
                        "restart" => set(&mut opts, "restart", json!(true)),
                        "inactive" | "front" | "hidden" => set(&mut opts, "show", json!(word)),
                        _ => return fail(format!("launch takes restart, inactive, front or hidden, not \"{word}\"")),
                    }
                }
            }
            r.next(1, call("launch", vec![Arg::Obj(opts)]))
        }
        "quit" => r.next(1, call("quit", vec![])),
        "status" => r.next(1, call("ping", vec![])),
        "state" => r.next(1, driver("state", vec![])),
        "find" => r.next(1, driver("find", vec![v(json!({ "query": arg, "limit": 30 }))])),
        "select" => r.next(1, driver("select", vec![v(json!(arg))])),
        "deselect" => r.next(1, driver("select", vec![v(Value::Null)])),
        "back" | "reset" => r.next(1, driver(cmd, vec![])),
        "next" | "prev" => r.next(1, driver("step", vec![v(json!(if cmd == "next" { 1 } else { -1 }))])),
        "stats" => {
            if arg.contains(':') {
                r.st.insert("id".into(), json!(arg));
                r.next(2, driver("stats", vec![v(json!(arg)), v(json!({ "settle": false }))]))
            } else {
                r.next(1, driver("find", vec![v(json!({ "query": arg, "kind": "session", "limit": 1 }))]))
            }
        }
        "command" => r.next(1, driver("command", vec![v(json!(arg))])),
        "commands" => r.next(1, driver("commands", vec![])),
        "simulate" => {
            // An actor and what it acts on may follow the kind, after
            // commas; ids have colons in them, never commas.
            let parts: Vec<&str> = arg.split(',').collect();
            let kind = json!(parts[0]);
            let a = parts.get(1).filter(|x| !x.is_empty());
            let b = parts.get(2).filter(|x| !x.is_empty());
            if a.is_some() || b.is_some() {
                let named = |x: Option<&&str>| x.map_or(Arg::Undef, |s| v(json!(s)));
                r.next(1, call("simulate", vec![v(kind), Arg::Obj(vec![("a", named(a)), ("b", named(b))])]))
            } else {
                r.next(1, call("simulate", vec![v(kind)]))
            }
        }
        "actors" => r.next(1, driver("actors", vec![])),
        "feed" => {
            let n = number(arg);
            r.next(1, driver("feed", vec![v(json!({ "limit": if n == 0.0 || n.is_nan() { num(20.0) } else { num(n) } }))]))
        }
        "identities" | "hosts" => {
            if arg.is_empty() {
                r.next(2, driver(cmd, vec![]))
            } else {
                r.next(1, driver(if cmd == "identities" { "filterIdentities" } else { "filterHosts" }, vec![v(split_list(arg))]))
            }
        }
        "mask" => r.next(1, driver("maskIdentities", vec![v(json!(arg != "off"))])),
        "root" => {
            if arg.is_empty() {
                r.next(1, driver("root", vec![]))
            } else {
                r.next(2, driver("setRoot", vec![v(if arg == "reset" { Value::Null } else { json!(arg) })]))
            }
        }
        "key" => r.next(1, input(Arg::Obj(vec![("keys", v(json!(arg)))]))),
        "type" => r.next(1, input(Arg::Obj(vec![("text", v(json!(arg)))]))),
        "click" | "click2" => r.next(1, input(Arg::Obj(vec![("click", point(arg)), ("count", v(json!(if cmd == "click2" { 2 } else { 1 })))]))),
        "hover" => r.next(1, input(Arg::Obj(vec![("move", point(arg))]))),
        "drag" | "alt-drag" => {
            let [x1, y1, x2, y2] = numbers(arg);
            let from = Arg::Obj(vec![("x", x1), ("y", y1)]);
            let to = Arg::Obj(vec![("x", x2), ("y", y2)]);
            let modifiers = if cmd == "alt-drag" { json!(["Alt"]) } else { json!([]) };
            r.next(1, input(Arg::Obj(vec![("drag", Arg::Obj(vec![("from", from), ("to", to)])), ("modifiers", v(modifiers))])))
        }
        "scroll" => {
            let [x, y, dy] = numbers(arg);
            r.next(1, input(Arg::Obj(vec![("scroll", Arg::Obj(vec![("x", x), ("y", y), ("dy", dy)]))])))
        }
        "size" => r.next(1, window(Arg::Obj(vec![("size", v(if arg == "window" { Value::Null } else { json!(arg) }))]))),
        "show" => r.next(1, window(Arg::Obj(vec![("show", v(json!(if arg.is_empty() { "inactive" } else { arg })))]))),
        "hide" => r.next(1, window(Arg::Obj(vec![("hide", v(json!(true)))]))),
        "minimize" => r.next(1, window(Arg::Obj(vec![("minimize", v(json!(true)))]))),
        "still" => r.next(1, driver("still", vec![v(json!(arg != "off"))])),
        "settle" => r.next(1, driver("settled", vec![])),
        "shot" => r.next(1, call("screenshot", vec![v(json!({ "name": if arg.is_empty() { "shot" } else { arg }, "inline": false }))])),
        "eval" => r.next(1, call("eval", vec![v(json!(arg))])),
        "logs" => r.next(1, call("logs", vec![v(json!({ "level": "warning" }))])),
        "listen" => r.next(1, call("logs", vec![v(json!({ "limit": 1 }))])),
        // Through the page, which switches without a reload and answers
        // once the theme is drawn and the view has settled.
        "theme" => r.next(1, driver("theme", vec![v(json!(arg))])),
        "film" => {
            let n = number(arg);
            let seconds = if n == 0.0 || n.is_nan() { r.st.get("filmSeconds").cloned().unwrap_or(json!(12)) } else { num(n) };
            r.next(50, driver("film", vec![Arg::Obj(vec![("seconds", v(seconds))])]))
        }
        "sleep" => r.next(1, Then::Sleep(number(arg))),
        // Any other driver method, with the text after the colon as its
        // one argument.
        _ => r.next(1, driver(cmd, if arg.is_empty() { vec![] } else { vec![v(json!(arg))] })),
    }
}

/// Takes what the last call came back with and says what's next.
pub fn resume(state: State, answer: Answer) -> (State, Out) {
    let mut r = Run {
        st: match state {
            Value::Object(m) => m,
            _ => Map::new(),
        },
        say: Vec::new(),
    };
    let then = match resume_in(&mut r, answer) {
        Ok(then) => then,
        Err(message) => type_error(message),
    };
    r.out(then)
}

fn resume_in(r: &mut Run, answer: Answer) -> Result<Then, String> {
    let kind = r.kind();
    let arg = r.arg();
    let stage = r.stage();
    let object = r.object();
    let mut undefined = false;
    let got = match answer {
        Answer::Ok(x) => x,
        Answer::Undefined => {
            undefined = true;
            Value::Null
        }
        Answer::Threw(message) => {
            // Where JS caught an error itself; everything else goes on up.
            if object.is_some() {
                return Ok(Then::Rethrow);
            }
            return Ok(match (kind.as_str(), stage) {
                ("select", 1) if message.contains("has no node") => r.next(2, driver("find", vec![v(json!({ "query": arg, "limit": 1 }))])),
                ("actors", 2) => after_requests(r, None)?,
                (k, 1) if !KNOWN.contains(&k) && message.contains("the driver has no") => fail(format!("no step \"{k}\"; xenonet console --help lists them")),
                _ => Then::Rethrow,
            });
        }
    };
    let g = Some(&got);
    // What the call answered, as it's said: undefined stays undefined.
    let as_said = |x: &Value| if undefined { Arg::Undef } else { Arg::V(x.clone()) };
    if stage >= 50 {
        return film_resume(r, got);
    }
    match object.as_deref() {
        Some("wait") => return Ok(Then::Done),
        Some(_) => {
            r.say_brief(&got)?;
            return Ok(Then::Done);
        }
        None => {}
    }
    Ok(match (kind.as_str(), stage) {
        ("sleep", _) => Then::Done,
        ("launch" | "quit" | "state" | "settle" | "eval" | "size" | "show" | "hide" | "minimize", _) => {
            r.say.push(as_said(&got));
            Then::Done
        }
        ("status", _) => {
            let name = t(r.st.get("name"));
            r.say(if got.is_null() { json!(format!("{name} isn't running")) } else { got });
            Then::Done
        }
        ("find", _) => {
            for n in list(read(g, "nodes")?)? {
                let n = Some(&n);
                let drawn = if truthy(raw(n, "drawn")) { "" } else { "  (not drawn)" };
                r.say(json!(format!("{}  {}  {}{drawn}", t(raw(n, "id")), t(raw(n, "kind")), t(raw(n, "title")))));
            }
            Then::Done
        }
        ("select" | "identities" | "hosts", 1) | ("select", 3) | ("deselect" | "back" | "reset" | "next" | "prev" | "command" | "mask", _) => {
            r.say_brief(&got)?;
            match kind.as_str() {
                "identities" | "hosts" => r.next(2, driver(kind.clone(), vec![])),
                _ => Then::Done,
            }
        }
        ("select", 2) => {
            let nodes = list(read(g, "nodes")?)?;
            match nodes.first() {
                None => fail(format!("nothing on the map matches \"{arg}\"")),
                Some(n) => r.next(3, driver("select", vec![raw(Some(n), "id").cloned().map_or(Arg::Undef, Arg::V)])),
            }
        }
        ("stats", 1) => {
            let nodes = list(read(g, "nodes")?)?;
            match nodes.first().and_then(|n| raw(Some(n), "id")).filter(|id| truthy(Some(id))).cloned() {
                None => fail(format!("no session matches \"{arg}\"")),
                Some(id) => r.next(2, driver("stats", vec![v(id), v(json!({ "settle": false }))])),
            }
        }
        ("stats", _) => {
            let mut o = Map::new();
            o.insert("overlays".into(), Value::Array(keys_on(read(g, "overlays")?)?));
            if let Some(s) = read(g, "stats")? {
                o.insert("stats".into(), s.clone());
            }
            r.say(Value::Object(o));
            Then::Done
        }
        ("commands", _) => {
            for c in list(g)? {
                r.say(json!(format!("{}  {}", t(raw(Some(&c), "id")), t(raw(Some(&c), "label")))));
            }
            Then::Done
        }
        ("simulate", _) => {
            // A made-up registration's token is still a token: it isn't printed.
            // `const { token, ...done } = …` copies the rest of an object
            // (and nothing of anything else).
            let done = match got {
                Value::Object(mut m) => {
                    m.shift_remove("token");
                    Value::Object(m)
                }
                Value::Null if !undefined => return Err("Cannot destructure 'null' as it is null.".into()),
                Value::Null => return Err("Cannot destructure 'undefined' as it is undefined.".into()),
                _ => json!({}),
            };
            r.say(done);
            Then::Done
        }
        ("actors", 1) => {
            let tunnels = match raw(g, "tunnels") {
                None | Some(Value::Null) => Vec::new(),
                t => list(t)?,
            };
            let gate = |id: Option<&Value>| tunnels.iter().rev().find(|x| crate::jsv::strict_eq(raw(Some(x), "actor"), id)).cloned();
            for a in list(read(g, "actors")?)? {
                let a = Some(&a);
                let status = if truthy(raw(a, "status")) { format!(" · {}", t(raw(a, "status"))) } else { String::new() };
                let beams = length(read(a, "beams")?)?.and_then(|n| n.as_f64()).unwrap_or(f64::NAN);
                let beams = if beams > 0.0 { format!("  → {} beam{}", num(beams), if beams == 1.0 { "" } else { "s" }) } else { String::new() };
                let tunnel = match gate(raw(a, "id")) {
                    Some(gt) => format!("  ⊙ tunnel {}", if js_number(raw(Some(&gt), "open")) >= 0.5 { "open" } else { "shut" }),
                    None => String::new(),
                };
                r.say(json!(format!("{}  {}  {}{status}{beams}{tunnel}", t(raw(a, "id")), t(raw(a, "presence")), t(raw(a, "title")))));
            }
            r.next(2, driver("actorRequests", vec![]))
        }
        ("actors", _) => after_requests(r, Some(got))?,
        ("feed", _) => {
            for l in list(read(g, "lines")?)? {
                let l = Some(&l);
                let detail = if truthy(raw(l, "detail")) { format!(" · {}", t(raw(l, "detail"))) } else { String::new() };
                r.say(json!(format!("{}  {}{detail}", t(raw(l, "kind")), t(raw(l, "title")))));
            }
            Then::Done
        }
        ("identities", _) => {
            r.say.push(as_said(&got));
            Then::Done
        }
        ("hosts", _) => {
            for x in list(read(g, "hosts")?)? {
                let x = Some(&x);
                let stale = if truthy(raw(x, "stale")) { "  (not live)" } else { "" };
                let drawn = if truthy(raw(x, "drawn")) { "" } else { "  (not drawn)" };
                r.say(json!(format!("{}  {}{stale}  {} sessions{drawn}", t(raw(x, "id")), t(raw(x, "state")), t(raw(x, "sessions")))));
            }
            Then::Done
        }
        ("root", 1) => {
            r.say(json!(format!("{}  {}", t(read(g, "path")?), t(read(g, "source")?))));
            Then::Done
        }
        ("root", _) => {
            let mut b = brief(&got)?;
            if let Value::Object(m) = &mut b {
                m.insert("root".into(), read(read(g, "root")?, "path")?.cloned().unwrap_or(Value::Null));
            }
            r.say(b);
            Then::Done
        }
        ("key" | "type" | "click" | "click2" | "hover" | "drag" | "alt-drag" | "scroll", _) => {
            match read(g, "did")? {
                Some(d) => r.say(d.clone()),
                None => r.say.push(Arg::Undef),
            }
            Then::Done
        }
        ("still", _) => {
            r.say.push(Arg::Obj(vec![("still", as_said(&got))]));
            Then::Done
        }
        ("shot", _) => {
            let notes = list(read(g, "notes")?)?;
            let notes = if notes.is_empty() { String::new() } else { format!(" {}", xenonet_base::text::join(notes.iter().map(Some), "; ")) };
            r.say(json!(format!("shot {} ({}×{}){notes}", t(read(g, "file")?), t(read(g, "width")?), t(read(g, "height")?))));
            Then::Done
        }
        ("logs", _) => {
            for e in list(read(g, "entries")?)? {
                r.say(json!(log_line(&e)?));
            }
            Then::Done
        }
        ("listen", 1) => {
            if let Some(c) = read(g, "cursor")? {
                r.st.insert("cursor".into(), c.clone());
            }
            let ms = number(&arg);
            r.next(2, Then::Sleep(if ms == 0.0 || ms.is_nan() { 3000.0 } else { ms }))
        }
        ("listen", 2) => {
            let since = r.st.get("cursor").cloned().map_or(Arg::Undef, Arg::V);
            r.next(3, call("logs", vec![Arg::Obj(vec![("since", since), ("level", v(json!("warning")))])]))
        }
        ("listen", _) => {
            let lines: Vec<String> = list(read(g, "entries")?)?.iter().map(log_line).collect::<Result<_, _>>()?;
            let text = lines.join("\n");
            r.say(json!(if text.is_empty() { "no errors".to_string() } else { text }));
            Then::Done
        }
        ("theme", _) => {
            r.say(json!(format!("theme {arg}")));
            Then::Done
        }
        _ => {
            if truthy(raw(g, "overlays")) {
                r.say_brief(&got)?;
            } else {
                r.say.push(as_said(&got));
            }
            Then::Done
        }
    })
}

/// The steps the switch knows; anything else goes to the driver as a method.
const KNOWN: [&str; 42] = [
    "launch",
    "quit",
    "status",
    "state",
    "find",
    "select",
    "deselect",
    "back",
    "reset",
    "next",
    "prev",
    "stats",
    "command",
    "commands",
    "simulate",
    "actors",
    "feed",
    "identities",
    "mask",
    "hosts",
    "root",
    "key",
    "type",
    "click",
    "click2",
    "hover",
    "drag",
    "alt-drag",
    "scroll",
    "size",
    "show",
    "hide",
    "minimize",
    "still",
    "settle",
    "shot",
    "eval",
    "logs",
    "listen",
    "theme",
    "film",
    "sleep",
];

fn list(x: Option<&Value>) -> Result<Vec<Value>, String> {
    match x {
        Some(Value::Array(a)) => Ok(a.clone()),
        Some(Value::Null) | None => Err("is not iterable".into()),
        Some(_) => Ok(Vec::new()),
    }
}

fn log_line(e: &Value) -> Result<String, String> {
    let e = Some(e);
    let level = read(e, "level")?.and_then(Value::as_str).ok_or("e.level.toUpperCase is not a function")?;
    Ok(format!("{} [{}] {}", level.to_uppercase(), t(raw(e, "source")), t(raw(e, "text"))))
}

fn after_requests(r: &mut Run, got: Option<Value>) -> Result<Then, String> {
    let requests = match got.as_ref().filter(|g| !g.is_null()).map(|g| g.get("requests")) {
        None | Some(None) => Vec::new(),
        Some(Some(x)) => list(Some(x))?,
    };
    for q in requests {
        let q = Some(&q);
        let asker = raw(q, "asker");
        let by = if truthy(asker) { format!(" · asked by {}", t(raw(asker, "program"))) } else { String::new() };
        r.say(json!(format!("{}  waiting  {} asks to join{by}", t(raw(q, "actor")), t(raw(q, "name")))));
    }
    Ok(Then::Done)
}

// A film plays and is waited out: the state is asked every 400 ms until
// its overlay has come and gone. 50: the film started; 51: a state came
// back; 52: the wait between two is over.
fn film_resume(r: &mut Run, got: Value) -> Result<Then, String> {
    match r.stage() {
        51 => {
            let film = read(read(Some(&got), "overlays")?, "film")?;
            if truthy(film) {
                r.st.insert("seen".into(), json!(true));
            } else if r.st.get("seen") == Some(&json!(true)) {
                let b = brief(&got)?;
                r.say(b);
                return Ok(Then::Done);
            }
            Ok(r.next(52, Then::Sleep(400.0)))
        }
        _ => Ok(r.next(51, driver("state", vec![]))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        Context { name: "XenoNet Dev (demo)".into(), film_seconds: 12.0 }
    }

    #[test]
    fn a_word_for_an_id_finds_the_node() {
        let (s, out) = begin(&json!("select:atlas"), &ctx());
        assert_eq!(out.then, driver("select", vec![v(json!("atlas"))]));
        let (s, out) = resume(s, Answer::Threw("XenoNet has no node \"atlas\"".into()));
        assert_eq!(out.then, driver("find", vec![v(json!({ "query": "atlas", "limit": 1 }))]));
        let (s, out) = resume(s, Answer::Ok(json!({ "nodes": [{ "id": "project:atlas" }] })));
        assert_eq!(out.then, driver("select", vec![v(json!("project:atlas"))]));
        let (_, out) =
            resume(s, Answer::Ok(json!({ "selection": { "id": "project:atlas" }, "gathered": null, "overlays": { "modal": false }, "alerts": [], "motion": { "settled": true } })));
        assert_eq!(out.say, [v(json!({ "selection": "project:atlas", "gathered": null, "overlays": [], "alerts": 0, "settled": true }))]);
        assert_eq!(out.then, Then::Done);
    }

    #[test]
    fn a_launch_typo_never_starts_anything() {
        let (_, out) = begin(&json!("launch:restart:hiden"), &ctx());
        assert_eq!(out.then, fail("launch takes restart, inactive, front or hidden, not \"hiden\""));
        let (_, out) = begin(&json!("launch:front:restart"), &ctx());
        assert_eq!(out.then, call("launch", vec![Arg::Obj(vec![("show", v(json!("front"))), ("restart", v(json!(true)))])]));
    }

    #[test]
    fn simulate_keeps_the_token_to_itself() {
        let (s, out) = begin(&json!("simulate:actor-request,Heron"), &ctx());
        assert_eq!(out.then, call("simulate", vec![v(json!("actor-request")), Arg::Obj(vec![("a", v(json!("Heron"))), ("b", Arg::Undef)])]));
        let (_, out) = resume(s, Answer::Ok(json!({ "ok": true, "token": "xenonet_actor_0", "expiresAt": 1 })));
        assert_eq!(out.say, [v(json!({ "ok": true, "expiresAt": 1 }))]);
    }

    #[test]
    fn an_unknown_step_says_so() {
        let (s, _) = begin(&json!("nonesuch"), &ctx());
        let (_, out) = resume(s, Answer::Threw("the driver has no nonesuch()".into()));
        assert_eq!(out.then, fail("no step \"nonesuch\"; xenonet console --help lists them"));
    }
}
