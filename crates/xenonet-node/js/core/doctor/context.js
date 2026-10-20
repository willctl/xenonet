// core/doctor/context.js beside the Rust doctor (crates/xenonet-doctor/):
// the doctor's view of this machine with XenoNet's own settings. Nothing
// here decides anything; it hands the machine() in index.js XenoNet's
// folders, its Codex and whether git works.
import { peer } from "../../peer.js";
import { machine } from "./index.js";

const { config } = await import(peer("config.js"));
const { findCodexBinary } = await import(peer("dirs.js"));
const { gitAvailable } = await import(peer("git.js"));

export const doctorContext = (extra = {}) => machine({ home: config.homeDir, paths: config, codexBin: findCodexBinary(), gitAvailable, ...extra });
