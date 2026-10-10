// The made-up repos in each lab host's ~/Projects, from the demo world's
// names and remotes (core/demo.js), laid out as docs/design/vm-lab.md says:
// atlas-api on every host with the same remote, so its checkouts share one
// repo key; the other repos on one host each; and on the Linux hosts, a
// folder whose name has a space and an accent, a worktree beside its repo,
// and a pnpm workspace. Histories are fixed (authors, dates, messages), so
// atlas-api has the same commits on every host, as clones of one repo
// would. The remotes are never fetched from or pushed to.
//
// reposScript(host) returns a bash script that lab.mjs runs in the VM.

const NOVA = { name: "Nova", email: "nova@example.com" };
const ATLAS = { name: "Nova", email: "nova@atlas.example" };
const ORBIT = { name: "Orbit", email: "hello@orbitworks.example" };
const LINUX = ["vm1", "vm2", "vm3"];

export const REPOS = [
  {
    path: "atlas-api",
    hosts: [...LINUX, "mac1"],
    who: ATLAS,
    remote: "https://github.com/atlas-co/atlas-api.git",
    commits: [
      ["2026-09-01T09:12:00Z", "Start the orders API", {
        "README.md": "# atlas-api\n\nOrders, inventory and payments API for the Atlas store.\n",
        "package.json": '{ "name": "atlas-api", "private": true }\n',
        "tsconfig.json": "{}\n",
        "src/orders/routes.ts": "export const routes = [];\n",
      }],
      ["2026-09-04T15:30:00Z", "Add full refunds", { "src/orders/refunds.ts": "export function refund() {}\n" }],
    ],
    dirty: { "src/orders/routes.ts": "export const routes = [refundRoute];\n" },
    // A worktree checked out beside its repo, on the Linux hosts.
    worktree: {
      path: "atlas-api-refunds",
      branch: "feat/partial-refunds",
      hosts: LINUX,
      commits: [["2026-09-08T11:05:00Z", "Start partial refunds", { "src/orders/refunds.ts": "export function refund(amount) {}\n" }]],
      dirty: { "src/orders/refunds.test.ts": 'test.todo("a second partial refund");\n' },
    },
  },
  {
    path: "atlas-web",
    hosts: ["vm1"],
    who: ATLAS,
    remote: "https://github.com/atlas-co/atlas-web.git",
    commits: [
      ["2026-09-02T10:00:00Z", "Start the storefront", {
        "README.md": "# atlas-web\n\nThe Atlas storefront, built with Next.js.\n",
        "package.json": '{ "name": "atlas-web", "private": true }\n',
        "tsconfig.json": "{}\n",
      }],
      ["2026-09-06T16:45:00Z", "Add the checkout page", { "app/checkout/page.tsx": "export default function Checkout() {}\n" }],
    ],
    dirty: { "app/checkout/page.tsx": "export default function Checkout() { /* payments v3 */ }\n" },
  },
  // A folder of experiments, not a repo, around the repos in it.
  { path: "labs", hosts: ["vm1"], files: { "ideas.md": "- geofences per postcode\n- simulate a day of routes\n" } },
  {
    path: "labs/geofence",
    hosts: ["vm1"],
    who: ORBIT,
    remote: "https://github.com/orbitworks/geofence.git",
    commits: [
      ["2026-09-03T08:20:00Z", "Start geofence", {
        "README.md": "# geofence\n\nDelivery zones drawn from postcodes.\n",
        "package.json": '{ "name": "geofence", "private": true }\n',
        "src/zones.ts": "export const zones = [];\n",
      }],
    ],
    dirty: { "src/zones.ts": "export const zones = loadPostcodes();\n" },
  },
  {
    path: "ledger",
    hosts: ["vm2"],
    who: NOVA,
    remote: "https://github.com/nova-dev/ledger.git",
    commits: [
      ["2026-09-01T13:00:00Z", "Start the ledger", {
        "README.md": "# ledger\n\nDouble-entry bookkeeping service with monthly reconciliation.\n",
        "pyproject.toml": '[project]\nname = "ledger"\n',
      }],
      ["2026-09-05T09:40:00Z", "Reconcile each month", { "ledger/reconcile.py": "def reconcile(): ...\n" }],
    ],
    dirty: { "ledger/reconcile.py": "def reconcile(rounding='half-even'): ...\n" },
  },
  {
    path: "docs",
    hosts: ["vm2"],
    who: ATLAS,
    remote: "https://github.com/atlas-co/docs.git",
    commits: [
      ["2026-09-02T17:10:00Z", "Start the docs site", {
        "README.md": "# docs\n\nDocs site for Atlas and Ledger.\n",
        "package.json": '{ "name": "docs", "private": true }\n',
      }],
      ["2026-09-07T12:00:00Z", "Outline the refunds guide", { "guides/refunds.md": "# Refunds\n" }],
    ],
    dirty: { "guides/refunds.md": "# Refunds\n\nFull refunds, and partial refunds once they land.\n" },
  },
  {
    path: "pathfinder",
    hosts: ["mac1"],
    who: ORBIT,
    remote: "https://github.com/orbitworks/pathfinder.git",
    commits: [
      ["2026-09-01T07:30:00Z", "Start the route planner", {
        "README.md": "# pathfinder\n\nRoute planner for delivery vans.\n",
        "Cargo.toml": '[package]\nname = "pathfinder"\n',
        "src/solver.rs": "pub fn solve() {}\n",
      }],
    ],
    dirty: { "src/solver.rs": "pub fn solve() { /* cache the distance matrix */ }\n" },
  },
  // A folder whose name has a space and an accent. No remote: it's local.
  {
    path: "Café Zones",
    hosts: LINUX,
    who: NOVA,
    commits: [
      ["2026-09-03T19:00:00Z", "List the cafés' delivery zones", {
        "README.md": "# Café Zones\n\nDelivery zones for the cafés along the van routes.\n",
        "zones.csv": "zone,postcode\nnorth,1010\n",
      }],
    ],
    dirty: { "zones.csv": "zone,postcode\nnorth,1010\nharbour,1011\n" },
  },
  // A pnpm workspace, with no packages installed. No remote: it's local.
  {
    path: "atlas-kit",
    hosts: LINUX,
    who: ATLAS,
    commits: [
      ["2026-09-04T10:30:00Z", "Start the UI kit", {
        "README.md": "# atlas-kit\n\nShared UI for the Atlas storefront.\n",
        "package.json": '{ "name": "atlas-kit", "private": true, "packageManager": "pnpm@9.15.0" }\n',
        "pnpm-workspace.yaml": 'packages:\n  - "packages/*"\n',
        "packages/button/package.json": '{ "name": "@atlas-kit/button", "version": "0.3.0" }\n',
        "packages/button/src/index.ts": "export const Button = () => null;\n",
      }],
      ["2026-09-09T14:15:00Z", "Add theme tokens", {
        "packages/theme/package.json": '{ "name": "@atlas-kit/theme", "version": "0.1.0" }\n',
        "packages/theme/src/tokens.ts": "export const tokens = {};\n",
      }],
    ],
    dirty: { "packages/button/src/index.ts": "export const Button = ({ label }) => label;\n" },
  },
];

const q = (s) => `'${String(s).replace(/'/g, `'\\''`)}'`;
const P = (rel) => `"$P"/${q(rel)}`;
const dirOf = (rel) => rel.split("/").slice(0, -1).join("/");

export const reposFor = (host) => REPOS.filter((r) => r.hosts.includes(host));

function writeFiles(lines, base, files) {
  for (const [file, content] of Object.entries(files)) {
    const rel = `${base}/${file}`;
    lines.push(`mkdir -p ${P(dirOf(rel))}`, `printf '%s' ${q(content)} >${P(rel)}`);
  }
}

function commits(lines, base, list) {
  for (const [date, message, files] of list) {
    writeFiles(lines, base, files);
    lines.push(`commit ${P(base)} ${q(date)} ${q(message)}`);
  }
}

export function reposScript(host) {
  const repos = reposFor(host);
  if (!repos.length) throw new Error(`the lab has no repos for "${host}"`);
  const lines = [
    `# Made-up repos for the XenoNet lab's ${host}, from scripts/lab/fixtures.mjs.`,
    "set -euo pipefail",
    'P="$HOME/Projects"',
    'mkdir -p "$P"',
    "commit() {",
    '  git -C "$1" add -A',
    '  GIT_AUTHOR_DATE="$2" GIT_COMMITTER_DATE="$2" git -C "$1" -c commit.gpgsign=false commit -q -m "$3"',
    "}",
    "# What an earlier run made goes first, so a run always ends the same.",
  ];
  for (const r of repos) {
    lines.push(`rm -rf -- ${P(r.path)}`);
    if (r.worktree?.hosts.includes(host)) lines.push(`rm -rf -- ${P(r.worktree.path)}`);
  }
  for (const r of repos) {
    lines.push(`mkdir -p ${P(r.path)}`);
    if (!r.commits) {
      writeFiles(lines, r.path, r.files);
      continue;
    }
    lines.push(
      `git init -q -b main ${P(r.path)}`,
      `git -C ${P(r.path)} config user.name ${q(r.who.name)}`,
      `git -C ${P(r.path)} config user.email ${q(r.who.email)}`,
    );
    if (r.remote) lines.push(`git -C ${P(r.path)} remote add origin ${q(r.remote)}`);
    commits(lines, r.path, r.commits);
    const wt = r.worktree?.hosts.includes(host) ? r.worktree : null;
    if (wt) {
      lines.push(`git -C ${P(r.path)} branch ${q(wt.branch)}`, `git -C ${P(r.path)} worktree add -q ${P(wt.path)} ${q(wt.branch)}`);
      commits(lines, wt.path, wt.commits);
      writeFiles(lines, wt.path, wt.dirty);
    }
    writeFiles(lines, r.path, r.dirty);
  }
  lines.push(`printf '  ~/Projects: %s\\n' ${q(repos.map((r) => r.path).join(", "))}`);
  return lines.join("\n") + "\n";
}
