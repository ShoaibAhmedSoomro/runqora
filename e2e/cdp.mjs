// Drive the REAL Runqora window over the WebView2 debugging port (works while the desktop is locked).
//   $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--remote-debugging-port=9333"; Start-Process runqora.exe
//   node e2e/cdp.mjs <script.json-free: uses the steps below>   (output PNGs go to %TEMP%\runqora-shots)
import { writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const PORT = process.env.CDP_PORT || 9333;
const out = join(tmpdir(), "runqora-shots");
mkdirSync(out, { recursive: true });

const targets = await (await fetch(`http://127.0.0.1:${PORT}/json`)).json();
const page = targets.find((t) => t.type === "page");
if (!page) throw new Error("no page target: is the app running with the debugging port?");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0;
const waiting = new Map();
ws.onmessage = (m) => { const d = JSON.parse(m.data); if (d.id && waiting.has(d.id)) { waiting.get(d.id)(d); waiting.delete(d.id); } };
const send = (method, params = {}) => new Promise((res) => { const i = ++id; waiting.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
const js = async (expression) => { const r = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true }); if (r.result.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails.exception?.description || r.result.exceptionDetails)); return r.result.result.value; };
const shot = async (name) => { const r = await send("Page.captureScreenshot", { format: "png" }); writeFileSync(join(out, `${name}.png`), Buffer.from(r.result.data, "base64")); return join(out, `${name}.png`); };
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const size = (w, h) => send("Emulation.setDeviceMetricsOverride", { width: w, height: h, deviceScaleFactor: 1, mobile: false });

const report = { shots: [] };
// what the real window knows about itself
report.env = await js(`({ ua: navigator.userAgent.split(" ").slice(-2).join(" "), tauri: !!window.__TAURI__, dark: matchMedia("(prefers-color-scheme: dark)").matches, fx: document.documentElement.className })`);
// do all five logo files load inside the real app, and does each tile's mask resolve?
report.logos = await js(`Promise.all(["apache","php","mariadb","phpmyadmin","tomcat","wordpress","filezilla","jre","runqora"].map(async (id) => { const r = await fetch("logos/" + id + ".svg"); const t = await r.text(); const ok = await new Promise((res) => { const i = new Image(); i.onload = () => res(i.naturalWidth > 0); i.onerror = () => res(false); i.src = "logos/" + id + ".svg"; }); return { id, status: r.status, type: r.headers.get("content-type"), bytes: t.length, decodes: ok }; }))`);
report.tiles = await js(`[...document.querySelectorAll("#modules .card")].map((c) => { const l = c.querySelector(".logo-mark"); const cs = l && getComputedStyle(l); return { name: c.getAttribute("aria-label"), mask: cs ? (cs.maskImage || cs.webkitMaskImage || "none").slice(0, 60) : "no logo", size: l ? Math.round(l.getBoundingClientRect().width) + "x" + Math.round(l.getBoundingClientRect().height) : "-", color: cs && cs.backgroundColor }; })`);

for (const w of [840, 1040]) {
  await size(w, 700);
  await wait(400);
  for (const [name, nav] of [["modules", null], ["sites", "sites"], ["updates", "updates"], ["ports", "ports"], ["settings", "settings"]]) {
    if (nav) { await js(`document.querySelector('[data-page="${nav}"]').click()`); await wait(name === "updates" ? 4000 : 1500); }
    else { await js(`document.querySelector('[data-page="modules"]').click()`); await wait(900); }
    report.shots.push(`${name}-${w}: shows ${await js(`[...document.querySelectorAll(".page")].find((p) => !p.hidden)?.id`)}`);
    await shot(`${name}-${w}`);
  }
}
if (process.argv.includes("--audit")) {
  // run the same layout audit that the preview uses, inside the real window with real data
  const { readFileSync } = await import("node:fs");
  const code = readFileSync(new URL("./ui-audit.js", import.meta.url), "utf8").replace(/^export /gm, "");
  report.audit = {};
  for (const w of [560, 700, 840, 1040, 1400]) {
    await size(w, 760);
    await wait(500);
    const res = await js(`(async () => { ${code}
 const r = []; for (let i = 0; i < steps.length; i++) { try { r.push(await runStep(i)); } catch (e) { r.push({ step: steps[i][0], issues: ["audit step failed: " + e.message] }); } } return r.filter((x) => x.issues.length); })()`);
    report.audit[w] = res;
  }
}
console.log(JSON.stringify(report, null, 1));
ws.close();
// with --audit the exit code is the number of widths that had findings
process.exit(report.audit ? Object.values(report.audit).filter((v) => v.length).length : 0);
