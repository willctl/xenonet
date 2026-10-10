// host/*: hosts, the other machines whose XenoNets a console shows
// (docs/design/hosts.md, contracts section 2). hosts.json in the state
// folder is a host/list/v1: this XenoNet's own host id and key, and the
// hosts it shows. A relay's first frame over its SSH connection is a
// host/hello/v1: which XenoNet is talking, and how.
import { define } from "./registry.js";
import { z, obj, ms } from "./common.js";
import { HOST_ID } from "./ids.js";

// The key a host makes once, at random, to be recognized across renames
// and SSH aliases. It names a host; it proves nothing (SSH does that).
export const HOST_KEY = /^[0-9a-f]{32}$/;
// What ssh's config and command line take as they are: a host name or
// address, a user name, a file (absolute or under ~/, with nothing ssh
// would expand or a line could break on), and how far back sessions go.
export const SSH_HOSTNAME = /^[A-Za-z0-9][A-Za-z0-9._-]{0,252}$/;
export const SSH_USER = /^[A-Za-z0-9_][A-Za-z0-9._-]{0,63}$/;
export const SSH_FILE = /^(?:\/|~\/)[^"\\%\u0000-\u001f\u007f]{1,1022}$/;
export const XENONET_COMMAND = /^(?:xenonet|\/[A-Za-z0-9._+/-]{1,1023})$/;
export const SINCE = /^\d{1,4}[mhd]$/;

const HostId = z.string().regex(HOST_ID).describe("A name someone chose, ^[a-z][a-z0-9-]{1,31}$: never the machine's own name, which often holds a person's");
const HostKey = z.string().regex(HOST_KEY).describe("Made once, at random, by the host: it recognizes the host across renames and SSH aliases. It names the host and proves nothing; SSH does that");
const Port = z.int().min(1).max(65535);

const Ssh = obj({
  hostname: z.string().regex(SSH_HOSTNAME).describe("ssh's HostName: the machine's name or address"),
  user: z.string().regex(SSH_USER).nullable(),
  port: Port.nullable().describe("ssh's port, when it isn't 22"),
  identity: z.string().regex(SSH_FILE).nullable().describe("A key of its own for this host (IdentityFile, with IdentitiesOnly)"),
  knownHosts: z.string().regex(SSH_FILE).nullable().describe("A known_hosts file of its own (UserKnownHostsFile)"),
  hostKeyAlias: z.string().regex(SSH_HOSTNAME).nullable().describe("The name its host key is kept under (HostKeyAlias)"),
}).describe("How ssh reaches the host, as XenoNet writes it into its own ssh config");

export const HostEntry = obj({
  id: HostId.describe("This console's name for the host, which its ids carry: @<id>:…"),
  transport: z.enum(["ssh", "local"]).describe("ssh: ssh -F <XenoNet's ssh config> <id> xenonet relay; local: the relay of another XenoNet on this machine"),
  ssh: Ssh.nullable().describe("Null for local"),
  port: Port.describe("The port of the XenoNet it reads on that machine"),
  command: z.string().regex(XENONET_COMMAND).nullable().describe("The xenonet command on the host: xenonet, or its absolute path where commands run over SSH don't find it; null for local"),
  since: z.string().regex(SINCE).describe("How far back the relay sends sessions: 7d, 12h, 90m"),
  detail: z.boolean().describe("Whether live detail crosses too (a waiting question's text, the last message, tool details, files touched), or only titles, states and counts"),
  start: z.boolean().optional().describe("The relay starts a XenoNet there, for as long as it runs, when none is running: for a machine with no service set up"),
  key: HostKey.nullable().describe("The host's key, learned the first time it answered"),
  name: HostId.nullable().describe("What the host calls itself, when that isn't the id here"),
  addedAt: ms,
});

export const HostList = define(
  "host/list",
  1,
  obj({
    schema: z.literal("host/list/v1"),
    self: obj({
      id: HostId.nullable().describe("The name this XenoNet gave itself (xenonet hosts self), or null"),
      key: HostKey,
      createdAt: ms,
    })
      .nullable()
      .describe("This XenoNet as a host; null until hosts are first used"),
    hosts: z.array(HostEntry).describe("The hosts this XenoNet shows"),
  }),
  {
    title: "Hosts",
    description: "hosts.json in XenoNet's state folder: this XenoNet's own host id and key, and the other machines' XenoNets it shows, with how to reach each.",
  },
);

export const HostHello = define(
  "host/hello",
  1,
  obj({
    t: z.literal("hello"),
    schema: z.literal("host/hello/v1"),
    protocol: z.int().min(1).describe("The relay's protocol: a console refuses one it doesn't speak, naming the host's version"),
    version: z.string().min(1).max(64).describe("The host's XenoNet"),
    host: obj({ id: HostId.nullable().describe("What the host calls itself, or null when it hasn't been named"), key: HostKey }),
    platform: z.string().min(1).max(32).describe("process.platform on the host: darwin, linux, win32"),
    demo: z.boolean().describe("The host shows made-up data"),
    now: ms.describe("The host's clock, as the hello leaves it"),
    sinceMs: z.int().min(0).nullable().describe("How far back the relay sends sessions, or null for all"),
    detail: z.boolean().describe("Whether live detail crosses, or only titles, states and counts"),
  }),
  {
    title: "A relay's hello",
    description: "The first frame xenonet relay writes on its standard output: which XenoNet is talking, in which protocol, and what it will send.",
  },
);
