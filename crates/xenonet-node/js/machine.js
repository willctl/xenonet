// What the Rust is told of the machine a twin runs on: its platform and
// arch, and the home folder; and an env as a plain object.
import os from "node:os";

export const machine = () => ({ platform: process.platform, arch: process.arch, home: os.homedir() });

export const envOf = (env) => ({ ...(env ?? {}) });
