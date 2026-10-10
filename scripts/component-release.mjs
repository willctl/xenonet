// A Desktop-only release must not change the bytes behind an existing
// Service or Tools version. Each component owns its release history.
export function verifyComponentVersions(packages, published) {
  for (const item of packages) {
    const previous = published.find((p) => p.component === item.component && p.platform === item.platform && p.arch === item.arch);
    if (!previous) continue;
    const a = item.version.split(".").map(Number), b = previous.version.split(".").map(Number);
    const difference = a.findIndex((value, i) => value !== b[i]);
    if (difference >= 0 && a[difference] < b[difference]) throw new Error(`${item.component} cannot release an older version than ${previous.version}.`);
    if (item.version === previous.version && item.sha256 !== previous.sha256) throw new Error(`${item.component} ${item.version} has different bytes from its published release. Bump its version in components/versions.json.`);
  }
}

export async function publishedComponents(fetcher = fetch) {
  const response = await fetcher("https://github.com/willctl/xenonet/releases/latest/download/components.json", { signal: AbortSignal.timeout(30000) });
  if (response.status === 404) return [];
  if (!response.ok) throw new Error("Could not verify published component versions. Try the release build again later.");
  const manifest = await response.json();
  if (manifest.format !== 1 || !Array.isArray(manifest.packages)) throw new Error("Invalid published component manifest.");
  return manifest.packages;
}
