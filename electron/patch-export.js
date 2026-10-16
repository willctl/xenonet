import { createHash } from "node:crypto";

// Keep the native save handler limited to a reviewed text patch. It never
// accepts a filesystem destination from the page; the save dialog chooses it.
export function patchExport(value = {}) {
  if (typeof value.patch !== "string" || Buffer.byteLength(value.patch) > 12 * 1024 * 1024 || !/^xenonet-[a-f0-9]{8}\.patch$/.test(value.filename ?? "")) return null;
  const { receipt } = value;
  if (!receipt || receipt.patchHash !== createHash("sha256").update(value.patch).digest("hex") || !/^[a-f0-9]{64}$/.test(receipt.baseline ?? "") || !/^[a-f0-9-]{36}$/.test(receipt.revision ?? "") || !Number.isFinite(receipt.retrievedAt)) return null;
  const metadata = JSON.stringify(receipt);
  if (Buffer.byteLength(metadata) > 256 * 1024) return null;
  return { filename: value.filename, content: `# XenoNet text review\n# Receipt: ${metadata}\n${value.patch}` };
}
