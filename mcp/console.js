// xenonet-console, the console operator: an MCP server through which SI
// (Claude Code, Codex and any other MCP client) can launch, drive and see
// XenoNet.
// Installed Tools operate the installed app; this checkout also offers
// synthetic development instances. Agent coordination lives in mcp/xenonet.js.
// It speaks MCP over stdio, through the official SDK,
// which serves clients of both the 2025 and the 2026-07-28 protocol.
// .mcp.json registers it for Claude Code, .codex/config.toml for Codex. Every
// tool goes through core/control.js, like xenonet console and the end-to-end
// tests.
//
// Tools act on one XenoNet Dev instance: demo (made-up data, the default), dev
// (your real sessions) or test. A tool's `instance` picks one, and it stays
// picked for later calls.
import { McpServer } from "@modelcontextprotocol/server";
import { serveStdio } from "@modelcontextprotocol/server/stdio";
import fs from "node:fs";
import path from "node:path";
import * as z from "zod/v4";
import { xenonet, INSTANCES, root } from "../core/control.js";

const pkg = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));

const INSTRUCTIONS = pkg.xenonetComponent === "tools" ? `XenoNet is the network for SI (super intelligence) on this machine, with a background service, a CLI, MCP tools and a desktop console. These tools operate installed Desktop. They launch it, find sessions, inspect panels, control terminals and take screenshots. Each action waits for the view to settle and returns the new state. Use logs to read page errors. The service, terminals and coordination keep running when Desktop closes or restarts, and window actions report that Desktop is unavailable until it returns. Allowing actors, connecting SI and trusting hosts are the person's to do. This console has no simulation and no arbitrary page execution.` : `XenoNet is the network for SI (super intelligence) on this machine, with a background service, a CLI, MCP tools and a desktop console. These tools operate installed XenoNet and development instances. Use installed for the person's app, or demo for made-up data. The service and the coordination between SI sessions keep running when Desktop closes or restarts. Simulation and arbitrary page execution work only in development instances.

The loop: \`launch\` (the demo instance, with made-up data, unless you need the user's real sessions: instance "dev"), then act with \`find\`, \`select\`, \`navigate\`, \`command\` and \`input\`, then \`screenshot\` to look. Every action waits until the view has settled (the layout cooled, the camera and any gathering at rest) and returns the new state, so don't sleep between calls. \`simulate\` puts sessions into rare states (a question waiting, an edit collision, a stuck loop) and has actors act. \`actors\` lists the actors beside the map, and \`feed\` reads the activity feed. After a change, check \`logs\` for page errors.

XenoNet Dev hot-reloads: saving under src/ reloads the page, under server/ restarts the server, under electron/ relaunches the app; the tools wait for it to come back. The window shows without taking focus, and the user may be watching it. Screenshots of the dev instance show the user's real session titles.`;

const instanceArg = z.enum(pkg.xenonetComponent === "tools" ? ["installed"] : Object.keys(INSTANCES)).optional().describe("Which XenoNet: installed (the default for installed Tools), demo (made-up data; the default in this checkout), dev, test or film. Stays picked for later calls.");
const settleArg = z.boolean().optional().describe("Wait for the view to settle before returning (default true).");
const pointArg = z
  .union([z.object({ x: z.number(), y: z.number() }), z.object({ node: z.string() })])
  .describe("A point on the page: { x, y } in CSS pixels from the top left, or { node: id } for a node's centre on screen.");

let current = process.env.XENONET_INSTANCE && INSTANCES[process.env.XENONET_INSTANCE] ? process.env.XENONET_INSTANCE : pkg.xenonetComponent === "tools" ? "installed" : "demo";
const target = (instance) => xenonet((current = instance ?? current));

// A tool's result: its JSON as text, which every client reads, and any
// images after it.
const reply = (value, extra = []) => ({ content: [{ type: "text", text: JSON.stringify(value, null, 1) }, ...extra] });

function buildServer() {
  const server = new McpServer({ name: "xenonet-console", version: pkg.version }, { capabilities: { tools: {} }, instructions: INSTRUCTIONS });
  const tool = (name, config, run) => pkg.xenonetComponent === "tools" && ["eval", "simulate"].includes(name) ? null : server.registerTool(name, config, async (args) => run(args ?? {}));

  tool("terminal", {
    title: "XenoNet terminals",
    description: "List, open, read, write to or close a terminal in Desktop. write sends the exact text; include a newline to run a command. Installed terminals survive Desktop restarts. read returns the active tab's output.",
    inputSchema: z.object({ instance: instanceArg, action: z.enum(["list", "open", "read", "write", "close"]).optional(), id: z.number().int().optional(), text: z.string().optional(), cwd: z.string().optional(), profile: z.string().optional(), lines: z.number().int().min(1).max(1000).optional() }),
    annotations: { readOnlyHint: false, destructiveHint: true, openWorldHint: true },
  }, async ({ instance, action = "list", ...options }) => reply(await target(instance).driver("terminal", action, options)));

  tool("updates", {
    title: "XenoNet updates",
    description: "Open Updates and read its state, check for a release, or change automatic checks. Development copies never download or install updates.",
    inputSchema: z.object({ instance: instanceArg, action: z.enum(["status", "check", "automatic", "components", "component-update"]).optional(), enabled: z.boolean().optional(), component: z.enum(["service", "tools"]).optional() }),
    annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: true },
  }, async ({ instance, action = "status", enabled, component }) => reply(await target(instance).driver("updates", action, action === "component-update" ? component : enabled)));

  tool(
    "status",
    {
      title: "XenoNet status",
      description: "Which XenoNet instances are running, and for the current one its window and the page's state: the selection, what's gathered, open panels, alerts, counts and whether the view is still moving.",
      inputSchema: z.object({ instance: instanceArg }),
      annotations: { readOnlyHint: true, openWorldHint: false },
    },
    async ({ instance }) => {
      const s = target(instance);
      const instances = {};
      for (const id of Object.keys(INSTANCES)) {
        const p = await xenonet(id).ping();
        instances[id] = p ? { running: true, port: p.port, window: p.window ?? null, ready: !!p.page?.ready, ...(p.error ? { error: p.error } : {}) } : { running: false, port: INSTANCES[id].port };
      }
      const state = instances[s.id].ready ? await s.driver("state") : null;
      return reply({ current: s.id, instances, state });
    },
  );

  tool(
    "launch",
    {
      title: "Launch XenoNet",
      description:
        "Starts installed XenoNet or a development instance unless it's running, waits until its page is ready, and makes it the current one. By default the window shows without taking focus (the test instance stays hidden). A demo or test copy stuck holding the instance's lock without answering is ended first, since its data is made up; `ended` in the result says so.",
      inputSchema: z.object({
        instance: instanceArg,
        show: z.enum(["inactive", "front", "hidden"]).optional().describe("inactive: shown, not focused (default); front: shown and focused; hidden: never shown, still renders and screenshots."),
        restart: z.boolean().optional().describe("Quit it first if it's running, for a fresh start. It also ends a copy stuck holding the instance's lock without answering, the dev instance's too."),
      }),
      annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: true, openWorldHint: false },
    },
    async ({ instance, show, restart }) => {
      const s = target(instance);
      const ping = await s.launch({ show, restart });
      return reply({ ...ping, state: await s.driver("state") });
    },
  );

  tool(
    "quit",
    {
      title: "Quit XenoNet",
      description: "Quits Desktop for the selected instance. Installed XenoNet keeps its service, terminals and coordination running. For development instances it returns once the app has exited, or once it's ended it if it was stuck on its way out (`killed`). The dev instance is the user's own XenoNet Dev, so quit it only when asked.",
      inputSchema: z.object({ instance: instanceArg }),
      annotations: { readOnlyHint: false, destructiveHint: true, idempotentHint: true, openWorldHint: false },
    },
    async ({ instance }) => reply(await target(instance).quit()),
  );

  tool(
    "find",
    {
      title: "Find nodes",
      description: "Finds nodes on the map whose id, title or name contain every word of `query`: projects, sessions, Codex projects (native), folders outside the projects folder (zone), shared systems (resource), wiki pages, other machines' XenoNets (host, with XENONET_HOSTS=1; their nodes' ids start with @<host>: and say which host), and actors beside the map (kind actor, by id, name and product). Drawn nodes come first, then the most recent, then actors. Each has its id, for select and input, and where it is on screen.",
      inputSchema: z.object({
        query: z.string().optional().describe("Words to match; empty lists everything."),
        kind: z.enum(["project", "session", "native", "zone", "resource", "wiki", "page", "core", "actor", "host"]).optional(),
        limit: z.number().int().min(1).max(200).optional().describe("Default 20."),
        instance: instanceArg,
      }),
      annotations: { readOnlyHint: true, openWorldHint: false },
    },
    async ({ query, kind, limit, instance }) => reply(await target(instance).driver("find", { query, kind, limit })),
  );

  tool(
    "select",
    {
      title: "Select a node",
      description: "Selects a node, as clicking it would: a project, Codex project or folder gathers its sessions around it and the camera frames them; an actor (actor:<slug>) is framed with what it acts on; anything else gets the camera's focus and the inspector. Give an id, or a query to select the best match from find. null clears the selection. Returns the state once the view has settled.",
      inputSchema: z.object({
        id: z.string().nullable().optional().describe("A node id from find or status, or an actor id from actors; null clears the selection."),
        query: z.string().optional().describe("Instead of an id: select the first node find gives for these words."),
        kind: z.enum(["project", "session", "native", "zone", "resource", "wiki", "page", "core", "actor", "host"]).optional().describe("With query: only this kind."),
        settle: settleArg,
        instance: instanceArg,
      }),
      annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: true, openWorldHint: false },
    },
    async ({ id, query, kind, settle, instance }) => {
      const s = target(instance);
      if (id === undefined && query) {
        const { nodes } = await s.driver("find", { query, kind, limit: 1 });
        if (!nodes.length) throw new Error(`nothing on the map matches "${query}"`);
        id = nodes[0].id;
      }
      return reply(await s.driver("select", id ?? null, { settle }));
    },
  );

  tool(
    "navigate",
    {
      title: "Step through the map",
      description: "back: Esc, from a gathered session to its project, then out (putting the gathering back). next / previous: ↓ / ↑, the next or previous gathered node. reset: R, frame the whole map. Returns the state once the view has settled.",
      inputSchema: z.object({ action: z.enum(["back", "next", "previous", "reset"]), settle: settleArg, instance: instanceArg }),
      annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
    },
    async ({ action, settle, instance }) => {
      const s = target(instance);
      const opts = { settle };
      const result = action === "back" ? await s.driver("back", opts) : action === "reset" ? await s.driver("reset", opts) : await s.driver("step", action === "next" ? 1 : -1, opts);
      return reply(result);
    },
  );

  tool(
    "stats",
    {
      title: "Open a session's stats",
      description: "Opens a session's stats panel, as Stats in the inspector or Mod+I in the palette would: what it would cost at API prices, its models over time, where its tokens went, its turns, context and tools. Give a session id, or a query for the best matching session. Returns the state and the numbers in brief (cost, tokens, calls, turns, models).",
      inputSchema: z.object({
        id: z.string().optional().describe("A session id from find."),
        query: z.string().optional().describe("Instead of an id: the first session find gives for these words."),
        settle: settleArg,
        instance: instanceArg,
      }),
      annotations: { readOnlyHint: true, openWorldHint: false },
    },
    async ({ id, query, settle, instance }) => {
      const s = target(instance);
      if (!id && query) id = (await s.driver("find", { query, kind: "session", limit: 1 })).nodes[0]?.id;
      if (!id) throw new Error(query ? `no session matches "${query}"` : "give a session id or a query");
      return reply(await s.driver("stats", id, { settle }));
    },
  );

  tool(
    "rename",
    {
      title: "Rename a session",
      description: "Renames a session, as typing over its title in the inspector (F2, or the pencil) would. The name is saved in the session's own app, so it shows there too: Claude Code's transcript and, for a session in Claude desktop, the desktop's record (which the desktop reads when it next starts), or Codex's thread name. The demo keeps the names of its made-up sessions in XenoNet. Another machine's sessions can't be renamed. Give a session id, or a query for the best matching session. Returns the state, and `renamed`: the name as saved, where it went and anything to know.",
      inputSchema: z.object({
        id: z.string().optional().describe("A session id from find."),
        query: z.string().optional().describe("Instead of an id: the first session find gives for these words."),
        title: z.string().min(1).max(120).describe("The new name, on one line."),
        settle: settleArg,
        instance: instanceArg,
      }),
      annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: true, openWorldHint: false },
    },
    async ({ id, query, title, settle, instance }) => {
      const s = target(instance);
      if (!id && query) id = (await s.driver("find", { query, kind: "session", limit: 1 })).nodes[0]?.id;
      if (!id) throw new Error(query ? `no session matches "${query}"` : "give a session id or a query");
      return reply(await s.driver("rename", id, title, { settle }));
    },
  );

  tool(
    "command",
    {
      title: "Run a palette command",
      description: "Runs a command from the command palette (⌘K / Ctrl+K) by id, such as wallpaper, replay, poster, daylog, away2, reset, scope-7d or focus-lighting, and returns the state once the view has settled. Without an id, lists the commands.",
      inputSchema: z.object({ id: z.string().optional(), settle: settleArg, instance: instanceArg }),
      annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
    },
    async ({ id, settle, instance }) => {
      const s = target(instance);
      return reply(id ? await s.driver("command", id, { settle }) : { commands: await s.driver("commands") });
    },
  );

  tool(
    "identities",
    {
      title: "Identities",
      description: 'The identities sessions ran as and projects belong to (Claude and ChatGPT accounts, GitHub accounts, git identities), as the chips under Filters show them, each with a key. `only` shows just those identities\' sessions and projects ([] shows everything again); `masked` hides them as "GitHub A", "Claude B"… for screenshots, or shows them. Returns the chips, and the state once the view has settled when something changed.',
      inputSchema: z.object({
        only: z.array(z.string()).optional().describe("Chip keys from an earlier call; [] clears the filter."),
        masked: z.boolean().optional(),
        settle: settleArg,
        instance: instanceArg,
      }),
      annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
    },
    async ({ only, masked, settle, instance }) => {
      const s = target(instance);
      let state = null;
      if (masked !== undefined) state = await s.driver("maskIdentities", masked, { settle });
      if (only) state = await s.driver("filterIdentities", only, { settle });
      return reply({ identities: await s.driver("identities"), ...(state ? { state } : {}) });
    },
  );

  tool(
    "hosts",
    {
      title: "Other machines",
      description: 'Other machines\' XenoNets on the map, with XENONET_HOSTS=1 (a prototype: the console reads each over SSH with xenonet relay, read-only). Each host with how it stands (connecting, live, stale, offline or error), its host node (@<host>:core) and where that is on screen, and its constellation beside this machine\'s map; the bridges between checkouts of one repo; and the machines\' chips under Filters. `only` shows just those machines\' work: host ids, and "@here" for this machine ([] shows every machine). In XenoNet Dev, simulate host-add shows another XenoNet on this machine as a host. Returns the hosts, and the state once the view has settled when something changed.',
      inputSchema: z.object({
        only: z.array(z.string()).optional().describe('Host ids, and "@here" for this machine; [] clears the filter.'),
        settle: settleArg,
        instance: instanceArg,
      }),
      annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
    },
    async ({ only, settle, instance }) => {
      const s = target(instance);
      const state = only ? await s.driver("filterHosts", only, { settle }) : null;
      return reply({ hosts: await s.driver("hosts"), ...(state ? { state } : {}) });
    },
  );

  tool(
    "root",
    {
      title: "The projects folder",
      description: "The projects folder XenoNet reads (its root) and where that comes from: XENONET_PROJECTS_DIR, a folder chosen in the console, or the default ~/Projects, with its last scan. `check` says what XenoNet would find in another folder (how many projects and groups, or why it can't use it) without switching. `path` switches to a folder, as Change the projects folder… in the palette does: XenoNet checks it, rescans and returns once the map shows it; null goes back to the default. The layout, your rules and Codex's picks stay with each folder, so switching back brings them back. The demo has a second made-up folder, ~/Clients.",
      inputSchema: z.object({
        path: z.string().nullable().optional().describe("Switch to this folder: a full path, or one starting with ~ (the demo's own home, in the demo). null goes back to the default."),
        check: z.string().optional().describe("A folder to check without switching to it."),
        settle: settleArg,
        instance: instanceArg,
      }),
      annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: true, openWorldHint: false },
    },
    async ({ path: folder, check, settle, instance }) => {
      const s = target(instance);
      if (check !== undefined) return reply({ check: await s.driver("checkRoot", check) });
      if (folder !== undefined) return reply(await s.driver("setRoot", folder, { settle }));
      return reply(await s.driver("root"));
    },
  );

  tool(
    "simulate",
    {
      title: "Simulate a session state, an actor acting, or another machine",
      description: "Puts sessions into states that are otherwise rare, so you can check how they look. The kinds are needs-you (a question waiting), collision (two sessions editing the same file, which needs a and b), stuck (a failing loop), wiki-write, turn-end, doctor (the doctor finding a problem), and clear, which undoes them all. Without session ids it picks the most recently active sessions. The actor-… kinds have an actor act instead, as an event it reports would. They are actor-spawn (it starts a session), actor-edit (it edits files in a project), actor-message (it messages a session), actor-online and actor-offline. The actor reacts, and a beam runs to what it touched. actor-request makes up a program asking to be an actor (named a, Heron by default), which waits for an Allow in the window. Its card shows in the attention strip, and actors lists it under requests. actor-needs-you has an actor (a, the first by default) wait on you with a made-up question. Its card shows in the attention strip, and its glow beats in the attention color until clear. actor-tunnel gives an actor (a) a made-up remote connection through a tunnel with an adapter connected, so the map draws its gate open between it and what it reads. actor-tunnel-down does the same with no adapter connected, so the gate is shut. actor-read sends a read through it, of node b or what it acts on, with light down the conduit and out to what it read. clear takes the tunnel away. Only the demo and test instances, whose data is made up, answer actor-request, actor-needs-you and the tunnel kinds, and actor-request's token stays out of this answer. With XENONET_HOSTS=1, it covers other machines too. host-add shows another XenoNet on this machine as a host (a is the new host's id, b that XenoNet's port). host-offline and host-online take a host's id as a. host-skew sets a host's clock b ms off, three minutes if b is left out. Everything stays in memory, apart from actor-request. Returns what was used and the page's state after the update arrives.",
      inputSchema: z.object({
        kind: z.enum(["needs-you", "collision", "stuck", "wiki-write", "turn-end", "doctor", "clear", "actor-spawn", "actor-edit", "actor-message", "actor-online", "actor-offline", "actor-request", "actor-needs-you", "actor-tunnel", "actor-tunnel-down", "actor-read", "host-add", "host-offline", "host-online", "host-skew"]),
        a: z.string().optional().describe("A session id (find with kind session); for the actor-… kinds, an actor id (actors lists them, and the first one that isn't you is the default); for actor-request, the name it asks with; for host-…, a host's id."),
        b: z.string().optional().describe("The second session, for collision; for the actor-… kinds, the node it acts on (a session for actor-spawn and actor-message, a project for actor-edit; the session busy most lately when left out); for host-add, the port of the XenoNet to show; for host-skew, milliseconds."),
        instance: instanceArg,
      }),
      annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
    },
    async ({ kind, a, b, instance }) => {
      const s = target(instance);
      // A made-up registration's token is still a token: it stays here.
      const { token, ...result } = await s.simulate(kind, { a, b });
      // The page hears about it over its event stream a moment later.
      await new Promise((r) => setTimeout(r, 400));
      return reply({ ...result, state: await s.driver("state") });
    },
  );

  tool(
    "actors",
    {
      title: "Actors",
      description: "The actors beside the map. They are the person at the console (You), always-on SI and actors of your own, drawn on a layer of their own above the map, with beams into the projects and sessions they act on. Each comes with its id, presence (working, idle, away, offline, unknown), status, how XenoNet knows of it (via), its product, its form and color in the theme, the node ids it beams into now and where it is on screen. The answer also has every beam drawn and the layer's time for its last frame. `requests` lists the programs waiting for an Allow in the window, each with what it asks, who asked, and whether its card is up. `tokens` says how each registered actor's token stands, as the Actors panel shows it (works, expired or none, when it expires, and when it was last used, null if never), and never holds a token itself. `remote` lists remote actors' connections (a dot reading through a tunnel, say) and whether cloud access is on. For each connection it gives the actor, state, how many projects it reads, whether labels go, its data mode, expiry and whether an adapter is connected, and never a credential. Its `tunnels` says whether tunnel-client is here, and for each tunnel XenoNet can run, its tunnel id, whether a key is kept (never the key), whether it starts with XenoNet, and whether it runs and why not. The `tunnels` beside the beams give each tunnel's gate on the map, how open it is and where. Allowing an actor, pairing, revoking or turning on a remote connection, and keeping a tunnel's key or starting it, are the person's alone, so no tool here does any of it. The command actors opens the Actors panel for them. `select` selects an actor, as clicking it would. The camera frames it with what it acts on and the inspector shows it (null clears the selection), and the answer then includes the state once the view has settled.",
      inputSchema: z.object({
        select: z.string().nullable().optional().describe("An actor id (actor:<slug>) to select; null clears the selection."),
        settle: settleArg,
        instance: instanceArg,
      }),
      annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: true, openWorldHint: false },
    },
    async ({ select, settle, instance }) => {
      const s = target(instance);
      const state = select !== undefined ? await s.driver("select", select, { settle }) : null;
      return reply({ ...(await s.driver("actors")), requests: await s.driver("actorRequests"), tokens: await s.driver("actorTokens"), remote: await s.driver("remoteConnections").catch(() => null), ...(state ? { state } : {}) });
    },
  );

  tool(
    "feed",
    {
      title: "Activity feed",
      description: "The activity feed as it shows now, newest first: each line's kind (for a session tool, prompt, message, agent-message, turn-start, turn-end, needs-you, stuck and the like; for an actor what it did: presence, status, spawned, edited, ran, messaged, said, finished, failed, or agent-message), who it's about (session: a session's id, or actor: an actor's), its title (a session's title, or an actor's name) and its words, and when.",
      inputSchema: z.object({
        limit: z.number().int().min(1).max(60).optional().describe("Default 20."),
        instance: instanceArg,
      }),
      annotations: { readOnlyHint: true, openWorldHint: false },
    },
    async ({ limit, instance }) => reply(await target(instance).driver("feed", { limit })),
  );

  tool(
    "screenshot",
    {
      title: "Screenshot XenoNet",
      description: "The window as it looks now, returned as an image, with a full-size PNG saved in the repo's .xenonet folder. It works when the window is covered, unfocused or hidden. By default it waits for the view to settle and holds every ambient animation still (sky, sonar, sweep, sparks, CSS animation), so two shots of the same state match; still: false shows them as they are.",
      inputSchema: z.object({
        name: z.string().optional().describe("The saved file's name, without .png (default shot)."),
        still: z.boolean().optional().describe("Hold ambient animation still for the shot (default true)."),
        settle: settleArg,
        maxWidth: z.number().int().min(200).max(4000).optional().describe("Width of the returned image in pixels (default 1568); the saved PNG is full size."),
        format: z.enum(["jpeg", "png"]).optional().describe("Of the returned image (default jpeg)."),
        instance: instanceArg,
      }),
      annotations: { readOnlyHint: true, openWorldHint: false },
    },
    async ({ name, still, settle, maxWidth, format, instance }) => {
      const s = target(instance);
      const { image, ...shot } = await s.screenshot({ name, still, settle, maxWidth, format });
      return reply({ instance: s.id, ...shot }, image ? [{ type: "image", data: image.data, mimeType: image.mimeType }] : []);
    },
  );

  tool(
    "input",
    {
      title: "Keyboard and mouse",
      description: 'Real keyboard and mouse input to the page, which needn\'t have focus. Any of, in this order: keys (combos like "Mod+K", "Escape", "ArrowDown", "W"; Mod is ⌘ on a Mac, Ctrl elsewhere), text (typed into whatever has focus), move (hover), click, drag (Alt-drag moves a node), scroll. Points are { x, y } or { node: id }. Prefer select, navigate and command for what they cover; use input to test the real interaction. Returns what it did and the state once the view has settled.',
      inputSchema: z.object({
        keys: z.union([z.string(), z.array(z.string())]).optional(),
        text: z.string().optional(),
        move: pointArg.optional(),
        click: pointArg.optional(),
        count: z.number().int().min(1).max(3).optional().describe("Clicks: 2 double-clicks (on a session, the transcript preview)."),
        button: z.enum(["left", "right", "middle"]).optional(),
        drag: z.object({ from: pointArg, to: pointArg }).optional(),
        scroll: z.object({ x: z.number(), y: z.number(), dx: z.number().optional(), dy: z.number().optional() }).optional().describe("A wheel turn at x, y; dy > 0 scrolls down (zooms the map out)."),
        modifiers: z.array(z.enum(["Alt", "Control", "Meta", "Shift", "Mod"])).optional().describe("Held for move, click, drag and scroll."),
        settle: settleArg,
        instance: instanceArg,
      }),
      annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
    },
    async ({ instance, ...input }) => reply(await target(instance).input(input)),
  );

  tool(
    "window",
    {
      title: "XenoNet's window",
      description: 'Shows the window ("inactive": without taking focus; "front": focused), hides it, or sets the page\'s size, such as "1600x900", or "1600x900@2" for a pixel ratio of 2 ("window" goes back to the window\'s own size).',
      inputSchema: z.object({
        show: z.enum(["inactive", "front"]).optional(),
        hide: z.boolean().optional(),
        size: z.string().optional(),
        instance: instanceArg,
      }),
      annotations: { readOnlyHint: false, destructiveHint: false, idempotentHint: true, openWorldHint: false },
    },
    async ({ instance, ...opts }) => reply(await target(instance).window(opts)),
  );

  tool(
    "logs",
    {
      title: "XenoNet's logs",
      description: "Recent page console messages, uncaught page errors, app events (loads, crashes) and server output, oldest first. Pass the returned cursor as since to get only what's new.",
      inputSchema: z.object({
        level: z.enum(["error", "warning", "all"]).optional().describe("error, warning (and errors), or all (default)."),
        source: z.enum(["page", "server", "app"]).optional(),
        since: z.number().int().optional(),
        limit: z.number().int().min(1).max(600).optional().describe("Default 200."),
        instance: instanceArg,
      }),
      annotations: { readOnlyHint: true, openWorldHint: false },
    },
    async ({ level, instance, ...opts }) => reply(await target(instance).logs({ ...opts, level: level === "all" ? undefined : level })),
  );

  tool(
    "eval",
    {
      title: "Run JavaScript in the page",
      description: "Evaluates JavaScript in XenoNet Dev's page and returns the result as JSON: an expression, or a function body with return; await works. window.__xenonet holds the scene, panels and the driver (window.__xenonet.driver). For checks the other tools don't cover.",
      inputSchema: z.object({ expression: z.string(), instance: instanceArg }),
      annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
    },
    async ({ expression, instance }) => reply({ value: await target(instance).eval(expression) }),
  );

  tool(
    "theme",
    {
      title: "Themes",
      description:
        'The console\'s themes, switched live without a reload. list: the themes (id, name, built in or the user\'s) and the one in use. set: use one in every window, as Theme: in the palette does. preview: show one in this window alone, to look at; keep makes it the one in use, revert goes back to it. switcher: open "Theme…", the list a person steps through; step: ↓ or ↑ in it (by 1 or -1), showing the theme it lands on in this window alone, then keep (Enter) or revert (Esc). gallery: open the gallery of theme cards (with id, looking at that one); focus: look at a card\'s theme, as pointing at it does; make: "Make your own" from the card looked at, a copy saved as a file of the user\'s and put in use (state.made has its file); close: close the switcher or the gallery, putting the theme in use back. Except for list, waits until the theme is drawn and the view has settled, and returns the state (state.theme says which is drawn and which is in use, state.themes what the switcher and the gallery show).',
      inputSchema: z.object({
        action: z.enum(["list", "set", "preview", "keep", "revert", "switcher", "step", "gallery", "focus", "make", "close"]).optional().describe("Default list."),
        id: z.string().optional().describe("For set, preview and focus: a theme's id from list; for gallery, the card to look at first."),
        by: z.number().int().optional().describe("For step: 1 (↓, the default) or -1 (↑), or further."),
        settle: settleArg,
        instance: instanceArg,
      }),
      annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
    },
    async ({ action = "list", id, by, settle, instance }) => {
      const s = target(instance);
      const opts = { settle };
      if (action === "list") return reply(await s.driver("themes"));
      if (!id && (action === "set" || action === "preview" || action === "focus")) throw new Error(`say which theme to ${action}: an id from list`);
      if (action === "switcher") return reply(await s.driver("themeSwitcher", true, opts));
      if (action === "step") return reply(await s.driver("stepTheme", by ?? 1, opts));
      if (action === "gallery") return reply(await s.driver("themeGallery", id ?? true, opts));
      if (action === "focus") return reply(await s.driver("focusTheme", id, opts));
      if (action === "make") return reply(await s.driver("makeTheme", opts));
      if (action === "close") {
        const { overlays } = await s.driver("state");
        if (overlays.switcher) return reply(await s.driver("themeSwitcher", false, opts));
        if (overlays.gallery) return reply(await s.driver("themeGallery", false, opts));
        return reply(await s.driver("state"));
      }
      const method = { set: "theme", preview: "previewTheme", keep: "keepTheme", revert: "revertTheme" }[action];
      return reply(await s.driver(method, ...(id && (action === "set" || action === "preview") ? [id] : []), opts));
    },
  );

  server.registerTool("mcp_info", {
    title: "Inspect an MCP shared system",
    description: "Open an MCP shared system's rich inspector, with declared author, client connection and authentication status, searchable tool descriptions and input schemas, resource and template context, and recent usage. This does not execute a server tool or read resource contents. After open, tab selects overview, tools, context or activity; search filters tools.",
    inputSchema: z.object({ action: z.enum(["open", "tab", "search"]).optional(), id: z.string().optional().describe("For open: a shared system's node id from find."), tab: z.enum(["overview", "tools", "context", "activity"]).optional(), query: z.string().optional(), instance: instanceArg, settle: settleArg }),
    annotations: { readOnlyHint: true, destructiveHint: false, openWorldHint: false },
  }, async ({ action = "open", id, tab, query, instance, settle }) => {
    const s = target(instance);
    if (action === "open") { if (!id) throw new Error("choose a shared system to inspect"); return reply(await s.driver("mcp", id, { settle })); }
    return reply(await s.driver(action === "tab" ? "mcpTab" : "mcpSearch", action === "tab" ? tab ?? "overview" : query ?? "", { settle }));
  });

  server.registerTool("environment", {
    title: "Choose the environment around the map",
    description: "List or preview Nebula, Aurora, Trench and Vault independently of the active theme. All inherit the theme's colors. set saves the choice for every window; preview changes only this window; keep saves the preview; revert cancels it; picker opens the person's environment cards. No reload is needed.",
    inputSchema: z.object({ action: z.enum(["list", "set", "preview", "keep", "revert", "picker"]).optional(), id: z.string().optional(), instance: instanceArg, settle: settleArg }),
    annotations: { readOnlyHint: false, destructiveHint: false, openWorldHint: false },
  }, async ({ action = "list", id, instance, settle }) => {
    const s = target(instance);
    if (action === "list") return reply(await s.driver("environments"));
    if (["set", "preview"].includes(action) && !id) throw new Error("choose an environment from list");
    const method = { set: "environment", preview: "previewEnvironment", keep: "keepEnvironment", revert: "revertEnvironment", picker: "environmentPicker" }[action];
    return reply(await s.driver(method, ...(["set", "preview"].includes(action) ? [id] : []), { settle }));
  });

  return server;
}

serveStdio(buildServer, { onerror: (err) => console.error(`[xenonet mcp] ${err.message}`) });
