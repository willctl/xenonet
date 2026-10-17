// The module a twin uses beside it: core/<path>'s own twin when there is
// one, else the JS. The resolve hook leaves a twin's own imports alone, so
// a twin importing another module through this gets the one copy the rest
// of XenoNet gets, whichever of the two that is.
import fs from "node:fs";

export function peer(file) {
  const twin = new URL(`./core/${file}`, import.meta.url);
  return fs.existsSync(twin) ? twin.href : new URL(`../../../core/${file}`, import.meta.url).href;
}
