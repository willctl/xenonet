// core/components/console-policy.js in Rust
// (crates/xenonet-components/src/console_policy.rs): production offers the
// supported driver, never arbitrary page execution.
import { native } from "../../index.js";

export function consolePolicy({ op, method, args }, production) {
  const why = native.compConsolePolicy(op, method, args, !!production);
  if (why) throw new Error(why);
}
