// On a narrow screen (a phone, a tablet held upright, a slim window) the
// two columns of panels don't fit beside the map. There they become one
// sheet at the bottom, and a row of tabs picks which panels it shows; the
// tab that's open closes it again and leaves the map. Selecting something
// opens its inspector in the sheet, and closing it puts back what was open.
// Wider, none of this shows, and every panel is where it always was.
//
// Each panel's icon is its tab's, and it also sits at the top left of the
// panel, before its title, on every screen, so a panel reads the same in a
// column as in the sheet. The Link panel's is its signal light, which it
// has already, and the inspector has none.
export const NARROW = "(max-width: 900px)";

const ICON = {
  sessions: '<path d="M2 3.5h12M2 8h12M2 12.5h8" />',
  activity: '<path d="M1.5 8.5h3l2-5 3 9 2-4h3" />',
  filters: '<path d="M2 3h12l-4.5 5.5V13l-3 1.5v-6Z" />',
  load: '<path d="M2.5 11.5a5.5 5.5 0 1 1 11 0" /><path d="M8 11.5l2.5-4" />',
  tokens: '<path d="M8 1.5 14.5 8 8 14.5 1.5 8Z" /><path d="M8 5 11 8 8 11 5 8Z" />',
  link: '<circle cx="8" cy="8" r="1.6" /><path d="M5 5a4.2 4.2 0 0 0 0 6M11 5a4.2 4.2 0 0 1 0 6M2.8 2.8a7.3 7.3 0 0 0 0 10.4M13.2 2.8a7.3 7.3 0 0 1 0 10.4" />',
  legend: '<path d="M3 3.5h1.5M3 8h1.5M3 12.5h1.5M7 3.5h6M7 8h6M7 12.5h6" />',
};
export const SHEETS = [
  ["sessions", "Sessions"],
  ["activity", "Activity"],
  ["filters", "Filters"],
  ["load", "Load"],
  ["tokens", "Tokens"],
  ["link", "Link"],
  ["legend", "Legend"],
];

const icon = (id, cls = "") => `<svg${cls ? ` class="${cls}"` : ""} viewBox="0 0 16 16" aria-hidden="true">${ICON[id]}</svg>`;

export function createSheets({ tabs, inspector, onChange = () => {} }) {
  const narrow = matchMedia(NARROW);
  let current = "";
  let before = "";

  tabs.innerHTML = SHEETS.map(([id, label]) => `<button type="button" data-sheet-tab="${id}" aria-pressed="false" title="${label}">${icon(id)}<span>${label}</span></button>`).join("");
  for (const p of document.querySelectorAll(".dock > .panel[data-sheet]")) {
    const title = p.querySelector(":scope > .panel-title > span:first-child");
    if (title && Object.hasOwn(ICON, p.dataset.sheet) && !title.querySelector(".panel-icon")) title.insertAdjacentHTML("afterbegin", icon(p.dataset.sheet, "panel-icon"));
  }

  function show(id = "") {
    current = id;
    for (const p of document.querySelectorAll(".dock > .panel[data-sheet]")) p.classList.toggle("sheet-on", !!id && p.dataset.sheet === id);
    for (const b of tabs.querySelectorAll("[data-sheet-tab]")) {
      const on = b.dataset.sheetTab === id;
      b.classList.toggle("on", on);
      b.setAttribute("aria-pressed", String(on));
    }
    document.body.dataset.sheet = id;
    onChange(id);
  }

  tabs.addEventListener("click", (e) => {
    const b = e.target.closest("[data-sheet-tab]");
    if (b) show(b.dataset.sheetTab === current ? "" : b.dataset.sheetTab);
  });

  new MutationObserver(() => {
    if (!inspector.hidden && current !== "selected") {
      before = current;
      show("selected");
    } else if (inspector.hidden && current === "selected") show(before);
  }).observe(inspector, { attributes: true, attributeFilter: ["hidden"] });

  // Turning a window narrow or wide again reframes the map around the panels.
  narrow.addEventListener("change", () => onChange(current));
  show("");

  return {
    show,
    get current() {
      return current;
    },
    get narrow() {
      return narrow.matches;
    },
  };
}
