// Secrets on the page (a token, or a message that carries one), masked by
// default, so a screenshot, a screen share or an agent looking at the
// window doesn't catch one by accident. The value stays here, out of the
// page's HTML, until you choose Show, and then for ten seconds, or until
// the window loses focus. Copy copies it without showing it. While
// identities are hidden (Hide identities) or the driver holds the page
// still for a screenshot, a shown one is masked again (actors.css).
import { escapeHtml as h } from "./format.js";

const SHOW_MS = 10e3;
const DOTS = "••••••••••••";
const vault = new Map(); // id → { plain, masked, timer }

// A token masked, its kind (xenonet_gateway_, xenonet_actor_) kept so you can
// tell which it is.
export const maskOf = (value) => `${/^xenonet_[a-z]+_/.exec(String(value))?.[0] ?? ""}${DOTS}`;

// Keeps a secret for the views that show it: `value` is the secret, and
// `text`, when given, the words it sits in (a message to paste).
export function keepSecret(id, value, text = value) {
  const had = vault.get(id);
  if (had?.plain === text) return;
  clearTimeout(had?.timer);
  vault.set(id, { plain: text, masked: String(text).split(value).join(maskOf(value)), timer: null });
}

// Lets every secret go, as a view or the panel closes.
export function forgetSecrets() {
  for (const s of vault.values()) clearTimeout(s.timer);
  vault.clear();
}

export const secretValue = (id) => vault.get(id)?.plain ?? null;

// The secret as it shows: masked, with Show. `attrs` go on its text.
export function secretHtml(id, { attrs = "", block = false } = {}) {
  const s = vault.get(id);
  if (!s) return "";
  return `<div class="secret${block ? " block" : ""}" data-secret="${h(id)}"><code class="secret-text" ${attrs}>${h(s.masked)}</code><button class="secret-show" data-secret-show="${h(id)}" aria-pressed="false" title="Show it for ten seconds">Show</button></div>`;
}

function draw(root, id, shown) {
  const s = vault.get(id);
  const box = root.querySelector(`[data-secret="${CSS.escape(id)}"]`);
  if (!s || !box) return;
  const text = box.querySelector(".secret-text");
  const btn = box.querySelector("[data-secret-show]");
  box.classList.toggle("shown", shown);
  // Shown, the masked words stay beside the plain ones, for the CSS that
  // masks it again while identities are hidden or a screenshot is taken.
  text.innerHTML = shown ? `<span class="secret-plain">${h(s.plain)}</span><span class="secret-masked">${h(s.masked)}</span>` : h(s.masked);
  btn.textContent = shown ? "Hide" : "Show";
  btn.setAttribute("aria-pressed", String(shown));
}

function hideAll(root) {
  for (const [id, s] of vault) {
    clearTimeout(s.timer);
    s.timer = null;
    draw(root, id, false);
  }
}

// Show and Copy, for a click in `root`: whether it was one of theirs. Copy
// says how it went with `said(ok, what)`.
export async function secretClick(root, target, said) {
  const show = target.closest("[data-secret-show]");
  if (show) {
    const id = show.dataset.secretShow;
    const s = vault.get(id);
    if (!s) return true;
    const shown = !show.closest(".secret").classList.contains("shown");
    clearTimeout(s.timer);
    s.timer = shown ? setTimeout(() => draw(root, id, false), SHOW_MS) : null;
    draw(root, id, shown);
    return true;
  }
  const copy = target.closest("[data-secret-copy]");
  if (copy) {
    const value = secretValue(copy.dataset.secretCopy);
    const ok = value != null && (await Promise.resolve(navigator.clipboard?.writeText(value)).then(() => !!navigator.clipboard, () => false));
    said(ok, copy.dataset.secretCopy);
    return true;
  }
  return false;
}

// Masks every shown secret in `root` again when the window loses focus,
// as when you switch to another app to paste.
export function hideOnBlur(root) {
  const off = () => hideAll(root);
  window.addEventListener("blur", off);
  return () => window.removeEventListener("blur", off);
}
