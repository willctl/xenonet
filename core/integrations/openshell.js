import fs from "node:fs";
import path from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createHash, randomUUID } from "node:crypto";
import { clean, fail, text, createTickets, sandboxGone } from "./common.js";
import { createReviews, COLLECT_TEXT } from "./reviews.js";

const execute = promisify(execFile);
export async function executionProfiles({ platform = process.platform, run = runCommand } = {}) {
  if (platform !== "win32") return [{ backend: "native", distro: "" }];
  try {
    const file = path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "wsl.exe");
    const { stdout } = await run(file, ["--list", "--quiet"]);
    return stdout.replace(/\0/g, "").split(/\r?\n/).map((s) => s.trim()).filter((s) => /^[\w.-]+$/.test(s) && !s.startsWith("docker-desktop")).map((distro) => ({ backend: "wsl", distro }));
  } catch { return []; }
}
export async function runCommand(file, args, { timeout = 15000, maxBuffer = 1024 * 1024 } = {}) {
  const result = execute(file, args, { timeout, maxBuffer, windowsHide: true, shell: false, encoding: "utf8", env: { ...process.env, NO_COLOR: "1" } });
  // None of these CLI operations accepts host input. OpenShell exec also
  // needs EOF on its own input pipe before it can finish a short command.
  result.child?.stdin?.end();
  return result;
}
export function openShellCommand(profile, args, platform = process.platform) {
  if (profile?.backend === "wsl") {
    if (platform !== "win32") fail("WSL profiles are available on Windows only.");
    const distro = text(profile.distro, "WSL distribution", 80);
    if (!/^[\w.-]+$/.test(distro)) fail("Use the installed WSL distribution name.");
    return { file: path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "wsl.exe"), args: ["--distribution", distro, "--exec", "openshell", ...args] };
  }
  if (profile?.backend && profile.backend !== "native") fail("Choose native or WSL execution.");
  if (platform === "win32") fail("OpenShell needs a WSL distribution on Windows. Native Windows execution is unavailable.", "unavailable", 409);
  return { file: "openshell", args };
}
const hash = (s) => createHash("sha256").update(s).digest("hex");
function sourceDir(value) {
  const dir = fs.realpathSync(text(value, "Project folder", 1024));
  if (!fs.statSync(dir).isDirectory() || dir === path.parse(dir).root) fail("Choose a project directory, not a filesystem root.");
  return dir;
}
function policyFile(value) {
  if (!value) return null;
  const file = fs.realpathSync(text(value, "Policy file", 1024));
  const st = fs.statSync(file);
  if (!st.isFile() || st.size > 256 * 1024) fail("Choose a policy file under 256 KiB.");
  const content = fs.readFileSync(file, "utf8");
  return { file, content, hash: hash(content) };
}
export function createOpenShell({ records, run = runCommand, platform = process.platform, now = Date.now, reviews = createReviews({ now }) }) {
  const tickets = createTickets({ now });
  const call = (profile, args, options) => { const c = openShellCommand(profile, args, platform); return run(c.file, c.args, options); };
  async function linuxPath(profile, file) {
    if (profile.backend !== "wsl") return file;
    const c = openShellCommand(profile, [], platform);
    const { stdout } = await run(c.file, [...c.args.slice(0, -1), "wslpath", "-a", file]);
    const translated = stdout.trim();
    if (!translated.startsWith("/") || /[\r\n\0:]/.test(translated)) fail("WSL could not translate this project path.");
    return translated;
  }
  function job(id) {
    const value = records.list().find((r) => r.id === id && r.kind === "sandbox");
    if (!value) fail("No such sandbox job.", "missing", 404);
    return value;
  }
  return {
    async status(profile) {
      try { const { stdout } = await call(profile, ["--version"]); await call(profile, ["sandbox", "list", "-o", "json"]); return { ready: true, version: clean(stdout), backend: profile.backend, experimental: profile.backend === "wsl" }; }
      catch { return { ready: false, backend: profile.backend, experimental: profile.backend === "wsl", reason: "OpenShell or its active gateway is unavailable. Configure the gateway in this execution environment, then check again." }; }
    },
    async preview(body) {
      tickets.check();
      const profile = { backend: body?.backend ?? (platform === "win32" ? "wsl" : "native"), distro: body?.distro ?? "" };
      openShellCommand(profile, [], platform);
      const agent = body?.agent;
      if (!["pi", "codex", "claude"].includes(agent)) fail("Choose Pi, Codex or Claude Code.");
      const request = text(body.request, "Task");
      if (/sk-[\w-]{16,}|Bearer\s+\S+|(?:password|api[_-]?key|token|secret)\s*[=:]/i.test(request)) fail("Attach credentials through an OpenShell provider, not the task.");
      const image = text(body.image, "Image", 240);
      if (!/^[\w./:@-]+$/.test(image) || image.startsWith("-")) fail("Use a container image reference.");
      const provider = body.provider ? text(body.provider, "Provider", 80) : "";
      if (provider && !/^[a-zA-Z0-9][\w.-]*$/.test(provider)) fail("Use an existing OpenShell provider name.");
      const source = body.source ? sourceDir(body.source) : null;
      if (source && body.upload !== true) fail("Confirm the project upload before previewing.");
      if (body.review === true && !source) fail("Choose a Git project for text change review.");
      const policy = policyFile(body.policy);
      const baseline = body.review === true ? await reviews.prepare(source) : null;
      // OpenShell 0.1.4 limits sandbox names to 19 characters.
      const name = `xeno-${randomUUID().replaceAll("-", "").slice(0, 14)}`;
      const args = ["sandbox", "create", "--name", name, "--from", image, "--label", `xenonet-job=${name}`, "--detach"];
      if (provider) args.push("--provider", provider);
      if (policy) args.push("--policy", await linuxPath(profile, policy.file));
      const agentArgs = [agent, ...(agent === "codex" ? ["exec"] : agent === "claude" ? ["-p"] : ["--print"]), request];
      const steps = [args];
      if (source) {
        // OpenShell 0.1.4 rejects create --upload together with a command.
        // Hold the canonical process until the separate upload succeeds.
        // All task text remains literal argv, never part of the shell code.
        const ready = `/tmp/${name}.ready`;
        // OpenShell cannot exec inside Completed sandboxes. A review job
        // keeps the sandbox available after its agent exits, until deletion.
        const workdir = baseline ? "/workspace/project" : `/workspace/project/${path.basename(source)}`;
        const command = baseline ? 'while [ ! -e "$1" ]; do sleep 1; done; shift; cd "$1" || exit 1; shift; "$@" < /dev/null; while :; do sleep 60; done' : 'while [ ! -e "$1" ]; do sleep 1; done; shift; cd "$1" || exit 1; shift; exec "$@" < /dev/null';
        args.push("--", "/bin/sh", "-c", command, "xenonet-upload", ready, workdir, ...agentArgs);
        // OpenShell nests directory uploads under their basename. The
        // frozen directory is deliberately named project, sent to /workspace.
        steps.push(["sandbox", "exec", name, "--no-login-shell", "--", "/bin/sh", "-c", "test ! -e /workspace/project"]);
        if (baseline) steps.push(["sandbox", "exec", name, "--no-login-shell", "--", "/usr/local/bin/node", "--version"]);
        steps.push(["sandbox", "upload", ...(baseline ? ["--no-git-ignore"] : []), name, await linuxPath(profile, baseline?.upload ?? source), baseline ? "/workspace" : "/workspace/project"]);
        if (baseline) steps.push(["sandbox", "exec", name, "--no-login-shell", "--timeout", "30", "--", "/usr/local/bin/node", "-e", COLLECT_TEXT]);
        steps.push(["sandbox", "exec", name, "--no-login-shell", "--", "/bin/sh", "-c", ': > "$1"', "xenonet-start", ready]);
      } else {
        // Detached print-mode agents can wait for piped stdin to end even
        // when a prompt is already on argv. These jobs have no input stream.
        args.push("--", "/bin/sh", "-c", 'exec "$@" < /dev/null', "xenonet-print", ...agentArgs);
      }
      const value = { profile, name, image, agent, provider, source, policy, steps, reviewId: baseline?.id ?? null, taskHash: hash(request) };
      const commands = steps.map((step) => openShellCommand(profile, step, platform));
      const scope = baseline ? (({ upload, ...publicScope }) => publicScope)(baseline) : null;
      let issued;
      try { issued = tickets.issue(value); } catch (err) { if (baseline) reviews.forget(baseline.id); throw err; }
      return { ...issued, command: commands[0], commands, baseline: scope, policy: policy?.content ?? null, source, note: `The selected image must contain the agent and /bin/sh. Print-mode stdin is closed. Uploads also need sleep and an unused /workspace/project path. Steps run in order; the agent waits for upload and starts in /workspace/project. ${baseline ? "Text review uploads only the frozen manifest below. The image needs /usr/local/bin/node. Its sandbox stays running after the agent exits until you delete it. A local baseline and retrieved changes stay on this computer until you forget the local review. " : ""}Existing providers can add network access. Inspect and delete the sandbox when finished.` };
    },
    async launch(ticket) {
      if (records.list().filter((r) => r.kind === "sandbox" && !sandboxGone(r)).length >= 64) fail("Delete or reconcile an old sandbox before launching another (64 maximum).", "busy", 409);
      const value = tickets.take(ticket);
      if (value.source && sourceDir(value.source) !== value.source) fail("The project folder changed. Preview again.", "changed", 409);
      if (value.policy && policyFile(value.policy.file)?.hash !== value.policy.hash) fail("The policy changed. Preview again.", "changed", 409);
      if (value.reviewId) await reviews.launch(value.reviewId);
      const record = records.add({ kind: "sandbox", status: "creating", name: value.name, profile: value.profile, image: value.image, agent: value.agent, provider: value.provider || null, source: value.source, policyHash: value.policy?.hash ?? null, taskHash: value.taskHash, reviewId: value.reviewId, execution: "unknown", launchSteps: value.steps.length, completedSteps: 0 });
      let completed = 0;
      try {
        for (let i = 0; i < value.steps.length; i++) {
          const args = value.steps[i], response = await call(value.profile, args, { timeout: 120000, ...(args.includes(COLLECT_TEXT) ? { maxBuffer: 8 * 1024 * 1024 } : {}) });
          if (value.reviewId && args.includes(COLLECT_TEXT)) reviews.verifyUploaded(value.reviewId, JSON.parse(response.stdout));
          completed = i + 1; records.update(record.id, { completedSteps: completed });
        }
        return records.update(record.id, { status: "created", eventKind: "sandbox-created" });
      } catch (err) {
        // The command itself wasn't found, so nothing reached a gateway and
        // there's no sandbox to reconcile or delete.
        if (err.code === "ENOENT" && completed === 0) return records.update(record.id, { status: "not-created", eventKind: "runtime-unavailable", reason: "XenoNet couldn't find the openshell command, so no sandbox was made. Install OpenShell where XenoNet's service can run it, then check the gateway." });
        const denied = /\bpolicy_denied\b/.test(err.stderr ?? "");
        // A lost CLI connection can leave a sandbox running. Never retry or
        // delete it implicitly; its unique name lets inspection reconcile it.
        return records.update(record.id, { status: "creation-unknown", eventKind: denied ? "execution-policy-denial" : "runtime-unavailable", reason: denied ? "OpenShell reported policy_denied. Inspect its effective policy." : "Creation did not complete locally. Inspect the gateway before retrying; the sandbox may exist." });
      }
    },
    async inspect(id) {
      const value = job(id);
      if (value.status === "deleted") fail("This sandbox was deleted.", "missing", 404);
      if (value.status === "not-created") fail("No sandbox was made for this job.", "missing", 404);
      const { stdout } = await call(value.profile, ["sandbox", "get", value.name, "--output", "json"]);
      const sandbox = JSON.parse(stdout);
      const policy = await call(value.profile, ["policy", "get", value.name, "--full"]);
      const logs = await call(value.profile, ["logs", value.name, "--since", "10m", "--source", "sandbox"]);
      // A deletion still pending stays pending: seeing the sandbox doesn't undo the request.
      records.update(id, { status: value.status === "deletion-requested" ? value.status : "observed", checkedAt: now() });
      // Logs are displayed on request and never copied to the record store.
      return { id, sandbox: { name: value.name, phase: clean(sandbox.phase ?? sandbox.sandbox?.phase ?? sandbox.status?.phase ?? "unknown") }, policy: clean(policy.stdout, 64000), logs: clean(logs.stdout, 64000), audit: "A bounded snapshot of best-effort gateway logs; absence of an event does not establish that an operation was allowed." };
    },
    review(id) {
      const value = job(id);
      if (!value.reviewId) return { available: false, reason: "This job has no frozen text baseline. It supports inspection and deletion." };
      try { return { available: true, ...reviews.view(value.reviewId) }; }
      catch (err) { if (err.code === "missing") return { available: false, reason: "The local review was forgotten or is unavailable." }; throw err; }
    },
    async retrieve(id) {
      const value = job(id);
      if (!value.reviewId || sandboxGone(value)) fail("This job has no retrievable text baseline.", "unavailable", 409);
      reviews.view(value.reviewId);
      try {
        const { stdout } = await call(value.profile, ["sandbox", "exec", value.name, "--no-login-shell", "--timeout", "30", "--", "/usr/local/bin/node", "-e", COLLECT_TEXT], { timeout: 45000, maxBuffer: 8 * 1024 * 1024 });
        const result = await reviews.accept(value.reviewId, JSON.parse(stdout));
        records.update(id, { retrieval: result.result.complete ? "retrieved" : "incomplete", retrievedAt: now() });
        return { available: true, ...result };
      } catch (err) {
        records.update(id, { retrieval: "unavailable", retrievalCheckedAt: now() });
        if (["retrieval-invalid", "review-too-large", "review-capacity"].includes(err.code)) throw err;
        fail("Changes could not be retrieved. The sandbox may be stopped, unavailable, changing, or outside the text limits. An earlier local review, if any, is retained.", "retrieval-unavailable", 502);
      }
    },
    exportReview(id, revision) { const value = job(id); if (!value.reviewId) fail("This job has no local review."); return reviews.export(value.reviewId, revision); },
    forgetReview(id) { const value = job(id); if (!value.reviewId) fail("This job has no local review."); const result = reviews.forget(value.reviewId); records.update(id, { reviewId: null, retrieval: "forgotten" }); return result; },
    async remove(id) {
      const value = job(id);
      if (sandboxGone(value)) return value;
      // The listing decides, not the delete. Delete fails for a sandbox
      // that's already gone, and such a job must still reach deleted, or it
      // counts toward the 64-job limit for good.
      try { await call(value.profile, ["sandbox", "delete", value.name], { timeout: 60000 }); } catch {}
      let gone = false;
      try {
        const { stdout } = await call(value.profile, ["sandbox", "list", "--selector", `xenonet-job=${value.name}`, "-o", "json"]);
        const listed = JSON.parse(stdout);
        gone = Array.isArray(listed.sandboxes) && listed.sandboxes.length === 0 && !listed.next_page_token;
      } catch {}
      return records.update(id, { status: gone ? "deleted" : "deletion-requested", eventKind: gone ? "sandbox-deleted" : "sandbox-deletion-requested" });
    },
  };
}
