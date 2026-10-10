import { DEFAULT_ENVIRONMENT, environmentOf } from "../core/environments.js";
const STORE = "xenonet.environment";
let inUse = DEFAULT_ENVIRONMENT;
try { inUse = environmentOf(localStorage.getItem(STORE)).id; } catch {}
let shown = inUse;
let preview = null;
const listeners = new Set();
function draw(id) {
  const next = environmentOf(id);
  if (shown !== next.id) { shown = next.id; for (const fn of listeners) fn(next); }
  if (typeof document !== "undefined") document.documentElement.dataset.environment = shown;
}
export const environmentState = () => ({ id: shown, inUse, preview });
export const onEnvironment = (fn) => { listeners.add(fn); return () => listeners.delete(fn); };
export function syncEnvironment(id) {
  inUse = environmentOf(id).id;
  try { localStorage.setItem(STORE, inUse); } catch {}
  if (!preview) draw(inUse);
}
export function previewEnvironment(id) { preview = environmentOf(id).id; draw(preview); }
export function revertEnvironment() { preview = null; draw(inUse); }
export function useEnvironment(id) { preview = null; syncEnvironment(id); }
draw(shown);
