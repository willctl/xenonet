// The doctor's view of this machine with XenoNet's own settings: the folders
// it reads (config.js), the Codex it runs and whether git works. The server
// adds what only it knows (its sources, live state and connections); the
// xenonet command, when no XenoNet is running, uses it as it is.
import { config } from "../config.js";
import { findCodexBinary } from "../dirs.js";
import { gitAvailable } from "../git.js";
import { machine } from "./index.js";

export const doctorContext = (extra = {}) => machine({ home: config.homeDir, paths: config, codexBin: findCodexBinary(), gitAvailable, ...extra });
