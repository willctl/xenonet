// The frames posters and films of the day come in, shared by the page
// (src/poster.js, src/film.js), the desktop app (electron/main.js) and the
// xenonet film command. Plain data and arithmetic, no Node imports, so the
// page uses it too.

// A poster's sizes, in pixels: a story (9:16), a portrait (4:5), a link
// card (what X and others show for a link) and a wide frame (16:9).
export const POSTER_SIZES = [
  { id: "story", width: 1080, height: 1920, label: "Story", ratio: "9:16" },
  { id: "portrait", width: 1080, height: 1350, label: "Portrait", ratio: "4:5" },
  { id: "card", width: 1200, height: 630, label: "Link card", ratio: "1.91:1" },
  { id: "wide", width: 1920, height: 1080, label: "Wide", ratio: "16:9" },
];
export const DEFAULT_SIZE = "portrait";
export const posterSize = (id) => POSTER_SIZES.find((s) => s.id === id) ?? POSTER_SIZES.find((s) => s.id === DEFAULT_SIZE);

// A film's frame: a story (9:16), the one the page makes, and a wide one
// for the xenonet film command's --size.
export const FILM_FRAMES = { story: { width: 1080, height: 1920 }, wide: { width: 1920, height: 1080 } };
export const DEFAULT_FRAME = "story";

// How long a film is, cards included: 15 to 30 seconds. The title card and
// the end card take 8 of them (src/film.js); the day plays for the rest.
export const FILM_SECONDS = { min: 15, max: 30, default: 20 };
export const FILM_CARDS_MS = 8000;
export const filmSeconds = (total) => Math.max(FILM_SECONDS.min, Math.min(FILM_SECONDS.max, Math.round(Number(total)) || FILM_SECONDS.default));
export const filmPlaySeconds = (total) => filmSeconds(total) - FILM_CARDS_MS / 1000;

// The largest size in a frame's shape that fits an area (a display's work
// area, in pixels), so the window a film records in fits the screen: the
// recording is scaled to the frame's width afterwards. Even numbers, which
// the video's pixel format needs.
export function fitFrame(frame, area) {
  const k = Math.min(1, area.width / frame.width, area.height / frame.height);
  const even = (n) => Math.max(2, Math.round(n) & ~1);
  return { width: even(frame.width * k), height: even(frame.height * k) };
}
