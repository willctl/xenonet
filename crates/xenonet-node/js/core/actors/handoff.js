// core/actors/handoff.js in Rust (crates/xenonet-actors/src/handoff.rs):
// what to give an app so its agent reaches XenoNet's actor gateway. The
// address's host is read with Node's URL.
import { native } from "../../index.js";

const hostOf = (url) => {
  try {
    return new URL(url).host;
  } catch {
    return "";
  }
};

export const handoff = (product, { url, token = null }) => native.actorHandoff(product, url, hostOf(url), token);
export const wakeHelp = (product) => native.actorWakeHelp(product);
