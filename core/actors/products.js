// The actor products XenoNet knows (docs/design/actors/interface.md, section
// 7, and the decisions at merge point A in docs/design/next-phase.md): the
// actor each one is (`slug`), what it's called, who makes it, whether XenoNet
// observes it (an observer you can turn off), and whether its actor may
// report itself (register). A product's slug, and the name it goes by, are
// that product's alone.
//
// Separately, whether you may pair it as a remote actor (`remote`): a
// connection you approve in XenoNet's window that lets it read what's going
// on, without registering (core/actors/connections.js). Pairing never makes
// a product register, and registering never goes through a pairing.
//
//   hermes-agent  observed and attributed; registers later, through its
//                 XenoNet plugin (Connect), from under its own processes;
//                 a remote connection joins its actor, as another instance
//   grok-bot      observed: presence and process ancestry; registers, since
//                 2026-10-08, from a command its app runs on this Mac (a
//                 Bot running xenonet actor, each run approved in the app),
//                 joining its observed actor; a remote connection joins
//                 its actor
//   muse          observed: presence alone, from its Mac app; registers,
//                 since 2026-10-08, when you connect it through the actor
//                 gateway (it runs nothing on this Mac), joining its
//                 observed actor; it isn't paired as a remote actor
//   numbat        an opt-in bridge, not an agent: XenoNet reads the events
//                 numbat records (XENONET_NUMBAT_FILE) and shows each agent
//                 in them as actor:numbat-<agent>, observed; none registers
//   dots          not observed and never registers; paired as a remote
//                 actor, actor:dots or actor:dots-<suffix> for another
export const PRODUCTS = {
  "hermes-agent": { id: "hermes-agent", slug: "hermes", name: "Hermes Agent", short: "Hermes", vendor: "Nous Research", observed: true, registers: true, remote: true },
  "grok-bot": { id: "grok-bot", slug: "grok-bot", name: "Grok Bot", short: "Grok Bot", vendor: "SpaceXAI", observed: true, registers: true, remote: true },
  muse: { id: "muse", slug: "muse", name: "Muse", short: "Muse", vendor: "Meta", observed: true, registers: true, remote: false },
  numbat: { id: "numbat", slug: "numbat", name: "numbat", short: "numbat", vendor: "Perplexity", observed: true, bridge: true, registers: false, remote: false },
  dots: { id: "dots", slug: "dots", name: "dots", short: "dots", vendor: "OpenAI", observed: false, registers: false, remote: true },
};

export const productOf = (id) => (Object.hasOwn(PRODUCTS, id ?? "") ? PRODUCTS[id] : null);

// Why a product's actor can't report itself, for those that can't.
export const WHY_NOT = {
  dots: "OpenAI's dots don't report themselves to XenoNet",
  numbat: "Agents seen through numbat are observed from its records; they don't report themselves to XenoNet",
};

// The products with an observer, which you can turn on and off.
// The numbat bridge is left out: it's on only when you name its file, and
// turned off with xenonet actor observe numbat off.
export const OBSERVED = Object.values(PRODUCTS).filter((p) => p.observed && !p.bridge);

// Whether a slug is a product's own: hermes and hermes-<profile> are
// Hermes's (one actor per profile), dots and dots-<suffix> OpenAI's dots
// (one actor per dot you pair, since one account can have several), the
// others their own slug.
export function productOfSlug(slug) {
  if (slug === "hermes" || /^hermes-/.test(slug)) return "hermes-agent";
  if (slug === "dots" || /^dots-/.test(slug)) return "dots";
  if (slug === "numbat" || /^numbat-/.test(slug)) return "numbat";
  return Object.values(PRODUCTS).find((p) => p.slug === slug)?.id ?? null;
}
