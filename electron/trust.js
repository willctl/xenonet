// Who the desktop app takes calls and navigation from, kept apart from
// Electron so the tests can run it: only XenoNet's own page, on the server's
// origin. The preload's bridge goes with whatever page the window shows,
// so every privileged call checks where it came from, and the window never
// leaves that origin. The DevTools port, which would let any program into
// the page, opens only in XenoNet Dev, and XenoNet Dev's control channel
// doesn't press what's yours alone.

// Whether a URL is on `base`'s origin, compared as origins rather than as
// text: http://127.0.0.1:47840@elsewhere.example/ starts with the same
// characters as XenoNet's address, and is another site.
export function sameOrigin(url, base) {
  try {
    return new URL(url).origin === new URL(base).origin;
  } catch {
    return false;
  }
}

// Whether an IPC call (`e`, from ipcMain) comes from XenoNet's own page: the
// main frame of the window, on the server's origin (`base`), not a frame
// in it or a page it was taken to.
export function fromOwnPage(e, { win, base }) {
  const frame = e?.senderFrame;
  return !!win && !win.isDestroyed() && e?.sender === win.webContents && !!frame && !frame.parent && frame.origin === new URL(base).origin;
}

// What's marked yours alone in the page (data-yours: Allow, Connect, New
// token, Add), which XenoNet Dev's control channel doesn't press outside the
// demo and test instances, whose data is made up (see control.js). Two
// questions for the page, as function bodies: whether the element at a
// point is one or lies inside one, and whether the element with focus is.
const YOURS = "[data-yours]";
export const yoursAt = ({ x, y }) => `return !!document.elementFromPoint(${Number(x)}, ${Number(y)})?.closest(${JSON.stringify(YOURS)});`;
export const yoursFocused = `return !!document.activeElement?.closest(${JSON.stringify(YOURS)});`;

// The control channel's guard: before a click, either end of a drag, and
// Enter or Space (the keys that press a focused button), it asks the page
// (`ask` runs a function body there) and refuses what would land on
// something yours. Unguarded (`guarded` false: the demo and test
// instances), it asks nothing.
export function createYoursGuard({ guarded, ask }) {
  const refuse = () => {
    throw new Error("Allow, Connect, New token and Add are yours alone to press: XenoNet Dev's control channel presses them only in the demo and test instances, whose data is made up");
  };
  return {
    async point(p) {
      if (guarded && (await ask(yoursAt(p)))) refuse();
    },
    async key({ key }) {
      if (guarded && (key === "Enter" || key === " ") && (await ask(yoursFocused))) refuse();
    },
  };
}

// The DevTools port XENONET_CDP_PORT asks for, or null: in XenoNet Dev only,
// run from source (`fromSource`). Anyone on this machine can attach to
// that port without a password and run anything in the page, the window
// key included, and a program can set the variable for apps you open
// later (launchctl setenv), so the installed XenoNet never opens it.
export function devtoolsPort({ fromSource, env = process.env }) {
  const port = env.XENONET_CDP_PORT;
  return fromSource && /^\d{1,5}$/.test(port ?? "") ? port : null;
}
