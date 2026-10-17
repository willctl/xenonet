// core/themes.js in Rust (crates/xenonet-app/src/themes.rs), for the
// service and the command: the themes, yours folded on top of the
// built-ins, what fits where, and the colors' contrast and distance. The
// console page keeps core/themes.js itself; the Rust's data is made from it
// (node crates/xenonet-app/themes.mjs).
import { native } from "../index.js";

export const DEFAULT_THEME = native.appDefaultTheme();
export const STYLES = native.appThemeStyles();
export const BUILT_IN = native.appBuiltInThemes();

export const merge = (base, over) => native.appThemeMerge(base, over);
export const conform = (base, over, at = "") => native.appThemeConform(base, over, String(at));
export const complete = (theme) => native.appThemeComplete(theme);
export const sonarGap = (every) => native.appSonarGap(every) ?? null;
// In an object with nothing inherited, so looking one up by id is safe.
export const themes = (custom = {}) => Object.assign(Object.create(null), native.appThemes(custom));
export const contrast = (a, b) => native.appContrast(String(a), String(b));
export const colorDistance = (a, b) => native.appColorDistance(String(a), String(b));
export const ringColors = (theme) => native.appRingColors(theme);
