import { test } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { Readable } from "node:stream";
import { downloadComponent } from "../core/components/updates.js";
import { digest } from "../core/components/packages.js";

test("failed component downloads clean up and a corrupt cached file can be retried", async (t) => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-download-"));
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  const env = { XENONET_RUNTIME_DIR: dir };
  const bytes = Buffer.from("verified archive bytes");
  const item = { size: bytes.length, sha256: digest(bytes), url: "https://github.com/willctl/xenonet/releases/latest/download/XenoNet-tools-0.6.0-win32-x64.tar.gz" };
  const response = (body) => async () => ({ ok: true, body });
  await assert.rejects(downloadComponent(item, { env, fetcher: response(Readable.from((async function* () { yield bytes.subarray(0, 4); throw new Error("connection lost"); })())) }), /connection lost/);
  assert.deepEqual(fs.readdirSync(path.join(dir, "downloads")), []);
  await assert.rejects(downloadComponent(item, { env, fetcher: response(Readable.from([Buffer.alloc(item.size)])) }), /hash or size/);
  assert.deepEqual(fs.readdirSync(path.join(dir, "downloads")), []);
  const cache = path.join(dir, "downloads", `${item.sha256}.tar.gz`);
  fs.writeFileSync(cache, "corrupt cache");
  const file = await downloadComponent(item, { env, fetcher: response(Readable.from([bytes])) });
  assert.deepEqual(fs.readFileSync(file), bytes);
  assert.equal(await downloadComponent(item, { env, fetcher: () => { throw new Error("valid cache should be reused"); } }), file);
});
