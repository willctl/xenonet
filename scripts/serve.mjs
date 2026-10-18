// Runs the service, `xenonet serve`, from this checkout, built first so it
// matches the source (or the binary XENONET_SERVICE_BIN names). With
// --watch, the page is rebuilt into dist/ as src/ is edited, which the
// service serves: reload the page to see a change. XenoNet Dev
// (npm run app) hot-reloads the page itself.
//
//   npm start       the page built once, then the service
//   npm run dev     on XenoNet Dev's port, the page rebuilt as you edit
//   npm run demo    the same, with the demo's made-up data
//   npm run serve   the service as the checkout is
import { spawn } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildBinary } from "./rust-bin.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const watch = args.includes("--watch");
let bin = process.env.XENONET_SERVICE_BIN;
try {
  bin ||= buildBinary();
} catch (err) {
  // cargo has said why.
  process.exit(err.status ?? 1);
}

const children = [];
if (watch) children.push(spawn(process.execPath, [path.join(root, "node_modules", "vite", "bin", "vite.js"), "build", "--watch"], { cwd: root, stdio: "inherit" }));
const service = spawn(bin, ["serve", ...args.filter((a) => a !== "--watch")], { cwd: root, stdio: "inherit" });
const stop = (signal) => service.kill(signal);
process.on("SIGINT", stop);
process.on("SIGTERM", stop);
service.on("exit", (code, signal) => {
  for (const c of children) c.kill();
  process.exitCode = code ?? (signal ? 1 : 0);
});
