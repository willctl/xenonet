// Loads the Rust twin of each core/ module that has one, in place of the
// module: node --import ./crates/xenonet-node/js/register.js …
import { register } from "node:module";

register("./resolve.js", import.meta.url);
