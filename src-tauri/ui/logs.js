import { AnsiUp } from "ansi_up";
import "./logs.css";

const ansiUp = new AnsiUp();
ansiUp.escape_html = true;
ansiUp.url_allowlist = {};

export async function initLogsView({ app, invoke, applyAccent, icon, escapeHtml, cleanError, writeClipboard, params }) {
  document.body.classList.add("console-view", "logs-view");
  const instanceId = params.get("instanceId") || "";
  const instanceName = params.get("name") || "Minecraft";
  const version = params.get("version") || "";
  let files = [];
  let selected = "";
  let lines = [];

  app.innerHTML = `
    <div class="console-shell logs-shell">
      <header class="console-header">
        <span class="console-app-icon">${icon("logs")}</span>
        <span class="console-heading"><strong>${escapeHtml(instanceName)}</strong><small>Minecraft ${escapeHtml(version)} logs</small></span>
        <div class="logs-file-picker">
          <button id="logs-file-trigger" class="logs-file-trigger" aria-haspopup="menu" aria-expanded="false"><span id="logs-file-name">Choose log</span>${icon("down")}</button>
          <div id="logs-file-menu" class="logs-file-menu" role="menu"></div>
        </div>
        <div class="console-tools">
          <label class="console-search"><i class="fa-solid fa-magnifying-glass" aria-hidden="true"></i><input id="logs-search" placeholder="Filter" aria-label="Filter logs" /></label>
          <button id="logs-reload" class="icon-button" aria-label="Reload log" title="Reload">${icon("refresh")}</button>
          <button id="logs-copy" class="icon-button" aria-label="Copy log" title="Copy log">${icon("copy")}</button>
        </div>
      </header>
      <div id="console-output" class="console-output" role="log" aria-live="off"><div class="logs-loading"><span class="spinner"></span>Loading logs...</div></div>
      <footer class="console-footer"><span id="console-count">0 lines</span><span id="logs-source">Instance logs</span></footer>
    </div>`;

  const output = document.querySelector("#console-output");
  const count = document.querySelector("#console-count");
  const search = document.querySelector("#logs-search");
  const trigger = document.querySelector("#logs-file-trigger");
  const menu = document.querySelector("#logs-file-menu");

  const formatSize = (bytes) => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  };

  const lineKind = (line) => {
    const message = String(line.message || "").toLowerCase();
    if (/\b(fatal|error|exception|crash|failed|failure)\b/.test(message)) return "error";
    if (/\bwarn(?:ing)?\b/.test(message)) return "warning";
    return "normal";
  };

  const renderMessage = (host, value) => {
    const raw = String(value || "");
    if (/\x1b(?:\[[0-?]*[ -/]*[@-~]|\])/.test(raw)) {
      host.innerHTML = ansiUp.ansi_to_html(raw);
      return;
    }
    const clean = raw.replace(/[\u0000-\u0008\u000b\u000c\u000e-\u001f\u007f]/g, "");
    const pattern = /(\b(?:TRACE|DEBUG|INFO|WARN|ERROR|FATAL)\b|"(?:\\.|[^"\\])*"|\b[a-z0-9_.-]+:[a-z0-9_./-]+\b|\b\d+(?:\.\d+)?(?:ms|MB|GB|KiB|MiB|%)?\b)/gi;
    let offset = 0;
    for (const match of clean.matchAll(pattern)) {
      if (match.index > offset) host.append(document.createTextNode(clean.slice(offset, match.index)));
      const token = document.createElement("span");
      token.textContent = match[0];
      token.className = /^(TRACE|DEBUG|INFO|WARN|ERROR|FATAL)$/i.test(match[0])
        ? `console-log-level ${match[0].toLowerCase()}`
        : match[0].startsWith('"') ? "console-token string" : match[0].includes(":") ? "console-token resource" : "console-token number";
      host.append(token);
      offset = match.index + match[0].length;
    }
    if (offset < clean.length) host.append(document.createTextNode(clean.slice(offset)));
  };

  const applyFilter = () => {
    const query = search.value.trim().toLowerCase();
    output.querySelectorAll(".console-line").forEach((element) => {
      element.hidden = Boolean(query) && !element.dataset.search.includes(query);
    });
  };

  const renderLines = () => {
    const fragment = document.createDocumentFragment();
    for (const line of lines) {
      const element = document.createElement("div");
      element.className = `console-line ${lineKind(line)}`;
      element.dataset.search = String(line.message || "").toLowerCase();
      const time = document.createElement("time");
      time.textContent = line.timestamp || "--:--:--";
      const message = document.createElement("span");
      renderMessage(message, line.message);
      element.append(time, message);
      fragment.append(element);
    }
    output.replaceChildren(fragment);
    count.textContent = `${lines.length.toLocaleString("en-US")} ${lines.length === 1 ? "line" : "lines"}`;
    applyFilter();
    requestAnimationFrame(() => requestAnimationFrame(() => { output.scrollTop = output.scrollHeight; }));
  };

  const renderFiles = () => {
    menu.innerHTML = files.length ? files.map((file) => `
      <button class="logs-file-option ${file.name === selected ? "active" : ""}" role="menuitem" data-log-file="${escapeHtml(file.name)}">
        <span><strong>${escapeHtml(file.name)}</strong><small>${escapeHtml(file.modified)}</small></span><small>${formatSize(file.size)}</small>
      </button>`).join("") : `<div class="logs-no-files">No Minecraft logs yet.</div>`;
    menu.querySelectorAll("[data-log-file]").forEach((button) => button.addEventListener("click", () => {
      closeMenu();
      void loadFile(button.dataset.logFile);
    }));
  };

  const closeMenu = () => {
    menu.classList.remove("open");
    trigger.setAttribute("aria-expanded", "false");
  };

  const loadFile = async (name) => {
    if (!name) return;
    selected = name;
    document.querySelector("#logs-file-name").textContent = name;
    document.querySelector("#logs-source").textContent = name.endsWith(".gz") ? "Compressed Minecraft log" : "Minecraft log";
    output.innerHTML = `<div class="logs-loading"><span class="spinner"></span>Reading ${escapeHtml(name)}...</div>`;
    renderFiles();
    try {
      lines = await invoke("read_instance_log", { instanceId, fileName: name });
      renderLines();
    } catch (error) {
      output.innerHTML = `<div class="logs-loading error">${escapeHtml(cleanError(error))}</div>`;
    }
  };

  const refreshFiles = async (keepSelection = true) => {
    try {
      files = await invoke("list_instance_logs", { instanceId });
      renderFiles();
      const next = keepSelection && files.some((file) => file.name === selected) ? selected : files[0]?.name;
      if (next) await loadFile(next);
      else output.innerHTML = `<div class="logs-loading">No Minecraft logs yet. Start this instance once to create one.</div>`;
    } catch (error) {
      output.innerHTML = `<div class="logs-loading error">${escapeHtml(cleanError(error))}</div>`;
    }
  };

  trigger.addEventListener("click", (event) => {
    event.stopPropagation();
    const open = !menu.classList.contains("open");
    menu.classList.toggle("open", open);
    trigger.setAttribute("aria-expanded", String(open));
  });
  document.addEventListener("click", closeMenu);
  search.addEventListener("input", applyFilter);
  document.querySelector("#logs-reload").addEventListener("click", () => refreshFiles(true));
  document.querySelector("#logs-copy").addEventListener("click", () => writeClipboard(lines.map((line) => `[${line.timestamp}] ${line.message}`).join("\n")));
  document.addEventListener("contextmenu", (event) => {
    event.preventDefault();
    closeMenu();
  });

  try { applyAccent(await invoke("get_system_accent")); } catch {}
  await refreshFiles(false);
}
