// One colour in, a readable accent set out. Pure functions (no DOM) so they can be tested in Node.
// The Windows accent can be anything from pale yellow to near black; the same colour is used as a button fill with a
// text label on it AND as text on a card, and those need different shades to reach 4.5:1 contrast.

const clamp = (v, a = 0, b = 1) => Math.min(b, Math.max(a, v));

/** "#abc", "abc", "#aabbcc" -> "#AABBCC", or null if it is not a colour. */
export function normalizeHex(s) {
  const m = /^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(String(s ?? "").trim());
  if (!m) return null;
  const h = m[1].length === 3 ? [...m[1]].map((c) => c + c).join("") : m[1];
  return "#" + h.toUpperCase();
}

export const hexToRgb = (hex) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
export const rgbToHex = (rgb) => "#" + rgb.map((v) => Math.round(clamp(v, 0, 255)).toString(16).padStart(2, "0")).join("").toUpperCase();

export function rgbToHsl([r, g, b]) {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b), l = (max + min) / 2, d = max - min;
  if (d === 0) return [0, 0, l];
  const s = d / (1 - Math.abs(2 * l - 1));
  const h = max === r ? ((g - b) / d) % 6 : max === g ? (b - r) / d + 2 : (r - g) / d + 4;
  return [(h * 60 + 360) % 360, s, l];
}

export function hslToRgb([h, s, l]) {
  const c = (1 - Math.abs(2 * l - 1)) * s, x = c * (1 - Math.abs(((h / 60) % 2) - 1)), m = l - c / 2;
  const [r, g, b] = h < 60 ? [c, x, 0] : h < 120 ? [x, c, 0] : h < 180 ? [0, c, x] : h < 240 ? [0, x, c] : h < 300 ? [x, 0, c] : [c, 0, x];
  return [(r + m) * 255, (g + m) * 255, (b + m) * 255];
}

/** WCAG relative luminance of "#RRGGBB". */
export function luminance(hex) {
  const [r, g, b] = hexToRgb(hex).map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrast(a, b) {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

const withLightness = (hsl, l) => rgbToHex(hslToRgb([hsl[0], hsl[1], clamp(l)]));

/** Move lightness one percent at a time until `ok(colour)` holds (or the end of the scale). */
function walk(hsl, direction, ok) {
  let l = hsl[2];
  for (let i = 0; i < 100; i++) {
    const c = withLightness(hsl, l);
    if (ok(c)) return c;
    l = clamp(l + direction * 0.01);
  }
  return withLightness(hsl, l);
}

export const SURFACE = { light: "#FFFFFF", dark: "#2B2B2B" }; // what text sits on: the card colour of each theme

/**
 * The set the UI needs from one base colour:
 *  fill    painted behind a label (buttons, switches, badges), with `on` as the label colour
 *  hover / pressed   the same, one step further in the direction that keeps the label readable
 *  text    the accent used AS text or a thin icon on a card
 * Every pairing is at least 4.5:1.
 */
export function deriveAccent(baseHex, dark) {
  const base = normalizeHex(baseHex);
  if (!base) throw new Error(`not a colour: ${baseHex}`);
  const hsl = rgbToHsl(hexToRgb(base));
  const on = contrast(base, "#FFFFFF") >= contrast(base, "#000000") ? "#FFFFFF" : "#000000";
  const toward = on === "#FFFFFF" ? -1 : 1; // darker under a white label, lighter under a black one
  const fill = walk(hsl, toward, (c) => contrast(c, on) >= 4.5);
  const fillHsl = rgbToHsl(hexToRgb(fill));
  const step = (n) => walk([fillHsl[0], fillHsl[1], clamp(fillHsl[2] + toward * n)], toward, (c) => contrast(c, on) >= 4.5);
  const surface = SURFACE[dark ? "dark" : "light"];
  const text = walk(hsl, dark ? 1 : -1, (c) => contrast(c, surface) >= 4.5);
  return { fill, hover: step(0.04), pressed: step(0.08), on, text };
}

/** Apply a derived set as CSS variables on an element (clears them when `set` is null). */
export function applyAccent(el, set) {
  const names = { "--color-accent": "fill", "--color-accent-hover": "hover", "--color-accent-pressed": "pressed", "--color-on-accent": "on", "--color-accent-text": "text" };
  for (const [prop, key] of Object.entries(names)) set ? el.style.setProperty(prop, set[key]) : el.style.removeProperty(prop);
}
