// core/components/updates.js in Rust (crates/xenonet-components/src/updates.rs):
// the components this XenoNet could update to, and their downloads: only
// from the release's own address, no bigger than the release says, and
// kept only when their SHA-256 is the release's. The bytes come through
// the fetcher given; the Rust writes them and checks them.
import { native } from "../../index.js";
import { settle, errorOf } from "../../settle.js";
import { drive } from "../../task.js";
import { envOf, machine } from "../../machine.js";
import { activatePackage } from "./install.js";

export function componentUpdates({ env = process.env, fetcher = fetch } = {}) {
  const responses = new Map();
  let next = 0;
  return drive(native.compUpdates(envOf(env), machine()), async (q) => {
    if (q.op === "fetch") {
      const res = await fetcher(q.url, { signal: AbortSignal.timeout(q.timeoutMs) });
      const id = ++next;
      responses.set(id, res);
      return { ok: !!res.ok, status: res.status, res: id };
    }
    if (q.op === "json") return responses.get(q.res).json();
    throw new Error(`componentUpdates asked for ${q.op}`);
  });
}

export async function downloadComponent(item, { env = process.env, fetcher = fetch } = {}) {
  const { file, have } = settle(native.compDownloadPlan(item, envOf(env), machine()));
  if (have) return file;
  const res = await fetcher(item.url, { signal: AbortSignal.timeout(300000) });
  if (!res.ok || !res.body) throw new Error("XenoNet could not download this component.");
  const download = new native.CompDownload(file, Number(item.size), String(item.sha256));
  settle(download.open());
  try {
    for await (const chunk of res.body) {
      const failed = download.write(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
      if (failed) throw errorOf(failed);
    }
    return settle(download.finish());
  } finally {
    download.discard();
  }
}

export async function updateComponent(item, options = {}) {
  const { env = process.env, operations } = options;
  const current = native.compUpdateSkip(item);
  if (current) return current;
  const file = await downloadComponent(item, options);
  return activatePackage(file, item, { env, operations });
}
