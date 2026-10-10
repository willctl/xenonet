# The XenoNet lab's steps inside a VM, run as the lab user. lab.mjs sends this
# over SSH to `bash -s`, with the values a step needs set above this text, so
# nothing from the Mac's environment comes along.
set -euo pipefail
umask 022
export PATH="$HOME/.local/bin:$HOME/.local/node/bin:/usr/local/bin:/usr/bin:/bin"
export DISABLE_AUTOUPDATER=1

say() { printf '  %s\n' "$*"; }
health() { curl -fsS -m 5 "http://127.0.0.1:$1/api/health"; }

# Node, Codex and Claude Code at the versions in versions.json, the demo
# world's made-up git identity, and the checkout lab.mjs pushes XenoNet to.
tools() {
  [ "$(uname -m)" = aarch64 ] || { echo "versions.json pins Node for linux-arm64, and this VM is $(uname -m)" >&2; exit 1; }
  mkdir -p "$HOME/.local/bin"

  if [ "$(node --version 2>/dev/null || true)" != "v$NODE_VERSION" ]; then
    local tmp tarball="node-v$NODE_VERSION-linux-arm64.tar.xz"
    tmp="$(mktemp -d)"
    curl -fsSL --retry 3 -o "$tmp/$tarball" "https://nodejs.org/dist/v$NODE_VERSION/$tarball"
    curl -fsSL --retry 3 -o "$tmp/SHASUMS256.txt" "https://nodejs.org/dist/v$NODE_VERSION/SHASUMS256.txt"
    # The tarball has to match versions.json and the release's own list.
    (cd "$tmp" && printf '%s  %s\n' "$NODE_SHA256" "$tarball" | sha256sum -c --quiet - && grep -E "  $tarball\$" SHASUMS256.txt | sha256sum -c --quiet -)
    rm -rf "$HOME/.local/node.new"
    mkdir -p "$HOME/.local/node.new"
    tar -xJf "$tmp/$tarball" -C "$HOME/.local/node.new" --strip-components=1
    rm -rf "$HOME/.local/node" "$tmp"
    mv "$HOME/.local/node.new" "$HOME/.local/node"
  fi
  say "node $(node --version)"

  # Global packages go under ~/.local, so their commands land in
  # ~/.local/bin with claude's and xenonet's.
  npm config set prefix "$HOME/.local"
  npm config set fund false
  npm config set audit false
  npm config set update-notifier false
  if [ "$(codex --version 2>/dev/null || true)" != "codex-cli $CODEX_VERSION" ]; then
    npm install --global --loglevel=error "@openai/codex@$CODEX_VERSION"
  fi
  say "$(codex --version)"
  # Sign-ins go to ~/.codex/auth.json, as on the development Mac, rather
  # than a keyring.
  mkdir -p "$HOME/.codex"
  [ -e "$HOME/.codex/config.toml" ] || printf '%s\n' '# Sign-ins go to ~/.codex/auth.json, as on the development Mac.' 'cli_auth_credentials_store = "file"' >"$HOME/.codex/config.toml"

  # Claude Code's native installer, pinned. It stays at that version.
  if [ "$(claude --version 2>/dev/null | cut -d' ' -f1 || true)" != "$CLAUDE_VERSION" ]; then
    curl -fsSL https://claude.ai/install.sh | bash -s "$CLAUDE_VERSION"
  fi
  node -e '
    const fs = require("fs"), path = require("path"), file = process.argv[1];
    let s = {};
    try { s = JSON.parse(fs.readFileSync(file, "utf8")); } catch {}
    s.env = { ...s.env, DISABLE_AUTOUPDATER: "1" };
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, JSON.stringify(s, null, 2) + "\n");
  ' "$HOME/.claude/settings.json"
  say "claude $(claude --version)"

  git config --global user.name Nova
  git config --global user.email nova@example.com
  git config --global init.defaultBranch main
  if [ ! -d "$HOME/xenonet/.git" ]; then
    git init -q -b lab "$HOME/xenonet"
    # A push to the branch that's checked out updates the files too.
    git -C "$HOME/xenonet" config receive.denyCurrentBranch updateInstead
  fi
}

# After a push: XenoNet's dependencies, the xenonet command, and the services'
# units. Services already enabled restart on the new code.
deploy() {
  cd "$HOME/xenonet"
  git log -1 --format='  xenonet %h %s'
  npm ci --omit=dev --prefer-offline --no-audit --no-fund --loglevel=error
  node cli/xenonet.js install
  mkdir -p "$HOME/.config/systemd/user" "$HOME/.config/xenonet-lab"
  unit "" "XenoNet, the lab host's own" >"$HOME/.config/systemd/user/xenonet.service"
  unit " --demo" "XenoNet's demo, with made-up data" >"$HOME/.config/systemd/user/xenonet-demo.service"
  systemctl --user daemon-reload
  local u restarted=0
  for u in xenonet xenonet-demo; do
    if systemctl --user -q is-enabled "$u" 2>/dev/null; then systemctl --user restart "$u"; restarted=1; fi
  done
  if [ "$restarted" = 1 ]; then answering; fi
}

# Until both XenoNets answer, for a minute at most.
answering() {
  local i
  for i in $(seq 1 60); do
    if health 47840 >/dev/null 2>&1 && health 47841 >/dev/null 2>&1; then return 0; fi
    sleep 1
  done
}

unit() {
  cat <<EOF
# Written by the XenoNet lab (scripts/lab). Settings for this VM are in
# ~/.config/xenonet-lab/xenonet.env.
[Unit]
Description=$2

[Service]
Environment=PATH=%h/.local/bin:%h/.local/node/bin:/usr/local/bin:/usr/bin:/bin
EnvironmentFile=-%h/.config/xenonet-lab/xenonet.env
ExecStart=%h/.local/node/bin/node %h/xenonet/server/index.js$1
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
EOF
}

# The golden copy runs the demo server once, to see it answer, then forgets
# it: no XenoNet state goes into a golden copy for its clones to share.
smoke() {
  cd "$HOME/xenonet"
  node server/index.js --demo >/tmp/xenonet-smoke.log 2>&1 &
  local pid=$! i answer
  for i in $(seq 1 60); do
    if health 47841 >/dev/null 2>&1; then break; fi
    sleep 1
  done
  answer="$(health 47841)"
  say "the demo XenoNet answers: ${answer:0:100}…"
  kill "$pid"
  wait "$pid" 2>/dev/null || true
  forget
}

forget() {
  rm -rf "$HOME/.local/state/xenonet" "$HOME/.cache/xenonet" /tmp/xenonet-demo-* /tmp/xenonet-smoke.log
}

# Before a golden copy stops: leftovers from the installs, Claude Code's ids
# for this machine (each clone makes its own on first run, rather than all
# of them sharing one), and what the VM has downloaded since it booted.
seal() {
  forget
  rm -rf "$HOME/.claude/downloads" "$HOME/.claude/backups" "$HOME/.claude/sessions"
  node -e '
    const fs = require("fs"), file = process.argv[1];
    let s;
    try { s = JSON.parse(fs.readFileSync(file, "utf8")); } catch { process.exit(0); }
    for (const k of ["machineID", "userID", "firstStartTime"]) delete s[k];
    fs.writeFileSync(file, JSON.stringify(s, null, 2) + "\n", { mode: 0o600 });
  ' "$HOME/.claude.json"
  local nic
  nic="$(ip -o route show default | awk '{print $5; exit}')"
  say "downloaded since boot: $(( $(cat "/sys/class/net/$nic/statistics/rx_bytes") / 1000000 )) MB"
}

# One working VM's settings: its environment, the console's key restricted
# to the relay, its services, and its name.
vm() {
  mkdir -p "$HOME/.config/xenonet-lab"
  printf '%s' "$XENONET_ENV" >"$HOME/.config/xenonet-lab/xenonet.env"
  key
  systemctl --user daemon-reload
  systemctl --user enable xenonet.service xenonet-demo.service
  systemctl --user restart xenonet.service xenonet-demo.service
  answering
  name
}

# The console's key, which can run the relay and nothing else. It replaces
# the lab's earlier line, and leaves Lima's own key alone.
key() {
  mkdir -p -m 700 "$HOME/.ssh"
  touch "$HOME/.ssh/authorized_keys"
  { grep -v ' xenonet-lab-console$' "$HOME/.ssh/authorized_keys" || true; printf '%s\n' "$CONSOLE_KEY_LINE"; } >"$HOME/.ssh/authorized_keys.new"
  chmod 600 "$HOME/.ssh/authorized_keys.new"
  mv "$HOME/.ssh/authorized_keys.new" "$HOME/.ssh/authorized_keys"
  if command -v restorecon >/dev/null; then restorecon -R "$HOME/.ssh" || true; fi
}

# The host names itself with `xenonet hosts self`, once the branch has it.
name() {
  if xenonet hosts --help >/dev/null 2>&1; then
    xenonet hosts self "$HOST_ID"
  else
    say "xenonet hosts self $HOST_ID: not in this branch yet; lab push names it once it is"
  fi
}

# What `npm run lab -- status` shows, one fact per line.
facts() {
  local v
  echo "commit=$(git -C "$HOME/xenonet" log -1 --format='%h %s' 2>/dev/null || echo none)"
  for v in xenonet xenonet-demo; do echo "$v=$(systemctl --user is-active "$v" 2>/dev/null || true)"; done
  echo "health-47840=$(health 47840 2>/dev/null | head -c 60 || true)"
  echo "health-47841=$(health 47841 2>/dev/null | head -c 60 || true)"
  echo "relay=$(sed -n 's/.*command="\([^"]*\)".* xenonet-lab-console$/\1/p' "$HOME/.ssh/authorized_keys" 2>/dev/null | head -1)"
  echo "zone=$(timedatectl show -p Timezone --value 2>/dev/null || true)"
  # Read from the installs rather than run: starting Claude Code writes ids
  # into ~/.claude.json.
  echo "versions=$(node --version 2>/dev/null) · claude $(basename "$(readlink -f "$HOME/.local/bin/claude" 2>/dev/null)") · codex $(node -p 'require(process.env.HOME + "/.local/lib/node_modules/@openai/codex/package.json").version' 2>/dev/null)"
}

"$STEP"
