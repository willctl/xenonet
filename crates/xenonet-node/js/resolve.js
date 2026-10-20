// The resolve hook register.js installs. An import of core/<path>.js loads
// ./core/<path>.js when there is one, from the tests, from the JS that
// imports the module, and from other twins alike, so the service has one of
// each module whoever imports it (a twin that loaded another's JS would keep
// a second copy of its state: config's folders, say). A twin's import of its
// own module is the JS it wraps, and is left alone.
import fs from "node:fs";

// XENONET_TWIN_LOG names a file each twin loaded is written to, one a line,
// so the parity runner knows which twins a test file reached.
const log = process.env.XENONET_TWIN_LOG;

const core = new URL("../../../core/", import.meta.url).href;
const twins = new URL("./core/", import.meta.url).href;
const known = new Map();

const hasTwin = (url) => {
  if (!known.has(url)) known.set(url, fs.existsSync(new URL(url)));
  return known.get(url);
};

export async function resolve(specifier, context, next) {
  const found = await next(specifier, context);
  if (log && found.url.startsWith(twins)) fs.appendFileSync(log, `core/${found.url.slice(twins.length)}\n`);
  if (!found.url.startsWith(core)) return found;
  const [file, query = ""] = found.url.slice(core.length).split(/(?=[?#])/);
  const twin = twins + file;
  if (context.parentURL?.split(/[?#]/)[0] === twin || !hasTwin(twin)) return found;
  if (log) fs.appendFileSync(log, `core/${file}\n`);
  return { ...found, url: twin + query };
}
