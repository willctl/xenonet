// A downloaded component installs itself, from its extracted folder.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import * as tar from "tar";
import { print } from "../lib/out.js";
import { digest } from "../../core/components/packages.js";
import { activatePackage, installLauncher } from "../../core/components/install.js";
import { installCommand } from "../../core/command.js";
import { runtimeDir } from "../../core/components/runtime.js";

export const help = "Install this downloaded component for your account. From its extracted folder, run bin/xenonet setup (bin\\xenonet.exe setup on Windows). Service and Tools can be installed in either order.";
export async function run() {
  const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
  const manifest = JSON.parse(fs.readFileSync(path.join(root, "component.json"), "utf8"));
  const dir = path.join(runtimeDir(), "downloads");
  fs.mkdirSync(dir, { recursive: true });
  const file = path.join(dir, `setup-${manifest.component}-${manifest.version}-${process.pid}.tar.gz`);
  await tar.c({ file, cwd: root, portable: true, gzip: true }, fs.readdirSync(root));
  const bytes = fs.readFileSync(file);
  const result = await activatePackage(file, { ...manifest, size: bytes.length, sha256: digest(bytes) });
  const launcher = installLauncher();
  if (launcher?.bin) installCommand({ bin: launcher.bin });
  print(JSON.stringify(result));
}
