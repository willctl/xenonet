// npm run dist[:win|:mac|:dir]: builds the UI, draws the icons and installer
// art, then packages. With no argument it builds the installers for the OS
// it runs on. The two Mac CPUs are packaged one after the other, because
// electron-builder's .pkg step writes both through the same temporary file.
import { spawnSync } from "node:child_process";

const target = process.argv[2] ?? (process.platform === "darwin" ? "mac" : process.platform === "win32" ? "win" : "dir");

function run(cmd, args) {
  console.log(`\n> ${[cmd, ...args].join(" ")}`);
  const { status } = spawnSync(cmd, args, { stdio: "inherit", shell: process.platform === "win32" });
  if (status !== 0) process.exit(status ?? 1);
}

run("npx", ["vite", "build"]);
run("npx", ["electron", ".", "--make-icons", "build"]);
if (target === "win") run("npx", ["electron-builder", "--win", "nsis", "--publish", "never"]);
else if (target === "mac") for (const arch of ["--arm64", "--x64"]) run("npx", ["electron-builder", "--mac", arch, "--publish", "never"]);
else run("npx", ["electron-builder", "--dir", "--publish", "never"]);
