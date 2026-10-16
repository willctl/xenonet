// The page keeps the same terminal bridge while shells live in the service.
import { ipcMain, shell } from "electron";
import { createTermPoll } from "./term-poll.js";

export function registerServicePty({ allowed, client, getWindow }) {
  const call = (method, params) => client.request(`terminal.${method}`, params);
  const win = () => {
    const w = getWindow();
    return w && !w.isDestroyed() ? w : null;
  };
  // Output is read by asking the service, quickly while a shell prints or
  // is typed into and slowly while all are quiet (term-poll.js).
  const poll = createTermPoll({
    read: (id, cursor) => call("read", { id, cursor }),
    ready: () => !!win(),
    deliver(id, read, { replay }) {
      if (read.data) win()?.webContents.send("term:data", id, read.data, { replay });
      if (read.exited) win()?.webContents.send("term:exit", id, read.exitCode);
    },
  });
  for (const method of ["profiles", "list", "create"]) ipcMain.handle(`term:${method}`, (e, params) => allowed(e) ? call(method, params) : null);
  ipcMain.handle("term:watch", (e, id, replay = false) => { if (allowed(e)) poll.watch(id, replay); });
  ipcMain.handle("term:kill", async (e, id) => { if (allowed(e)) { poll.unwatch(id); await call("kill", { id }); } });
  ipcMain.on("term:write", (e, id, data) => { if (allowed(e)) { call("write", { id, data }).catch(console.error); poll.soon(); } });
  ipcMain.on("term:resize", (e, id, cols, rows) => { if (allowed(e)) { call("resize", { id, cols, rows }).catch(console.error); poll.soon(); } });
  ipcMain.handle("app:open-external", (e, url) => { if (allowed(e) && /^https?:\/\//i.test(url)) shell.openExternal(url); });
  return { count: () => poll.count(), killAll: () => {}, dispose: () => poll.stop() };
}
