// Run from source, XenoNet is XenoNet Dev, and it runs as one of these
// instances. Each is its own app, with its own settings, port and
// single-instance lock, so they run side by side:
//
//   dev   your sessions                                port 47842
//   demo  made-up projects and sessions (XENONET_DEMO=1)  port 47841
//   test  made-up data, for the end-to-end tests       port 47843
//   film  your sessions, for xenonet film to record       port 47844
//   showcase made-up data, for the public demo video    port 47845
//
// XENONET_INSTANCE picks one; otherwise XENONET_DEMO=1 means demo, and anything
// else dev. A frozen instance runs the code as it was at launch, so an edit
// can't reload it mid-test or mid-film. No Electron here, so
// core/control.js can share the table.
export const INSTANCES = {
  dev: { name: "XenoNet Dev", port: 47842, demo: false },
  demo: { name: "XenoNet Dev (demo)", port: 47841, demo: true },
  test: { name: "XenoNet Dev (test)", port: 47843, demo: true, frozen: true },
  film: { name: "XenoNet Dev (film)", port: 47844, demo: false, frozen: true },
  installed: { name: "XenoNet", port: 47840, demo: false, frozen: true },
  showcase: { name: "XenoNet Dev (showcase)", port: 47845, demo: true, frozen: true },
};

export function instanceFrom(env = process.env) {
  const id = env.XENONET_INSTANCE || (env.XENONET_DEMO === "1" ? "demo" : "dev");
  if (!INSTANCES[id]) throw new Error(`XENONET_INSTANCE is ${id}; it can be ${Object.keys(INSTANCES).join(", ")}`);
  return { id, ...INSTANCES[id], port: Number(env.XENONET_PORT ?? INSTANCES[id].port) };
}
