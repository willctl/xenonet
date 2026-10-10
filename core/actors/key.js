// The window key (docs/design/actors/interface.md, section 2): 32 random
// bytes, made at each start of the server, for the routes only XenoNet's own
// window may use (the person's, and later allowing, adding and rotating
// actors). The server hands it to the desktop app over the utility
// process's port, and the app gives it only to XenoNet's own page, through
// the preload bridge. It's never written down, logged or put in a URL, and
// no other program has it: without it, XenoNet shows actors but can't act
// for you, which is the case in a browser tab and in a desktop app that
// reused a server it didn't start.
import crypto from "node:crypto";
import { inspect } from "node:util";

export function createWindowKey() {
  const value = crypto.randomBytes(32).toString("hex");
  const mine = Buffer.from(value);
  const hidden = () => "[the window key]";
  return {
    // For the desktop app, over the utility process's port; nowhere else.
    reveal: () => value,
    // Whether a request's x-xenonet-window is the key, compared in constant
    // time.
    matches(given) {
      if (typeof given !== "string") return false;
      const theirs = Buffer.from(given);
      return theirs.length === mine.length && crypto.timingSafeEqual(theirs, mine);
    },
    // Logged or sent by mistake, it says what it is and nothing more.
    toJSON: hidden,
    toString: hidden,
    [inspect.custom]: hidden,
  };
}
