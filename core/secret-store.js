// Secrets XenoNet keeps for you in your computer's own credential store:
// Windows Credential Manager, the macOS login keychain, or the Secret
// Service on Linux (secret-tool). Never in a file of XenoNet's and never on a
// command line, where other programs could read it: a secret goes to the
// store's program on its standard input and comes back on its standard
// output, and nothing here logs it. A target is the item's name, such as
// "XenoNet/OpenAI tunnel/tunnel_…", or the name of one you keep already.
//
//   available()        whether this computer has a store XenoNet can use
//   has(target)        whether an item is there, without reading it
//   get(target)        its secret, or null
//   set(target, secret, { comment })
//   remove(target)     true when there was one
//
// `run(file, args, { input })` stands in for tests: it resolves to
// { code, stdout } and never throws for a non-zero exit.
import { spawn } from "node:child_process";
import { onPath } from "./paths.js";

// What a target may be: printable, no quotes or backslashes, so it reads
// the same to every store.
export const TARGET = /^[A-Za-z0-9][A-Za-z0-9 ._:/@+-]{0,126}$/;
// What a secret may be here: one line of printable text without quotes or
// backslashes. The secrets XenoNet keeps (API keys) are well within it, and
// it keeps the macOS store's command line safe.
export const SECRET = /^[!#-&(-[\]-~]{8,1024}$/;
const ACCOUNT = "xenonet";
const MISSING = 3;

function runFile(file, args, { input = "", timeout = 20000, env = process.env } = {}) {
  return new Promise((resolve) => {
    let child;
    try {
      child = spawn(file, args, { windowsHide: true, env, stdio: ["pipe", "pipe", "pipe"] });
    } catch {
      return resolve({ code: -1, stdout: "" });
    }
    let stdout = "";
    const timer = setTimeout(() => child.kill(), timeout);
    child.stdout.on("data", (d) => (stdout += d));
    child.stderr.on("data", () => {});
    child.on("error", () => (clearTimeout(timer), resolve({ code: -1, stdout: "" })));
    child.on("close", (code) => (clearTimeout(timer), resolve({ code: code ?? -1, stdout })));
    child.stdin.on("error", () => {});
    child.stdin.end(input);
  });
}

// Windows: Credential Manager's generic credentials, through CredRead,
// CredWrite and CredDelete, in PowerShell. The request (and a secret to
// keep) comes on standard input as JSON; the script itself holds none.
const WINDOWS = String.raw`
$ErrorActionPreference = 'Stop'
$req = [Console]::In.ReadToEnd() | ConvertFrom-Json
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class XenoNetSecret {
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
  private struct CREDENTIAL {
    public int Flags; public int Type; public string TargetName; public string Comment;
    public System.Runtime.InteropServices.ComTypes.FILETIME LastWritten;
    public int CredentialBlobSize; public IntPtr CredentialBlob; public int Persist;
    public int AttributeCount; public IntPtr Attributes; public string TargetAlias; public string UserName;
  }
  [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)] private static extern bool CredReadW(string target, int type, int flags, out IntPtr credential);
  [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)] private static extern bool CredWriteW(ref CREDENTIAL credential, int flags);
  [DllImport("advapi32.dll", CharSet = CharSet.Unicode, SetLastError = true)] private static extern bool CredDeleteW(string target, int type, int flags);
  [DllImport("advapi32.dll")] private static extern void CredFree(IntPtr buffer);
  public static string Read(string target) {
    IntPtr ptr;
    if (!CredReadW(target, 1, 0, out ptr)) return null;
    try {
      CREDENTIAL c = (CREDENTIAL)Marshal.PtrToStructure(ptr, typeof(CREDENTIAL));
      if (c.CredentialBlobSize == 0) return null;
      byte[] blob = new byte[c.CredentialBlobSize];
      Marshal.Copy(c.CredentialBlob, blob, 0, blob.Length);
      return Encoding.Unicode.GetString(blob);
    } finally { CredFree(ptr); }
  }
  public static void Write(string target, string secret, string user, string comment) {
    byte[] blob = Encoding.Unicode.GetBytes(secret);
    IntPtr buffer = Marshal.AllocHGlobal(blob.Length);
    try {
      Marshal.Copy(blob, 0, buffer, blob.Length);
      CREDENTIAL c = new CREDENTIAL();
      c.Type = 1; c.TargetName = target; c.Comment = comment; c.CredentialBlobSize = blob.Length;
      c.CredentialBlob = buffer; c.Persist = 2; c.UserName = user;
      if (!CredWriteW(ref c, 0)) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
    } finally { Marshal.FreeHGlobal(buffer); }
  }
  public static bool Delete(string target) { return CredDeleteW(target, 1, 0); }
}
'@
switch ($req.op) {
  'has' { if ([XenoNetSecret]::Read($req.target)) { exit 0 } else { exit ${MISSING} } }
  'get' { $s = [XenoNetSecret]::Read($req.target); if ($s) { [Console]::Out.Write($s) } else { exit ${MISSING} } }
  'set' { [XenoNetSecret]::Write($req.target, $req.secret, 'XenoNet', $req.comment) }
  'remove' { if (-not [XenoNetSecret]::Delete($req.target)) { exit ${MISSING} } }
}
exit 0
`;

export function createSecretStore({ platform = process.platform, env = process.env, run = runFile } = {}) {
  const tool = platform === "darwin" ? "/usr/bin/security" : platform === "win32" ? "powershell.exe" : onPath("secret-tool", { env, platform });
  const check = (target) => {
    if (!TARGET.test(String(target ?? ""))) throw new Error("a credential's name is printable text, without quotes or backslashes");
    return target;
  };

  // Each store's four operations, as { code, stdout }.
  const ops = {
    win32(op, target, secret, comment) {
      const script = Buffer.from(WINDOWS, "utf16le").toString("base64");
      return run(tool, ["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", script], { input: JSON.stringify({ op, target, secret, comment }) });
    },
    darwin(op, target, secret, comment) {
      // Found by its name alone, so an item you made yourself is found
      // too; made with XenoNet as its account.
      if (op === "has") return run(tool, ["find-generic-password", "-s", target]);
      if (op === "get") return run(tool, ["find-generic-password", "-s", target, "-w"]);
      if (op === "remove") return run(tool, ["delete-generic-password", "-s", target]);
      // security -i reads its commands from standard input, so the secret
      // never shows in a process list.
      return run(tool, ["-i"], { input: `add-generic-password -U -a "${ACCOUNT}" -s "${target}" -l "${target}" -j "${comment.replace(/["\\]/g, "")}" -w "${secret}"\n` });
    },
    linux(op, target, secret) {
      if (op === "has" || op === "get") return run(tool, ["lookup", "xenonet-target", target]).then((r) => ({ ...r, code: r.code === 0 && r.stdout ? 0 : MISSING }));
      if (op === "remove") return run(tool, ["clear", "xenonet-target", target]);
      return run(tool, ["store", "--label", target, "xenonet-target", target], { input: secret });
    },
  };
  const call = (op, target, secret = null, comment = "") => (ops[platform] ?? ops.linux)(op, check(target), secret, comment);
  const kind = platform === "win32" ? "Windows Credential Manager" : platform === "darwin" ? "the macOS keychain" : "the Secret Service (secret-tool)";

  return {
    kind,
    available: () => !!tool,
    async has(target) {
      if (!tool) return false;
      return (await call("has", target)).code === 0;
    },
    async get(target) {
      if (!tool) return null;
      const r = await call("get", target);
      if (r.code !== 0) return null;
      // The keychain ends what it prints with a newline; a secret has none.
      const secret = String(r.stdout).replace(/\r?\n$/, "");
      return secret || null;
    },
    async set(target, secret, { comment = "Kept by XenoNet" } = {}) {
      if (!tool) throw new Error(`XenoNet can't reach ${kind} on this computer`);
      if (!SECRET.test(String(secret ?? ""))) throw new Error("that isn't a secret XenoNet can keep: one line, without quotes or backslashes");
      const r = await call("set", target, secret, comment);
      if (r.code !== 0) throw new Error(`${kind} didn't take it`);
      return true;
    },
    async remove(target) {
      if (!tool) return false;
      return (await call("remove", target)).code === 0;
    },
  };
}

// A store that keeps secrets in memory only, for the demo and test
// instances, whose connections are made up: nothing reaches your real
// credential store from them.
export function createMemorySecrets() {
  const kept = new Map();
  return {
    kind: "memory (this demo forgets it when it stops)",
    available: () => true,
    has: async (target) => kept.has(target),
    get: async (target) => kept.get(target) ?? null,
    async set(target, secret) {
      if (!TARGET.test(String(target ?? ""))) throw new Error("a credential's name is printable text, without quotes or backslashes");
      if (!SECRET.test(String(secret ?? ""))) throw new Error("that isn't a secret XenoNet can keep: one line, without quotes or backslashes");
      kept.set(target, secret);
      return true;
    },
    remove: async (target) => kept.delete(target),
  };
}
