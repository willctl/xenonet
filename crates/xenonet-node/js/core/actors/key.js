// core/actors/key.js in Rust (crates/xenonet-actors/src/key.rs): the window
// key, made at each start, kept in Rust, and compared there in constant
// time.
import { inspect } from "node:util";
import { native } from "../../index.js";

export function createWindowKey() {
  const key = new native.WindowKey();
  const hidden = () => key.hidden;
  return {
    // For the desktop app, over the utility process's port; nowhere else.
    reveal: () => key.reveal(),
    // Whether a request's x-xenonet-window is the key.
    matches: (given) => typeof given === "string" && key.matches(given),
    // Logged or sent by mistake, it says what it is and nothing more.
    toJSON: hidden,
    toString: hidden,
    [inspect.custom]: hidden,
  };
}
