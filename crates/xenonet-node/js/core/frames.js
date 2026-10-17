// core/frames.js in Rust (crates/xenonet-state/src/frames.rs): the frames
// posters and films of the day come in. The page keeps the JS, which it
// imports itself.
import { native } from "../index.js";

const F = native.framesConstants();

export const POSTER_SIZES = F.posterSizes;
export const DEFAULT_SIZE = F.defaultSize;
export const posterSize = (id) => POSTER_SIZES[native.posterSize(typeof id === "string" ? id : null)];

export const FILM_FRAMES = F.filmFrames;
export const DEFAULT_FRAME = F.defaultFrame;

export const FILM_SECONDS = F.filmSeconds;
export const FILM_CARDS_MS = F.filmCardsMs;
export const filmSeconds = (total) => native.filmSeconds(Number(total));
export const filmPlaySeconds = (total) => native.filmPlaySeconds(Number(total));

export const fitFrame = (frame, area) => native.fitFrame(frame, area);
