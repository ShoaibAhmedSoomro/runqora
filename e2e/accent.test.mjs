// Node test for app/ui/accent.js: run with `node e2e/accent.test.mjs`. No framework: exits 1 on the first failure.
import { deriveAccent, normalizeHex, contrast, SURFACE, rgbToHsl, hexToRgb, hslToRgb, rgbToHex } from "../app/ui/accent.js";

let checks = 0;
const ok = (cond, what) => { checks++; if (!cond) { console.error("FAIL:", what); process.exit(1); } };

// parsing
ok(normalizeHex("#abc") === "#AABBCC" && normalizeHex("0067c0") === "#0067C0" && normalizeHex(" #FfEeDd ") === "#FFEEDD", "normalizes");
ok(["", "#12", "#12345", "#gggggg", "red", null, undefined, "#1234567"].every((v) => normalizeHex(v) === null), "rejects what is not a colour");
// round trip through HSL
for (const hex of ["#0067C0", "#FF0000", "#00FF00", "#808080", "#FFEB3B"]) ok(rgbToHex(hslToRgb(rgbToHsl(hexToRgb(hex)))) === hex, `hsl round trip ${hex}`);

const named = { "Windows 11 blue": "#0067C0", "Windows 10 blue": "#0078D4", yellow: "#FFEB3B", white: "#FFFFFF", black: "#000000", grey: "#808080", "pale pink": "#FFD1DC", "dark navy": "#000080", lime: "#00FF00", "Windows red": "#E81123" };
const cases = Object.values(named);
for (let h = 0; h < 360; h += 7) for (const s of [0.15, 0.5, 1]) for (const l of [0.1, 0.25, 0.4, 0.5, 0.6, 0.75, 0.9]) cases.push(rgbToHex(hslToRgb([h, s, l])));

for (const base of cases) for (const dark of [false, true]) {
  const a = deriveAccent(base, dark);
  const tag = `${base} ${dark ? "dark" : "light"}`;
  ok(Object.values(a).every((v) => /^#[0-9A-F]{6}$/.test(v)), `${tag}: every value is a colour (${JSON.stringify(a)})`);
  ok(contrast(a.fill, a.on) >= 4.5, `${tag}: label on fill ${contrast(a.fill, a.on).toFixed(2)}`);
  ok(contrast(a.hover, a.on) >= 4.5 && contrast(a.pressed, a.on) >= 4.5, `${tag}: label on hover and pressed`);
  ok(contrast(a.text, SURFACE[dark ? "dark" : "light"]) >= 4.5, `${tag}: accent as text on a card ${contrast(a.text, SURFACE[dark ? "dark" : "light"]).toFixed(2)}`);
  ok(deriveAccent(base, dark).fill === a.fill, `${tag}: deterministic`);
}
// a colour that already works is not changed needlessly
ok(deriveAccent("#0067C0", false).fill === "#0067C0" && deriveAccent("#0067C0", false).text === "#0067C0", "a good colour passes through untouched");
ok(deriveAccent("#0067C0", false).on === "#FFFFFF" && deriveAccent("#60CDFF", true).on === "#000000", "label colour follows the fill");
let threw = false; try { deriveAccent("nope", false); } catch { threw = true; }
ok(threw, "a bad colour is an error, not NaN");

console.log(`accent.js: ${checks} checks over ${cases.length} colours x 2 themes passed`);
