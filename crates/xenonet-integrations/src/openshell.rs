//! core/integrations/openshell.js: agents in OpenShell sandboxes. Every
//! command is argv, never shell code built from input: the task stays a
//! literal argument, a project goes up only when you confirm it, and a
//! sandbox is never retried or deleted behind your back. The commands run
//! through the twin, and so do the records and the reviews.

use crate::b64;
use crate::common::{clean, fail, failed, has_credential, sandbox_gone, text, Fail, Tickets};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::rc::Rc;
use xenonet_app::fsx;
use xenonet_app::jsv::truthy;
use xenonet_app::nodepath::NATIVE;
use xenonet_app::task::{Failure, Io};
use xenonet_base::text::js_string;

fn word(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
}

/// wsl.exe, where Windows keeps it.
pub fn wsl_file(system_root: Option<&str>) -> String {
    NATIVE.join(&[system_root.unwrap_or("C:\\Windows"), "System32", "wsl.exe"])
}

/// The WSL distributions `wsl --list --quiet` printed, as profiles.
pub fn wsl_profiles(stdout: &str) -> Value {
    let s = stdout.replace('\0', "");
    Value::Array(
        s.split('\n')
            .map(|l| xenonet_base::text::js_trim(l.strip_suffix('\r').unwrap_or(l)))
            .filter(|d| word(d) && !d.starts_with("docker-desktop"))
            .map(|d| json!({ "backend": "wsl", "distro": d }))
            .collect(),
    )
}

/// How an OpenShell command runs for a profile: { file, args }.
pub fn command(profile: &Value, args: &[Value], platform: &str, system_root: Option<&str>) -> Result<Value, Fail> {
    let backend = profile.get("backend").filter(|v| !v.is_null());
    if backend.and_then(Value::as_str) == Some("wsl") {
        if platform != "win32" {
            return fail("WSL profiles are available on Windows only.");
        }
        let distro = text(profile.get("distro"), "WSL distribution", 80)?;
        if !word(&distro) {
            return fail("Use the installed WSL distribution name.");
        }
        let mut a = vec![json!("--distribution"), json!(distro), json!("--exec"), json!("openshell")];
        a.extend(args.iter().cloned());
        return Ok(json!({ "file": wsl_file(system_root), "args": a }));
    }
    if backend.is_some_and(|b| truthy(Some(b)) && b.as_str() != Some("native")) {
        return fail("Choose native or WSL execution.");
    }
    if platform == "win32" {
        return failed("OpenShell needs a WSL distribution on Windows. Native Windows execution is unavailable.", "unavailable", 409);
    }
    Ok(json!({ "file": "openshell", "args": args }))
}

fn root_of(dir: &str) -> bool {
    if cfg!(windows) {
        let d = dir.trim_end_matches(['\\', '/']);
        d.len() <= 2 || dir == "\\" || dir == "/"
    } else {
        dir == "/"
    }
}

/// A project folder, resolved: a directory, and not a filesystem's root.
pub fn source_dir(value: Option<&Value>) -> Result<String, Fail> {
    let dir = fsx::realpath(&text(value, "Project folder", 1024)?)?;
    let is_dir = std::fs::metadata(&dir).map_err(|e| fsx::FsError::of(e, "stat", &dir))?.is_dir();
    if !is_dir || root_of(&dir) {
        return fail("Choose a project directory, not a filesystem root.");
    }
    Ok(dir)
}

/// A policy file, resolved and read: { file, content, hash }, or null.
pub fn policy_file(value: Option<&Value>) -> Result<Value, Fail> {
    if !truthy(value) {
        return Ok(Value::Null);
    }
    let file = fsx::realpath(&text(value, "Policy file", 1024)?)?;
    let st = std::fs::metadata(&file).map_err(|e| fsx::FsError::of(e, "stat", &file))?;
    if !st.is_file() || st.len() > 256 * 1024 {
        return fail("Choose a policy file under 256 KiB.");
    }
    let content = fsx::read_text(&file)?;
    let hash = b64::sha256_hex(content.as_bytes());
    Ok(json!({ "file": file, "content": content, "hash": hash }))
}

/// One OpenShell integration: its tickets, where it runs, and the program
/// that collects text inside a sandbox.
pub struct OpenShell {
    pub tickets: Tickets,
    pub platform: String,
    pub system_root: Option<String>,
    pub collect_text: String,
}

pub type Shared = Rc<RefCell<OpenShell>>;

fn failure(f: Fail) -> Failure {
    if f.status == 0 {
        let mut m = Map::new();
        if !f.code.is_empty() {
            m.insert("code".into(), json!(f.code));
        }
        return Failure::with(f.message, m);
    }
    let mut m = Map::new();
    m.insert("status".into(), json!(f.status));
    m.insert("code".into(), json!(f.code));
    m.insert("integration".into(), json!(true));
    Failure::with(f.message, m)
}

async fn ask(io: &Io, q: Value) -> Result<Value, Failure> {
    io.ask(q).await.map_err(|_| Failure::Rethrow)
}

async fn now(io: &Io) -> Result<f64, Failure> {
    ask(io, json!({ "op": "now" })).await.map(|v| xenonet_base::text::js_number(Some(&v)))
}

/// Why a command didn't answer: what the twin's run threw (its message,
/// code and stderr), or a refusal of XenoNet's own.
enum CallErr {
    Thrown(Value),
    Own(Fail),
}

impl CallErr {
    fn code(&self) -> String {
        match self {
            CallErr::Thrown(e) => e.get("code").and_then(Value::as_str).unwrap_or("").to_string(),
            CallErr::Own(f) => f.code.clone(),
        }
    }
    fn stderr(&self) -> String {
        match self {
            CallErr::Thrown(e) => e.get("stderr").filter(|v| !v.is_null()).map_or_else(String::new, js_string),
            CallErr::Own(_) => String::new(),
        }
    }
    // Thrown on as the JS let it through.
    fn thrown(self) -> Failure {
        match self {
            CallErr::Thrown(_) => Failure::Rethrow,
            CallErr::Own(f) => failure(f),
        }
    }
}

async fn call(io: &Io, s: &Shared, profile: &Value, args: &[Value], options: Value) -> Result<Value, CallErr> {
    let c = {
        let o = s.borrow();
        command(profile, args, &o.platform, o.system_root.as_deref())
    };
    let c = c.map_err(CallErr::Own)?;
    io.ask(json!({ "op": "run", "file": c["file"], "args": c["args"], "options": options })).await.map_err(CallErr::Thrown)
}

async fn asked(io: &Io, q: Value) -> Result<Value, CallErr> {
    io.ask(q).await.map_err(CallErr::Thrown)
}

fn parsed(text: &str) -> Result<Value, CallErr> {
    xenonet_base::json::parse(text).map_err(|e| CallErr::Own(Fail { status: 0, code: String::new(), message: e.message }))
}

fn strs(v: &[&str]) -> Vec<Value> {
    v.iter().map(|s| json!(s)).collect()
}

async fn linux_path(io: &Io, s: &Shared, profile: &Value, file: &str) -> Result<String, Failure> {
    if profile.get("backend").and_then(Value::as_str) != Some("wsl") {
        return Ok(file.to_string());
    }
    let c = {
        let o = s.borrow();
        command(profile, &[], &o.platform, o.system_root.as_deref()).map_err(failure)?
    };
    let mut args: Vec<Value> = c["args"].as_array().cloned().unwrap_or_default();
    args.pop();
    args.extend(strs(&["wslpath", "-a"]));
    args.push(json!(file));
    let out = ask(io, json!({ "op": "run", "file": c["file"], "args": args })).await?;
    let translated = xenonet_base::text::js_trim(&js_string(&out["stdout"])).to_string();
    if !translated.starts_with('/') || translated.contains(['\r', '\n', '\0', ':']) {
        return Err(failure(Fail { status: 400, code: "bad-request".into(), message: "WSL could not translate this project path.".into() }));
    }
    Ok(translated)
}

async fn job(io: &Io, id: &Value) -> Result<Value, Failure> {
    let list = ask(io, json!({ "op": "records.list" })).await?;
    list.as_array()
        .into_iter()
        .flatten()
        .find(|r| xenonet_app::jsv::strict_eq(r.get("id"), Some(id)) && r.get("kind").and_then(Value::as_str) == Some("sandbox"))
        .cloned()
        .ok_or_else(|| failure(Fail { status: 404, code: "missing".into(), message: "No such sandbox job.".into() }))
}

async fn update(io: &Io, id: &Value, fields: Value) -> Result<Value, Failure> {
    ask(io, json!({ "op": "records.update", "id": id, "fields": fields })).await
}

/// status(profile): whether OpenShell and its gateway answer.
pub async fn status(io: Io, s: Shared, profile: Value) -> Result<Value, Failure> {
    let backend = profile.get("backend").cloned().unwrap_or(Value::Null);
    let experimental = backend.as_str() == Some("wsl");
    let version = call(&io, &s, &profile, &strs(&["--version"]), Value::Null).await;
    let listed = match &version {
        Ok(_) => call(&io, &s, &profile, &strs(&["sandbox", "list", "-o", "json"]), Value::Null).await.is_ok(),
        Err(_) => false,
    };
    Ok(match (version, listed) {
        (Ok(v), true) => json!({ "ready": true, "version": clean(v.get("stdout"), 160), "backend": backend, "experimental": experimental }),
        _ => {
            json!({ "ready": false, "backend": backend, "experimental": experimental, "reason": "OpenShell or its active gateway is unavailable. Configure the gateway in this execution environment, then check again." })
        }
    })
}

/// preview(body): what a launch would run, step by step, and its ticket.
pub async fn preview(io: Io, s: Shared, body: Value) -> Result<Value, Failure> {
    let t = now(&io).await?;
    s.borrow_mut().tickets.check(t).map_err(failure)?;
    let platform = s.borrow().platform.clone();
    let profile = json!({
        "backend": body.get("backend").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!(if platform == "win32" { "wsl" } else { "native" })),
        "distro": body.get("distro").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| json!("")),
    });
    {
        let o = s.borrow();
        command(&profile, &[], &o.platform, o.system_root.as_deref()).map_err(failure)?;
    }
    let f = |m: &str| failure(Fail { status: 400, code: "bad-request".into(), message: m.into() });
    let agent =
        body.get("agent").and_then(Value::as_str).filter(|a| ["pi", "codex", "claude"].contains(a)).map(String::from).ok_or_else(|| f("Choose Pi, Codex or Claude Code."))?;
    let request = text(body.get("request"), "Task", 2000).map_err(failure)?;
    if has_credential(&request) {
        return Err(f("Attach credentials through an OpenShell provider, not the task."));
    }
    let image = text(body.get("image"), "Image", 240).map_err(failure)?;
    if !image.bytes().all(|c| c.is_ascii_alphanumeric() || b"_./:@-".contains(&c)) || image.starts_with('-') {
        return Err(f("Use a container image reference."));
    }
    let provider = if truthy(body.get("provider")) { text(body.get("provider"), "Provider", 80).map_err(failure)? } else { String::new() };
    if !provider.is_empty() && !(provider.as_bytes()[0].is_ascii_alphanumeric() && provider.bytes().all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))) {
        return Err(f("Use an existing OpenShell provider name."));
    }
    let source = if truthy(body.get("source")) { Some(source_dir(body.get("source")).map_err(failure)?) } else { None };
    if source.is_some() && body.get("upload") != Some(&json!(true)) {
        return Err(f("Confirm the project upload before previewing."));
    }
    let review = body.get("review") == Some(&json!(true));
    if review && source.is_none() {
        return Err(f("Choose a Git project for text change review."));
    }
    let policy = policy_file(body.get("policy")).map_err(failure)?;
    let baseline = if review { Some(ask(&io, json!({ "op": "reviews.prepare", "source": source })).await?) } else { None };
    // OpenShell 0.1.4 limits sandbox names to 19 characters.
    let name = format!("xeno-{}", &b64::uuid().replace('-', "")[..14]);
    let mut args: Vec<Value> = strs(&["sandbox", "create", "--name", &name, "--from", &image, "--label", &format!("xenonet-job={name}"), "--detach"]);
    if !provider.is_empty() {
        args.extend(strs(&["--provider", &provider]));
    }
    if !policy.is_null() {
        let p = linux_path(&io, &s, &profile, &js_string(&policy["file"])).await?;
        args.extend(strs(&["--policy", &p]));
    }
    let mut agent_args = vec![json!(agent)];
    agent_args.extend(strs(match agent.as_str() {
        "codex" => &["exec"],
        "claude" => &["-p"],
        _ => &["--print"],
    }));
    agent_args.push(json!(request));
    let collect = s.borrow().collect_text.clone();
    let mut rest: Vec<Vec<Value>> = Vec::new();
    if let Some(source) = &source {
        // OpenShell 0.1.4 rejects create --upload together with a command:
        // the agent waits for the separate upload, and the task stays argv.
        let ready = format!("/tmp/{name}.ready");
        let workdir = if baseline.is_some() { "/workspace/project".to_string() } else { format!("/workspace/project/{}", NATIVE.basename(source, None)) };
        let cmd = if baseline.is_some() {
            r#"while [ ! -e "$1" ]; do sleep 1; done; shift; cd "$1" || exit 1; shift; "$@" < /dev/null; while :; do sleep 60; done"#
        } else {
            r#"while [ ! -e "$1" ]; do sleep 1; done; shift; cd "$1" || exit 1; shift; exec "$@" < /dev/null"#
        };
        args.extend(strs(&["--", "/bin/sh", "-c", cmd, "xenonet-upload", &ready, &workdir]));
        args.extend(agent_args.iter().cloned());
        rest.push(strs(&["sandbox", "exec", &name, "--no-login-shell", "--", "/bin/sh", "-c", "test ! -e /workspace/project"]));
        if baseline.is_some() {
            rest.push(strs(&["sandbox", "exec", &name, "--no-login-shell", "--", "/usr/local/bin/node", "--version"]));
        }
        let from = baseline.as_ref().map_or_else(|| source.clone(), |b| js_string(&b["upload"]));
        let up = linux_path(&io, &s, &profile, &from).await?;
        let mut upload = strs(&["sandbox", "upload"]);
        if baseline.is_some() {
            upload.push(json!("--no-git-ignore"));
        }
        upload.extend(strs(&[&name, &up, if baseline.is_some() { "/workspace" } else { "/workspace/project" }]));
        rest.push(upload);
        if baseline.is_some() {
            rest.push(strs(&["sandbox", "exec", &name, "--no-login-shell", "--timeout", "30", "--", "/usr/local/bin/node", "-e", &collect]));
        }
        rest.push(strs(&["sandbox", "exec", &name, "--no-login-shell", "--", "/bin/sh", "-c", ": > \"$1\"", "xenonet-start", &ready]));
    } else {
        // Detached print-mode agents can wait for piped stdin to end.
        args.extend(strs(&["--", "/bin/sh", "-c", "exec \"$@\" < /dev/null", "xenonet-print"]));
        args.extend(agent_args.iter().cloned());
    }
    let mut steps = vec![Value::Array(args)];
    steps.extend(rest.into_iter().map(Value::Array));
    let review_id = baseline.as_ref().map_or(Value::Null, |b| b["id"].clone());
    let value = json!({ "profile": profile, "name": name, "image": image, "agent": agent, "provider": provider, "source": source, "policy": policy, "steps": steps, "reviewId": review_id, "taskHash": b64::sha256_hex(request.as_bytes()) });
    let commands: Vec<Value> = {
        let o = s.borrow();
        steps.iter().map(|st| command(&profile, st.as_array().map_or(&[][..], Vec::as_slice), &o.platform, o.system_root.as_deref())).collect::<Result<_, _>>().map_err(failure)?
    };
    let scope = baseline.as_ref().map(|b| {
        let mut m: Map<String, Value> = b.as_object().cloned().unwrap_or_default();
        m.shift_remove("upload");
        Value::Object(m)
    });
    let t = now(&io).await?;
    let issued = match s.borrow_mut().tickets.issue(value, t) {
        Ok(i) => i,
        Err(e) => {
            if baseline.is_some() {
                let _ = ask(&io, json!({ "op": "reviews.forget", "id": review_id })).await;
            }
            return Err(failure(e));
        }
    };
    let review_note = if baseline.is_some() {
        "Text review uploads only the frozen manifest below. The image needs /usr/local/bin/node. Its sandbox stays running after the agent exits until you delete it. A local baseline and retrieved changes stay on this computer until you forget the local review. "
    } else {
        ""
    };
    let mut out: Map<String, Value> = issued.as_object().cloned().unwrap_or_default();
    out.insert("command".into(), commands[0].clone());
    out.insert("commands".into(), Value::Array(commands));
    out.insert("baseline".into(), scope.unwrap_or(Value::Null));
    out.insert("policy".into(), policy.get("content").cloned().unwrap_or(Value::Null));
    out.insert("source".into(), json!(source));
    out.insert("note".into(), json!(format!("The selected image must contain the agent and /bin/sh. Print-mode stdin is closed. Uploads also need sleep and an unused /workspace/project path. Steps run in order; the agent waits for upload and starts in /workspace/project. {review_note}Existing providers can add network access. Inspect and delete the sandbox when finished.")));
    Ok(Value::Object(out))
}

/// launch(ticket): the previewed steps, in order, with the job recorded.
pub async fn launch(io: Io, s: Shared, ticket: Value) -> Result<Value, Failure> {
    let list = ask(&io, json!({ "op": "records.list" })).await?;
    let live = list.as_array().into_iter().flatten().filter(|r| r.get("kind").and_then(Value::as_str) == Some("sandbox") && !sandbox_gone(r)).count();
    if live >= 64 {
        return Err(failure(Fail { status: 409, code: "busy".into(), message: "Delete or reconcile an old sandbox before launching another (64 maximum).".into() }));
    }
    let t = now(&io).await?;
    let value = s.borrow_mut().tickets.take(&js_string(&ticket), t).map_err(failure)?;
    let changed = |m: &str| failure(Fail { status: 409, code: "changed".into(), message: m.into() });
    if truthy(value.get("source")) && source_dir(value.get("source")).map_err(failure)? != js_string(&value["source"]) {
        return Err(changed("The project folder changed. Preview again."));
    }
    if truthy(value.get("policy")) {
        let now_policy = policy_file(value["policy"].get("file")).map_err(failure)?;
        if !xenonet_app::jsv::strict_eq(now_policy.get("hash"), value["policy"].get("hash")) {
            return Err(changed("The policy changed. Preview again."));
        }
    }
    if truthy(value.get("reviewId")) {
        ask(&io, json!({ "op": "reviews.launch", "id": value["reviewId"] })).await?;
    }
    let steps: Vec<Value> = value["steps"].as_array().cloned().unwrap_or_default();
    let provider = value.get("provider").filter(|p| truthy(Some(p))).cloned().unwrap_or(Value::Null);
    let policy_hash = value.get("policy").and_then(|p| p.get("hash")).filter(|h| !h.is_null()).cloned().unwrap_or(Value::Null);
    let record = ask(&io, json!({ "op": "records.add", "value": { "kind": "sandbox", "status": "creating", "name": value["name"], "profile": value["profile"], "image": value["image"], "agent": value["agent"], "provider": provider, "source": value["source"], "policyHash": policy_hash, "taskHash": value["taskHash"], "reviewId": value["reviewId"], "execution": "unknown", "launchSteps": steps.len(), "completedSteps": 0 } })).await?;
    let collect = s.borrow().collect_text.clone();
    let mut completed = 0;
    let run = async {
        for (i, step) in steps.iter().enumerate() {
            let args = step.as_array().cloned().unwrap_or_default();
            let collecting = args.iter().any(|a| a.as_str() == Some(collect.as_str()));
            let options = if collecting { json!({ "timeout": 120000, "maxBuffer": 8 * 1024 * 1024 }) } else { json!({ "timeout": 120000 }) };
            let response = call(&io, &s, &value["profile"], &args, options).await?;
            if truthy(value.get("reviewId")) && collecting {
                let got = parsed(&js_string(&response["stdout"]))?;
                asked(&io, json!({ "op": "reviews.verifyUploaded", "id": value["reviewId"], "value": got })).await?;
            }
            completed = i + 1;
            asked(&io, json!({ "op": "records.update", "id": record["id"], "fields": { "completedSteps": completed } })).await?;
        }
        asked(&io, json!({ "op": "records.update", "id": record["id"], "fields": { "status": "created", "eventKind": "sandbox-created" } })).await
    };
    match run.await {
        Ok(r) => Ok(r),
        Err(err) => {
            // The command itself wasn't found, so nothing reached a gateway
            // and there's no sandbox to reconcile or delete.
            if err.code() == "ENOENT" && completed == 0 {
                return update(&io, &record["id"], json!({ "status": "not-created", "eventKind": "runtime-unavailable", "reason": "XenoNet couldn't find the openshell command, so no sandbox was made. Install OpenShell where XenoNet's service can run it, then check the gateway." })).await;
            }
            let stderr = err.stderr();
            let denied = stderr.match_indices("policy_denied").any(|(i, _)| {
                let before = stderr[..i].chars().next_back();
                let after = stderr[i + "policy_denied".len()..].chars().next();
                !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') && !after.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
            });
            // A lost CLI connection can leave a sandbox running. Never retry
            // or delete it implicitly; its unique name lets inspection
            // reconcile it.
            let (kind, reason) = if denied {
                ("execution-policy-denial", "OpenShell reported policy_denied. Inspect its effective policy.")
            } else {
                ("runtime-unavailable", "Creation did not complete locally. Inspect the gateway before retrying; the sandbox may exist.")
            };
            update(&io, &record["id"], json!({ "status": "creation-unknown", "eventKind": kind, "reason": reason })).await
        }
    }
}

/// inspect(id): the sandbox, its policy and recent logs, read now.
pub async fn inspect(io: Io, s: Shared, id: Value) -> Result<Value, Failure> {
    let value = job(&io, &id).await?;
    match value.get("status").and_then(Value::as_str) {
        Some("deleted") => return Err(failure(Fail { status: 404, code: "missing".into(), message: "This sandbox was deleted.".into() })),
        Some("not-created") => return Err(failure(Fail { status: 404, code: "missing".into(), message: "No sandbox was made for this job.".into() })),
        _ => {}
    }
    let name = js_string(&value["name"]);
    let profile = &value["profile"];
    let got = call(&io, &s, profile, &strs(&["sandbox", "get", &name, "--output", "json"]), Value::Null).await.map_err(CallErr::thrown)?;
    let sandbox = xenonet_base::json::parse(&js_string(&got["stdout"])).map_err(|e| Failure::with(e.message, Map::new()))?;
    let policy = call(&io, &s, profile, &strs(&["policy", "get", &name, "--full"]), Value::Null).await.map_err(CallErr::thrown)?;
    let logs = call(&io, &s, profile, &strs(&["logs", &name, "--since", "10m", "--source", "sandbox"]), Value::Null).await.map_err(CallErr::thrown)?;
    // A deletion still pending stays pending: seeing the sandbox doesn't undo the request.
    let status = if value.get("status").and_then(Value::as_str) == Some("deletion-requested") { value["status"].clone() } else { json!("observed") };
    let t = now(&io).await?;
    update(&io, &id, json!({ "status": status, "checkedAt": xenonet_base::json::number(t) })).await?;
    let phase = [sandbox.get("phase"), sandbox.get("sandbox").and_then(|x| x.get("phase")), sandbox.get("status").and_then(|x| x.get("phase"))]
        .into_iter()
        .flatten()
        .find(|v| !v.is_null())
        .cloned()
        .unwrap_or_else(|| json!("unknown"));
    // Logs are shown on request and never copied to the record store.
    Ok(
        json!({ "id": id, "sandbox": { "name": value["name"], "phase": clean(Some(&phase), 160) }, "policy": clean(policy.get("stdout"), 64000), "logs": clean(logs.get("stdout"), 64000), "audit": "A bounded snapshot of best-effort gateway logs; absence of an event does not establish that an operation was allowed." }),
    )
}

/// review(id): the job's local review, if it has one.
pub async fn review(io: Io, id: Value) -> Result<Value, Failure> {
    let value = job(&io, &id).await?;
    if !truthy(value.get("reviewId")) {
        return Ok(json!({ "available": false, "reason": "This job has no frozen text baseline. It supports inspection and deletion." }));
    }
    match io.ask(json!({ "op": "reviews.view", "id": value["reviewId"] })).await {
        Ok(v) => {
            let mut m = Map::new();
            m.insert("available".into(), json!(true));
            for (k, x) in v.as_object().into_iter().flatten() {
                m.insert(k.clone(), x.clone());
            }
            Ok(Value::Object(m))
        }
        Err(e) if e.get("code").and_then(Value::as_str) == Some("missing") => Ok(json!({ "available": false, "reason": "The local review was forgotten or is unavailable." })),
        Err(_) => Err(Failure::Rethrow),
    }
}

/// retrieve(id): the text the sandbox has now, reviewed against the
/// baseline.
pub async fn retrieve(io: Io, s: Shared, id: Value) -> Result<Value, Failure> {
    let value = job(&io, &id).await?;
    if !truthy(value.get("reviewId")) || sandbox_gone(&value) {
        return Err(failure(Fail { status: 409, code: "unavailable".into(), message: "This job has no retrievable text baseline.".into() }));
    }
    ask(&io, json!({ "op": "reviews.view", "id": value["reviewId"] })).await?;
    let collect = s.borrow().collect_text.clone();
    let name = js_string(&value["name"]);
    let got = async {
        let out = call(
            &io,
            &s,
            &value["profile"],
            &strs(&["sandbox", "exec", &name, "--no-login-shell", "--timeout", "30", "--", "/usr/local/bin/node", "-e", &collect]),
            json!({ "timeout": 45000, "maxBuffer": 8 * 1024 * 1024 }),
        )
        .await?;
        let got = parsed(&js_string(&out["stdout"]))?;
        let result = asked(&io, json!({ "op": "reviews.accept", "id": value["reviewId"], "value": got })).await?;
        let t = asked(&io, json!({ "op": "now" })).await?;
        let complete = truthy(result.get("result").and_then(|r| r.get("complete")));
        asked(&io, json!({ "op": "records.update", "id": id, "fields": { "retrieval": if complete { "retrieved" } else { "incomplete" }, "retrievedAt": t } })).await?;
        Ok::<Value, CallErr>(result)
    };
    match got.await {
        Ok(result) => {
            let mut m = Map::new();
            m.insert("available".into(), json!(true));
            for (k, x) in result.as_object().into_iter().flatten() {
                m.insert(k.clone(), x.clone());
            }
            Ok(Value::Object(m))
        }
        Err(err) => {
            let t = now(&io).await?;
            update(&io, &id, json!({ "retrieval": "unavailable", "retrievalCheckedAt": xenonet_base::json::number(t) })).await?;
            if ["retrieval-invalid", "review-too-large", "review-capacity"].contains(&err.code().as_str()) {
                return Err(err.thrown());
            }
            Err(failure(Fail {
                status: 502,
                code: "retrieval-unavailable".into(),
                message:
                    "Changes could not be retrieved. The sandbox may be stopped, unavailable, changing, or outside the text limits. An earlier local review, if any, is retained."
                        .into(),
            }))
        }
    }
}

/// The job's review id, or "This job has no local review."
pub async fn review_of(io: &Io, id: &Value) -> Result<Value, Failure> {
    let value = job(io, id).await?;
    if !truthy(value.get("reviewId")) {
        return Err(failure(Fail { status: 400, code: "bad-request".into(), message: "This job has no local review.".into() }));
    }
    Ok(value["reviewId"].clone())
}

pub async fn export_review(io: Io, id: Value, revision: Value) -> Result<Value, Failure> {
    let review = review_of(&io, &id).await?;
    ask(&io, json!({ "op": "reviews.export", "id": review, "revision": revision })).await
}

pub async fn forget_review(io: Io, id: Value) -> Result<Value, Failure> {
    let review = review_of(&io, &id).await?;
    let result = ask(&io, json!({ "op": "reviews.forget", "id": review })).await?;
    update(&io, &id, json!({ "reviewId": null, "retrieval": "forgotten" })).await?;
    Ok(result)
}

/// remove(id): asks for the sandbox's deletion, and records it deleted
/// only once the listing says it's gone.
pub async fn remove(io: Io, s: Shared, id: Value) -> Result<Value, Failure> {
    let value = job(&io, &id).await?;
    if sandbox_gone(&value) {
        return Ok(value);
    }
    let name = js_string(&value["name"]);
    let _ = call(&io, &s, &value["profile"], &strs(&["sandbox", "delete", &name]), json!({ "timeout": 60000 })).await;
    let listed = call(&io, &s, &value["profile"], &strs(&["sandbox", "list", "--selector", &format!("xenonet-job={name}"), "-o", "json"]), Value::Null).await;
    let gone = listed
        .ok()
        .and_then(|l| xenonet_base::json::parse(&js_string(&l["stdout"])).ok())
        .is_some_and(|l| l.get("sandboxes").and_then(Value::as_array).is_some_and(Vec::is_empty) && !truthy(l.get("next_page_token")));
    update(&io, &id, json!({ "status": if gone { "deleted" } else { "deletion-requested" }, "eventKind": if gone { "sandbox-deleted" } else { "sandbox-deletion-requested" } }))
        .await
}
