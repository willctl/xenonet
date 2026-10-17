// core/private-file.js in Rust (crates/xenonet-base/src/private_file.rs):
// files only you can read, by Unix mode or Windows ACL. Commands still run
// through JS: the `run` given, or runPrivateCommand.
import { native } from "../index.js";
import { withRun } from "../run.js";
import { runPrivateCommand } from "../../../../core/private-file.js";
export { runPrivateCommand };

const windows = (platform) => platform === "win32";

export const userSid = ({ run = runPrivateCommand } = {}) => withRun((r) => native.userSid(r), run);

export const aclOf = (file, { run = runPrivateCommand } = {}) => withRun((r) => native.aclOf(String(file), r), run);

export const othersIn = (sddl, sid) => native.othersIn(String(sddl), String(sid));

export const otherReaders = (file, { platform = process.platform, run = runPrivateCommand } = {}) => withRun((r) => native.otherReaders(String(file), windows(platform), r), run);

export function protectFile(file, { platform = process.platform, run = runPrivateCommand } = {}) {
  withRun((r) => native.protectFile(String(file), windows(platform), r), run);
}
