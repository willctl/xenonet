// Money, tokens, durations, shares and long text as people read them.
// Plain functions with no Node imports, so the console page uses them as
// well as the xenonet command.

export function money(n) {
  if (n == null) return "no price";
  if (n > 0 && n < 0.01) return "<$0.01";
  return `$${n.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
}

export function tokens(n) {
  const a = Math.abs(n ?? 0);
  const one = (v, unit) => `${v.toFixed(1).replace(/\.0$/, "")}${unit}`;
  if (a >= 1e9) return one(n / 1e9, "B");
  if (a >= 1e6) return one(n / 1e6, "M");
  if (a >= 1e3) return one(n / 1e3, "k");
  return String(Math.round(n ?? 0));
}

export function duration(ms) {
  if (ms == null) return "—";
  const m = Math.round(ms / 60000);
  if (m < 1) return `${Math.max(1, Math.round(ms / 1000))} s`;
  return m < 60 ? `${m} min` : `${Math.floor(m / 60)} h${m % 60 ? ` ${m % 60} min` : ""}`;
}

export const plural = (n, word, many = `${word}s`) => `${Number(n).toLocaleString("en-US")} ${n === 1 ? word : many}`;

export function share(part, whole) {
  if (!whole) return "";
  const p = part / whole;
  return p > 0 && p < 0.005 ? "<1%" : `${Math.round(p * 100)}%`;
}

// A long text cut at n characters, saying how much more there was.
export const excerpt = (s, n = 4000) => (s && s.length > n ? s.slice(0, n) + `\n… (${s.length - n} more chars)` : s ?? "");
