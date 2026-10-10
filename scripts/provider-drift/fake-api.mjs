// A stand-in for a model API on 127.0.0.1, so a provider's CLI can run a
// turn without a key or a network: Claude Code against the Messages API
// (ANTHROPIC_BASE_URL), Codex against the Responses API (a model provider
// whose base_url is this server). The CLI then writes its own transcript or
// rollout, in the version under test, which is what XenoNet reads.
//
// Each conversation goes the same way: the first request that offers a
// harmless tool (Claude Code's TodoWrite, or Read on a made-up file in the
// project; Codex's update_plan) gets a call to it, and the request that
// comes back with the tool's result gets a short text reply. Every other
// request gets the text reply. Nothing here is anyone's data.
import http from "node:http";

const TEXT = "Checked the plan; nothing else to do.";
const TODOS = { todos: [{ content: "Check the plan", status: "completed", activeForm: "Checking the plan" }] };
const PLAN = { explanation: "A one-step plan", plan: [{ step: "Check the plan", status: "completed" }] };

const sse = (res, events) => {
  res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache", connection: "keep-alive" });
  for (const [event, data] of events) res.write(`event: ${event}\ndata: ${JSON.stringify(data)}\n\n`);
  res.end();
};

const json = (res, status, body) => {
  res.writeHead(status, { "content-type": "application/json" });
  res.end(JSON.stringify(body));
};

// ---------- the Messages API ----------

// Whether a Messages request already carries a tool's result.
const hasToolResult = (body) => (body?.messages ?? []).some((m) => Array.isArray(m?.content) && m.content.some((b) => b?.type === "tool_result"));
const offers = (tools, name) => Array.isArray(tools) && tools.some((t) => t?.name === name);

// The reply to a Messages request, as a whole message. `readFile` is a
// made-up file Read may be called on, when TodoWrite isn't offered.
export function anthropicReply(body, n, { readFile = null } = {}) {
  const id = `msg_fake_${String(n).padStart(4, "0")}`;
  const model = typeof body?.model === "string" ? body.model : "claude-fake";
  const call = hasToolResult(body) ? null : offers(body?.tools, "TodoWrite") ? ["TodoWrite", TODOS] : readFile && offers(body?.tools, "Read") ? ["Read", { file_path: readFile }] : null;
  const useTool = !!call;
  const content = call ? [{ type: "tool_use", id: `toolu_fake_${String(n).padStart(4, "0")}`, name: call[0], input: call[1] }] : [{ type: "text", text: TEXT }];
  const usage = { input_tokens: 12, cache_creation_input_tokens: 0, cache_read_input_tokens: 0, output_tokens: 8 };
  return { id, type: "message", role: "assistant", model, content, stop_reason: useTool ? "tool_use" : "end_turn", stop_sequence: null, usage };
}

function anthropicStream(res, message) {
  const { content, stop_reason, usage, ...rest } = message;
  const events = [["message_start", { type: "message_start", message: { ...rest, content: [], stop_reason: null, usage: { ...usage, output_tokens: 1 } } }]];
  content.forEach((block, index) => {
    if (block.type === "text") {
      events.push(["content_block_start", { type: "content_block_start", index, content_block: { type: "text", text: "" } }]);
      events.push(["content_block_delta", { type: "content_block_delta", index, delta: { type: "text_delta", text: block.text } }]);
    } else {
      events.push(["content_block_start", { type: "content_block_start", index, content_block: { ...block, input: {} } }]);
      events.push(["content_block_delta", { type: "content_block_delta", index, delta: { type: "input_json_delta", partial_json: JSON.stringify(block.input) } }]);
    }
    events.push(["content_block_stop", { type: "content_block_stop", index }]);
  });
  events.push(["message_delta", { type: "message_delta", delta: { stop_reason, stop_sequence: null }, usage: { output_tokens: usage.output_tokens } }]);
  events.push(["message_stop", { type: "message_stop" }]);
  sse(res, events);
}

// ---------- the Responses API ----------

const hasCallOutput = (body) => Array.isArray(body?.input) && body.input.some((i) => i?.type === "function_call_output" || i?.type === "custom_tool_call_output");
const offersFunction = (tools, name) => Array.isArray(tools) && tools.some((t) => t?.name === name || t?.function?.name === name);

// Codex's tools that change nothing, with their arguments, in the order
// they're picked: a call to either is a function call and its output, with
// no command run and no sandbox needed.
const QUIET_FUNCTIONS = [
  ["update_plan", PLAN],
  ["get_goal", {}],
];

// The reply to a Responses request: { id, item, usage }.
export function openaiReply(body, n) {
  const id = `resp_fake_${String(n).padStart(4, "0")}`;
  const call = hasCallOutput(body) ? null : QUIET_FUNCTIONS.find(([name]) => offersFunction(body?.tools, name));
  const item = call
    ? { type: "function_call", id: `fc_fake_${n}`, call_id: `call_fake_${String(n).padStart(4, "0")}`, name: call[0], arguments: JSON.stringify(call[1]) }
    : { type: "message", id: `msg_fake_${n}`, role: "assistant", content: [{ type: "output_text", text: TEXT, annotations: [] }] };
  const usage = { input_tokens: 12, input_tokens_details: { cached_tokens: 0 }, output_tokens: 8, output_tokens_details: { reasoning_tokens: 0 }, total_tokens: 20 };
  return { id, item, usage };
}

function openaiStream(res, { id, item, usage }) {
  sse(res, [
    ["response.created", { type: "response.created", response: { id, object: "response", status: "in_progress", output: [] } }],
    ["response.output_item.added", { type: "response.output_item.added", output_index: 0, item: { ...item, ...(item.type === "message" ? { content: [] } : {}) } }],
    ["response.output_item.done", { type: "response.output_item.done", output_index: 0, item }],
    ["response.completed", { type: "response.completed", response: { id, object: "response", status: "completed", output: [item], usage } }],
  ]);
}

// ---------- the server ----------

// Starts the server on a free port: { url, requests, close() }. `requests`
// lists what came in (method, path, whether it streamed, the tools offered
// and the reply), to say what a CLI asked for when a run goes wrong.
export async function startFakeApi({ readFile = null } = {}) {
  const requests = [];
  let n = 0;
  const server = http.createServer((req, res) => {
    let raw = "";
    req.setEncoding("utf8");
    req.on("data", (chunk) => (raw += chunk));
    req.on("end", () => {
      let body = null;
      try {
        body = raw ? JSON.parse(raw) : null;
      } catch {}
      const url = new URL(req.url, "http://127.0.0.1");
      const tools = Array.isArray(body?.tools) ? body.tools.map((t) => t?.name ?? t?.function?.name ?? t?.type).filter((t) => typeof t === "string") : [];
      const entry = { method: req.method, path: url.pathname, stream: !!body?.stream, tools };
      requests.push(entry);
      n++;
      if (req.method === "POST" && /\/messages\/count_tokens$/.test(url.pathname)) return json(res, 200, { input_tokens: 12 });
      if (req.method === "POST" && /\/messages$/.test(url.pathname)) {
        const message = anthropicReply(body, n, { readFile });
        entry.reply = message.content.map((b) => b.type).join(",");
        return body?.stream ? anthropicStream(res, message) : json(res, 200, message);
      }
      if (req.method === "POST" && /\/responses$/.test(url.pathname)) {
        const reply = openaiReply(body, n);
        entry.reply = reply.item.type;
        if (body?.stream === false) return json(res, 200, { id: reply.id, object: "response", status: "completed", output: [reply.item], usage: reply.usage });
        return openaiStream(res, reply);
      }
      if (req.method === "GET" && /\/models$/.test(url.pathname)) return json(res, 200, { object: "list", data: [], models: [] });
      json(res, 404, { error: { type: "not_found", message: `the stand-in API doesn't answer ${req.method} ${url.pathname}` } });
    });
  });
  await new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolve);
  });
  const { port } = server.address();
  return {
    url: `http://127.0.0.1:${port}`,
    requests,
    close: () => new Promise((resolve) => server.close(() => resolve())),
  };
}
