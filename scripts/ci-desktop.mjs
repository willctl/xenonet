// Check that the runner can open Electron before every suite tries it.
// Only the test instance's made-up data appears in these diagnostics.
import fs from "node:fs";
import { spawnSync } from "node:child_process";
import { xenonet } from "../core/control.js";

const instance = xenonet("test");
try {
  await instance.launch({ show: "hidden", timeout: 60000 });
  await instance.quit();
  const files = process.argv.slice(2);
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
