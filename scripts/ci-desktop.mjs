// Check that the runner can open Electron before every suite tries it.
// Only the test instance's made-up data appears in these diagnostics.
// With XENONET_E2E_SHARD (such as 2/4) and no files named, it runs that
// shard of the suite (scripts/e2e-shard.mjs).
import fs from "node:fs";
import { spawnSync } from "node:child_process";
import { xenonet } from "../core/control.js";
import { parseShard, readTimings, shards, suiteFiles } from "./e2e-shard.mjs";

const instance = xenonet("test");
try {
  await instance.launch({ show: "hidden", timeout: 60000 });
  await instance.quit();
  let files = process.argv.slice(2);
  if (!files.length && process.env.XENONET_E2E_SHARD) {
    const { index, total } = parseShard(process.env.XENONET_E2E_SHARD);
    const timings = readTimings();
    const untimed = suiteFiles().filter((f) => typeof timings[f] !== "number");
    files = shards(suiteFiles(), timings, total)[index - 1].map((f) => `test/e2e/${f}`);
    console.log(`shard ${index} of ${total}: ${files.join(" ")}`);
    if (untimed.length) console.log(`no time yet, counted as the median: ${untimed.join(" ")} (node scripts/e2e-shard.mjs --measure <log>)`);
  }
  const result = files.length
    ? spawnSync(process.execPath, ["--test", "--test-concurrency=1", ...files], { stdio: "inherit" })
    : spawnSync("npm", ["run", "test:e2e"], { stdio: "inherit", shell: process.platform === "win32" });
  process.exitCode = result.status ?? 1;
} catch (error) {
  console.error(error);
  process.exitCode = 1;
} finally {
  if (process.exitCode) {
    const file = ".xenonet/logs/test.log";
    if (fs.existsSync(file)) console.error(fs.readFileSync(file, "utf8").split("\n").slice(-100).join("\n"));
  }
  await instance.quit().catch(() => {});
}
