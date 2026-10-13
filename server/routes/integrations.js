import { IntegrationError } from "../../core/integrations/common.js";

export async function integrationRoutes({ req, res, url, integrations, yours, owner, readJson, send }) {
  if (url.pathname !== "/api/integrations" && !url.pathname.startsWith("/api/integrations/")) return false;
  if (req.headers?.authorization !== undefined || !(await yours(req, res))) {
    if (req.headers?.authorization !== undefined) send(res, 403, { error: "Integration data belongs to the owner.", code: "yours" });
    return true;
  }
  if (req.method !== "GET" && !(await owner(req, res))) return true;
  const route = url.pathname.slice("/api/integrations".length), params = Object.fromEntries(url.searchParams);
  try {
    let value;
    if (req.method === "GET" && !route) value = await integrations.status();
    else if (req.method === "GET" && route === "/history") value = integrations.history(params);
    else if (req.method === "GET" && route === "/pi") value = await integrations.pi(params);
    else if (req.method === "POST" && route === "/route/preview") value = await integrations.routing.preview(await readJson(req, 8192));
    else if (req.method === "POST" && route === "/route/send") value = await integrations.routing.route((await readJson(req, 1024))?.ticket);
    else if (req.method === "POST" && route === "/route/outcome") { const body = await readJson(req, 1024); value = integrations.routing.outcome(body?.id, body?.outcome); }
    else if (req.method === "POST" && route === "/sandbox/check") value = await integrations.shell.status(await readJson(req, 1024));
    else if (req.method === "POST" && route === "/sandbox/preview") value = await integrations.shell.preview(await readJson(req, 8192));
    else if (req.method === "POST" && route === "/sandbox/launch") value = await integrations.shell.launch((await readJson(req, 1024))?.ticket);
    else if (req.method === "POST" && route === "/sandbox/inspect") value = await integrations.shell.inspect((await readJson(req, 1024))?.id);
    else if (req.method === "POST" && route === "/sandbox/delete") value = await integrations.shell.remove((await readJson(req, 1024))?.id);
    else if (req.method === "POST" && route === "/pi/enable") value = integrations.enablePi((await readJson(req, 1024))?.on);
    else if (req.method === "POST" && route === "/pi/import") value = integrations.importPi(await readJson(req, 1024 * 1024));
    else { send(res, 404, { error: "No such integration route." }); return true; }
    send(res, 200, value);
  } catch (err) {
    const input = [400, 413].includes(err.status) && ["the body is too big", "the body isn't JSON"].includes(err.message);
    send(res, err instanceof IntegrationError || input ? err.status : 502, { error: err instanceof IntegrationError ? err.message : input ? err.message : "The integration could not complete this operation. Check its configuration and gateway, then refresh.", code: err instanceof IntegrationError ? err.code : input ? "bad-request" : "integration-failed" });
  }
  return true;
}
