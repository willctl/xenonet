// The xenonet command's arguments: flags (--flag, --flag=value, --flag value
// for the flags that take one, --no-flag, -h, -v) anywhere, and the rest in
// order. Everything after -- is taken as it is. A flag that may come more
// than once (xenonet actor targets --add … --add …) collects its values.
const TAKES_VALUE = new Set([
  "port", "only", "limit", "dir", "instance", "seconds", "theme", "size", "out", "last", "since",
  // xenonet hosts
  "sshPort", "xenonetPort", "identity", "knownHosts", "hostKeyAlias", "command",
  // xenonet actor
  "name", "slug", "capabilities", "heartbeat", "product", "tokenFile", "presence", "status", "cwd", "pid",
  "target", "path", "count", "paths", "platform", "subject", "ref", "app", "add", "remove", "wait", "replyTo",
  // xenonet mcp assistant, xenonet assistant
  "grant",
  // xenonet gateway
  "tunnel", "host",
  // xenonet actor listen
  "exec",
]);
const REPEATS = new Set(["add", "remove", "paths"]);

export function parse(argv) {
  const flags = {};
  const rest = [];
  const set = (key, value) => (flags[key] = REPEATS.has(key) ? [...(flags[key] ?? []), value] : value);
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--") {
      rest.push(...argv.slice(i + 1));
      break;
    }
    if (a === "-h") flags.help = true;
    else if (a === "-v") flags.version = true;
    else if (a.startsWith("--")) {
      const [k, v] = a.slice(2).split(/=(.*)/s);
      const camel = (name) => name.replace(/-([a-z])/g, (_, ch) => ch.toUpperCase());
      const key = camel(k);
      if (v !== undefined) set(key, v);
      else if (TAKES_VALUE.has(key) && i + 1 < argv.length) set(key, argv[++i]);
      else if (k.startsWith("no-")) flags[camel(k.slice(3))] = false;
      else flags[key] = true;
    } else rest.push(a);
  }
  return { flags, rest };
}
