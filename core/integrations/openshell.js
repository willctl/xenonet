import fs from "node:fs";
import path from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { createHash, randomUUID } from "node:crypto";
import { clean, fail, text, createTickets } from "./common.js";

const execute = promisify(execFile);
export async function executionProfiles({ platform = process.platform, run = runCommand } = {}) {
  if (platform !== "win32") return [{ backend: "native", distro: "" }];
  try {
    const file = path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "wsl.exe");
    const { stdout } = await run(file, ["--list", "--quiet"]);
    return stdout.replace(/\0/g, "").split(/\r?\n/).map((s) => s.trim()).filter((s) => /^[\w.-]+$/.test(s) && !s.startsWith("docker-desktop")).map((distro) => ({ backend: "wsl", distro }));
  } catch { return []; }
}
export async function runCommand(file, args, { timeout = 15000 } = {}) {
  return execute(file, args, { timeout, maxBuffer: 1024 * 1024, windowsHide: true, shell: false, encoding: "utf8", env: { ...process.env, NO_COLOR: "1" } });
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
export function createOpenShell({ records, run = runCommand, platform = process.platform, now = Date.now }) {
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
      const policy = policyFile(body.policy);
      const name = `xeno-${randomUUID()}`;
      const args = ["sandbox", "create", "--name", name, "--from", image, "--label", `xenonet-job=${name}`, "--detach"];
      if (provider) args.push("--provider", provider);
      if (source) args.push("--upload", `${await linuxPath(profile, source)}:/workspace`);
      if (policy) args.push("--policy", await linuxPath(profile, policy.file));
      args.push("--", agent, ...(agent === "codex" ? ["exec"] : agent === "claude" ? ["-p"] : ["--print"]), request);
      const value = { profile, name, image, agent, provider, source, policy, args, taskHash: hash(request) };
      return { ...tickets.issue(value), command: openShellCommand(profile, args, platform), policy: policy?.content ?? null, source, note: "The selected image must contain the agent. Existing providers can add network access. Changes stay in the sandbox; XenoNet does not copy them back. Detached jobs can continue after this window closes. Inspect and delete the sandbox when finished." };
    },
    async launch(ticket) {
      if (records.list().filter((r) => r.kind === "sandbox" && r.status !== "deleted").length >= 64) fail("Delete or reconcile an old sandbox before launching another (64 maximum).", "busy", 409);
      const value = tickets.take(ticket);
      if (value.source && sourceDir(value.source) !== value.source) fail("The project folder changed. Preview again.", "changed", 409);
      if (value.policy && policyFile(value.policy.file)?.hash !== value.policy.hash) fail("The policy changed. Preview again.", "changed", 409);
      const record = records.add({ kind: "sandbox", status: "creating", name: value.name, profile: value.profile, image: value.image, agent: value.agent, provider: value.provider || null, source: value.source, policyHash: value.policy?.hash ?? null, taskHash: value.taskHash });
      try {
        await call(value.profile, value.args, { timeout: 120000 });
        return records.update(record.id, { status: "created", eventKind: "sandbox-created" });
      } catch (err) {
        const denied = /\bpolicy_denied\b/.test(err.stderr ?? "");
        // A lost CLI connection can leave a sandbox running. Never retry or
        // delete it implicitly; its unique name lets inspection reconcile it.
        return records.update(record.id, { status: "creation-unknown", eventKind: denied ? "execution-policy-denial" : "runtime-unavailable", reason: denied ? "OpenShell reported policy_denied. Inspect its effective policy." : "Creation did not complete locally. Inspect the gateway before retrying; the sandbox may exist." });
      }
    },
    async inspect(id) {
      const value = job(id);
      if (value.status === "deleted") fail("This sandbox was deleted.", "missing", 404);
      const { stdout } = await call(value.profile, ["sandbox", "get", value.name, "--output", "json"]);
      const sandbox = JSON.parse(stdout);
      const policy = await call(value.profile, ["policy", "get", value.name, "--full"]);
      const logs = await call(value.profile, ["logs", value.name, "--since", "10m", "--source", "sandbox"]);
      records.update(id, { status: "observed", checkedAt: now() });
      // Logs are displayed on request and never copied to the record store.
      return { id, sandbox: { name: value.name, phase: clean(sandbox.phase ?? sandbox.sandbox?.phase ?? sandbox.status?.phase ?? "unknown") }, policy: clean(policy.stdout, 64000), logs: clean(logs.stdout, 64000), audit: "A bounded snapshot of best-effort gateway logs; absence of an event does not establish that an operation was allowed." };
    },
    async remove(id) {
      const value = job(id);
      if (value.status === "deleted") return value;
      await call(value.profile, ["sandbox", "delete", value.name], { timeout: 60000 });
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
