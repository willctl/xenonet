//! Secrets XenoNet keeps for you in your computer's own credential store
//! (core/secret-store.js): Windows Credential Manager, the macOS login
//! keychain, or the Secret Service on Linux (secret-tool). Never in a file
//! of XenoNet's and never on a command line, where other programs could
//! read it: a secret goes to the store's program on its standard input and
//! comes back on its standard output, and nothing here logs it. A target is
//! the item's name, such as "XenoNet/OpenAI tunnel/tunnel_…", or the name
//! of one you keep already.
//!
//! The store's program runs through `Run`, which answers with its exit
//! code and output and never fails for a non-zero exit.

use std::collections::HashMap;

/// What a target may be: printable, no quotes or backslashes, so it reads
/// the same to every store.
pub const TARGET: &str = "^[A-Za-z0-9][A-Za-z0-9 ._:/@+-]{0,126}$";
/// What a secret may be here: one line of printable text without quotes or
/// backslashes. The secrets XenoNet keeps (API keys) are well within it,
/// and it keeps the macOS store's command line safe.
pub const SECRET: &str = "^[!#-&(-\\[\\]-~]{8,1024}$";
const ACCOUNT: &str = "xenonet";
const MISSING: i64 = 3;

pub const BAD_TARGET: &str = "a credential's name is printable text, without quotes or backslashes";
pub const BAD_SECRET: &str = "that isn't a secret XenoNet can keep: one line, without quotes or backslashes";

pub fn is_target(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphanumeric()) && s.len() <= 127 && chars.all(|c| c.is_ascii_alphanumeric() || " ._:/@+-".contains(c))
}

pub fn is_secret(s: &str) -> bool {
    (8..=1024).contains(&s.len()) && s.bytes().all(|b| (b'!'..=b'~').contains(&b) && !matches!(b, b'"' | b'\'' | b'\\'))
}

/// Windows: Credential Manager's generic credentials, through CredRead,
/// CredWrite and CredDelete, in PowerShell. The request (and a secret to
/// keep) comes on standard input as JSON; the script itself holds none.
pub const WINDOWS: &str = r#"
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
  'has' { if ([XenoNetSecret]::Read($req.target)) { exit 0 } else { exit 3 } }
  'get' { $s = [XenoNetSecret]::Read($req.target); if ($s) { [Console]::Out.Write($s) } else { exit 3 } }
  'set' { [XenoNetSecret]::Write($req.target, $req.secret, 'XenoNet', $req.comment) }
  'remove' { if (-not [XenoNetSecret]::Delete($req.target)) { exit 3 } }
}
exit 0
"#;

/// A run of the store's program: what goes on its standard input, if
/// anything.
#[derive(Clone, Debug)]
pub struct Command {
    pub file: String,
    pub args: Vec<String>,
    pub input: Option<String>,
}

/// What a run said: its exit code and its output.
#[derive(Clone, Debug, Default)]
pub struct Ran {
    pub code: f64,
    pub stdout: String,
}

pub trait Run {
    /// Err: the runner itself failed (it's meant never to), with its words.
    fn run(&mut self, command: Command) -> Result<Ran, String>;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Has,
    Get,
    Set,
    Remove,
}

impl Op {
    fn name(self) -> &'static str {
        match self {
            Op::Has => "has",
            Op::Get => "get",
            Op::Set => "set",
            Op::Remove => "remove",
        }
    }
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(B64[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn utf16le(s: &str) -> Vec<u8> {
    s.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

/// PowerShell's -EncodedCommand: the script in UTF-16LE, in base64.
pub fn encoded_command(script: &str) -> String {
    base64(&utf16le(script))
}

fn json_string(s: &str) -> String {
    xenonet_base::json::stringify(&serde_json::Value::String(s.into()), 0)
}

/// The credential store of this platform: `tool` is its program, when
/// there is one (secret-tool on PATH, for Linux).
pub struct SecretStore {
    platform: String,
    tool: Option<String>,
}

impl SecretStore {
    pub fn new(platform: &str, tool_on_path: Option<String>) -> Self {
        let tool = match platform {
            "darwin" => Some("/usr/bin/security".to_string()),
            "win32" => Some("powershell.exe".to_string()),
            _ => tool_on_path,
        };
        SecretStore { platform: platform.into(), tool }
    }

    /// Whether this computer has a store XenoNet can use.
    pub fn available(&self) -> bool {
        self.tool.is_some()
    }

    pub fn kind(&self) -> &'static str {
        match self.platform.as_str() {
            "win32" => "Windows Credential Manager",
            "darwin" => "the macOS keychain",
            _ => "the Secret Service (secret-tool)",
        }
    }

    fn command(&self, op: Op, target: &str, secret: Option<&str>, comment: &str) -> Command {
        let tool = self.tool.clone().unwrap_or_default();
        let c = |args: &[&str], input: Option<String>| Command { file: tool.clone(), args: args.iter().map(ToString::to_string).collect(), input };
        match self.platform.as_str() {
            "win32" => {
                let input = format!(
                    "{{\"op\":{},\"target\":{},\"secret\":{},\"comment\":{}}}",
                    json_string(op.name()),
                    json_string(target),
                    secret.map_or("null".to_string(), json_string),
                    json_string(comment)
                );
                c(&["-NoLogo", "-NoProfile", "-NonInteractive", "-EncodedCommand", &encoded_command(WINDOWS)], Some(input))
            }
            // Found by its name alone, so an item you made yourself is found
            // too; made with XenoNet as its account.
            "darwin" => match op {
                Op::Has => c(&["find-generic-password", "-s", target], None),
                Op::Get => c(&["find-generic-password", "-s", target, "-w"], None),
                Op::Remove => c(&["delete-generic-password", "-s", target], None),
                // security -i reads its commands from standard input, so the
                // secret never shows in a process list.
                Op::Set => {
                    let comment: String = comment.chars().filter(|c| *c != '"' && *c != '\\').collect();
                    c(&["-i"], Some(format!("add-generic-password -U -a \"{ACCOUNT}\" -s \"{target}\" -l \"{target}\" -j \"{comment}\" -w \"{}\"\n", secret.unwrap_or("null"))))
                }
            },
            _ => match op {
                Op::Has | Op::Get => c(&["lookup", "xenonet-target", target], None),
                Op::Remove => c(&["clear", "xenonet-target", target], None),
                Op::Set => c(&["store", "--label", target, "xenonet-target", target], Some(secret.unwrap_or("null").to_string())),
            },
        }
    }

    fn call(&self, run: &mut dyn Run, op: Op, target: &str, secret: Option<&str>, comment: &str) -> Result<Ran, String> {
        if !is_target(target) {
            return Err(BAD_TARGET.into());
        }
        let ran = run.run(self.command(op, target, secret, comment))?;
        // secret-tool's lookup says 0 for an item that isn't there, with
        // nothing printed.
        if self.platform != "win32" && self.platform != "darwin" && matches!(op, Op::Has | Op::Get) {
            let code = if ran.code == 0.0 && !ran.stdout.is_empty() { 0.0 } else { MISSING as f64 };
            return Ok(Ran { code, ..ran });
        }
        Ok(ran)
    }

    /// Whether an item is there, without reading it.
    pub fn has(&self, run: &mut dyn Run, target: &str) -> Result<bool, String> {
        if self.tool.is_none() {
            return Ok(false);
        }
        Ok(self.call(run, Op::Has, target, None, "")?.code == 0.0)
    }

    /// Its secret, or None.
    pub fn get(&self, run: &mut dyn Run, target: &str) -> Result<Option<String>, String> {
        if self.tool.is_none() {
            return Ok(None);
        }
        let r = self.call(run, Op::Get, target, None, "")?;
        if r.code != 0.0 {
            return Ok(None);
        }
        // The keychain ends what it prints with a newline; a secret has none.
        let s = r.stdout.strip_suffix('\n').map_or(r.stdout.as_str(), |s| s.strip_suffix('\r').unwrap_or(s));
        Ok((!s.is_empty()).then(|| s.to_string()))
    }

    pub fn set(&self, run: &mut dyn Run, target: &str, secret: &str, comment: &str) -> Result<bool, String> {
        if self.tool.is_none() {
            return Err(format!("XenoNet can't reach {} on this computer", self.kind()));
        }
        if !is_secret(secret) {
            return Err(BAD_SECRET.into());
        }
        if self.call(run, Op::Set, target, Some(secret), comment)?.code != 0.0 {
            return Err(format!("{} didn't take it", self.kind()));
        }
        Ok(true)
    }

    /// True when there was one.
    pub fn remove(&self, run: &mut dyn Run, target: &str) -> Result<bool, String> {
        if self.tool.is_none() {
            return Ok(false);
        }
        Ok(self.call(run, Op::Remove, target, None, "")?.code == 0.0)
    }
}

/// A store that keeps secrets in memory only, for the demo and test
/// instances, whose connections are made up: nothing reaches your real
/// credential store from them.
#[derive(Default)]
pub struct MemorySecrets {
    kept: HashMap<String, String>,
}

impl MemorySecrets {
    pub const KIND: &'static str = "memory (this demo forgets it when it stops)";

    pub fn has(&self, target: &str) -> bool {
        self.kept.contains_key(target)
    }
    pub fn get(&self, target: &str) -> Option<String> {
        self.kept.get(target).cloned()
    }
    pub fn set(&mut self, target: &str, secret: &str) -> Result<bool, String> {
        if !is_target(target) {
            return Err(BAD_TARGET.into());
        }
        if !is_secret(secret) {
            return Err(BAD_SECRET.into());
        }
        self.kept.insert(target.into(), secret.into());
        Ok(true)
    }
    pub fn remove(&mut self, target: &str) -> bool {
        self.kept.remove(target).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_and_secrets() {
        assert!(is_target("XenoNet/OpenAI tunnel/tunnel_1"));
        assert!(!is_target("bad\"name") && !is_target("") && !is_target(" lead") && !is_target(&"a".repeat(128)));
        assert!(is_target(&"a".repeat(127)));
        assert!(is_secret("sk-or-v1-made-up-key") && !is_secret("sk-\"; evil") && !is_secret("short") && !is_secret("has space in it"));
    }

    #[test]
    fn base64_of_utf16() {
        assert_eq!(encoded_command("a"), "YQA=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(b"fo"), "Zm8=");
    }
}
