// What you do in the console, as driver steps ({ op, args }; see
// driver.js), oldest first: what xenonet script record saves, and
// xenonet script run plays again (core/script.js).
const MAX = 500;
const log = [];

export function note(op, ...args) {
  log.push({ op, args, at: Date.now() });
  if (log.length > MAX) log.shift();
}

export const ops = () => log.map(({ op, args, at }) => ({ op, args, at }));

export function clearOps() {
  log.length = 0;
}
