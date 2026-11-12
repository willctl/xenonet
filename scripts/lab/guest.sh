# The XenoNet lab's steps inside a VM, run as the lab user. lab.mjs sends this
# over SSH to `bash -s`, with the values a step needs set above this text, so
# nothing from the Mac's environment comes along.
set -euo pipefail
umask 022
export PATH="$HOME/.local/bin:$HOME/.local/node/bin:$HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin"
export DISABLE_AUTOUPDATER=1

say() { printf '  %s\n' "$*"; }
health() { curl -fsS -m 5 "http://127.0.0.1:$1/api/health"; }

# Node, Codex and Claude Code at the versions in versions.json, rustup for
# building XenoNet, the demo world's made-up git identity, and the checkout
# lab.mjs pushes XenoNet to.
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

  # rustup alone: the toolchain is the one the checkout's
  # rust-toolchain.toml pins, installed at each deploy.
  if ! command -v rustup >/dev/null; then
    curl --proto '=https' --tlsv1.2 -fsSL --retry 3 https://sh.rustup.rs | sh -s -- -y -q --profile minimal --default-toolchain none --no-modify-path
  fi
  say "$(rustup --version 2>/dev/null)"

  git config --global user.name Nova
  git config --global user.email nova@example.com
  git config --global init.defaultBranch main
  if [ ! -d "$HOME/xenonet/.git" ]; then
    git init -q -b lab "$HOME/xenonet"
    # A push to the branch that's checked out updates the files too.
    git -C "$HOME/xenonet" config receive.denyCurrentBranch updateInstead
  fi
}

# After a push: the xenonet binary built from it, the xenonet command, and
# the services' units. Services already enabled restart on the new build.
deploy() {
  cd "$HOME/xenonet"
  git log -1 --format='  xenonet %h %s'
  rustup toolchain install
  CARGO_PROFILE_DEV_DEBUG=0 cargo build -q -p xenonet-cli
  target/debug/xenonet install
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
ExecStart=%h/xenonet/target/debug/xenonet serve$1
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
  target/debug/xenonet serve --demo >/tmp/xenonet-smoke.log 2>&1 &
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


# One working VM's settings: its environment and its two XenoNets. The VM
# gets its host id in a network when the owner joins it to one.
vm() {
  mkdir -p "$HOME/.config/xenonet-lab"
  printf '%s' "$XENONET_ENV" >"$HOME/.config/xenonet-lab/xenonet.env"
  systemctl --user daemon-reload
  systemctl --user enable xenonet.service xenonet-demo.service
  systemctl --user restart xenonet.service xenonet-demo.service
  answering
}

# What `npm run lab -- status` shows, one fact per line.
facts() {
  local v
  echo "commit=$(git -C "$HOME/xenonet" log -1 --format='%h %s' 2>/dev/null || echo none)"
  for v in xenonet xenonet-demo; do echo "$v=$(systemctl --user is-active "$v" 2>/dev/null || true)"; done
  echo "health-47840=$(health 47840 2>/dev/null | head -c 60 || true)"
  echo "health-47841=$(health 47841 2>/dev/null | head -c 60 || true)"
  echo "lan=$(ip -o -4 addr show dev lima0 2>/dev/null | awk '{print $4}' | paste -sd, -)"
  echo "zone=$(timedatectl show -p Timezone --value 2>/dev/null || true)"
  # Read from the installs rather than run: starting Claude Code writes ids
  # into ~/.claude.json.
  echo "versions=$(node --version 2>/dev/null) · claude $(basename "$(readlink -f "$HOME/.local/bin/claude" 2>/dev/null)") · codex $(node -p 'require(process.env.HOME + "/.local/lib/node_modules/@openai/codex/package.json").version' 2>/dev/null)"
}

# The subjects of the commits the VM's XenoNet was built from, newest
# first: the transport checks look in them for the phase 5 commits they
# wait on.
subjects() {
  git -C "$HOME/xenonet" log --format=%s -n 5000 2>/dev/null || true
}

# The VM's addresses on the lab's LAN (lima0), as `ip -j` gives them.
lanaddrs() {
  ip -j addr show dev lima0
}

# This VM's XenoNet's network, as `xenonet net status --json` gives it, or
# nothing when the build or the service can't say.
netstate() {
  xenonet net status --json 2>/dev/null || true
}

# A stand-in session: a made-up Claude Code session in ~/Projects/$PROJECT,
# which XenoNet hears of through its hook (`xenonet hook claude`,
# SessionStart), and which calls the engine's tools through `xenonet mcp`,
# as a real session's would.
# CALLS is a JSON list of {name, arguments, until, within}: with until,
# the call is made again every half second, for `within` ms at most, until
# its answer has that text in it. Each answer is printed as a JSON line,
# with the VM's clock at the call. END=1 ends the session after the calls.
standin() {
  node - <<'EOF'
const { spawn, spawnSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const env = process.env;
const id = env.SESSION;
const cwd = path.join(env.HOME, "Projects", env.PROJECT);
const dir = path.join(env.HOME, ".claude", "projects", cwd.replace(/[^A-Za-z0-9]/g, "-"));
const transcript = path.join(dir, `${id}.jsonl`);
const out = (o) => process.stdout.write(JSON.stringify(o) + "\n");
const hook = (event, more) => spawnSync("xenonet", ["hook", "claude"], { input: JSON.stringify({ session_id: id, transcript_path: transcript, cwd, hook_event_name: event, ...more }), encoding: "utf8", timeout: 15000 });
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  if (env.REGISTER !== "0") {
    fs.mkdirSync(dir, { recursive: true });
    if (!fs.existsSync(transcript)) fs.writeFileSync(transcript, "");
    hook("SessionStart", { source: "startup" });
  }
  const calls = JSON.parse(env.CALLS || "[]");
  if (calls.length) {
    const mcp = spawn("xenonet", ["mcp"], { cwd, env: { ...env, CLAUDE_CODE_SESSION_ID: id, XENONET_PORT: "47840" }, stdio: ["pipe", "pipe", "ignore"] });
    const waiting = new Map();
    let buf = "";
    let n = 0;
    mcp.stdout.on("data", (d) => {
      buf += d;
      for (let i; (i = buf.indexOf("\n")) >= 0; ) {
        const line = buf.slice(0, i);
        buf = buf.slice(i + 1);
        try {
          const m = JSON.parse(line);
          if (waiting.has(m.id)) waiting.get(m.id)(m);
        } catch {}
      }
    });
    const rpc = (method, params, ms = 150000) =>
      new Promise((resolve, reject) => {
        const k = ++n;
        const timer = setTimeout(() => waiting.delete(k) && reject(new Error(`${method} got no answer`)), ms);
        waiting.set(k, (m) => (clearTimeout(timer), waiting.delete(k), resolve(m)));
        mcp.stdin.write(JSON.stringify({ jsonrpc: "2.0", id: k, method, params }) + "\n");
      });
    await rpc("initialize", { protocolVersion: "2025-11-25", capabilities: {}, clientInfo: { name: "xenonet-lab", version: "1" } }, 30000);
    mcp.stdin.write(JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" }) + "\n");
    out({ ready: true, at: Date.now() });
    for (const c of calls) {
      const until = Date.now() + (c.within ?? 0);
      for (;;) {
        const at = Date.now();
        const m = await rpc("tools/call", { name: c.name, arguments: c.arguments ?? {} });
        const text = (m.result?.content ?? []).map((x) => x.text ?? "").join("\n") || m.error?.message || "";
        const found = c.until ? text.includes(c.until) : undefined;
        if (!c.until || found || Date.now() >= until) {
          out({ name: c.name, at, done: Date.now(), error: Boolean(m.error || m.result?.isError), found, text });
          break;
        }
        await sleep(500);
      }
    }
    mcp.kill();
  }
  if (env.END === "1") {
    hook("SessionEnd", { reason: "other" });
    fs.rmSync(transcript, { force: true });
  }
}
main().catch((e) => {
  out({ error: String(e?.message ?? e) });
  process.exit(1);
});
EOF
}

# The lab's own relays for the relay checks: iroh-relay, built at the
# version versions.json pins with its server feature and its own lockfile,
# from a config file, with TLS from the lab's CA (made here once, in
# ~/.local/share/xenonet-lab/relay) on a certificate for this VM's lima0
# address. RELAY names one: a and b, which the lab's network lists, and
# elsewhere, which it doesn't. Each runs as a transient user service, so it
# stops with `relayoff` or the VM. The first build takes a few minutes.
RELAY_DIR="$HOME/.local/share/xenonet-lab/relay"

relay_ports() {
  case "$1" in
    a) echo "3340 3443 9090 true" ;;
    b) echo "3341 4443 9091 false" ;;
    elsewhere) echo "3342 5443 9092 false" ;;
    *) echo "no relay named $1" >&2; exit 1 ;;
  esac
}

relay_certs() {
  local ip="$1"
  mkdir -p "$RELAY_DIR"
  chmod 700 "$RELAY_DIR"
  command -v openssl >/dev/null || { echo "openssl isn't in this VM" >&2; exit 1; }
  if [ ! -s "$RELAY_DIR/ca.pem" ]; then
    openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -days 365 \
      -subj "/CN=XenoNet lab relay CA" -addext "basicConstraints=critical,CA:TRUE" -addext "keyUsage=critical,keyCertSign,cRLSign" \
      -keyout "$RELAY_DIR/ca.key" -out "$RELAY_DIR/ca.pem" 2>/dev/null
  fi
  if [ ! -s "$RELAY_DIR/relay.pem" ] || [ "$(cat "$RELAY_DIR/relay.ip" 2>/dev/null)" != "$ip" ]; then
    openssl req -new -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes -subj "/CN=$ip" \
      -keyout "$RELAY_DIR/relay.key" -out "$RELAY_DIR/relay.csr" 2>/dev/null
    printf 'subjectAltName=IP:%s\nbasicConstraints=CA:FALSE\nkeyUsage=critical,digitalSignature\nextendedKeyUsage=serverAuth\n' "$ip" >"$RELAY_DIR/relay.ext"
    openssl x509 -req -in "$RELAY_DIR/relay.csr" -CA "$RELAY_DIR/ca.pem" -CAkey "$RELAY_DIR/ca.key" -CAcreateserial \
      -days 90 -extfile "$RELAY_DIR/relay.ext" -out "$RELAY_DIR/relay.pem" 2>/dev/null
    echo "$ip" >"$RELAY_DIR/relay.ip"
  fi
}

relay() {
  if [ "$("$HOME/.local/bin/iroh-relay" --version 2>/dev/null | awk '{print $2}')" != "$RELAY_VERSION" ]; then
    (cd "$HOME/xenonet" && rustup toolchain install >/dev/null && cargo install -q --locked --features server --root "$HOME/.local" iroh-relay --version "$RELAY_VERSION")
  fi
  local ip http https metrics qad
  ip="$(ip -o -4 addr show dev lima0 | awk '{print $4}' | cut -d/ -f1 | head -1)"
  read -r http https metrics qad <<<"$(relay_ports "$RELAY")"
  relay_certs "$ip"
  cat >"$RELAY_DIR/$RELAY.toml" <<EOF
# The lab's relay $RELAY (scripts/lab/guest.sh).
enable_relay = true
http_bind_addr = "0.0.0.0:$http"
enable_quic_addr_discovery = $qad
enable_metrics = true
metrics_bind_addr = "127.0.0.1:$metrics"

[tls]
https_bind_addr = "0.0.0.0:$https"
quic_bind_addr = "0.0.0.0:7842"
cert_mode = "Manual"
manual_cert_path = "$RELAY_DIR/relay.pem"
manual_key_path = "$RELAY_DIR/relay.key"
EOF
  systemctl --user stop "xenonet-lab-relay-$RELAY.service" 2>/dev/null || true
  systemd-run --user -q --collect --unit="xenonet-lab-relay-$RELAY" "$HOME/.local/bin/iroh-relay" --config-path "$RELAY_DIR/$RELAY.toml"
  local i
  for i in $(seq 1 30); do
    if curl -fs -o /dev/null -m 2 --cacert "$RELAY_DIR/ca.pem" "https://$ip:$https/healthz"; then echo "relay=https://$ip:$https"; return 0; fi
    sleep 1
  done
  echo "iroh-relay $RELAY didn't answer on https://$ip:$https" >&2
  exit 1
}

# Stops the relay RELAY names, or every one of the lab's.
relayoff() {
  systemctl --user stop "xenonet-lab-relay-${RELAY:-*}.service" 2>/dev/null || true
}

# The lab's CA, base64 on one line, for the other VMs to trust.
relayca() {
  echo "ca=$(base64 -w0 "$RELAY_DIR/ca.pem")"
}

# The CA (CA_PEM, base64) in XenoNet's relay-ca.pem, the per-machine
# setting for relays with a CA of their own; XenoNet starts again when it
# changed, since the endpoint reads it when it starts.
trustca() {
  local dir="$HOME/.local/state/xenonet/network" pem
  pem="$(printf %s "$CA_PEM" | base64 -d)"
  mkdir -p "$dir"
  if [ "$(cat "$dir/relay-ca.pem" 2>/dev/null)" != "$pem" ]; then
    printf '%s\n' "$pem" >"$dir/relay-ca.pem"
    systemctl --user restart xenonet.service
    echo changed
  fi
}

# A relay of this machine's own (URL), in (ADD=1) or out.
ownrelay() {
  if [ "$ADD" = 1 ]; then xenonet net relay add "$URL" --mine --json >/dev/null; else xenonet net relay remove "$URL" --mine --json >/dev/null || true; fi
}

# The metrics of the relay RELAY names, from 127.0.0.1.
relaymetrics() {
  local http https metrics qad
  read -r http https metrics qad <<<"$(relay_ports "$RELAY")"
  curl -fsS -m 5 "http://127.0.0.1:$metrics/metrics" || true
}

# One UDP datagram, NONCE, to ADDR:PORT, or to the multicast group GROUP
# on lima0; and a listener that waits 5 s for it, printing `ready` once it
# listens, then `got` or `none`.
udpsend() {
  node - <<'EOF'
const dgram = require("dgram");
const os = require("os");
const { ADDR, GROUP, PORT, NONCE } = process.env;
const mine = (os.networkInterfaces().lima0 ?? []).find((a) => a.family === "IPv4")?.address;
const s = dgram.createSocket("udp4");
s.bind(0, mine, () => {
  if (GROUP) {
    s.setMulticastInterface(mine);
    s.setMulticastTTL(1);
  }
  s.send(NONCE, Number(PORT), GROUP || ADDR, (err) => {
    if (err) console.error(err.message);
    s.close();
  });
});
EOF
}

udplisten() {
  node - <<'EOF'
const dgram = require("dgram");
const os = require("os");
const { GROUP, PORT, NONCE } = process.env;
const mine = (os.networkInterfaces().lima0 ?? []).find((a) => a.family === "IPv4")?.address;
const s = dgram.createSocket({ type: "udp4", reuseAddr: true });
const done = (word) => (console.log(word), s.close(), process.exit(0));
s.on("message", (m) => String(m) === NONCE && done("got"));
s.bind(Number(PORT), GROUP ? undefined : mine, () => {
  if (GROUP) s.addMembership(GROUP, mine);
  console.log("ready");
  setTimeout(() => done("none"), 5000);
});
EOF
}

"$STEP"
