// UI layout audit. Load it into the preview page (served with the mock backend) and call `runAll()`.
//   const { runAll } = await import("/e2e/ui-audit.js"); await runAll();
// It visits every page, tab and dialog and reports text that overlaps other text, content that sticks out of its box,
// clipped text, hit targets under 24 px, logos that failed to load and page-level horizontal scrolling.

const label = (e) => `${e.tagName.toLowerCase()}${e.id ? "#" + e.id : ""}${e.className && typeof e.className === "string" ? "." + e.className.trim().split(/\s+/).slice(0, 2).join(".") : ""}${(e.textContent || "").trim() ? ` "${(e.textContent || "").trim().replace(/\s+/g, " ").slice(0, 28)}"` : ""}`;
const visible = (e) => {
  const r = e.getBoundingClientRect(), cs = getComputedStyle(e);
  return r.width > 0 && r.height > 0 && cs.visibility !== "hidden" && cs.display !== "none" && !e.closest("[hidden]") && !(e.closest("[inert]") && !e.closest("dialog"));
};
const scrollsByDesign = (e) => e.closest(".table-wrap, .log, .tabs, .import-list, .dialog-body, .sample, .content") === e || ["auto", "scroll"].includes(getComputedStyle(e).overflowX);

/** The part of a text box that is really drawn: cut by every box (the element's own included) that hides overflow. Null if none. */
function drawn(e, b) {
  let box = { left: b.left, right: b.right, top: b.top, bottom: b.bottom };
  for (let a = e; a && a !== document.body; a = a.parentElement) {
    const cs = getComputedStyle(a);
    if (cs.overflowX === "visible" && cs.overflowY === "visible") continue;
    const r = a.getBoundingClientRect();
    box = { left: Math.max(box.left, r.left), right: Math.min(box.right, r.right), top: Math.max(box.top, r.top), bottom: Math.min(box.bottom, r.bottom) };
    if (box.right - box.left < 1 || box.bottom - box.top < 1) return null;
  }
  return box;
}

/** Boxes of the text an element draws itself (not its children's). */
function textBoxes(e) {
  const out = [];
  for (const n of e.childNodes) {
    if (n.nodeType !== 3 || !n.textContent.trim()) continue;
    const r = document.createRange();
    r.selectNodeContents(n);
    for (const b of r.getClientRects()) {
      const d = b.width > 1 && b.height > 1 && drawn(e, b);
      if (d) out.push(d);
    }
  }
  return out;
}
const area = (a, b) => Math.max(0, Math.min(a.right, b.right) - Math.max(a.left, b.left)) * Math.max(0, Math.min(a.bottom, b.bottom) - Math.max(a.top, b.top));

export function audit(root) {
  const issues = [];
  const overlay = root.matches?.(".infobars") || root.closest?.(".infobars");
  const els = [...root.querySelectorAll("*")].filter((e) => visible(e) && !e.closest(".sr") && (overlay || !e.closest(".infobars")));
  const add = (kind, el, detail, other) => issues.push({ kind, el: label(el), other: other && label(other), detail });

  // 1. text drawn on top of other text
  const texts = els.map((e) => ({ e, boxes: textBoxes(e) })).filter((t) => t.boxes.length);
  for (let i = 0; i < texts.length; i++) for (let j = i + 1; j < texts.length; j++) {
    const a = texts[i], b = texts[j];
    if (a.e.contains(b.e) || b.e.contains(a.e)) continue;
    if (a.e.closest(".plist-head") || b.e.closest(".plist-head")) continue; // a sticky header sits over rows that scroll under it, by design
    const hit = a.boxes.some((x) => b.boxes.some((y) => area(x, y) > 8));
    if (hit) add("text overlaps text", a.e, "", b.e);
  }
  // 2. content sticking out of a box that does not scroll
  for (const e of els) {
    const cs = getComputedStyle(e);
    if (cs.display === "inline" || scrollsByDesign(e)) continue;
    if (e.scrollWidth > e.clientWidth + 1 && cs.overflowX === "visible" && e.clientWidth > 0) add("content sticks out", e, `${e.scrollWidth}px of content in ${e.clientWidth}px`);
    if (e.scrollWidth > e.clientWidth + 1 && ["hidden", "clip"].includes(cs.overflowX) && cs.textOverflow !== "ellipsis" && !e.matches(".module-body, .inner")) add("content clipped", e, `${e.scrollWidth}px of content in ${e.clientWidth}px`);
  }
  // 3. siblings whose boxes overlap in a row or grid
  for (const p of els) {
    const d = getComputedStyle(p).display;
    if (!/flex|grid/.test(d)) continue;
    const kids = [...p.children].filter((k) => visible(k) && !["absolute", "fixed"].includes(getComputedStyle(k).position));
    for (let i = 0; i < kids.length; i++) for (let j = i + 1; j < kids.length; j++) {
      if (area(kids[i].getBoundingClientRect(), kids[j].getBoundingClientRect()) > 12) add("boxes overlap", kids[i], "", kids[j]);
    }
  }
  // 4. text squeezed into a sliver
  for (const e of els) if ((e.children.length === 0) && e.textContent.trim().length > 6 && e.getBoundingClientRect().width < 28 && getComputedStyle(e).display !== "inline") add("text squeezed", e, `${Math.round(e.getBoundingClientRect().width)}px wide`);
  // 5. truncated names (info: allowed, but never when almost nothing is left)
  for (const e of els) if (getComputedStyle(e).textOverflow === "ellipsis" && e.scrollWidth > e.clientWidth + 1 && e.clientWidth < 60) add("name nearly gone", e, `${e.clientWidth}px shown of ${e.scrollWidth}px`);
  // 6. hit targets
  for (const e of els.filter((x) => x.matches("button, input:not([type=file]), select, a[href], [role=switch], [role=tab]"))) {
    const r = e.getBoundingClientRect();
    if (r.width < 24 || r.height < 24) add("hit target under 24px", e, `${Math.round(r.width)}x${Math.round(r.height)}`);
    if (e.matches("button, [role=switch], select") && !(e.textContent.trim() || e.getAttribute("aria-label") || e.title)) add("control without a name", e, "");
  }
  // 7. inputs without a label
  for (const e of els.filter((x) => x.matches("input:not([type=file]), select, textarea"))) if (!e.getAttribute("aria-label") && !e.labels?.length) add("input without a label", e, "");
  // 8. empty logo tiles
  for (const e of els.filter((x) => x.matches(".logo-mark:not(.fallback)"))) {
    const m = getComputedStyle(e).maskImage || getComputedStyle(e).webkitMaskImage;
    if (!m || m === "none") add("logo has no mask", e, "");
  }
  return issues;
}

async function logosLoad() {
  const bad = [];
  for (const id of ["apache", "php", "mariadb", "phpmyadmin", "tomcat", "wordpress", "filezilla", "jre", "mailpit", "runqora"]) {
    const ok = await fetch(`logos/${id}.svg`).then((r) => r.ok && r.headers.get("content-type")?.includes("svg")).catch(() => false);
    if (!ok) bad.push({ kind: "logo file did not load", el: `logos/${id}.svg`, detail: "" });
  }
  if (!(await fetch("logos/sftpgo.png").then((r) => r.ok && r.headers.get("content-type")?.includes("png")).catch(() => false))) bad.push({ kind: "logo file did not load", el: "logos/sftpgo.png", detail: "" });
  return bad;
}

const wait = (ms) => new Promise((r) => setTimeout(r, ms));
const click = async (sel, ms = 700) => { document.querySelector(sel)?.click(); await wait(ms); };

/** The walk, in steps small enough to run one per call. Each returns its issues (an empty list is a clean page). */
const find = (sel, re) => [...document.querySelectorAll(sel)].find((b) => re.test(b.textContent) && !b.disabled);
export const steps = [
  ["Whole page and logo files", async () => {
    const out = [];
    if (document.documentElement.scrollWidth > innerWidth + 1) out.push({ kind: "page scrolls sideways", el: "html", detail: `${document.documentElement.scrollWidth}px in ${innerWidth}px` });
    return [...out, ...(await logosLoad())];
  }],
  ["Modules (all cards open)", async () => { await click('[data-page="modules"]', 2000); document.querySelectorAll("#modules .chev[aria-expanded=false]").forEach((b) => b.click()); await wait(500); return audit(document.querySelector(".content")); }],
  ["Modules (cards closed)", async () => { document.querySelectorAll("#modules .chev[aria-expanded=true]").forEach((b) => b.click()); await wait(500); return audit(document.querySelector(".content")); }],
  ["Sites", async () => { await click('[data-page="sites"]', 2800); return audit(document.querySelector(".content")); }],
  ["Dialog: new site", async () => { await click('[data-page="sites"]', 600); await click("#site-new", 800); const d = document.querySelector("dialog"); const i = d ? audit(d) : [{ kind: "dialog did not open", el: "#site-new", detail: "" }]; d?.close(); document.querySelectorAll("dialog").forEach((x) => x.remove()); return i; }],
  ["Updates (all cards open)", async () => { await click('[data-page="updates"]', 3200); document.querySelectorAll("#updates .chev[aria-expanded=false]").forEach((b) => b.click()); await wait(500); return audit(document.querySelector(".content")); }],
  ["Dialog: confirm upgrade", async () => { const b = find("#updates .btn.sm", /Review/); if (!b) return []; b.click(); await wait(800); const d = document.querySelector("dialog"); const i = d ? audit(d) : []; d?.close(); await wait(200); return i; }],
  ["Ports", async () => { await click('[data-page="ports"]', 2200); const l = document.querySelector("#ports"); if (l) l.scrollTop = 0; return audit(document.querySelector(".content")); }],
  ["Activity", async () => { await click('[data-page="activity"]', 400); return audit(document.querySelector(".content")); }],
  ...["general", "apache", "php", "mariadb", "file"].map((t) => [`Settings: ${t}`, async () => { await click('[data-page="settings"]', 300); await click(`#tab-${t}`, 1800); return audit(document.querySelector(".content")); }]),
  ["About", async () => { await click('[data-page="about"]', 1500); return audit(document.querySelector(".content")); }],
  ["Notifications (three at once, long text)", async () => {
    await click('[data-page="settings"]', 300); await click("#tab-php", 1800);
    for (const label of ["Largest form post", "Largest upload", "Memory limit"]) {
      const i = [...document.querySelectorAll("#settings input")].find((x) => x.getAttribute("aria-label") === label);
      if (i) { i.value = "this is not a size and the message about it will be rather long indeed"; i.dispatchEvent(new Event("change")); await wait(1300); }
    }
    const i = audit(document.querySelector("#infobars"));
    document.querySelectorAll("#infobars .btn.ghost").forEach((b) => b.click());
    await wait(200);
    return i;
  }],
  ["Dialog: check setup", async () => { await click('[data-page="modules"]', 300); await click("#check-setup", 2200); const d = document.querySelector("dialog"); const i = d ? audit(d) : []; d?.close(); document.querySelectorAll("dialog").forEach((x) => x.remove()); return i; }],
];

export const names = steps.map(([n]) => n);
export async function runStep(i) {
  const issues = await steps[i][1]();
  return { step: steps[i][0], issues: [...new Set(issues.map((x) => `${x.kind}: ${x.el}${x.other ? "  <->  " + x.other : ""} ${x.detail}`))] };
}
