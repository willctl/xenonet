//! xenonet-engine's procs for the twin of core/procs.js: the decisions and
//! what's kept are Rust's; the twin runs the commands (`exec`) and reads
//! the clock (`now`) when asked.

use crate::js::{Json, Jv, MaybeJson};
use crate::machine::{start, Answer, Asker, Machine};
use napi_derive::napi;
use serde_json::{json, Map, Value};
use std::sync::Arc;
use xenonet_base::json::number;
use xenonet_base::text::js_string;
use xenonet_engine::procs::{self, Call, Failure, Link, PidMap, Procs, Product, Row};

/// The twin's exec and clock, through a machine's questions.
pub struct Bridged<'a>(pub &'a Asker);

fn text_of(v: Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s,
        v => js_string(&v),
    }
}

pub fn failure_of(e: Value) -> Failure {
    Failure { message: e.get("message").cloned().map(text_of).unwrap_or_default(), stdout: e.get("stdout").and_then(Value::as_str).map(str::to_string) }
}

fn call_json(c: &Call) -> Value {
    let mut q = json!({ "op": "exec", "file": c.file, "args": c.args });
    if let Some(s) = &c.share {
        q["share"] = Value::String(s.clone());
    }
    q
}

fn exec_answer(a: Answer) -> Result<String, Failure> {
    a.map(text_of).map_err(failure_of)
}

impl procs::Io for Bridged<'_> {
    fn exec(&mut self, call: Call) -> Result<String, Failure> {
        exec_answer(self.0.ask(call_json(&call)))
    }
    fn exec_all(&mut self, calls: Vec<Call>) -> Vec<Result<String, Failure>> {
        self.0.ask_all(calls.iter().map(call_json).collect()).into_iter().map(exec_answer).collect()
    }
    fn now(&mut self) -> f64 {
        self.0.ask(json!({ "op": "now" })).ok().and_then(|v| v.as_f64()).unwrap_or(f64::NAN)
    }
}

pub fn row_json(r: &Row) -> Map<String, Value> {
    let mut o = Map::new();
    o.insert("pid".into(), number(r.pid));
    o.insert("ppid".into(), number(r.ppid));
    o.insert("start".into(), number(r.start));
    o.insert("program".into(), Value::String(r.program.clone()));
    o
}

pub fn product_json(p: Option<Product>) -> Value {
    p.map_or(Value::Null, |p| json!({ "product": p.product, "role": p.role }))
}

pub fn link_json(l: &Link) -> Value {
    let mut o = row_json(&l.row);
    if let Some(exe) = &l.exe {
        o.insert("exe".into(), exe.clone().map_or(Value::Null, Value::String));
    }
    o.insert("agent".into(), l.agent.map_or(Value::Null, |a| Value::String(a.into())));
    o.insert("product".into(), product_json(l.product));
    Value::Object(o)
}

fn table_json(t: &PidMap<Row>) -> Value {
    Value::Array(t.iter().map(|(pid, r)| json!([number(*pid), Value::Object(row_json(r))])).collect())
}

fn strings_json(t: &PidMap<String>) -> Value {
    Value::Array(t.iter().map(|(pid, s)| json!([number(*pid), s])).collect())
}

#[napi(js_name = "procsAgentOf")]
pub fn agent_of(program: String, args: String) -> Option<&'static str> {
    procs::agent_of(&program, &args)
}

#[napi(js_name = "procsWantsArgs")]
pub fn wants_args(program: String) -> bool {
    procs::wants_args(&program)
}

#[napi(js_name = "procsProductOf")]
pub fn product_of(program: String, args: String) -> Json {
    Json(product_json(procs::product_of(&program, &args)))
}

#[napi(js_name = "procsParseTexts")]
pub fn parse_texts(text: String) -> Json {
    Json(strings_json(&procs::parse_texts(&text)))
}

#[napi(js_name = "procsParseCwd")]
pub fn parse_cwd(text: String) -> Option<String> {
    procs::parse_cwd(&text)
}

#[napi(js_name = "procsParseCwds")]
pub fn parse_cwds(text: String) -> Json {
    Json(strings_json(&procs::parse_cwds(&text)))
}

#[napi(js_name = "procsParseCommands")]
pub fn parse_commands(text: String) -> Json {
    Json(strings_json(&procs::parse_commands(&text)))
}

#[napi(js_name = "procsParsePs")]
pub fn parse_ps(text: String) -> Json {
    Json(table_json(&procs::parse_ps(&text)))
}

#[napi(js_name = "procsParseNetstat")]
pub fn parse_netstat(text: String, local_port: MaybeJson, remote_port: MaybeJson) -> Option<f64> {
    procs::parse_netstat(&text, &xenonet_engine::js::template(local_port.0.as_ref()), &xenonet_engine::js::template(remote_port.0.as_ref()))
}

#[napi(js_name = "procsSelfTest")]
pub fn self_test(me: f64, peer: Option<f64>, first: MaybeJson) -> Json {
    let first = first.0.filter(Value::is_object);
    let at = first.as_ref().map(|f| (f.get("pid").and_then(Value::as_f64).unwrap_or(f64::NAN), f.get("exe")));
    let (peer, chain, exe) = procs::self_test(me, peer, at);
    Json(json!({ "supported": true, "peer": peer, "chain": chain, "exe": exe }))
}

/// The state core/procs.js's createProcs keeps, and its lookups.
#[napi]
pub struct ProcsCore {
    inner: Arc<Procs>,
}

fn run<F>(p: &Arc<Procs>, f: F) -> Machine
where
    F: FnOnce(&Procs, &mut Bridged<'_>) -> Result<Jv, String> + Send + 'static,
{
    let p = p.clone();
    start(move |asker| f(&p, &mut Bridged(asker)).map_err(Into::into))
}

#[napi]
impl ProcsCore {
    #[napi(constructor)]
    pub fn new(platform: String) -> Self {
        ProcsCore { inner: Arc::new(Procs::new(&platform)) }
    }

    #[napi(getter)]
    pub fn supported(&self) -> bool {
        self.inner.supported
    }

    /// The table, as [pid, row] pairs.
    #[napi]
    pub fn snapshot(&self) -> Machine {
        run(&self.inner, |p, io| Ok(Jv::Json(table_json(&*p.snapshot(io)?))))
    }

    /// {pid, keep}: the pid, and whether the twin keeps it on the socket.
    #[napi]
    pub fn peer(&self, local_port: MaybeJson, remote_port: MaybeJson, kept: MaybeJson) -> Machine {
        run(&self.inner, move |p, io| {
            let (pid, keep) = p.peer(io, local_port.0.as_ref(), remote_port.0.as_ref(), kept.0.as_ref());
            Ok(Jv::Obj(vec![("pid", Jv::Json(pid)), ("keep", Jv::Bool(keep))]))
        })
    }

    #[napi]
    pub fn chain(&self, pid: MaybeJson, again: bool, exe: bool) -> Machine {
        let pid = pid.0.as_ref().and_then(Value::as_f64);
        run(&self.inner, move |p, io| Ok(Jv::Json(Value::Array(p.chain(io, pid, again, exe)?.iter().map(link_json).collect()))))
    }

    #[napi]
    pub fn cwd(&self, pid: MaybeJson) -> Machine {
        run(&self.inner, move |p, io| Ok(Jv::Json(p.cwd(io, pid.0.as_ref()).map_or(Value::Null, Value::String))))
    }

    #[napi]
    pub fn alive(&self, pid: MaybeJson, start: MaybeJson) -> Machine {
        run(&self.inner, move |p, io| Ok(Jv::Bool(p.alive(io, pid.0.as_ref(), start.0.as_ref())?)))
    }

    /// The table, or null where XenoNet can't look.
    #[napi]
    pub fn census(&self) -> Machine {
        run(&self.inner, |p, io| Ok(Jv::Json(p.census(io)?.map_or(Value::Null, |t| table_json(&t)))))
    }

    /// What the census says a row is.
    #[napi]
    pub fn of(&self, row: MaybeJson) -> Json {
        let row = row.0.filter(|r| xenonet_engine::js::truthy(Some(r))).unwrap_or(Value::Null);
        if row.is_null() {
            return Json(json!({ "agent": null, "product": null }));
        }
        let (agent, product) = self.inner.of(row.get("pid"), row.get("start"), row.get("program"));
        Json(json!({ "agent": agent, "product": product_json(product) }))
    }

    #[napi]
    pub fn children(&self, pids: Json) -> Machine {
        let pids = list(pids.0);
        run(&self.inner, move |p, io| Ok(Jv::Json(Value::Array(p.children(io, &pids).into_iter().map(number).collect()))))
    }

    #[napi]
    pub fn describe(&self, pids: Json) -> Machine {
        let pids = list(pids.0);
        run(&self.inner, move |p, io| Ok(Jv::Json(table_json(&p.describe(io, &pids)))))
    }

    #[napi]
    pub fn cwds(&self, pids: Json) -> Machine {
        let pids = list(pids.0);
        run(&self.inner, move |p, io| Ok(Jv::Json(strings_json(&p.cwds(io, &pids)))))
    }

    /// `p`: the process as a chain has it (its pid, start, program, agent
    /// and product).
    #[napi]
    pub fn signed(&self, p: MaybeJson) -> Machine {
        let p = p.0.unwrap_or(Value::Null);
        run(&self.inner, move |procs, io| {
            let agent = p.get("agent").and_then(Value::as_str);
            let product =
                p.get("product").filter(|v| v.is_object()).map(|v| (v.get("product").and_then(Value::as_str).unwrap_or(""), v.get("role").and_then(Value::as_str).unwrap_or("")));
            Ok(Jv::Bool(procs.signed(io, p.get("pid"), p.get("start"), p.get("program"), agent, product)))
        })
    }
}

fn list(v: Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a,
        _ => Vec::new(),
    }
}
