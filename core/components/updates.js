import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { pipeline } from "node:stream/promises";
import { Transform } from "node:stream";
import { currentComponent, newer, packageInfo, digest } from "./packages.js";
import { activatePackage } from "./install.js";
import { runtimeDir } from "./runtime.js";

const FEED = "https://github.com/willctl/xenonet/releases/latest/download/components.json";
export async function componentUpdates({ env = process.env, fetcher = fetch } = {}) {
  const response = await fetcher(FEED, { signal: AbortSignal.timeout(20000) });
  if (response.status === 404) return [];
  if (!response.ok) throw new Error("XenoNet could not check component updates. Try again later.");
  const manifest = await response.json();
  if (manifest.format !== 1 || !Array.isArray(manifest.packages)) throw new Error("Invalid component update manifest.");
  return manifest.packages.filter((item) => item.platform === process.platform && item.arch === process.arch).map((item) => {
    packageInfo(item);
    if (!/^XenoNet-(service|tools)-[\w.-]+\.tar\.gz$/.test(item.file) || !Number.isSafeInteger(item.size) || item.size < 1 || item.size > 512 << 20 || !/^[a-f0-9]{64}$/.test(item.sha256)) throw new Error("Invalid component download.");
    const current = currentComponent(item.component, env);
    return { ...item, current: current?.version ?? null, available: !current || newer(item.version, current.version), url: `https://github.com/willctl/xenonet/releases/latest/download/${item.file}` };
  });
}

export async function downloadComponent(item, { env = process.env, fetcher = fetch } = {}) {
  if (!/^https:\/\/github\.com\/willctl\/xenonet\/releases\/latest\/download\/XenoNet-[\w.-]+\.tar\.gz$/.test(item.url)) throw new Error("Unexpected component download address.");
  if (!/^[a-f0-9]{64}$/.test(item.sha256) || !Number.isSafeInteger(item.size) || item.size < 1 || item.size > 512 << 20) throw new Error("Invalid component download.");
  const dir = path.join(runtimeDir(env), "downloads");
  fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  const file = path.join(dir, `${item.sha256}.tar.gz`);
  const valid = (candidate) => fs.statSync(candidate).size === item.size && digest(fs.readFileSync(candidate)) === item.sha256;
  if (fs.existsSync(file) && !valid(file)) fs.unlinkSync(file);
  if (!fs.existsSync(file)) {
    const res = await fetcher(item.url, { signal: AbortSignal.timeout(300000) });
    if (!res.ok || !res.body) throw new Error("XenoNet could not download this component.");
    let size = 0;
    const limit = new Transform({ transform(chunk, _encoding, done) { size += chunk.length; done(size > item.size ? new Error("Component download exceeds its expected size.") : null, chunk); } });
    const tmp = `${file}.${crypto.randomUUID()}.tmp`;
    try {
      await pipeline(res.body, limit, fs.createWriteStream(tmp, { flags: "wx", mode: 0o600 }));
      if (!valid(tmp)) throw new Error("Component download failed its hash or size check. Try again.");
      fs.renameSync(tmp, file);
    } finally { fs.rmSync(tmp, { force: true }); }
  }
  return file;
}

export async function updateComponent(item, options = {}) {
  const { env = process.env } = options;
  if (!item.available) return { component: item.component, status: "current", version: item.current };
  const file = await downloadComponent(item, options);
  return activatePackage(file, item, { env });
}
