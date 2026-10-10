// The page keeps the same terminal bridge while shells live in the service.
import { ipcMain, shell } from "electron";

export function registerServicePty({ allowed, client, getWindow }) {
  const watching = new Map();
  const call = (method, params) => client.request(`terminal.${method}`, params);
  for (const method of ["profiles", "list", "create"]) ipcMain.handle(`term:${method}`, (e, params) => allowed(e) ? call(method, params) : null);
  ipcMain.handle("term:watch", (e, id, replay = false) => { if (allowed(e)) watching.set(id, { cursor: 0, replay }); });
  ipcMain.handle("term:kill", async (e, id) => { if (allowed(e)) { watching.delete(id); await call("kill", { id }); } });
  ipcMain.on("term:write", (e, id, data) => { if (allowed(e)) call("write", { id, data }).catch(console.error); });
  ipcMain.on("term:resize", (e, id, cols, rows) => { if (allowed(e)) call("resize", { id, cols, rows }).catch(console.error); });
  ipcMain.handle("app:open-external", (e, url) => { if (allowed(e) && /^https?:\/\//i.test(url)) shell.openExternal(url); });
  let busy = false;
  const timer = setInterval(async () => {
    if (busy) return;
    busy = true;
    try {
      for (const [id, { cursor, replay }] of watching) {
        const win = getWindow();
        if (!win || win.isDestroyed()) break;
        try {
          const read = await call("read", { id, cursor });
          if (!watching.has(id)) continue;
          watching.set(id, { cursor: read.cursor, replay: false });
          if (read.data) win.webContents.send("term:data", id, read.data, { replay });
          if (read.exited) { win.webContents.send("term:exit", id, read.exitCode); watching.delete(id); }
        } catch { watching.delete(id); }
      }
    } finally { busy = false; }
  }, 32);
  return { count: () => watching.size, killAll: () => {}, dispose: () => clearInterval(timer) };
}
