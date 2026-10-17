// core/export-labels.js in Rust (crates/xenonet-state/src/export_labels.rs):
// the map's own labels during an export of the day. The page keeps the JS,
// which it imports itself.
import { native } from "../index.js";

export const LABELLED_KINDS = new Set(native.labelledKinds());

export const exportLabel = (node, labels) => native.exportLabel(node, labels) ?? null;
