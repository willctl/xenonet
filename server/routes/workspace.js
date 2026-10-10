// The projects folder XenoNet reads (core/workspace.js), over the API:
//
//   GET    /api/workspace/root    the folder, where that comes from
//                                 (XENONET_PROJECTS_DIR, chosen, or the
//                                 default) and its last scan
//   POST   /api/workspace/root    { path } reads another folder, once it's
//                                 checked; path null goes back to the default
//   DELETE /api/workspace/root    the same as path null
//   POST   /api/workspace/check   { path } what XenoNet would find there,
//                                 without reading it for the map
//
// A change rescans and sends the new map like any refresh (see
// XenoNetState.setRoot), and answers once it has. The console's panel
// (src/root.js) and xenonet root use these. Changing the folder is yours, as
// connecting agents is: from XenoNet's window or your own terminal, never
// from under an agent or an actor product (verify.person); looking and
// checking are anyone's.
export function workspaceRoutes({ state, verify, readJson, send }) {
  return async function route(req, res, url) {
    const route = url.pathname;
    try {
      if (route === "/api/workspace/root") {
        if (req.method === "GET") return send(res, 200, state.rootInfo());
        if (req.method === "DELETE" || req.method === "POST") {
          const you = await verify.person(req.socket);
          if (!you.ok) return send(res, 403, { error: you.reason, code: "yours" });
        }
        if (req.method === "DELETE") return send(res, 200, await state.setRoot(null));
        if (req.method === "POST") {
          const { path } = await readJson(req);
          if (path === undefined) return send(res, 400, { error: "say which folder: { path }, or path null for the default" });
          return send(res, 200, await state.setRoot(path));
        }
      }
      if (req.method === "POST" && route === "/api/workspace/check") return send(res, 200, await state.checkRoot((await readJson(req)).path));
      return send(res, 404, { error: "not found" });
    } catch (err) {
      return send(res, err.status ?? 400, { error: String(err.message ?? err) });
    }
  };
}
