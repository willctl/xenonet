import { test, after } from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { staticFile, CSP, PAGE_HEADERS } from "../server/static.js";

// A built app: dist/ with the page and an asset.
const made = [];
after(() => made.forEach((dir) => fs.rmSync(dir, { recursive: true, force: true })));
function app({ built = true } = {}) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "xenonet-static-"));
  made.push(root);
  const dist = path.join(root, "dist");
  fs.mkdirSync(path.join(dist, "assets"), { recursive: true });
  if (built) fs.writeFileSync(path.join(dist, "index.html"), "<!doctype html>");
  fs.writeFileSync(path.join(dist, "assets", "app.js"), "export {}");
  fs.writeFileSync(path.join(root, "package.json"), "{}");
  return { root, dist };
}

test("files come from dist, and any other path gets the page", () => {
  const { dist } = app();
  const page = { file: path.join(dist, "index.html"), type: "text/html", headers: PAGE_HEADERS };
  assert.deepEqual(staticFile(dist, "/assets/app.js"), { file: path.join(dist, "assets", "app.js"), type: "text/javascript" });
  assert.deepEqual(staticFile(dist, "/"), page);
  assert.deepEqual(staticFile(dist, "/assets"), page);
  assert.deepEqual(staticFile(dist, "/some/route"), page);
  assert.equal(staticFile(dist, "/assets/app%2Ejs").file, path.join(dist, "assets", "app.js"));
});

test("a path that climbs out of dist is refused", () => {
  const { dist } = app();
  assert.equal(staticFile(dist, "/..%2fpackage.json").status, 403);
  assert.equal(staticFile(dist, "/..%2F..%2F..%2Fetc%2Fpasswd").status, 403);
});

test("a folder beside dist whose name starts the same is out of reach too", () => {
  const { root, dist } = app();
  fs.mkdirSync(path.join(root, "dist-private"));
  fs.writeFileSync(path.join(root, "dist-private", "secret.txt"), "not for the page");
  assert.equal(staticFile(dist, "/..%2fdist-private%2fsecret.txt").status, 403);
});

test("a path with a broken escape is a bad request", () => {
  const { dist } = app();
  assert.deepEqual(staticFile(dist, "/%E0%A4%A"), { status: 400, error: "bad path" });
});

test("without a build, it says how to make one", () => {
  const { dist } = app({ built: false });
  assert.equal(staticFile(dist, "/").status, 503);
  assert.match(staticFile(dist, "/").error, /npm run build/);
});

test("the page goes out with a Content-Security-Policy that runs no script but XenoNet's own, and XenoNet Dev's dev server sends the same", async () => {
  const { dist } = app();
  assert.equal(staticFile(dist, "/").headers["content-security-policy"], CSP);
  const policy = Object.fromEntries(CSP.split("; ").map((d) => [d.split(" ")[0], d.split(" ").slice(1)]));
  // Scripts from XenoNet's own server only: nothing inline, nothing evaluated.
  // WebAssembly may compile (the terminal's image decoding), which runs no
  // script of the page's.
  assert.deepEqual(policy["script-src"], ["'self'", "'wasm-unsafe-eval'"]);
  assert.deepEqual(policy["worker-src"], ["'self'", "blob:"]);
  for (const [name, sources] of Object.entries(policy)) assert.ok(!sources.includes("'unsafe-eval'") && !(name.startsWith("script") && sources.includes("'unsafe-inline'")), `${name}: ${sources.join(" ")}`);
  assert.deepEqual([policy["default-src"], policy["object-src"], policy["base-uri"], policy["frame-ancestors"]], [["'self'"], ["'none'"], ["'none'"], ["'none'"]]);
  assert.ok(!CSP.includes("http:") && !CSP.includes("https:") && !CSP.includes("*"), "no other server");
  const vite = (await import("../vite.config.js")).default;
  assert.equal(vite.server.headers["content-security-policy"], CSP);
});
