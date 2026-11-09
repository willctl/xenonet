// Desktop's components (Service, Tools and the Mac's tray) are installed
// and updated by the xenonet binary (`xenonet components`, from
// crates/xenonet-components), not by Desktop's own JS. This runs it and
// reads its JSON answer; a failure is the binary's message.
import { execFile } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { binName, launcherBin } from "./runtime.js";

// The binary that does it: the one a bundle (the Mac's disk image,
// Windows' XenoNet Setup) carries beside its packages, which is as new as
// this Desktop; else the launcher's copy of the installed Tools' binary.
// Null when there's neither.
export function componentsBin({ env = process.env, bundle = null, platform = process.platform, exists = fs.existsSync } = {}) {
  for (const f of [bundle && path.join(bundle, binName(platform)), launcherBin(env, platform)]) if (f && exists(f)) return f;
  return null;
}

// `xenonet components <args>` with `env`: its answer. Downloads and a
// service's restart can take minutes, so it waits up to fifteen.
export function components(bin, args, { env = process.env, timeout = 15 * 60e3 } = {}) {
  if (!bin) return Promise.reject(new Error("Install the XenoNet bundle to add its background service and tools."));
  return new Promise((resolve, reject) => {
    execFile(bin, ["components", ...args], { env, windowsHide: true, timeout, maxBuffer: 16 << 20, encoding: "utf8" }, (err, stdout, stderr) => {
      if (err) return reject(new Error(String(stderr).trim() || err.message));
      try {
        resolve(JSON.parse(stdout));
      } catch {
        reject(new Error(`xenonet components ${args[0]} gave no answer XenoNet could read`));
      }
    });
  });
}
