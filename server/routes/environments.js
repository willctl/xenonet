import { ENVIRONMENTS, environmentOf } from "../../core/environments.js";

export function environmentRoutes({ store, yours, send, readJson, broadcast }) {
  const current = () => environmentOf(store.getEnvironment()).id;
  return async (req, res, url) => {
    if (url.pathname !== "/api/environment") return false;
    if (req.method === "GET") send(res, 200, { current: current(), environments: ENVIRONMENTS });
    else if (req.method === "POST") {
      if (!(await yours(req, res))) return true;
      const { id } = await readJson(req);
      if (!ENVIRONMENTS.some((e) => e.id === id)) { send(res, 404, { error: "that environment isn't available" }); return true; }
      store.setEnvironment(id);
      broadcast("environment", { id });
      send(res, 200, { id });
    } else send(res, 405, { error: "use GET or POST" });
    return true;
  };
}
