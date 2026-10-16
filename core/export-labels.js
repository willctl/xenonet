// The map's own labels during an export of the day, a film or a poster:
// star names in place of the names of projects, groups, folders, pages,
// shared systems and machines, and no label at all on a session, whose
// title is its user's prompt. Actors keep their names (Hermes Agent, Grok
// Bot, Muse, you): they name products, not the work. This runs in the page
// and in Node alike; redact.js makes the names.
export const LABELLED_KINDS = new Set(["core", "project", "zone", "native", "wiki", "page", "resource", "host"]);

// What a node's label shows under `labels` ({ id: star name }): the star
// name in capitals, as the map writes its labels, with a group's slash.
// Null means the label stays hidden: a session, or a node the names don't
// cover, such as one that appeared after they were made.
export function exportLabel(node, labels) {
  if (!node || !labels || !LABELLED_KINDS.has(node.kind)) return null;
  const name = labels[node.id];
  if (typeof name !== "string" || !name) return null;
  return node.kind === "project" && node.group ? `${name.toUpperCase()} /` : name.toUpperCase();
}
