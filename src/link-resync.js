// The page's event stream: the link light, and a catch-up after an outage.
// The service sends an event only when something changes, so a page whose
// stream dropped (the service restarted, the machine slept) has missed
// events it will never be sent again: when the stream returns, `resync`
// reads the state afresh.
export function linkWatcher({ state, resync }) {
  let down = false;
  return {
    open() {
      state("live");
      if (down) {
        down = false;
        resync();
      }
    },
    error() {
      down = true;
      state("down");
    },
  };
}
