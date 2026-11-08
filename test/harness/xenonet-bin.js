// The xenonet binary the tests start: XENONET_CLI_BIN when it names one,
// else this checkout's newer build (target/release or target/debug), which
// npm run test:e2e builds first.
import { builtBinary } from "../../scripts/xenonet.mjs";

const bin = () => {
  const found = process.env.XENONET_CLI_BIN || builtBinary();
  if (!found) throw new Error("no xenonet binary: run npm run rust:bin");
  return found;
};

// [file, args] that run `xenonet <args>`.
export function command(args = []) {
  return [bin(), args];
}

// [file, args] that run the service with `args` (--demo, --dev…).
export function service(args = []) {
  return [bin(), ["serve", ...args]];
}
