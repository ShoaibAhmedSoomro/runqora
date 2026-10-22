// Read what the REAL window currently shows. Needs the app started with --remote-debugging-port (see ui.ps1).
//   node e2e/probe.mjs [screenshot-name]   -> prints JSON, optionally saves %TEMP%\runqora-shots\<name>.png
import { writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const PORT = process.env.CDP_PORT || 9333;
const page = (await (await fetch(`http://127.0.0.1:${PORT}/json`)).json()).find((t) => t.type === "page");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => (ws.onopen = r));
let id = 0;
const waiting = new Map();
ws.onmessage = (m) => { const d = JSON.parse(m.data); waiting.get(d.id)?.(d); waiting.delete(d.id); };
const send = (method, params = {}) => new Promise((res) => { const i = ++id; waiting.set(i, res); ws.send(JSON.stringify({ id: i, method, params })); });
const js = async (expression) => (await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true })).result.result.value;

const out = await js(`(async () => {
  const root = document.documentElement, cs = getComputedStyle(root);
  const v = (n) => cs.getPropertyValue(n).trim();
  const probe = document.createElement("button"); probe.className = "btn accent"; probe.textContent = "x"; document.body.append(probe);
  const btn = getComputedStyle(probe); const filled = btn.backgroundColor, label = btn.color; probe.remove();
  return {
    dataTheme: root.getAttribute("data-theme"),
    osDark: matchMedia("(prefers-color-scheme: dark)").matches,
    pageBackground: getComputedStyle(document.body).backgroundColor,
    accent: { fill: v("--color-accent"), hover: v("--color-accent-hover"), on: v("--color-on-accent"), text: v("--color-accent-text") },
    accentButton: { background: filled, label },
    appearance: await window.__TAURI__.core.invoke("appearance"),
  };
})()`);
if (process.argv[2]) {
  mkdirSync(join(tmpdir(), "runqora-shots"), { recursive: true });
  const shot = await send("Page.captureScreenshot", { format: "png" });
  writeFileSync(join(tmpdir(), "runqora-shots", `${process.argv[2]}.png`), Buffer.from(shot.result.data, "base64"));
}
console.log(JSON.stringify(out));
ws.close();
process.exit(0);
