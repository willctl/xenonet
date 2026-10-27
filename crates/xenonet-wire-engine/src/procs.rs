//! The process lookups (core/procs.js over crates/xenonet-engine/src/procs.rs)
//! on this machine: the commands Rust asks for (ps, netstat, lsof,
//! codesign, plutil, pgrep) run here, with start times in English and UTC
//! and text as UTF-8 whatever XenoNet's own environment says, five seconds
//! each at most. A command asked for while the same one runs waits for
//! that one's answer, as the JS's shared promises did.

use crate::exec::{self, Spec};
use crate::js::{alive, at, no_method, now_ms, number, obj, Locked};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use xenonet_engine::peer::{Result, Thrown};
use xenonet_engine::procs::{Call, Failure, Io, Link, PidMap, Procs, Product, Row};
use xenonet_engine::sql::Arg;

pub type Answer = std::result::Result<String, Failure>;

/// A run under way, which others asking for the same command wait on.
#[derive(Default)]
struct Slot {
    done: Mutex<Option<Answer>>,
    ready: Condvar,
}

/// The machine's process table and what's read of it, shared by the
/// verifier, the observers and delivery.
pub struct System {
    pub procs: Procs,
    running: Mutex<HashMap<String, Arc<Slot>>>,
    /// What runs a command: the system, or a test's made-up machine.
    runner: Runner,
}

/// A command's answer, from the machine or a test's stand-in for it.
pub type Runner = Box<dyn Fn(&Call) -> Answer + Send + Sync>;

fn spec_of(c: &Call) -> Spec {
    let mut s = Spec::new(&c.file, &c.args, 5000);
    s.env = vec![("LC_TIME".into(), Some("C".into())), ("LC_CTYPE".into(), Some("UTF-8".into())), ("TZ".into(), Some("UTC".into())), ("LC_ALL".into(), None)];
    s
}

fn answer(r: std::result::Result<String, exec::Failed>) -> Answer {
    r.map_err(|f| Failure { message: f.message, stdout: Some(f.stdout) })
}

impl System {
    pub fn new() -> Arc<System> {
        // procs' own calls on Linux and Windows are read from the system
        // itself; the rest are commands.
        System::with(crate::js::platform(), Box::new(|c| xenonet_engine::procs_native::answer(c).unwrap_or_else(|| answer(exec::run(&spec_of(c))))))
    }

    /// The lookups with commands run by `runner`, as if on `platform`.
    pub fn with(platform: &str, runner: Runner) -> Arc<System> {
        Arc::new(System { procs: Procs::new(platform), running: Mutex::new(HashMap::new()), runner })
    }

    fn run(&self, c: &Call) -> Answer {
        let Some(key) = c.share.clone() else { return (self.runner)(c) };
        let (slot, mine) = {
            let mut r = self.running.locked();
            match r.get(&key) {
                Some(s) => (s.clone(), false),
                None => {
                    let s = Arc::new(Slot::default());
                    r.insert(key.clone(), s.clone());
                    (s, true)
                }
            }
        };
        if !mine {
            let mut done = slot.done.locked();
            loop {
                if let Some(answer) = done.clone() {
                    return answer;
                }
                done = slot.ready.wait(done).unwrap_or_else(std::sync::PoisonError::into_inner);
            }
        }
        let out = (self.runner)(c);
        *slot.done.locked() = Some(out.clone());
        slot.ready.notify_all();
        self.running.locked().remove(&key);
        out
    }

    /// The commands and the clock, for one lookup.
    pub fn io(&self) -> SystemIo<'_> {
        SystemIo(self)
    }
}

pub struct SystemIo<'a>(&'a System);

impl Io for SystemIo<'_> {
    fn exec(&mut self, call: Call) -> Answer {
        self.0.run(&call)
    }
    fn exec_all(&mut self, calls: Vec<Call>) -> Vec<Answer> {
        if calls.len() <= 1 {
            return calls.into_iter().map(|c| self.0.run(&c)).collect();
        }
        let sys = self.0;
        std::thread::scope(|s| {
            let hs: Vec<_> = calls.iter().map(|c| s.spawn(move || sys.run(c))).collect();
            hs.into_iter().map(|h| h.join().unwrap_or_else(|_| Err(Failure { message: "the command's thread failed".into(), stdout: None }))).collect()
        })
    }
    fn now(&mut self) -> f64 {
        now_ms()
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

/// A process of a chain, as core/procs.js's chain() gives it.
pub fn link_json(l: &Link) -> Value {
    let mut o = row_json(&l.row);
    if let Some(exe) = &l.exe {
        o.insert("exe".into(), exe.clone().map_or(Value::Null, Value::String));
    }
    o.insert("agent".into(), l.agent.map_or(Value::Null, |a| Value::String(a.into())));
    o.insert("product".into(), product_json(l.product));
    Value::Object(o)
}

/// A table as a Map's [pid, row] pairs.
pub fn table_json(t: &PidMap<Row>) -> Value {
    Value::Array(t.iter().map(|(pid, r)| json!([number(*pid), Value::Object(row_json(r))])).collect())
}

fn strings_json(t: &PidMap<String>) -> Value {
    Value::Array(t.iter().map(|(pid, s)| json!([number(*pid), s])).collect())
}

fn values(a: Arg) -> Vec<Value> {
    crate::js::list(a).iter().map(Arg::to_json).collect()
}

fn failed(m: String) -> Thrown {
    Thrown::new(m)
}

/// What a census reads, as readCensus gives it to an observer: the
/// table's [pid, row] pairs and what each process is.
pub fn census(sys: &System) -> Result<Value> {
    let t = sys.procs.census(&mut sys.io()).map_err(failed)?;
    let Some(t) = t else { return Ok(json!({ "table": [], "of": [] })) };
    let of: Vec<Value> = t
        .iter()
        .map(|(_, r)| {
            let (agent, product) = sys.procs.of(Some(&number(r.pid)), Some(&number(r.start)), Some(&Value::String(r.program.clone())));
            json!({ "agent": agent, "product": product_json(product) })
        })
        .collect();
    Ok(json!({ "table": table_json(&t), "of": of }))
}

/// The process on the other end of a connection, by its ports, or null.
pub fn peer_of(sys: &System, local_port: u16, remote_port: u16) -> Value {
    sys.procs.peer(&mut sys.io(), Some(&Value::from(local_port)), Some(&Value::from(remote_port)), None).0
}

/// `procs.<method>(...args)`, as createProcs's object answers it.
pub fn call(sys: &System, method: &str, args: &[Arg]) -> Result<Arg> {
    let p = &sys.procs;
    let io = &mut sys.io();
    let v = |a: Arg| a.to_json();
    let out: Value = match method {
        "snapshot" => table_json(&*p.snapshot(io).map_err(failed)?),
        "chain" => {
            let o = at(args, 1);
            // { again = true, exe = false }: a default only for undefined.
            let again = match o.field("again") {
                Arg::Undef => true,
                a => a.truthy(),
            };
            let exe = o.field("exe").truthy();
            let pid = at(args, 0).as_f64();
            Value::Array(p.chain(io, pid, again, exe).map_err(failed)?.iter().map(link_json).collect())
        }
        "alive" => {
            let o = at(args, 0);
            Value::Bool(p.alive(io, Some(&v(o.field("pid"))), Some(&v(o.field("start")))).map_err(failed)?)
        }
        "signed" => {
            let o = v(at(args, 0));
            let agent = o.get("agent").and_then(Value::as_str);
            let product =
                o.get("product").filter(|x| x.is_object()).map(|x| (x.get("product").and_then(Value::as_str).unwrap_or(""), x.get("role").and_then(Value::as_str).unwrap_or("")));
            Value::Bool(p.signed(io, o.get("pid"), o.get("start"), o.get("program"), agent, product))
        }
        "cwd" => p.cwd(io, Some(&v(at(args, 0)))).map_or(Value::Null, Value::String),
        "census" => census(sys)?,
        "children" => Value::Array(p.children(io, &values(at(args, 0))).into_iter().map(number).collect()),
        "describe" => table_json(&p.describe(io, &values(at(args, 0)))),
        "cwds" => strings_json(&p.cwds(io, &values(at(args, 0)))),
        "of" => {
            let r = v(at(args, 0));
            let (agent, product) = p.of(r.get("pid"), r.get("start"), r.get("program"));
            json!({ "agent": agent, "product": product_json(product) })
        }
        _ => return Err(no_method("procs", method)),
    };
    Ok(crate::js::arg(&out))
}

/// `procs.<path>` read: its `supported` and `chains`, and its calls.
pub fn get(sys: &System, field: &str) -> Arg {
    match field {
        "supported" => Arg::Bool(sys.procs.supported),
        "chains" => Arg::Bool(sys.procs.chains),
        "" => obj(vec![("supported", Arg::Bool(sys.procs.supported)), ("chains", Arg::Bool(sys.procs.chains))]),
        _ => Arg::Undef,
    }
}

pub fn kind(field: &str) -> &'static str {
    match field {
        "" => "object",
        "supported" | "chains" => "boolean",
        "snapshot" | "chain" | "alive" | "signed" | "cwd" | "census" | "children" | "describe" | "cwds" | "of" | "peer" => "function",
        _ => "undefined",
    }
}

/// Whether a process runs, as the twins' `alive` asks.
pub fn running(pid: &Arg) -> bool {
    pid.as_f64().is_some_and(alive)
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    #[test]
    fn this_process_and_the_ones_above_it() {
        let sys = System::new();
        let me = f64::from(std::process::id());
        let chain = crate::js::list(call(&sys, "chain", &[Arg::Num(me), obj(vec![("exe", Arg::Bool(true))])]).unwrap());
        assert_eq!(chain[0].field("pid"), Arg::Num(me));
        assert!(chain[0].field("exe").as_str().is_some(), "{chain:?}");
        let first = chain[0].clone();
        assert_eq!(call(&sys, "alive", &[first]).unwrap(), Arg::Bool(true));
        assert!(call(&sys, "cwd", &[Arg::Num(me)]).unwrap().as_str().is_some());
        assert!(matches!(get(&sys, "supported"), Arg::Bool(true)));
    }
}
