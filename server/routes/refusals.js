import fs from "node:fs";
import path from "node:path";
import { queryRefusals } from "../../core/refusals/query.js";
import { contextOn, setContext } from "../../core/refusals/store.js";

export async function refusalRoutes({ req, res, url, refusals, yours, readJson, send, demoRecords = null }) {
  const route = url.pathname;
  if (route !== "/api/refusals" && !route.startsWith("/api/refusals/")) return false;
  if (!(await yours(req, res))) return true;
  if (route === "/api/refusals/context" && req.method === "POST") {
    const body = await readJson(req, 1024);
    send(res, 200, { context: setContext(refusals.dir, body?.on === true) });
  } else if (route === "/api/refusals" && req.method === "GET") {
    send(res, 200, { context: contextOn(refusals.dir), ...queryRefusals(demoRecords ?? refusals.list(), Object.fromEntries(url.searchParams)) });
  } else if (/^\/api\/refusals\/\d{8}-[0-9a-f]{8}\/request$/.test(route) && req.method === "GET") {
    const id = route.split("/")[3];
    const record = demoRecords?.find((r) => r.id === id) ?? (demoRecords ? null : refusals.get(id));
    if (!record?.context) send(res, 404, { error: "No saved request for this record." });
    else if (demoRecords) send(res, 200, { request: { provider: record.provider, model: record.signal.model, messages: [{ role: "user", content: "A made-up demo request." }] } });
    else {
      // Use fixed paths, never a filename supplied by a record or caller.
      const file = path.join(refusals.dir, id, "request.json");
      try {
        if (!fs.lstatSync(file).isFile()) throw new Error("missing");
        if (fs.statSync(file).size > 1024 * 1024) send(res, 413, { error: "This request is too large for the panel. Use xenonet refusals show --context or export --with-context." });
        else send(res, 200, { request: JSON.parse(fs.readFileSync(file, "utf8")) });
      } catch {
        send(res, 404, { error: "The saved request is missing or unreadable." });
      }
    }
  } else send(res, 404, { error: "No such refusal route." });
  return true;
}
