// Prints the Claude Code settings block that streams session events into
// XenoNet. It does not edit any settings; paste it into
// ~/.claude/settings.json (merge with any existing "hooks") yourself.
import { config } from "../core/config.js";

const url = `http://127.0.0.1:${config.port}/api/hook/claude`;
const events = ["SessionStart", "UserPromptSubmit", "Stop", "SessionEnd", "Notification"];
const hooks = Object.fromEntries(events.map((e) => [e, [{ hooks: [{ type: "http", url, timeout: 2 }] }]]));
console.log(JSON.stringify({ hooks }, null, 2));
