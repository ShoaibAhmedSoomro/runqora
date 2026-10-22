// UI for Runqora. All state lives in the Rust core; this file only renders it and sends commands.
const invoke = window.__TAURI__ ? window.__TAURI__.core.invoke : window.mockInvoke; // mock.js: design preview in a plain browser

import { applyAccent, deriveAccent, normalizeHex } from "./accent.js";

const $ = (s) => document.querySelector(s);
function h(tag, props = {}, ...kids) {
  const e = document.createElement(tag);
  for (const [k, v] of Object.entries(props)) {
    if (v === false || v == null) continue;
    if (k === "class") e.className = v;
    else if (k === "on") for (const [ev, fn] of Object.entries(v)) e.addEventListener(ev, fn);
    else if (k === "data") for (const [dk, dv] of Object.entries(v)) e.dataset[dk] = dv;
    else if (k in e && k !== "list") e[k] = v;
    else e.setAttribute(k, v === true ? "" : v);
  }
  e.append(...kids.flat().filter((x) => x != null && x !== false));
  return e;
}
const icon = (code) => h("i", { class: "icon", "aria-hidden": "true", textContent: String.fromCodePoint(code) });

// official brand colours (logos/brand.json), with a lighter variant for dark backgrounds
const BRAND = await fetch("logos/brand.json").then((r) => r.json()).catch(() => ({}));
function lum(hex) {
  const c = [0, 2, 4].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((v) => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
}
const lift = (hex) => "#" + [0, 2, 4].map((i) => Math.round(parseInt(hex.slice(i, i + 2), 16) * 0.4 + 255 * 0.6).toString(16).padStart(2, "0")).join("");
// a very light brand colour (Tomcat's yellow) vanishes on white: darken it for the light theme
const deepen = (hex) => "#" + [0, 2, 4].map((i) => Math.round(parseInt(hex.slice(i, i + 2), 16) * 0.5).toString(16).padStart(2, "0")).join("");
/** A module's own logo mark in its brand colour (a CSS mask, so it follows light/dark). */
function logo(id, size = 24) {
  const hex = BRAND[id];
  if (hex?.startsWith("img:")) return h("img", { class: "logo-img-mark", alt: "", "aria-hidden": "true", src: `logos/${id}.${hex.slice(4)}`, style: `--size:${size}px` }); // full-colour marks keep their own colours
  if (!hex) return h("span", { class: "logo-mark fallback", "aria-hidden": "true", style: `--size:${size}px`, textContent: id[0].toUpperCase() });
  const url = `url(logos/${id}.svg)`;
  return h("span", { class: "logo-mark", "aria-hidden": "true", style: `--size:${size}px;--brand:${lum(hex) > 0.5 ? deepen(hex) : "#" + hex};--brand-dark:${lum(hex) < 0.1 ? lift(hex) : "#" + hex};-webkit-mask-image:${url};mask-image:${url}` });
}
const logoByName = (name) => view.modules.find((m) => m.name === name)?.id;

/** Fluent ContentDialog on the native <dialog>: focus trap and Esc come for free. Resolves true when confirmed. */
function dialog({ title, content, confirm, accent = true }) {
  return new Promise((resolve) => {
    const d = h("dialog", { class: "dialog", "aria-labelledby": "dlg-title" },
      h("h2", { id: "dlg-title" }, title),
      h("div", { class: "dialog-body" }, content),
      h("div", { class: "dialog-actions" },
        h("button", { class: `btn ${accent ? "accent" : ""}`, on: { click: () => d.close("ok") } }, confirm),
        h("button", { class: "btn", autofocus: true, on: { click: () => d.close("cancel") } }, "Cancel"))); // default focus on the safe choice
    d.addEventListener("close", () => { d.remove(); resolve(d.returnValue === "ok"); });
    document.body.append(d);
    d.showModal();
  });
}

/** One-line facts shown inside a module's details. */
const NOTE = {
  jre: "Java for Tomcat. Installed for you the first time you install Tomcat.",
  tomcat: "Runs Java web applications (.war files go in the tomcat/webapps folder). Needs the Java runtime, which Install adds.",
  mailpit: "Catches every email your PHP sites send and shows it on the Open page, so nothing leaves this PC. PHP is pointed at it for you (SMTP port 1025).",
  sftpgo: "FTP on port 21 and SFTP on port 2022, this PC only. The first time you Open it, it asks you to make an admin account.",
  wordpress: "The WordPress files. New WordPress site copies them into a new site, makes its database and fills in wp-config.php.",
};
const STATUS = { missing: "Not installed", installed: "Installed", stopped: "Stopped", running: "Running", blocked: "Port in use" };
const G = { play: 0xe768, stop: 0xe71a, globe: 0xe774, page: 0xe7c3, down: 0xe70d, download: 0xe896, ok: 0xe930, err: 0xea39, warn: 0xe7ba, info: 0xe946, close: 0xe711, folder: 0xe8b7, sync: 0xe895, back: 0xe7a7, link: 0xe71b };

const view = { modules: [], externals: [], sites: [], loaded: false, entered: false, updEntered: false, updates: null, checking: false, checkedAt: null, autoMode: "notify", settingsTab: "general", problems: 0, page: "modules", open: new Set(), busy: new Map(), log: [], bars: [], errors: 0, json: "" };

// ---- appearance: theme and accent, synced with Windows ----
const dark = matchMedia("(prefers-color-scheme: dark)");
let appearance = { theme: "system", accent_mode: "system", accent_color: "#0067C0", system_accent: null };
const isDark = () => appearance.theme === "dark" || (appearance.theme === "system" && dark.matches);

function paintAppearance() {
  const root = document.documentElement;
  appearance.theme === "system" ? root.removeAttribute("data-theme") : root.setAttribute("data-theme", appearance.theme);
  const base = normalizeHex(appearance.accent_mode === "custom" ? appearance.accent_color : appearance.system_accent);
  applyAccent(root, base ? deriveAccent(base, isDark()) : null); // no readable colour known: keep the built-in blue
  paintLogos();
}

/** Brand colours that are too dark for the dark theme are lightened by CSS; nothing to repaint, but icons may need it. */
function paintLogos() {}

async function loadAppearance() {
  let next;
  try { next = await invoke("appearance"); } catch { return; }
  const changed = JSON.stringify(next) !== JSON.stringify(appearance);
  const themeChanged = next.theme !== appearance.theme;
  appearance = next;
  if (changed) paintAppearance();
  if (themeChanged) invoke("apply_theme", { theme: next.theme }).catch(() => {}); // title bar and Mica follow
}
dark.addEventListener("change", paintAppearance);

// ---- feedback: InfoBars (transient) + Activity (history) ----
function note(level, title, message, action) {
  const t = new Date().toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  view.log.push({ t, level, text: title ? `${title}: ${message}` : message });
  if (level === "error" && view.page !== "activity") view.errors++;
  if (level !== "info") {
    const bar = { level, title, message, action, id: Math.random() };
    view.bars = [...view.bars, bar].slice(-3);
    if (level === "success" && !action) setTimeout(() => dismiss(bar.id), 6000); // errors and anything with a button stay until closed, like Windows InfoBars
  }
  paintBars();
  paintLog();
  paintNav();
}
function dismiss(id) {
  view.bars = view.bars.filter((b) => b.id !== id);
  paintBars();
}
function paintBars() {
  $("#infobars").replaceChildren(
    ...view.bars.map((b) => infobar(b.level, b.title, b.message, h("button", { class: "btn ghost sm", "aria-label": "Close", on: { click: () => dismiss(b.id) } }, icon(G.close)),
      b.action && [h("button", { class: "btn sm", on: { click: () => { dismiss(b.id); b.action.run(); } } }, b.action.label)])),
  );
}
function infobar(level, title, message, close, actions) {
  const g = { success: G.ok, error: G.err, warning: G.warn, info: G.info }[level];
  return h("div", { class: `infobar ${level}`, role: level === "error" ? "alert" : "status" }, icon(g), h("div", { class: "msg" }, h("span", { class: "strong" }, title ? `${title} ` : ""), message, actions && h("div", { class: "infobar-actions" }, actions)), close || h("span"));
}

// ---- commands ----
async function act(m, label, verb, command, args = {}) {
  view.busy.set(m.id, label);
  paintModules(true);
  try {
    note("success", "", await invoke(command, { id: m.id, ...args }));
  } catch (e) {
    note("error", `Couldn't ${verb} ${m.name}.`, String(e));
  }
  view.busy.delete(m.id);
  refresh(true);
}
const open = (m, f) => invoke("module_open_file", { id: m.id, rel: f.rel }).catch((e) => note("error", m.name, String(e)));

// ---- modules page ----
function moduleCard(m, i) {
  const busy = view.busy.get(m.id);
  const hasBody = m.configs.length || m.logs.length || m.can_service || m.blocker || NOTE[m.id];
  const isOpen = view.open.has(m.id) || !!m.blocker;
  const k = (a) => ({ key: `${m.id}:${a}` });

  const status = busy
    ? h("div", { class: "status" }, h("span", { class: "ring" }), busy)
    : h("div", { class: `status ${m.status}` }, h("span", { class: "dot" }), STATUS[m.status] + (m.pid ? ` · PID ${m.pid}` : ""));

  const actions = h("div", { class: "actions" });
  if (m.status === "missing") {
    actions.append(h("button", { class: "btn accent", disabled: !!busy, data: k("install"), on: { click: () => act(m, "Installing", "install", "component_install") } }, icon(G.download), "Install"));
  } else if (m.id === "wordpress") {
    actions.append(h("button", { class: "btn accent", disabled: !!busy, data: k("newsite"), on: { click: newWordPress } }, icon(0xe710), "New WordPress site"));
  } else if (m.server) {
    const running = m.status === "running";
    actions.append(
      h("button", { class: `btn ${running ? "" : "accent"}`, disabled: !!busy || m.status === "blocked", data: k("toggle"), on: { click: () => act(m, running ? "Stopping" : "Starting", running ? "stop" : "start", running ? "module_stop" : "module_start") } }, icon(running ? G.stop : G.play), running ? "Stop" : "Start"),
      m.has_admin && h("button", { class: "btn", disabled: !!busy || !running, data: k("admin"), on: { click: () => invoke("module_admin", { id: m.id }).catch((e) => note("error", m.name, String(e))) } }, icon(G.globe), "Open"),
    );
  }

  const head = h("div", { class: `module-head ${m.status}` },
    h("div", { class: "tile", "aria-hidden": "true" }, logo(m.id, 24)),
    h("div", { class: "titles" }, h("div", { class: "name" }, m.name), h("div", { class: "muted small" }, m.version + (m.server ? ` · Port ${m.port}` : ""))),
    status, actions,
    hasBody && h("button", { class: "btn ghost chev", "aria-expanded": String(isOpen), "aria-label": `Details for ${m.name}`, data: k("expand"), on: { click: (e) => toggleBody(e.currentTarget, m.id) } }, icon(G.down)),
  );

  const inner = h("div", { class: "inner" });
  if (m.blocker) {
    const b = m.blocker;
    inner.append(infobar("warning", `Port ${m.port} is in use.`, `${b.name} (PID ${b.pid}) is listening on it.${b.path ? ` ${b.path}` : ""}`, undefined, blockerActions(m)));
  }
  if (NOTE[m.id]) inner.append(h("div", { class: "row plain" }, h("div", { class: "label" }, h("div", { class: "muted small" }, NOTE[m.id])), h("span")));
  if (m.can_service) {
    inner.append(h("div", { class: "row" },
      h("div", { class: "label" }, h("div", { class: "strong" }, "Run as a Windows service"), h("div", { class: "muted small" }, "Starts with Windows. Changing this asks for administrator approval.")),
      h("button", { class: "switch", role: "switch", "aria-checked": String(m.service_installed), "aria-label": `Run ${m.name} as a Windows service`, disabled: !!busy, data: k("service"), on: { click: () => act(m, m.service_installed ? "Removing service" : "Installing service", m.service_installed ? "remove the service for" : "install the service for", "module_service", { install: !m.service_installed }) } },
        h("span", { class: "track" }, h("span", { class: "thumb" })), m.service_installed ? "On" : "Off"),
    ));
  }
  if (["apache", "php", "mariadb"].includes(m.id)) {
    inner.append(h("div", { class: "row" },
      h("div", { class: "label" }, h("div", { class: "strong" }, "Settings"), h("div", { class: "muted small" }, "Port, limits, extensions and more: change them here instead of editing files.")),
      h("button", { class: "btn sm accent", data: k("configure"), on: { click: () => { view.settingsTab = m.id; goto("settings"); } } }, icon(0xe713), "Configure")));
  }
  for (const [title, desc, files] of [["Configuration files", "Advanced: opens the raw file. Settings above covers normal changes.", m.configs], ["Logs", "Opens in your editor.", m.logs]]) {
    if (!files.length) continue;
    inner.append(h("div", { class: "row" },
      h("div", { class: "label" }, h("div", { class: "strong" }, title), h("div", { class: "muted small" }, desc)),
      h("div", { class: "files" }, files.map((f) => h("button", { class: "btn sm", disabled: !f.exists, title: f.exists ? f.rel : `${f.rel} does not exist yet`, data: k(f.rel), on: { click: () => open(m, f) } }, icon(G.page), f.label))),
    ));
  }
  const body = h("div", { class: `module-body${isOpen ? " open" : ""}`, inert: !isOpen }, inner);
  return h("article", { class: `card${view.entered ? "" : " enter"}`, style: `--i:${i}`, "aria-label": m.name }, head, body);
}

/** Open or close a card body in place, so the height can animate (a repaint would replace the node). */
function toggleBody(btn, id) {
  const body = btn.closest(".card").querySelector(".module-body");
  const open = !body.classList.contains("open");
  body.classList.toggle("open", open);
  body.inert = !open;
  btn.setAttribute("aria-expanded", String(open));
  open ? view.open.add(id) : view.open.delete(id);
}

const skeleton = (i) => h("div", { class: "card skeleton enter", style: `--i:${i}`, "aria-hidden": "true" }, h("div", { class: "sk-tile" }), h("div", { class: "sk-lines" }, h("div", { class: "sk-line w60" }), h("div", { class: "sk-line w30" })));
function paintModules() {
  const key = document.activeElement?.dataset?.key;
  $("#modules").setAttribute("aria-busy", String(!view.loaded));
  $("#modules").replaceChildren(...(view.loaded ? view.modules.map(moduleCard) : [0, 1, 2, 3].map(skeleton)));
  if (view.loaded && !view.entered) setTimeout(() => (view.entered = true), 900); // play the entrance once, not on every refresh
  if (key) $(`[data-key="${CSS.escape(key)}"]`)?.focus();
}

// ---- programs found on this PC (FileZilla Server, Mercury Mail): run, never installed, by Runqora ----
async function actExt(x, label, verb, command) {
  view.busy.set(`ext:${x.id}`, label);
  paintExternals();
  try { note("success", "", await invoke(command, { id: x.id })); } catch (e) { note("error", `Couldn't ${verb} ${x.name}.`, String(e)); }
  view.busy.delete(`ext:${x.id}`);
  refresh(true);
}
function externalCard(x, i) {
  const busy = view.busy.get(`ext:${x.id}`);
  const k = (a) => ({ key: `ext:${x.id}:${a}` });
  const status = busy ? h("div", { class: "status" }, h("span", { class: "ring" }), busy) : h("div", { class: `status ${x.status}` }, h("span", { class: "dot" }), STATUS[x.status] + (x.pid ? ` · PID ${x.pid}` : ""));
  const actions = h("div", { class: "actions" });
  if (x.status === "missing") {
    actions.append(h("button", { class: "btn", disabled: !!busy, data: k("get"), title: "Opens the publisher's own download page", on: { click: () => invoke("external_get", { id: x.id }).catch((e) => note("error", x.name, String(e))) } }, icon(G.link), "Get it"));
  } else {
    const running = x.status === "running";
    actions.append(
      h("button", { class: `btn ${running ? "" : "accent"}`, disabled: !!busy || x.status === "blocked", data: k("toggle"), on: { click: () => actExt(x, running ? "Stopping" : "Starting", running ? "stop" : "start", running ? "external_stop" : "external_start") } }, icon(running ? G.stop : G.play), running ? "Stop" : "Start"),
      h("button", { class: "btn", disabled: !!busy, data: k("open"), on: { click: () => invoke("external_open", { id: x.id }).catch((e) => note("error", x.name, String(e))) } }, icon(0xe713), x.has_gui ? "Settings" : "Open"));
  }
  const sub = x.status === "missing" ? "Not found on this PC. Runqora does not install it; get it from its publisher." : `Port ${x.port} · ${x.path}`;
  const head = h("div", { class: `module-head ${x.status}` }, h("div", { class: "tile", "aria-hidden": "true" }, logo(x.id, 24)), h("div", { class: "titles" }, h("div", { class: "name" }, x.name), h("div", { class: "muted small path-line", title: sub }, sub)), status, actions);
  const inner = x.blocker ? h("div", { class: "inner" }, infobar("warning", `Port ${x.port} is in use.`, `${x.blocker.name} (PID ${x.blocker.pid}) is listening on it.`)) : null;
  return h("article", { class: "card", style: `--i:${i}`, "aria-label": x.name }, head, inner && h("div", { class: "module-body open" }, inner));
}
function paintExternals() {
  const key = document.activeElement?.dataset?.key;
  $("#externals").replaceChildren(...(view.loaded && view.externals.length ? [h("div", { class: "group" }, "Also on this PC"), ...view.externals.map(externalCard)] : []));
  if (key) $(`[data-key="${CSS.escape(key)}"]`)?.focus();
}

// ---- sites page ----
async function loadSites() {
  try { view.sites = await invoke("sites_list"); } catch (e) { note("error", "Couldn't read your sites.", String(e)); }
  paintSites();
}
function siteCard(s, i) {
  const k = (a) => ({ key: `site:${s.name}:${a}` });
  const fail = (e) => note("error", s.host, String(e));
  const head = h("div", { class: "module-head" },
    h("div", { class: "tile", "aria-hidden": "true" }, icon(G.globe)),
    h("div", { class: "titles" }, h("div", { class: "name" }, s.host, " ", h("span", { class: `tag ${s.exists ? "" : "warn"}` }, s.exists ? "Ready" : "Folder missing")), h("div", { class: "muted small path-line", title: s.folder }, s.folder)),
    h("div", { class: "actions" },
      h("button", { class: "btn accent", data: k("open"), on: { click: () => invoke("open_url", { url: s.url }).catch(fail) } }, icon(G.globe), "Open"),
      s.https_url && h("button", { class: "btn", data: k("https"), on: { click: () => invoke("open_url", { url: s.https_url }).catch(fail) } }, icon(0xe72e), "Open (HTTPS)"),
      h("button", { class: "btn", data: k("folder"), on: { click: () => invoke("site_open_folder", { name: s.name }).catch(fail) } }, icon(G.folder), "Folder"),
      h("button", { class: "btn ghost", "aria-label": `Remove ${s.host}`, title: "Remove from Runqora (files stay)", data: k("remove"), on: { click: () => removeSite(s) } }, icon(0xe74d))));
  return h("article", { class: `card${view.sitesEntered ? "" : " enter"}`, style: `--i:${i}`, "aria-label": s.host }, head);
}
function paintSites() {
  const key = document.activeElement?.dataset?.key;
  const apache = view.modules.find((m) => m.id === "apache");
  const hint = !apache || apache.status === "missing" ? infobar("info", "", "Install Apache on the Modules page to serve your sites.") : apache.status !== "running" ? infobar("info", "", "Start Apache to open your sites.") : null;
  $("#sites").replaceChildren(...[hint, ...(view.sites.length ? view.sites.map(siteCard) : [h("div", { class: "card" }, h("div", { class: "empty" }, "No sites yet. New site makes one in a moment; New WordPress site also sets up its database."))]), h("p", { class: "muted small fine" }, "Browsers open name.localhost on their own. A program that uses Windows' own name lookup (a PHP script calling file_get_contents on its own site, for example) may not find it; use localhost with the port there.")].filter(Boolean));
  if (!view.sitesEntered) setTimeout(() => (view.sitesEntered = true), 900);
  if (key) $(`[data-key="${CSS.escape(key)}"]`)?.focus();
}
function afterSiteChange(message) {
  const apache = view.modules.find((m) => m.id === "apache");
  note("success", "", message, apache?.status === "running" ? { label: "Restart Apache", run: () => restartModule("apache") } : undefined);
  loadSites();
  refreshProblems();
}
function field(label, input, help) {
  return h("label", { class: "field" }, h("span", { class: "strong" }, label), input, help && h("span", { class: "muted small" }, help));
}
async function newSite() {
  const name = h("input", { type: "text", placeholder: "shop", autocomplete: "off", spellcheck: false, "aria-label": "Site name" });
  const folder = h("input", { type: "text", class: "wide", placeholder: "Leave empty to make a folder for it", autocomplete: "off", spellcheck: false, "aria-label": "Folder" });
  if (!(await dialog({ title: "New site", content: [field("Name", name, "Letters, digits and dashes. It becomes name.localhost."), field("Folder", folder, "Use a folder you already have, or leave this empty.")], confirm: "Create site" }))) return;
  try { afterSiteChange(await invoke("site_add", { name: name.value, folder: folder.value })); } catch (e) { note("error", "Couldn't make the site.", String(e)); }
}
async function newWordPress() {
  const wp = view.modules.find((m) => m.id === "wordpress"), db = view.modules.find((m) => m.id === "mariadb");
  if (!wp || wp.status === "missing") return note("warning", "WordPress isn't installed.", "Install it from the WordPress card on the Modules page first.");
  if (!db || db.status !== "running") return note("warning", "MariaDB isn't running.", "WordPress keeps its pages in the database. Start MariaDB, then try again.");
  const name = h("input", { type: "text", placeholder: "blog", autocomplete: "off", spellcheck: false, "aria-label": "Site name" });
  if (!(await dialog({ title: "New WordPress site", content: [field("Name", name, "It opens at name.localhost. Runqora copies WordPress, makes its database and fills in wp-config.php; you finish in the browser.")], confirm: "Create site" }))) return;
  try { afterSiteChange(await invoke("wordpress_create", { name: name.value })); goto("sites"); } catch (e) { note("error", "Couldn't make the WordPress site.", String(e)); }
}
async function removeSite(s) {
  const ok = await dialog({ title: `Remove ${s.host}?`, content: [h("p", {}, "The site stops being served. Its files in the folder below are not deleted."), h("p", { class: "muted small", style: "user-select:text;word-break:break-all" }, s.folder)], confirm: "Remove", accent: false });
  if (!ok) return;
  try { afterSiteChange(await invoke("site_remove", { name: s.name })); } catch (e) { note("error", "Couldn't remove it.", String(e)); }
}
$("#site-new").addEventListener("click", newSite);
$("#site-wp").addEventListener("click", newWordPress);

async function refresh(force) {
  try {
    const [mods, ext] = await Promise.all([invoke("modules_status"), invoke("externals_status").catch(() => [])]);
    const json = JSON.stringify([mods, ext]);
    if (json === view.json && !force) return; // nothing changed: do not rebuild under the user's cursor
    view.json = json;
    view.modules = mods;
    view.externals = ext;
    view.loaded = true;
    paintModules();
    paintExternals();
    if (view.page === "sites") paintSites();
    if (view.page === "settings") paintSettings();
  } catch (e) {
    note("error", "", String(e));
  }
}

// ---- activity page ----
const LOGICON = { success: G.ok, error: G.err, info: G.info, warning: G.warn };
function paintLog() {
  $("#log").replaceChildren(...(view.log.length
    ? view.log.map((r) => h("div", { class: `log-row ${r.level}` }, h("time", {}, r.t), icon(LOGICON[r.level]), h("div", { class: "text" }, r.text)))
    : [h("div", { class: "empty" }, "No activity yet.")]));
  $("#log").scrollTop = $("#log").scrollHeight;
}

// ---- about page ----
const DEV = [["github", "GitHub", "https://github.com/ShoaibAhmedSoomro"], ["linkedin", "LinkedIn", "https://www.linkedin.com/in/shoaibaofficial/"], ["instagram", "Instagram", "https://www.instagram.com/shoaib_ahmedsoomro"], ["facebook", "Facebook", "https://www.facebook.com/shoaibahmedsoomroofficial/"]];
async function paintAbout() {
  const info = await invoke("app_info").catch(() => ({ version: "", root: "" }));
  $("#about").replaceChildren(
    h("div", { class: "card hero" }, h("img", { class: "app-logo", src: "logos/runqora.svg", alt: "" }), h("div", {}, h("div", { class: "strong big" }, "Runqora"), h("div", { class: "muted" }, `Version ${info.version}`))),
    h("div", { class: "card" },
      h("div", { class: "row plain" }, h("div", { class: "label" }, h("div", { class: "strong" }, "Install folder"), h("div", { class: "muted small", style: "user-select:text" }, info.root)), h("button", { class: "btn sm", on: { click: () => invoke("open_root") } }, icon(G.folder), "Open folder")),
      info.cli && h("div", { class: "row plain" }, h("div", { class: "label" }, h("div", { class: "strong" }, "Command line"), h("div", { class: "muted small", style: "user-select:text" }, `${info.cli} status | start | stop | ports`))),
    ),
    h("div", { class: "group" }, "Developer"),
    h("div", { class: "card" }, DEV.map(([id, name, url]) => h("div", { class: "row plain" },
      h("div", { class: "with-logo" }, logo(id, 20), h("div", { class: "label" }, h("div", { class: "strong" }, name), h("div", { class: "muted small path-line", title: url }, url.replace(/^https:\/\/(www\.)?/, "")))),
      h("button", { class: "btn sm", "aria-label": `Open ${name}`, on: { click: () => invoke("open_developer", { id }).catch((e) => note("error", name, String(e))) } }, icon(G.link), "Open")))),
    h("div", { class: "card" }, view.modules.map((m) => h("div", { class: "row plain" }, h("div", { class: "with-logo" }, logo(m.id, 20), h("div", { class: "strong" }, m.name)), h("div", { class: "value muted" }, `${m.version} · ${STATUS[m.status]}`)))),
    h("p", { class: "muted small fine" }, "Apache, Tomcat, PHP, MariaDB, phpMyAdmin, WordPress, FileZilla, Mailpit, SFTPGo, Java, GitHub, LinkedIn, Instagram and Facebook names and logos belong to their projects and are shown only to identify the programs. Runqora is not affiliated with them."),
  );
}

// ---- ports page (netstat with names) ----
let portRows = [];
async function loadPorts() {
  try {
    portRows = await invoke("ports_scan");
  } catch (e) {
    note("error", "Couldn't read the ports.", String(e));
  }
  paintPorts();
}
function paintPorts() {
  const q = $("#ports-filter").value.trim().toLowerCase();
  const rows = portRows.filter((r) => !q || `${r.port} ${r.pid} ${r.name} ${r.path} ${r.module ?? ""}`.toLowerCase().includes(q));
  const used = (r) => (r.module ? h("span", { class: `tag ${r.ours ? "" : "warn"}` }, logoByName(r.module) && logo(logoByName(r.module), 14), r.ours ? r.module : `Blocking ${r.module}`) : r.ours ? h("span", { class: "tag" }, "This install") : "");
  $("#ports").replaceChildren(
    rows.length
      ? h("div", { class: "plist", role: "list", "aria-label": "Listening ports" },
          h("div", { class: "plist-head", "aria-hidden": "true" }, h("span", {}, "Port"), h("span", {}, "Program"), h("span", {}, "Used by"), h("span", {}, "PID"), h("span")),
          ...rows.map((r) => h("div", { class: "prow", role: "listitem" },
            h("div", { class: "pnum" }, String(r.port)),
            h("div", { class: "pprog" }, h("div", { class: "pname" }, r.name), r.path && h("div", { class: "path", title: r.path }, r.path)),
            h("div", { class: "pused" }, used(r)),
            h("div", { class: "ppid" }, h("span", { class: "muted small" }, "PID "), String(r.pid)),
            h("div", { class: "pact" }, h("button", { class: "btn sm", "aria-label": `Free port ${r.port}`, on: { click: () => freePort(r) } }, "Free...")))))
      : h("div", { class: "empty" }, q ? "No listening ports match." : "Nothing is listening."),
  );
}
$("#ports-filter").addEventListener("input", paintPorts);
$("#ports-refresh").addEventListener("click", loadPorts);

// ---- settings page ----
function setRow(title, desc, control, logoId) {
  return h("div", { class: "row plain" }, h("div", { class: "with-logo" }, logoId && logo(logoId, 20), h("div", { class: "label" }, h("div", { class: "strong" }, title), h("div", { class: "muted small" }, desc))), control);
}
// ---- freeing a port: end the program, stop the service, or use another port ----
async function afterPortChange() {
  await refresh(true);
  refreshProblems();
}

async function endProgram(m, pid, rem) {
  const ok = await dialog({ title: `End ${rem.name}?`, content: [h("p", {}, rem.detail), h("p", { class: "muted small" }, "Windows may ask for administrator approval if the program belongs to another account.")], confirm: `End ${rem.name}` });
  if (!ok) return;
  try { note("success", "", await invoke("port_end", { pid })); } catch (e) { note("error", `Couldn't end ${rem.name}.`, String(e)); }
  afterPortChange();
}

async function stopService(m, pid, rem, svc) {
  const ok = await dialog({ title: `Stop the ${svc} service?`, content: [h("p", {}, rem.detail), h("p", { class: "muted small" }, "Windows will ask for administrator approval. Start the service again from Windows Services whenever you need it.")], confirm: `Stop ${svc}` });
  if (!ok) return;
  try { note("success", "", await invoke("port_stop_service", { pid, name: svc })); } catch (e) { note("error", `Couldn't stop ${svc}.`, String(e)); }
  afterPortChange();
}

async function usePort(id, name, port) {
  try {
    const a = await invoke("config_set", { module: id, key: "port", value: String(port) });
    note("success", "", a.message);
  } catch (e) {
    note("error", `Couldn't switch ${name} to port ${port}.`, String(e));
  }
  afterPortChange();
}

/** Buttons for a blocked module: end the program, stop its service, or move to a free port. Filled in once Windows has answered. */
function blockerActions(m, after = afterPortChange) {
  const box = h("div", { class: "infobar-actions-inner" });
  const pid = m.blocker.pid;
  Promise.all([invoke("port_remedy", { pid }), invoke("port_suggest", { id: m.id })]).then(([rem, alt]) => {
    if (rem.kind === "end") box.append(h("button", { class: "btn sm", on: { click: async () => { await endProgram(m, pid, rem); after(); } } }, `End ${rem.name}`));
    if (rem.kind === "service") for (const svc of rem.services.slice(0, 2)) box.append(h("button", { class: "btn sm", on: { click: async () => { await stopService(m, pid, rem, svc); after(); } } }, `Stop ${svc}`));
    if (alt) box.append(h("button", { class: "btn sm", on: { click: async () => { await usePort(m.id, m.name, alt); after(); } } }, `Use port ${alt} instead`));
    if (rem.kind === "none") box.append(h("span", { class: "muted small" }, rem.detail));
  }).catch(() => {});
  return [box];
}

/** From the Ports page: any listening program, with the same safeguards. */
async function freePort(r) {
  let rem;
  try { rem = await invoke("port_remedy", { pid: r.pid }); } catch (e) { return note("error", "Couldn't look at that program.", String(e)); }
  if (rem.kind === "end") { await endProgram(null, r.pid, rem); loadPorts(); }
  else if (rem.kind === "service") { await stopService(null, r.pid, rem, rem.services[0]); loadPorts(); }
  else note("warning", `Port ${r.port} stays as it is.`, rem.detail);
}

// ---- check setup ----
async function refreshProblems() {
  try {
    view.problems = (await invoke("setup_check")).length;
  } catch { view.problems = 0; }
  $("#setup-label").textContent = view.problems ? `Check setup (${view.problems})` : "Check setup";
  $("#check-setup").classList.toggle("attention", view.problems > 0);
}

async function openSetupCheck() {
  const list = h("div", { class: "dialog-body" });
  const fixAll = h("button", { class: "btn accent", hidden: true }, "Fix everything I can");
  const d = h("dialog", { class: "dialog wide", "aria-labelledby": "dlg-title" },
    h("h2", { id: "dlg-title" }, "Check setup"), list,
    h("div", { class: "dialog-actions" }, fixAll, h("button", { class: "btn", autofocus: true, on: { click: () => d.close() } }, "Close")));
  async function render() {
    list.replaceChildren(h("div", { class: "muted" }, "Looking..."));
    let problems = [];
    try { problems = await invoke("setup_check"); } catch (e) { list.replaceChildren(infobar("error", "Couldn't check.", String(e))); return; }
    view.problems = problems.length;
    $("#setup-label").textContent = problems.length ? `Check setup (${problems.length})` : "Check setup";
    $("#check-setup").classList.toggle("attention", problems.length > 0);
    fixAll.hidden = !problems.some((p) => p.fix);
    list.replaceChildren(...(problems.length ? problems.map((p) => {
      const acts = [];
      if (p.fix) acts.push(h("button", { class: "btn sm", on: { click: async () => { try { note("success", "", await invoke("setup_fix", { fix: p.fix, module: p.module })); } catch (e) { note("error", "Couldn't fix that.", String(e)); } await refresh(true); render(); } } }, "Fix"));
      if (p.port) acts.push(...blockerActions({ id: p.module, name: p.module, blocker: { pid: p.pid } }, render));
      return infobar(p.severity === "error" ? "error" : "warning", p.title, p.detail, undefined, acts);
    }) : [infobar("success", "", "Everything looks right: paths, ports and settings all match.")]));
  }
  fixAll.addEventListener("click", async () => {
    for (const r of await invoke("setup_fix_all")) r.Ok != null ? note("success", "", r.Ok) : note("error", "Couldn't fix that.", String(r.Err ?? r));
    await refresh(true);
    render();
  });
  d.addEventListener("close", () => d.remove());
  document.body.append(d);
  d.showModal();
  render();
}

// ---- settings: forms generated from the core's catalogue ----
const TABS = [["general", "General"], ["apache", "Apache"], ["php", "PHP"], ["mariadb", "MariaDB"], ["file", "Configuration file"]];

async function applyField(f, value, m) {
  try {
    const a = await invoke("config_set", { module: f.module, key: f.key, value });
    note("success", "", a.message, a.restart ? { label: `Restart ${a.restart === "apache" ? "Apache" : "MariaDB"}`, run: () => restartModule(a.restart) } : undefined);
  } catch (e) {
    note("error", `Couldn't change ${f.label || f.key}.`, String(e));
  }
  if (f.module === "general") loadAppearance();
  await refresh(true);
  refreshProblems();
}

async function restartModule(id) {
  view.busy.set(id, "Restarting");
  paintModules();
  try { note("success", "", await invoke("module_restart", { id })); } catch (e) { note("error", "Couldn't restart.", String(e)); }
  view.busy.delete(id);
  refresh(true);
}

function fieldControl(f, m) {
  const locked = f.key === "port" && m?.status === "running";
  const label = `${f.label || f.key}`;
  if (f.kind === "bool") {
    const on = f.value === "on";
    return h("button", { class: "switch", role: "switch", "aria-checked": String(on), "aria-label": label, on: { click: () => applyField(f, on ? "off" : "on", m) } }, h("span", { class: "track" }, h("span", { class: "thumb" })), on ? "On" : "Off");
  }
  if (f.kind === "choice") {
    return h("select", { "aria-label": label, on: { change: (e) => applyField(f, e.target.value, m) } }, ...f.choices.map(([v, t]) => h("option", { value: v, selected: v.toLowerCase() === f.value.toLowerCase() }, t)));
  }
  if (f.kind === "color") {
    const pick = h("input", { type: "color", value: f.value.toLowerCase(), "aria-label": label, class: "swatch", on: { change: (e) => applyField(f, e.target.value, m) } });
    return h("div", { class: "color-field" }, pick, h("code", { class: "hex" }, f.value));
  }
  const input = h("input", { type: f.kind === "number" ? "number" : "text", class: f.kind === "path" ? "wide" : f.kind === "text" ? "mid" : "", value: f.value, min: f.kind === "number" ? String(f.min) : false, max: f.kind === "number" ? String(f.max) : false, disabled: locked, placeholder: f.kind === "path" ? "Default" : false, "aria-label": label,
    on: { change: (e) => applyField(f, e.target.value, m), keydown: (e) => { if (e.key === "Enter") e.target.blur(); } } });
  return input;
}

function fieldRow(f, m) {
  const locked = f.key === "port" && m?.status === "running";
  const changed = f.value.toLowerCase() !== f.default.toLowerCase();
  return h("div", { class: "row plain" },
    h("div", { class: "label" }, h("div", { class: "strong" }, f.label || f.key), h("div", { class: "muted small" }, locked ? "Stop it to change the port." : f.help)),
    h("div", { class: "control" }, changed && !locked && f.kind !== "path" && h("button", { class: "btn ghost sm", title: `Back to ${f.default || "the default"}`, on: { click: () => applyField(f, f.default, m) } }, "Reset"), fieldControl(f, m)));
}

async function paintSettings() {
  const tab = view.settingsTab;
  const tabs = h("div", { class: "tabs", role: "tablist", "aria-label": "Settings" }, ...TABS.map(([id, text]) =>
    h("button", { class: "tab", role: "tab", id: `tab-${id}`, "aria-selected": String(id === tab), on: { click: () => { view.settingsTab = id; paintSettings(); } } }, ["apache", "php", "mariadb"].includes(id) && logo(id, 16), text)));
  const body = h("div", { class: "stack", role: "tabpanel", "aria-labelledby": `tab-${tab}` });
  $("#settings").replaceChildren(tabs, body);
  if (tab === "file") return paintConfigFile(body);
  const fields = await invoke("config_get", { module: tab }).catch(() => []);
  const m = view.modules.find((x) => x.id === tab);
  if (!fields.length) {
    body.append(h("div", { class: "card" }, h("div", { class: "empty" }, `${TABS.find((t) => t[0] === tab)[1]} is not installed yet. Install it on the Modules page, then its settings appear here.`)));
    return;
  }
  const custom = fields.find((f) => f.key === "accent_mode")?.value === "custom";
  fields.splice(0, fields.length, ...fields.filter((f) => f.key !== "accent_color" || custom));
  const groups = [...new Set(fields.map((f) => f.group))];
  for (const g of groups) {
    body.append(h("div", { class: "group" }, g), h("div", { class: "card" }, fields.filter((f) => f.group === g).map((f) => fieldRow(f, m))));
  }
  if (tab === "apache") body.append(...(await httpsCard(fields)));
  if (tab === "general") {
    body.append(h("div", { class: "group" }, "Setup"), h("div", { class: "card" }, setRow("Check setup", "Look for broken paths, wrong ports and settings that do not match, and fix them in one click.", h("button", { class: "btn", on: { click: openSetupCheck } }, "Check setup"))));
  }
}

// ---- HTTPS certificate card ----
async function httpsCard(fields) {
  const on = fields.find((f) => f.key === "https")?.value === "on";
  const port = fields.find((f) => f.key === "https_port")?.value || "443";
  let s;
  try { s = await invoke("https_status"); } catch { return []; }
  const date = (d) => (d ? new Date(d).toLocaleDateString(undefined, { year: "numeric", month: "long", day: "numeric" }) : "");
  const rows = [];
  if (!s.cert_exists) {
    rows.push(setRow("No certificate yet", on ? "It is made the next time HTTPS is applied." : "Switch on \u201cServe sites over HTTPS\u201d above and Runqora creates one for you.", h("span")));
  } else {
    rows.push(setRow("Certificate", `Valid until ${date(s.expires)}. Covers ${s.names.join(", ")}.`, h("button", { class: "btn sm", on: { click: async () => {
      const ok = await dialog({ title: "Make a new certificate?", content: [h("p", {}, "This replaces the private authority and the certificate. Sites keep working, but browsers will warn again until you trust the new one."), s.trusted ? infobar("warning", "", "The current one is trusted on this PC; you would trust the new one again.") : null], confirm: "Make a new one", accent: false });
      if (!ok) return;
      try { await invoke("https_regenerate"); note("success", "", "Made a new certificate. Restart Apache, then trust it again."); } catch (e) { note("error", "Couldn't make a certificate.", String(e)); }
      paintSettings();
    } } }, "Make a new one")));
    rows.push(setRow(s.trusted ? "Trusted on this PC" : "Not trusted yet", s.trusted ? "Browsers on this PC accept your sites with no warning." : "Until you trust it, browsers warn \u201cNot secure\u201d. Trusting adds Runqora's private authority to your own certificate store only.", s.trusted
      ? h("button", { class: "btn sm", on: { click: async () => { try { note("success", "", await invoke("https_untrust")); } catch (e) { note("error", "Couldn't remove trust.", String(e)); } paintSettings(); } } }, "Remove trust")
      : h("button", { class: "btn sm accent", on: { click: async () => {
          const ok = await dialog({ title: "Trust the Runqora certificate?", content: [h("p", {}, "Windows will show its own confirmation. If you accept, this PC trusts certificates made by Runqora's private authority \u201cRunqora Local CA\u201d, for your account only."), h("p", { class: "muted small" }, "The authority's key never leaves your install folder. You can remove trust here at any time.")], confirm: "Continue" });
          if (!ok) return;
          try { note("success", "", await invoke("https_trust")); } catch (e) { note("error", "Not trusted.", String(e)); }
          paintSettings();
        } } }, "Trust on this PC...")));
    if (on) rows.push(setRow(`https://localhost${port === "443" ? "" : ":" + port}/`, "Open your site over HTTPS (Apache must be running).", h("button", { class: "btn sm", on: { click: () => invoke("open_url", { url: `https://localhost${port === "443" ? "" : ":" + port}/` }).catch((e) => note("error", "", String(e))) } }, icon(G.globe), "Open")));
  }
  return [h("div", { class: "group" }, "Certificate"), h("div", { class: "card" }, rows)];
}

// ---- settings: the whole configuration as one runqora.ini ----
async function previewImport(text, source) {
  let items;
  try { items = await invoke("config_import", { text, apply: false }); } catch (e) { return note("error", "Couldn't read that file.", String(e)); }
  const todo = items.filter((i) => i.status === "ok"), bad = items.filter((i) => i.status === "error" || i.status === "unknown");
  if (!todo.length && !bad.length) return note("success", "", `${source} matches your current settings. Nothing to change.`);
  const row = (i) => h("div", { class: `import-row ${i.status}` }, icon(i.status === "ok" ? G.ok : G.err), h("div", {}, h("span", { class: "strong" }, `${i.module}.${i.key}`), ` = ${i.value}`, i.status !== "ok" && h("div", { class: "muted small" }, i.message)));
  const content = [
    h("p", {}, todo.length ? `${todo.length} setting${todo.length === 1 ? "" : "s"} will change. ${items.length - todo.length - bad.length} already match.` : "Nothing can be applied."),
    bad.length && infobar("warning", "", `${bad.length} line${bad.length === 1 ? "" : "s"} cannot be used and will be skipped.`),
    h("div", { class: "import-list" }, ...todo.map(row), ...bad.map(row)),
  ];
  if (!todo.length) return void (await dialog({ title: "Nothing to apply", content, confirm: "Close", accent: false }));
  if (!(await dialog({ title: `Apply ${source}?`, content, confirm: `Apply ${todo.length} change${todo.length === 1 ? "" : "s"}` }))) return;
  const done = await invoke("config_import", { text, apply: true });
  const ok = done.filter((i) => i.status === "ok").length;
  note("success", "", `Applied ${ok} setting${ok === 1 ? "" : "s"}.${done.some((i) => i.status === "ok" && /Restart|port/i.test(i.message)) ? " Restart the servers that are running to use them." : ""}`);
  await refresh(true);
  refreshProblems();
  paintSettings();
}

function paintConfigFile(body) {
  const file = h("input", { type: "file", accept: ".ini,.txt", hidden: true, on: { change: async (e) => { const f = e.target.files[0]; e.target.value = ""; if (f) previewImport(await f.text(), f.name); } } });
  body.append(
    h("div", { class: "card" },
      setRow("Export", "Saves every setting to runqora.ini in the install folder. Keep it, edit it, or copy it to another PC.", h("button", { class: "btn", on: { click: async () => {
        try { const p = await invoke("config_export"); note("success", "", `Saved ${p}`, { label: "Show folder", run: () => invoke("open_root") }); } catch (e) { note("error", "Couldn't export.", String(e)); }
      } } }, icon(G.download), "Export settings")),
      setRow("Import", "Read a runqora.ini and apply it. You see exactly what will change before anything is written.", h("div", { class: "control" },
        h("button", { class: "btn", on: { click: () => file.click() } }, icon(G.folder), "Choose a file..."),
        h("button", { class: "btn", on: { click: async () => { try { previewImport(await invoke("config_ini_text"), "runqora.ini"); } catch (e) { note("error", "No file yet.", String(e)); } } } }, "Use the one in the install folder")))),
    h("div", { class: "group" }, "What the file looks like"),
    h("div", { class: "card" }, h("pre", { class: "sample" }, "[php]\nmemory_limit = 512M\nupload_max_filesize = 64M\ndate.timezone = Asia/Dubai\nextensions = curl, gd, mbstring, mysqli, openssl, zip\n\n[mariadb]\nmax_connections = 300\n\n[apache]\nport = 8080\ndocument_root = C:\\Sites\n\n[general]\neditor = notepad.exe")),
    h("p", { class: "muted small fine" }, "Keys are not case sensitive and _ and - mean the same. Leave a line out to leave that setting alone. The command line does the same: rq config export, rq config import <file> --apply."),
    file,
  );
}


// ---- updates page ----
const KIND = { patch: "Update", minor: "Minor upgrade", major: "Major upgrade", toolchain: "New build" };
const MODE_TEXT = {
  off: "Runqora will not look for updates by itself.",
  notify: "Runqora tells you when something newer is published. You decide what to install.",
  install: "Patch updates install by themselves when the server is stopped. Minor and major upgrades always ask first.",
};
const newest = (u) => u.candidates.filter((c) => c.kind === "patch").at(-1); // candidates arrive oldest to newest
const hasNews = (u) => u.candidates.length > 0;

async function checkUpdates() {
  if (view.checking) return;
  view.checking = true;
  paintUpdates();
  try {
    view.updates = await invoke("updates_check");
    view.checkedAt = new Date();
  } catch (e) {
    note("error", "Couldn't check for updates.", String(e));
  }
  view.checking = false;
  paintUpdates();
  paintNav();
}

async function runUpdate(u, c) {
  view.busy.set(`upd:${u.id}`, "Downloading and installing...");
  paintUpdates();
  try {
    note("success", "", await invoke("update_apply", { id: u.id, version: c.version }));
  } catch (e) {
    note("error", `Couldn't update ${u.name}.`, String(e));
  }
  view.busy.delete(`upd:${u.id}`);
  await refresh(true);
  await checkUpdates();
}

async function installCandidate(u, c) {
  if (c.kind !== "patch" || c.warnings.length) {
    const ok = await dialog({
      title: `${c.kind === "patch" ? "Update" : "Upgrade"} ${u.name} to ${c.version}?`,
      content: [
        h("p", {}, `${u.name} will go from ${u.current} to ${c.version}. ${c.kind === "patch" ? "" : `This is a ${KIND[c.kind].toLowerCase()}, not just a fix. `}Your settings and data are carried over, and the current version is kept so you can roll back.`),
        ...c.warnings.map((w) => infobar("warning", "", w)),
        h("p", { class: "muted small" }, "It downloads from the publisher and checks the file against their checksum before anything changes."),
      ],
      confirm: `Install ${c.version}`,
    });
    if (!ok) return;
  }
  runUpdate(u, c);
}

async function rollbackModule(u) {
  const b = u.backup;
  const ok = await dialog({
    title: `Roll ${u.name} back to ${b.version}?`,
    content: [
      h("p", {}, `${u.name} goes from ${u.current} back to ${b.version}. Your settings are kept.`),
      b.data_copied ? infobar("warning", "", "Your databases return to how they were before that update. Anything saved since is lost.") : h("p", { class: "muted small" }, "Your data stays as it is now."),
    ],
    confirm: `Roll back to ${b.version}`,
    accent: false,
  });
  if (!ok) return;
  view.busy.set(`upd:${u.id}`, "Rolling back...");
  paintUpdates();
  try {
    note("success", "", await invoke("update_rollback", { id: u.id }));
  } catch (e) {
    note("error", `Couldn't roll ${u.name} back.`, String(e));
  }
  view.busy.delete(`upd:${u.id}`);
  await refresh(true);
  await checkUpdates();
}

const notes = (c) => invoke("open_notes", { url: c.notes_url }).catch((e) => note("error", "", String(e)));

function updateCard(u, i) {
  const best = newest(u);
  const others = u.candidates.filter((c) => c !== best);
  const busy = view.busy.get(`upd:${u.id}`);
  const k = (a) => ({ key: `upd:${u.id}:${a}` });
  const status = busy ? h("div", { class: "status" }, h("span", { class: "ring" }), busy)
    : u.error ? h("div", { class: "status blocked" }, h("span", { class: "dot" }), "Couldn't check")
    : best ? h("div", { class: "status avail" }, h("span", { class: "dot" }), `Update available: ${best.version}`)
    : h("div", { class: "status running" }, h("span", { class: "dot" }), others.length ? "Up to date · newer lines exist" : "Up to date");
  const actions = h("div", { class: "actions" },
    best && h("button", { class: "btn accent", disabled: !!busy || !!u.blocked, data: k("update"), on: { click: () => installCandidate(u, best) } }, icon(G.download), "Update"),
    u.backup && h("button", { class: "btn", disabled: !!busy || !!u.blocked, data: k("rollback"), on: { click: () => rollbackModule(u) } }, icon(G.back), "Roll back"),
  );
  const inner = h("div", { class: "inner" });
  if (u.blocked) inner.append(infobar("warning", "Waiting.", `${u.blocked} Updates never touch a running server.`));
  if (u.error) inner.append(infobar("error", "Couldn't check.", u.error));
  if (u.backup) inner.append(h("div", { class: "row" }, h("div", { class: "label" }, h("div", { class: "strong" }, `Previous version ${u.backup.version} is kept`), h("div", { class: "muted small" }, u.backup.data_copied ? "Including a copy of your databases from before the upgrade." : "Your data is shared with the current version.")), h("span")));
  if (best) inner.append(h("div", { class: "row" }, h("div", { class: "label" }, h("div", { class: "strong" }, `${best.version} `, h("span", { class: "tag" }, KIND.patch)), h("div", { class: "muted small" }, "Same release line: fixes only.")), h("button", { class: "btn sm", on: { click: () => notes(best) } }, icon(G.link), "What's new")));
  for (const c of others) {
    inner.append(h("div", { class: "row" },
      h("div", { class: "label" }, h("div", { class: "strong" }, `${c.version} `, h("span", { class: `tag ${c.kind === "patch" ? "" : "warn"}` }, KIND[c.kind])), c.warnings.map((w) => h("div", { class: "muted small" }, w))),
      h("div", { class: "files" },
        h("button", { class: "btn sm", on: { click: () => notes(c) } }, icon(G.link), "What's new"),
        h("button", { class: "btn sm", disabled: !!busy || !!u.blocked, on: { click: () => installCandidate(u, c) } }, "Review and install..."))));
  }
  const hasBody = inner.children.length > 0;
  const isOpen = view.open.has(`upd:${u.id}`) || !!u.blocked || !!u.error;
  const head = h("div", { class: "module-head" },
    h("div", { class: "tile", "aria-hidden": "true" }, logo(u.id, 24)),
    h("div", { class: "titles" }, h("div", { class: "name" }, u.name), h("div", { class: "muted small" }, `Installed ${u.current}`)),
    status, actions,
    hasBody && h("button", { class: "btn ghost chev", "aria-expanded": String(isOpen), "aria-label": `Versions for ${u.name}`, data: k("expand"), on: { click: (e) => toggleBody(e.currentTarget, `upd:${u.id}`) } }, icon(G.down)));
  return h("article", { class: `card${view.updEntered ? "" : " enter"}`, style: `--i:${i}`, "aria-label": u.name }, head, h("div", { class: `module-body${isOpen ? " open" : ""}`, inert: !isOpen }, inner));
}

function paintUpdates() {
  const key = document.activeElement?.dataset?.key;
  $("#upd-meta").textContent = view.checking ? "Checking the publishers..." : view.checkedAt ? `Last checked ${view.checkedAt.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}.` : "Not checked yet.";
  const check = $("#upd-check");
  check.disabled = view.checking;
  check.replaceChildren(view.checking ? h("span", { class: "ring" }) : icon(G.sync), view.checking ? "Checking..." : "Check for updates");
  const ready = (view.updates || []).filter((u) => newest(u) && !u.blocked && !view.busy.has(`upd:${u.id}`));
  $("#upd-all").disabled = !ready.length || view.checking;
  $("#upd-all").onclick = async () => { for (const u of ready) await runUpdate(u, newest(u)); };

  const mode = h("select", { "aria-label": "Update automatically", on: { change: async (e) => {
    view.autoMode = e.target.value;
    try { note("success", "", await invoke("settings_set_auto_update", { mode: e.target.value })); } catch (err) { note("error", "Couldn't save that.", String(err)); }
    paintUpdates();
  } } },
    ...[["off", "Off"], ["notify", "Tell me about updates"], ["install", "Install patch updates automatically"]].map(([v, t]) => h("option", { value: v, selected: v === view.autoMode }, t)));
  const list = !view.updates
    ? [0, 1, 2].map(skeleton)
    : view.updates.length ? view.updates.map(updateCard) : [h("div", { class: "card" }, h("div", { class: "empty" }, "Nothing is installed yet. Install a module on the Modules page first."))];
  $("#updates").replaceChildren(
    h("div", { class: "card" }, setRow("Update automatically", MODE_TEXT[view.autoMode], h("div", { class: "control" }, mode))),
    ...list,
  );
  if (view.updates && !view.updEntered) setTimeout(() => (view.updEntered = true), 900);
  if (key) $(`[data-key="${CSS.escape(key)}"]`)?.focus();
}

// ---- navigation ----
function paintNav() {
  document.querySelectorAll(".nav-item").forEach((b) => (b.dataset.page === view.page ? b.setAttribute("aria-current", "page") : b.removeAttribute("aria-current")));
  for (const p of ["modules", "sites", "updates", "ports", "activity", "settings", "about"]) $(`#page-${p}`).hidden = p !== view.page;
  const c = $("#errcount");
  c.hidden = !view.errors;
  c.textContent = view.errors;
  c.setAttribute("aria-label", `${view.errors} new errors`);
  const n = (view.updates || []).filter(hasNews).length;
  const u = $("#updcount");
  u.hidden = !n;
  u.textContent = n;
  u.setAttribute("aria-label", `${n} modules have updates`);
}
function goto(page) {
  view.page = page;
  if (view.page === "activity") { view.errors = 0; paintLog(); }
  if (view.page === "about") paintAbout();
  if (view.page === "settings") paintSettings();
  if (view.page === "ports") loadPorts();
  if (view.page === "sites") loadSites();
  if (view.page === "updates") { paintUpdates(); if (!view.updates) checkUpdates(); }
  paintNav();
}
document.querySelectorAll(".nav-item").forEach((b) => b.addEventListener("click", () => goto(b.dataset.page)));
$("#upd-check").addEventListener("click", checkUpdates);
$("#check-setup").addEventListener("click", openSetupCheck);

// ---- command bar ----
for (const [id, on] of [["#start-all", true], ["#stop-all", false]]) {
  $(id).addEventListener("click", async () => {
    note("info", "", on ? "Starting all servers..." : "Stopping all servers...");
    await invoke("modules_all", { start: on }).catch((e) => note("error", "", String(e)));
    for (const ms of [800, 2500, 5000]) setTimeout(() => refresh(), ms); // the core works in the background
  });
}
$("#log-clear").addEventListener("click", () => { view.log = []; paintLog(); });
$("#log-copy").addEventListener("click", () => navigator.clipboard.writeText(view.log.map((r) => `${r.t} [${r.level}] ${r.text}`).join("\n")).then(() => note("success", "", "Copied the activity log.")));

// Mica: the core switches the window material on; only then do the surfaces turn translucent
invoke("fx_enabled").then((on) => document.documentElement.classList.toggle("fx", !!on)).catch(() => {});
// Fluent "reveal": a soft light that follows the pointer across a card
document.addEventListener("pointermove", (e) => {
  const c = e.target.closest?.(".card");
  if (!c) return;
  const r = c.getBoundingClientRect();
  c.style.setProperty("--mx", `${e.clientX - r.left}px`);
  c.style.setProperty("--my", `${e.clientY - r.top}px`);
});

paintNav();
paintLog();
paintModules();
note("info", "", "Ready.");
refresh(true);
refreshProblems();
loadAppearance();
invoke("settings_get").then((s) => {
  view.autoMode = ["off", "notify", "install"].includes(s.auto_update) ? s.auto_update : "notify";
  if (view.autoMode !== "off") setTimeout(checkUpdates, 3000); // quietly fills the badge on the Updates item
}).catch(() => {});
// ponytail: poll; the core could push events if 2 s latency matters
async function pullNotes() {
  try {
    for (const n of await invoke("take_notes")) note(n.level, "", n.text); // results of the tray and autostart
  } catch {}
}
setInterval(() => { pullNotes(); loadAppearance(); if (!view.busy.size) refresh(); }, 2000);
setInterval(() => { if (view.page === "ports" && !document.hidden) loadPorts(); }, 3000);
pullNotes();
