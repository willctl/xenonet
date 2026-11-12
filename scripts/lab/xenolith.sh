# Xenolith's steps inside a lab VM, run as the lab user, for the Xenolith checks
# (xenolith.mjs). lab.mjs sends this over SSH to `bash -s`, with the values a step needs set above
# this text, as it does guest.sh. Xenolith is built from the checkout lab.mjs pushed XenoNet to
# (~/xenonet/xenolith), and 1.10.0, for the rehearsal, from the tag pushed beside it into a
# worktree of its own (~/xenolith-1x). Each Xenolith here has its own home and folder under
# ~/.local/share/xenonet-lab, runs as a transient user service, and is never installed: nothing
# here touches a Xenolith a person uses.
set -euo pipefail
umask 022
export PATH="$HOME/.local/bin:$HOME/.local/node/bin:$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin"

LAB="$HOME/.local/share/xenonet-lab"
NEW="$HOME/xenonet/xenolith/rust/target/debug"
OLD="$HOME/xenolith-1x/xenolith/rust/target/debug"
# Where the lab's Xenoliths live, and the prefix of their services' names: the rehearsal keeps
# its own.
XROOT="$LAB/xenolith"
KIND=xenolith

xdir() { echo "$XROOT/$1"; }
unit_of() { echo "xenonet-lab-$KIND-$1"; }
# A command as the Xenolith NAME: its home and its folder.
xrun() {
  local d
  d="$(xdir "$1")"
  shift
  XENOLITH_HOME="$d/home" XENOLITH_DIR="$d/wiki" "$@"
}

# One value from JSON on stdin, by a dotted path ("upgrade.stage"), or nothing.
jget() {
  node -e 'let s = ""; process.stdin.on("data", (d) => (s += d)).on("end", () => { let v; try { v = JSON.parse(s); } catch { return; } for (const k of process.argv[1].split(".")) v = v?.[k]; process.stdout.write(v == null ? "" : String(v)); });' "$1"
}

# Whether the service NAME's home names answers, as the service that wrote its owner key.
answers() {
  local f="$1/runtime/owner.key" port pid
  [ -s "$f" ] || return 1
  port="$(jget port <"$f")"
  pid="$(jget pid <"$f")"
  [ -n "$port" ] && curl -fsS -m 3 "http://127.0.0.1:$port/status" 2>/dev/null | grep -q "\"pid\":$pid[,}]"
}

# The service of Xenolith NAME from the program BIN (2.0's xenolith-service, or 1.x's xenolith),
# on PORT (a free one when not given), until it answers (30 s at most).
xup() {
  local name="$1" bin="$2" d unit i
  d="$(xdir "$name")"
  unit="$(unit_of "$name")"
  systemctl --user stop "$unit" 2>/dev/null || true
  rm -f "$d/home/runtime/owner.key"
  systemd-run --user -q --collect --unit="$unit" --setenv=XENOLITH_HOME="$d/home" --setenv=XENOLITH_DIR="$d/wiki" --setenv=PATH="$PATH" "$bin" serve --http --port "${3:-0}"
  for i in $(seq 1 60); do
    if answers "$d/home"; then return 0; fi
    sleep 0.5
  done
  echo "Xenolith $name didn't answer in 30 s: $(journalctl --user -u "$unit" -n 3 --no-pager -o cat 2>/dev/null | tr '\n' ' ')" >&2
  return 1
}

xdown() {
  systemctl --user stop "$(unit_of "$1")" 2>/dev/null || true
}

# A new Xenolith NAME from BIN: its config (a curator that never runs, so notes wait in the inbox
# to be looked at; sync bound to BIND when given) and its folder.
xinit() {
  local name="$1" bin="$2" bind="${3:-}" d
  d="$(xdir "$name")"
  rm -rf "$d"
  mkdir -p "$d/home"
  node -e '
    const [file, bind] = process.argv.slice(1);
    const c = { writeMode: "curated", curator: { codexPath: "/bin/false", lint: "off", debounceSeconds: 86400, maxWaitSeconds: 86400 } };
    if (bind) c.sync = { bind };
    require("fs").writeFileSync(file, JSON.stringify(c, null, 2) + "\n");
  ' "$d/home/config.json" "$bind"
  xrun "$name" "$bin" serve --init >/dev/null
}

# ---------- the steps xenolith.mjs runs ----------

# 2.0 from the pushed checkout, once per commit: the CLI and the service, without the tray.
xbuild() {
  local head stamp="$NEW/.lab-built"
  head="$(git -C "$HOME/xenonet" rev-parse HEAD)"
  if [ -x "$NEW/xenolith" ] && [ -x "$NEW/xenolith-service" ] && [ "$(cat "$stamp" 2>/dev/null)" = "$head" ]; then
    echo "built=$head"
    return 0
  fi
  cd "$HOME/xenonet/xenolith/rust"
  rustup toolchain install >/dev/null
  CARGO_PROFILE_DEV_DEBUG=0 cargo build -q -p xenolith-cli -p xenolith-service
  echo "$head" >"$stamp"
  echo "built=$head"
}

# 1.10.0, from the tag lab.mjs pushed to the branch xenolith-1x, in a worktree of its own.
xold() {
  if [ -d "$HOME/xenolith-1x" ]; then
    git -C "$HOME/xenolith-1x" checkout -q --detach xenolith-1x
  else
    git -C "$HOME/xenonet" worktree add -q -f --detach "$HOME/xenolith-1x" xenolith-1x
  fi
  cd "$HOME/xenolith-1x/xenolith/rust"
  rustup toolchain install >/dev/null
  CARGO_PROFILE_DEV_DEBUG=0 cargo build -q -p xenolith-cli
  echo "version=$("$OLD/xenolith" --version)"
}

# A new Xenolith NAME (2.0), its service started; SPACE, when given, founded here with PAGE
# (base64) as pages/lab.md.
xreset() {
  xdown "$NAME"
  xinit "$NAME" "$NEW/xenolith" "${BIND:-}"
  if [ -n "${SPACE:-}" ]; then
    xrun "$NAME" "$NEW/xenolith" spaces add "$SPACE" >/dev/null
    printf %s "$PAGE" | base64 -d >"$(xdir "$NAME")/wiki/spaces/${SPACE%%:*}/${SPACE#*:}/pages/lab.md"
  fi
  xup "$NAME" "$NEW/xenolith-service"
}

# NAME's service, when NAME has been made (xreset) and isn't running.
xstart() {
  [ -d "$(xdir "$NAME")/home" ] || return 0
  if systemctl --user -q is-active "$(unit_of "$NAME")"; then return 0; fi
  xup "$NAME" "$NEW/xenolith-service"
}

xstop() {
  xdown "$NAME"
}

# `xenolith` as NAME, with ARGS (a JSON list) and INPUT on stdin: a code goes there, never into
# an argument.
xcli() {
  local args=()
  mapfile -t args < <(ARGS="$ARGS" node -e 'for (const a of JSON.parse(process.env.ARGS)) console.log(a)')
  printf '%s' "${INPUT:-}" | xrun "$NAME" "$NEW/xenolith" "${args[@]}"
}

# A file of NAME's folder (REL), or exit 3 when it isn't there.
xread() {
  local f
  f="$(xdir "$NAME")/wiki/$REL"
  [ -f "$f" ] || exit 3
  cat "$f"
}

xwrite() {
  local f
  f="$(xdir "$NAME")/wiki/$REL"
  mkdir -p "$(dirname "$f")"
  printf %s "$TEXT" | base64 -d >"$f.lab-tmp"
  mv "$f.lab-tmp" "$f"
}

# The Markdown files under REL in NAME's folder that hold NEEDLE, as a JSON list of their texts.
xfind() {
  node - "$(xdir "$NAME")/wiki/$REL" "$NEEDLE" <<'EOF'
const fs = require("fs");
const path = require("path");
const [dir, needle] = process.argv.slice(2);
const out = [];
const walk = (d) => {
  let entries = [];
  try {
    entries = fs.readdirSync(d, { withFileTypes: true });
  } catch {
    return;
  }
  for (const e of entries) {
    const p = path.join(d, e.name);
    if (e.isDirectory() && e.name !== ".history") walk(p);
    else if (e.isFile() && e.name.endsWith(".md")) {
      const t = fs.readFileSync(p, "utf8");
      if (t.includes(needle)) out.push(t);
    }
  }
};
walk(dir);
console.log(JSON.stringify(out));
EOF
}

# ---------- the rehearsal (phase 6 revision 33g) ----------

stage() { echo "stage=$1 $2 ${*:3}"; }
die() {
  stage "$1" FAIL "${*:2}"
  exit 1
}

# A digest of NAME's folder, without its lock files.
fp() {
  (cd "$(xdir "$1")/wiki" && find . -name .locks -prune -o -type f -print0 | sort -z | xargs -0 sha256sum | sha256sum | cut -c1-16)
}

# The rehearsal's services keep their ports across restarts, since the satellite's pairing names
# the curator's URL.
port_of() {
  case "$1" in
    inix) echo 47863 ;;
    neo) echo 47864 ;;
  esac
}

# A 1.x curator ("inix") with a page, and a 1.x satellite ("neo") paired with it, in this VM on
# 127.0.0.1 (1.x serves only there); a note queued on neo while inix is stopped; the cutover
# (both stopped, both copied); 2.0's dry run, then `upgrade --run`; the rollback in its written
# order (`upgrade --undo` with 2.0 still running, 2.0 stopped, the config and state folders put
# back from the copy, 1.x again); and 1.x syncing again, the queued note reaching inix. One
# `stage=<id> ok|FAIL <detail>` line a stage, stopping at the first FAIL. Everything it made is
# deleted afterwards.
xrehearse() {
  XROOT="$LAB/xenolith-rehearsal"
  KIND=rehearsal
  local O="$OLD/xenolith" N="$NEW/xenolith" S="$NEW/xenolith-service"
  local note m offer code port fpi fpn st i
  note="lab-queued-$(od -An -N6 -tx1 /dev/urandom | tr -d ' \n')"
  trap 'for m in inix neo; do xdown "$m"; done; rm -rf "$XROOT"' EXIT
  [ -x "$O" ] || die built "no 1.x build at $O"
  [ -x "$N" ] && [ -x "$S" ] || die built "no 2.0 build at $NEW"
  stage built ok "1.x $("$O" --version), 2.0 $("$N" --version)"

  for m in inix neo; do xdown "$m"; done
  rm -rf "$XROOT"
  for m in inix neo; do xinit "$m" "$O" || die 1x-synced "1.x's serve --init failed on $m"; done
  xrun inix "$O" spaces add project:atlas >/dev/null || die 1x-synced "1.x couldn't add project:atlas on inix"
  printf -- '---\ntitle: Atlas database\ntype: project\n---\n\n# Atlas database\n\nPostgres 16 on inix.\n' >"$(xdir inix)/wiki/spaces/project/atlas/pages/db.md"
  for m in inix neo; do xup "$m" "$O" "$(port_of "$m")" || die 1x-synced "1.x didn't answer on $m"; done
  offer="$(xrun inix "$O" pair offer --json)" || die 1x-synced "1.x's pair offer was refused on inix"
  port="$(port_of inix)"
  # The code goes from the offer to the join's stdin: never an argument, never this output.
  code="$(printf %s "$offer" | jget code)"
  printf '%s\n' "$code" | xrun neo "$O" pair join --url "http://127.0.0.1:$port" --name neo --peer inix --json >/dev/null || die 1x-synced "1.x's pair join failed on neo"
  code=""
  xup neo "$O" "$(port_of neo)" || die 1x-synced "1.x didn't answer on neo after the join"
  xrun neo "$O" sync --json >/dev/null || die 1x-synced "neo's 1.x sync pass failed"
  cmp -s "$(xdir inix)/wiki/spaces/project/atlas/pages/db.md" "$(xdir neo)/wiki/spaces/project/atlas/pages/db.md" || die 1x-synced "inix's page didn't reach neo"
  stage 1x-synced ok "neo paired with inix and holds its page"

  xdown inix
  xrun neo "$O" note "$note" --body "Queued on neo while inix was away." --space project:atlas >/dev/null || die queued "1.x refused the note on neo"
  xrun neo "$O" sync --json >/dev/null 2>&1 || true
  grep -rlqF -- "$note" "$(xdir neo)/wiki/spaces/project/atlas/inbox" || die queued "the note isn't waiting in neo's inbox"
  stage queued ok "a note waits in neo's 1.x inbox with inix stopped"

  xdown neo
  for m in inix neo; do cp -a "$(xdir "$m")" "$XROOT/$m.copy"; done
  fpi="$(fp inix)"
  fpn="$(fp neo)"
  # Two 2.0s in one VM, each on a port of its own, apart from the lab's own Xenolith.
  i=47861
  for m in inix neo; do
    node -e 'const f = process.argv[1]; const c = JSON.parse(require("fs").readFileSync(f, "utf8")); c.sync = { bind: process.argv[2] }; require("fs").writeFileSync(f, JSON.stringify(c, null, 2) + "\n");' "$(xdir "$m")/home/config.json" "127.0.0.1:$i"
    i=$((i + 1))
    xup "$m" "$S" "$(port_of "$m")" || die upgraded "2.0 didn't answer on $m"
  done
  for m in inix neo; do
    st="$(xrun "$m" "$N" upgrade --json | jget upgrade.stage)"
    [ "$st" = waiting ] || die upgraded "$m: 2.0's dry run says the upgrade is \"$st\", not waiting"
  done
  [ "$(fp inix)" = "$fpi" ] && [ "$(fp neo)" = "$fpn" ] || die upgraded "a 2.0 start changed a folder before upgrade --run"
  for m in inix neo; do
    xrun "$m" "$N" upgrade --run --json >/dev/null || die upgraded "$m: upgrade --run failed"
  done
  [ -f "$(xdir inix)/wiki/spaces/project/atlas/xenolith-space.json" ] || die upgraded "inix's project:atlas has no marker after --run"
  grep -rlqF -- "$note" "$(xdir neo)/home/1x-copies/project/atlas/inbox" || die upgraded "neo's queued note isn't kept with its moved-aside copy"
  stage upgraded ok "inix founded project:atlas; neo's copy moved aside with its queued note"

  for m in inix neo; do
    xrun "$m" "$N" upgrade --undo --json >/dev/null || die rolledback "$m: upgrade --undo failed"
  done
  for m in inix neo; do xdown "$m"; done
  for m in inix neo; do
    rm -rf "$(xdir "$m")/home"
    cp -a "$XROOT/$m.copy/home" "$(xdir "$m")/home"
  done
  [ "$(fp inix)" = "$fpi" ] || die rolledback "inix's folder isn't as 1.x left it after --undo"
  [ "$(fp neo)" = "$fpn" ] || die rolledback "neo's folder isn't as 1.x left it after --undo"
  stage rolledback ok "both folders as 1.x left them, byte for byte; config and state back from the copy"

  for m in inix neo; do xup "$m" "$O" "$(port_of "$m")" || die 1x-again "1.x didn't answer on $m again"; done
  xrun neo "$O" sync --json >/dev/null || die 1x-again "neo's 1.x sync pass failed"
  for i in $(seq 1 30); do
    if grep -rlqF --exclude-dir=.history -- "$note" "$(xdir inix)/wiki/spaces/project/atlas"; then
      cmp -s "$(xdir inix)/wiki/spaces/project/atlas/pages/db.md" "$(xdir neo)/wiki/spaces/project/atlas/pages/db.md" || die 1x-again "neo's copy of the page no longer matches inix's"
      stage 1x-again ok "neo's queued note reached inix's 1.x"
      return 0
    fi
    sleep 1
  done
  die 1x-again "neo's queued note didn't reach inix in 30 s"
}

"$STEP"
