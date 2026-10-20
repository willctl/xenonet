// core/assistant/source.js in Rust (crates/xenonet-assistant/src/
// source.rs): what the assistant reads from, XenoNet's own state as the
// service holds it. The view is the Rust's; the state and the actors'
// registry are other modules', handed over and asked here.
import { native } from "../../index.js";
import { peer } from "../../peer.js";

const { productOf } = await import(peer("actors/products.js"));

const VIEW_MS = native.asstConstants().viewMs;

const product = (id) => productOf(id ?? undefined)?.id ?? null;
const health = (block) => block?.health?.() ?? null;

export function stateSource({ state, actors = null, now = Date.now }) {
  let cached = null;
  function view() {
    const t = now();
    if (cached && t - cached.at < VIEW_MS) return cached;
    const live = state.liveSnapshot();
    const nodes = [...state.index.values()];
    const list = actors ? actors.list({ now: t }).actors : [];
    const v = native.asstView(
      { t, nodes, live, actors: list, hasActors: !!actors, claude: native.asstSourceOf(health(state.claudeSessions), health(state.live?.tailer), t), codex: native.asstSourceOf(health(state.codexThreads), health(state.live?.tailer), t) },
      (id) => state.homeOf(id) ?? null,
      product,
      (id) => !!actors?.isRegistered(id),
    );
    cached = { ...v, places: new Map(v.places) };
    return cached;
  }
  return {
    view,
    stats: (id) => state.sessionStats(id),
    actor(id) {
      const a = actors?.snapshot(id);
      return a ? native.asstActor(a, product(a.product?.id)) : null;
    },
    has: (id) => state.index.has(id),
  };
}
