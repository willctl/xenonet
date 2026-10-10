export function escapeHtml(s) {
  return String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
}

export function timeAgo(ms, now = Date.now()) {
  if (!ms) return "—";
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 45) return "just now";
  const m = Math.round(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.round(m / 60);
  if (h < 36) return `${h}h ago`;
  const d = Math.round(h / 24);
  if (d < 45) return `${d}d ago`;
  return new Date(ms).toLocaleDateString();
}

export function stamp(ms) {
  if (!ms) return "—";
  const d = new Date(ms);
  return `${d.toLocaleDateString(undefined, { month: "short", day: "numeric" })} ${d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" })}`;
}

export function clock(ms) {
  return ms ? new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" }) : "";
}

// Paths under the home folder show as "~", so screenshots don't carry the
// user name. The graph's core node says where home is.
let home = null;
export function setHome(dir) {
  home = dir ? String(dir).replace(/[\\/]+$/, "") : null;
}

export function tildePath(p) {
  const s = String(p ?? "").replace(/^\\\\\?\\/, "");
  if (!home) return s;
  const a = s.replace(/\\/g, "/").toLowerCase();
  const b = home.replace(/\\/g, "/").toLowerCase();
  return a === b ? "~" : a.startsWith(b + "/") ? "~" + s.slice(home.length) : s;
}

export function shortPath(p, max = 48) {
  if (!p) return "—";
  const s = tildePath(p);
  return s.length > max ? "…" + s.slice(-(max - 1)) : s;
}

// Just enough Markdown for transcripts: fenced code, inline code, bold, headings.
// Everything is escaped first; no raw HTML from transcripts reaches the page.
export function renderText(text) {
  const parts = String(text ?? "").split(/```[^\n]*\n?/);
  return parts
    .map((part, i) => {
      if (i % 2 === 1) return `<pre class="code">${escapeHtml(part.replace(/\n$/, ""))}</pre>`;
      return escapeHtml(part)
        .replace(/`([^`\n]+)`/g, "<code>$1</code>")
        .replace(/\*\*([^*\n]+)\*\*/g, "<strong>$1</strong>")
        .replace(/^#{1,6}\s+(.+)$/gm, '<strong class="h">$1</strong>');
    })
    .join("");
}
