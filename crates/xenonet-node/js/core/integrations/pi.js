// core/integrations/pi.js in Rust (crates/xenonet-integrations/src/pi.rs):
// Pi's sessions and pi-harness exports, read as metadata only.
import { native } from "../../index.js";
import { settle } from "./common.js";

export const piExportMetadata = (value) => settle(native.intgPiExport(value));

export const piSessionMetadata = (content, file) => settle(native.intgPiSession(String(content), String(file)));

export async function scanPi(root, { maxFiles = 500, maxBytes = 8 * 1024 * 1024 } = {}) {
  return native.intgScanPi(String(root), Number(maxFiles), Number(maxBytes));
}
