import "./style.css";
import "@fortawesome/fontawesome-free/css/all.min.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const app = document.querySelector("#app");

const state = {
  data: null,
  activeId: null,
  selectedVersion: null,
  page: "library",
  modal: null,
  contextMenu: null,
  accountMenu: false,
  busy: null,
  status: "Loading launcher...",
  progress: 0,
  toast: null,
};

document.addEventListener("click", (event) => {
  if (!event.target.closest(".custom-select")) closeCustomSelects();
  let changed = false;
  if (state.contextMenu && !event.target.closest(".context-menu")) {
    state.contextMenu = null;
    changed = true;
  }
  if (state.accountMenu && !event.target.closest(".sidebar-account")) {
    state.accountMenu = false;
    changed = true;
  }
  if (changed) render();
});

document.addEventListener("keydown", (event) => {
  const openSelect = document.querySelector(".custom-select.open");
  if (event.key === "Escape" && openSelect) {
    event.preventDefault();
    closeCustomSelect(openSelect, true);
  } else if (event.key === "Escape" && (state.contextMenu || state.accountMenu)) {
    state.contextMenu = null;
    state.accountMenu = false;
    render();
  } else if ((event.key === "ArrowDown" || event.key === "ArrowUp") && state.contextMenu) {
    event.preventDefault();
    const actions = [...document.querySelectorAll(".context-action:not(:disabled)")];
    const current = actions.indexOf(document.activeElement);
    const direction = event.key === "ArrowDown" ? 1 : -1;
    actions[(current + direction + actions.length) % actions.length]?.focus();
  }
});

window.addEventListener("resize", () => closeCustomSelects());

const icons = {
  library: "fa-solid fa-border-all",
  settings: "fa-solid fa-gear",
  plus: "fa-solid fa-plus",
  play: "fa-solid fa-play",
  folder: "fa-solid fa-folder",
  edit: "fa-solid fa-pen",
  chevron: "fa-solid fa-chevron-right",
  down: "fa-solid fa-chevron-down",
  close: "fa-solid fa-xmark",
  check: "fa-solid fa-check",
  trash: "fa-solid fa-trash-can",
  more: "fa-solid fa-ellipsis",
  spark: "fa-brands fa-microsoft",
  instance: "fa-solid fa-cube",
  user: "fa-solid fa-user",
};

function icon(name) {
  return `<span class="icon"><i class="${icons[name] || icons.user}" aria-hidden="true"></i></span>`;
}

function escapeHtml(value = "") {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function applyAccent(value) {
  const match = /^#([0-9a-f]{6})$/i.exec(String(value || ""));
  if (!match) return;
  const number = Number.parseInt(match[1], 16);
  const red = (number >> 16) & 255;
  const green = (number >> 8) & 255;
  const blue = number & 255;
  const brightness = red * 299 + green * 587 + blue * 114;
  document.documentElement.style.setProperty("--accent", `#${match[1]}`);
  document.documentElement.style.setProperty("--accent-rgb", `${red}, ${green}, ${blue}`);
  document.documentElement.style.setProperty("--accent-contrast", brightness > 155000 ? "#050505" : "#ffffff");
}

function activeInstance() {
  return state.data?.instances.find((instance) => instance.id === state.activeId) || state.data?.instances[0];
}

function isRunning(instanceId) {
  return state.data?.runningInstances?.includes(instanceId) || false;
}

function skinHead(account) {
  const url = String(account?.skinUrl || "");
  const match = /^https?:\/\/textures\.minecraft\.net\/texture\/([a-z0-9]+)$/i.exec(url);
  if (!match) {
    return `<span class="skin-head fallback">${icon("user")}</span>`;
  }
  const safeUrl = `https://textures.minecraft.net/texture/${match[1]}`;
  return `<span class="skin-head" aria-hidden="true" style="--skin:url('${safeUrl}')"><span class="skin-face"></span><span class="skin-hat"></span></span>`;
}

function renderAccountMenu() {
  if (!state.accountMenu) return "";
  const activeUuid = state.data.account?.uuid;
  return `
    <div class="account-popover" role="menu" aria-label="Minecraft accounts">
      <div class="account-popover-title">Accounts</div>
      <div class="account-list">
        ${state.data.accounts.map((account) => `
          <div class="account-option ${account.uuid === activeUuid ? "active" : ""}">
            <button data-select-account="${escapeHtml(account.uuid)}" role="menuitem">
              ${skinHead(account)}
              <strong>${escapeHtml(account.name)}</strong>
              ${account.uuid === activeUuid ? icon("check") : ""}
            </button>
            <button class="account-remove" data-remove-account="${escapeHtml(account.uuid)}" aria-label="Remove ${escapeHtml(account.name)}" title="Remove account">${icon("trash")}</button>
          </div>`).join("")}
      </div>
      <button id="add-account" class="account-add">${icon("plus")}<span>Add account</span></button>
    </div>`;
}

function versionList(selected = "") {
  const showSnapshots = state.data.settings.showSnapshots;
  return state.data.versions.filter((version) => version.kind === "release" || showSnapshots || version.id === selected);
}

function versionOptions(selected) {
  return versionList(selected).map((version) => ({
    value: version.id,
    label: `${version.id}${version.kind === "snapshot" ? " · Snapshot" : ""}`,
  }));
}

function customSelect(id, selected, options, ariaLabel) {
  const current = options.find((option) => option.value === selected) || options[0];
  const searchable = options.length > 12;
  return `
    <div class="custom-select" data-custom-select>
      <input id="${escapeHtml(id)}" type="hidden" value="${escapeHtml(current?.value || "")}" />
      <button type="button" class="select-trigger" role="combobox" aria-label="${escapeHtml(ariaLabel)}" aria-expanded="false" aria-controls="${escapeHtml(id)}-menu" aria-haspopup="listbox">
        <span class="select-value">${escapeHtml(current?.label || "Select")}</span>${icon("down")}
      </button>
      <div id="${escapeHtml(id)}-menu" class="select-menu" role="listbox" aria-label="${escapeHtml(ariaLabel)}" hidden>
        ${searchable ? `<div class="select-search-wrap"><i class="fa-solid fa-magnifying-glass" aria-hidden="true"></i><input class="select-search" type="text" placeholder="Search versions" autocomplete="off" spellcheck="false" aria-label="Search versions" /></div>` : ""}
        <div class="select-options">
          ${options.map((option) => `
            <button type="button" class="select-option ${option.value === current?.value ? "selected" : ""}" role="option" aria-selected="${option.value === current?.value}" data-select-value="${escapeHtml(option.value)}" data-select-label="${escapeHtml(option.label)}" data-search="${escapeHtml(option.label.toLowerCase())}">
              <span>${escapeHtml(option.label)}</span>${option.value === current?.value ? icon("check") : ""}
            </button>`).join("")}
          <div class="select-empty" hidden>No matching versions</div>
        </div>
      </div>
    </div>`;
}

function closeCustomSelect(root, restoreFocus = false) {
  if (!root?.classList.contains("open")) return;
  root.classList.remove("open");
  const trigger = root.querySelector(".select-trigger");
  const menu = root.querySelector(".select-menu");
  trigger?.setAttribute("aria-expanded", "false");
  if (menu) {
    menu.hidden = true;
    menu.style.removeProperty("left");
    menu.style.removeProperty("top");
    menu.style.removeProperty("width");
    menu.style.removeProperty("max-height");
  }
  const search = root.querySelector(".select-search");
  if (search) search.value = "";
  root.querySelectorAll(".select-option").forEach((option) => { option.hidden = false; });
  root.querySelector(".select-empty")?.setAttribute("hidden", "");
  if (restoreFocus) trigger?.focus();
}

function closeCustomSelects(except = null) {
  document.querySelectorAll(".custom-select.open").forEach((root) => {
    if (root !== except) closeCustomSelect(root);
  });
}

function openCustomSelect(root) {
  const trigger = root.querySelector(".select-trigger");
  const menu = root.querySelector(".select-menu");
  if (!trigger || !menu) return;
  closeCustomSelects(root);
  root.classList.add("open");
  trigger.setAttribute("aria-expanded", "true");
  menu.hidden = false;

  const bounds = trigger.getBoundingClientRect();
  const below = window.innerHeight - bounds.bottom - 8;
  const above = bounds.top - 8;
  const openAbove = below < 210 && above > below;
  const available = Math.max(150, Math.min(360, openAbove ? above - 6 : below - 6));
  menu.style.left = `${Math.max(8, Math.min(bounds.left, window.innerWidth - bounds.width - 8))}px`;
  menu.style.width = `${bounds.width}px`;
  menu.style.maxHeight = `${available}px`;
  menu.style.top = `${openAbove ? Math.max(8, bounds.top - available - 6) : bounds.bottom + 6}px`;

  requestAnimationFrame(() => {
    const search = root.querySelector(".select-search");
    const selected = root.querySelector(".select-option.selected");
    selected?.scrollIntoView({ block: "nearest" });
    (search || selected || root.querySelector(".select-option"))?.focus({ preventScroll: true });
  });
}

function visibleSelectOptions(root) {
  return [...root.querySelectorAll(".select-option")].filter((option) => !option.hidden);
}

function moveSelectFocus(root, direction) {
  const options = visibleSelectOptions(root);
  if (!options.length) return;
  const current = options.indexOf(document.activeElement);
  const next = current < 0 ? (direction > 0 ? 0 : options.length - 1) : (current + direction + options.length) % options.length;
  options[next].focus({ preventScroll: true });
  options[next].scrollIntoView({ block: "nearest" });
}

function chooseSelectOption(option) {
  const root = option.closest(".custom-select");
  const input = root?.querySelector('input[type="hidden"]');
  if (!root || !input) return;
  const value = option.dataset.selectValue;
  const label = option.dataset.selectLabel;
  input.value = value;
  root.querySelector(".select-value").textContent = label;
  root.querySelectorAll(".select-option").forEach((item) => {
    const selected = item === option;
    item.classList.toggle("selected", selected);
    item.setAttribute("aria-selected", String(selected));
    item.querySelector(".icon")?.remove();
    if (selected) item.insertAdjacentHTML("beforeend", icon("check"));
  });
  closeCustomSelect(root, true);
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

function filterSelectOptions(event) {
  const root = event.currentTarget.closest(".custom-select");
  const query = event.currentTarget.value.trim().toLowerCase();
  let matches = 0;
  root.querySelectorAll(".select-option").forEach((option) => {
    const visible = !query || option.dataset.search.includes(query);
    option.hidden = !visible;
    if (visible) matches += 1;
  });
  root.querySelector(".select-empty").hidden = matches !== 0;
}

function bindCustomSelects() {
  document.querySelectorAll(".custom-select").forEach((root) => {
    const trigger = root.querySelector(".select-trigger");
    trigger.addEventListener("click", () => {
      if (root.classList.contains("open")) closeCustomSelect(root, true);
      else openCustomSelect(root);
    });
    trigger.addEventListener("keydown", (event) => {
      if (["Enter", " ", "ArrowDown", "ArrowUp"].includes(event.key)) {
        event.preventDefault();
        openCustomSelect(root);
        if (event.key === "ArrowUp") requestAnimationFrame(() => moveSelectFocus(root, -1));
      }
    });
    root.querySelectorAll(".select-option").forEach((option) => option.addEventListener("click", () => chooseSelectOption(option)));
    root.querySelector(".select-search")?.addEventListener("input", filterSelectOptions);
    root.querySelector(".select-menu").addEventListener("keydown", (event) => {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        moveSelectFocus(root, event.key === "ArrowDown" ? 1 : -1);
      } else if (event.key === "Home" || event.key === "End") {
        event.preventDefault();
        const options = visibleSelectOptions(root);
        const option = event.key === "Home" ? options[0] : options.at(-1);
        option?.focus({ preventScroll: true });
        option?.scrollIntoView({ block: "nearest" });
      }
    });
  });
}

function formatLastPlayed(value) {
  if (!value) return "Never played";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Ready";
  return `Last played ${new Intl.DateTimeFormat("en-US", { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" }).format(date)}`;
}

function shell(content) {
  const account = state.data.account;
  return `
    <div class="app-shell">
      <aside class="sidebar">
        <div class="brand"><span>Wisdom</span></div>
        <nav class="primary-nav" aria-label="Main navigation">
          <button class="nav-item ${state.page === "library" ? "active" : ""}" data-page="library">${icon("library")}<span>Library</span></button>
          <button class="nav-item ${state.page === "settings" ? "active" : ""}" data-page="settings">${icon("settings")}<span>Settings</span></button>
        </nav>
        <div class="sidebar-section">
          <div class="sidebar-label"><span>Instances</span><button id="sidebar-add" class="mini-button" aria-label="Create instance">${icon("plus")}</button></div>
          <div class="instance-nav">
            ${state.data.instances.map((instance) => `
              <button class="instance-nav-item ${instance.id === state.activeId && state.page === "library" ? "active" : ""}" data-instance="${escapeHtml(instance.id)}">
                <span class="instance-symbol">${icon("instance")}</span>
                <span class="instance-nav-copy"><strong>${escapeHtml(instance.name)}</strong><small>${isRunning(instance.id) ? "Running" : escapeHtml(instance.version)}</small></span>
                ${isRunning(instance.id) ? `<span class="nav-running" title="Running"></span>` : ""}
              </button>`).join("")}
          </div>
        </div>
        <div class="sidebar-account">
          ${account ? `
            <button id="account-trigger" class="account-trigger" aria-haspopup="menu" aria-expanded="${state.accountMenu}">
              ${skinHead(account)}<strong>${escapeHtml(account.name)}</strong>${icon("chevron")}
            </button>
            ${renderAccountMenu()}
          ` : `
            <button id="signin" class="signin-card">${icon("user")}<strong>Add account</strong>${icon("chevron")}</button>
          `}
        </div>
      </aside>
      <main class="workspace">
        ${content}
        <div class="statusbar ${state.busy ? "busy" : ""}">
          <span class="status-dot"></span><span id="activity-text">${escapeHtml(state.status)}</span>
          <div class="status-progress"><span id="progress-fill" style="width:${Math.round(state.progress * 100)}%"></span></div>
        </div>
      </main>
      ${renderContextMenu()}
      ${renderModal()}
      ${state.toast ? `<div class="toast ${state.toast.type}">${icon(state.toast.type === "success" ? "check" : "close")}<span>${escapeHtml(state.toast.message)}</span></div>` : ""}
    </div>`;
}

function renderLibrary() {
  const instance = activeInstance();
  const selectedVersion = state.selectedVersion || instance.version;
  const account = state.data.account;
  const running = isRunning(instance.id);
  const launching = state.busy === `launch:${instance.id}`;
  return `
    <header class="topbar">
      <h1>Library</h1>
      <button id="new-instance" class="button secondary">${icon("plus")}New instance</button>
    </header>
    <div class="content-scroll">
      <section class="launch-surface">
        <div class="instance-summary">
          <div class="instance-heading"><h2>${escapeHtml(instance.name)}</h2><p>${running ? "Running" : escapeHtml(formatLastPlayed(instance.lastPlayed))}</p></div>
          <div class="instance-actions">
            <button id="open-instance" class="icon-button" aria-label="Open instance folder" title="Open instance folder">${icon("folder")}</button>
            <button id="edit-instance" class="icon-button" aria-label="Edit instance" title="Edit instance">${icon("edit")}</button>
          </div>
        </div>
        <div class="launch-controls">
          <div class="version-field"><span>Version</span>${customSelect("launch-version", selectedVersion, versionOptions(selectedVersion), "Minecraft version")}</div>
          <button id="primary-action" class="button play-button ${running ? "running" : ""}" ${state.busy || running ? "disabled" : ""}>
            ${launching ? `<span class="spinner"></span><span><strong>Starting</strong><small id="play-version-label">${escapeHtml(selectedVersion)}</small></span>` : running ? `${icon("check")}<span><strong>Running</strong><small id="play-version-label">${escapeHtml(selectedVersion)}</small></span>` : `${icon(account ? "play" : "spark")}<span><strong>${account ? "Play" : "Add account"}</strong><small id="play-version-label">${escapeHtml(selectedVersion)}</small></span>`}
          </button>
        </div>
      </section>

      <section class="library-section">
        <div class="section-heading"><h3>Instances</h3><span class="count-badge">${state.data.instances.length}</span></div>
        <div class="instance-grid">
          ${state.data.instances.map((item) => {
            const itemRunning = isRunning(item.id);
            return `
            <button class="instance-card ${item.id === instance.id ? "selected" : ""}" data-instance="${escapeHtml(item.id)}">
              <span class="instance-symbol card-symbol">${icon("instance")}</span>
              <span class="card-copy"><strong>${escapeHtml(item.name)}</strong><small>Minecraft ${escapeHtml(item.version)}</small></span>
              ${itemRunning ? `<span class="running-badge"><span></span>Running</span>` : ""}
              ${icon("chevron")}
            </button>`;
          }).join("")}
        </div>
      </section>
    </div>`;
}

function renderContextMenu() {
  if (!state.contextMenu) return "";
  const instance = state.data.instances.find((item) => item.id === state.contextMenu.instanceId);
  if (!instance) return "";
  const canDelete = state.data.instances.length > 1;
  const running = isRunning(instance.id);
  return `
    <div class="context-menu" role="menu" aria-label="Actions for ${escapeHtml(instance.name)}" style="left:${state.contextMenu.x}px;top:${state.contextMenu.y}px">
      <div class="context-title"><span class="instance-symbol">${icon("instance")}</span><span><strong>${escapeHtml(instance.name)}</strong><small>Minecraft ${escapeHtml(instance.version)}</small></span></div>
      <div class="context-separator"></div>
      <button class="context-action" role="menuitem" data-context-action="play" ${running ? "disabled" : ""}>${icon(running ? "check" : "play")}<span>${running ? "Already running" : "Play"}</span></button>
      <button class="context-action" role="menuitem" data-context-action="edit">${icon("edit")}<span>Edit</span></button>
      <button class="context-action" role="menuitem" data-context-action="folder">${icon("folder")}<span>Open folder</span></button>
      <div class="context-separator"></div>
      <button class="context-action danger-action" role="menuitem" data-context-action="delete" ${canDelete && !running ? "" : "disabled"} title="${running ? "A running instance cannot be deleted" : canDelete ? "Delete instance permanently" : "The last instance cannot be deleted"}">${icon("trash")}<span>${running ? "Currently running" : canDelete ? "Delete instance" : "Keep last instance"}</span></button>
    </div>`;
}

function renderSettings() {
  const settings = state.data.settings;
  const ramGb = (settings.ramMb / 1024).toFixed(settings.ramMb % 1024 ? 1 : 0);
  return `
    <header class="topbar"><h1>Settings</h1><button id="save-settings" class="button primary" ${state.busy ? "disabled" : ""}>${icon("check")}Save</button></header>
    <div class="content-scroll settings-content">
      <section class="settings-group">
        <div class="settings-intro"><h2>Game</h2></div>
        <div class="settings-card">
          <label class="setting-row range-row"><span><strong>Memory</strong></span><span class="range-control"><output id="ram-output">${ramGb} GB</output><input id="ram" type="range" min="1024" max="16384" step="512" value="${settings.ramMb}" /></span></label>
          <label class="setting-row"><span><strong>Show snapshots</strong></span><input id="snapshots" class="switch" type="checkbox" ${settings.showSnapshots ? "checked" : ""} /></label>
          <label class="setting-row"><span><strong>Open Java console</strong></span><input id="console" class="switch" type="checkbox" ${settings.openConsole ? "checked" : ""} /></label>
        </div>
      </section>
      <section class="settings-group">
        <div class="settings-intro"><h2>Advanced</h2></div>
        <div class="settings-card form-card">
          <label class="field"><span>Additional JVM arguments</span><input id="global-jvm" value="${escapeHtml(settings.jvmArgs)}" placeholder='e.g. -XX:+UseG1GC' /></label>
          <label class="field"><span>Additional game arguments</span><input id="global-game" value="${escapeHtml(settings.gameArgs)}" placeholder="Optional" /></label>
        </div>
      </section>
      <section class="settings-group">
        <div class="settings-intro"><h2>Storage</h2></div>
        <div class="settings-card storage-row"><code>${escapeHtml(state.data.dataDirectory)}</code><button id="open-data" class="button secondary">${icon("folder")}Open folder</button></div>
      </section>
    </div>`;
}

function renderModal() {
  if (!state.modal) return "";
  const instance = activeInstance();
  if (state.modal === "delete") {
    return `<div class="modal-backdrop"><section class="modal compact" role="dialog" aria-modal="true"><div class="danger-mark">${icon("trash")}</div><h2>Delete instance?</h2><p>“${escapeHtml(instance.name)}” and all worlds stored inside it will be permanently removed.</p><div class="modal-actions"><button data-close-modal class="button secondary">Cancel</button><button id="confirm-delete" class="button danger">Delete permanently</button></div></section></div>`;
  }
  const editing = state.modal === "edit";
  const selected = editing ? instance.version : state.data.latestVersion;
  return `
    <div class="modal-backdrop">
      <section class="modal" role="dialog" aria-modal="true" aria-labelledby="modal-title">
        <div class="modal-header"><h2 id="modal-title">${editing ? escapeHtml(instance.name) : "New instance"}</h2><button data-close-modal class="icon-button">${icon("close")}</button></div>
        <form id="instance-form">
          <label class="field"><span>Name</span><input id="instance-name" maxlength="48" value="${editing ? escapeHtml(instance.name) : "New instance"}" required autofocus /></label>
          <div class="field"><span>Minecraft version</span>${customSelect("instance-version", selected, versionOptions(selected), "Minecraft version")}</div>
          ${editing ? `
            <label class="setting-row inline-setting"><span><strong>Custom memory</strong></span><input id="override-ram" class="switch" type="checkbox" ${instance.ramMb ? "checked" : ""} /></label>
            <label id="instance-ram-wrap" class="field ${instance.ramMb ? "" : "disabled"}"><span>Memory <output id="instance-ram-output">${((instance.ramMb || state.data.settings.ramMb) / 1024).toFixed(1)} GB</output></span><input id="instance-ram" type="range" min="1024" max="16384" step="512" value="${instance.ramMb || state.data.settings.ramMb}" ${instance.ramMb ? "" : "disabled"} /></label>
            <details class="advanced"><summary>Advanced launch options</summary><div class="advanced-fields"><label class="field"><span>JVM arguments</span><input id="instance-jvm" value="${escapeHtml(instance.jvmArgs || "")}" placeholder="Use global setting" /></label><label class="field"><span>Game arguments</span><input id="instance-game" value="${escapeHtml(instance.gameArgs || "")}" placeholder="Use global setting" /></label></div></details>
          ` : ""}
          <div class="modal-footer">${editing && state.data.instances.length > 1 && !isRunning(instance.id) ? `<button id="delete-instance" type="button" class="button text-danger">${icon("trash")}Delete instance</button>` : ""}<span></span><button type="button" data-close-modal class="button secondary">Cancel</button><button type="submit" class="button primary">${editing ? "Save changes" : "Create instance"}</button></div>
        </form>
      </section>
    </div>`;
}

function render() {
  if (!state.data) {
    app.innerHTML = `<div class="boot"><div><strong>Wisdom</strong><span>${escapeHtml(state.status)}</span></div><span class="boot-line"></span></div>`;
    return;
  }
  const content = state.page === "settings" ? renderSettings() : renderLibrary();
  app.innerHTML = shell(content);
  bindEvents();
}

function bindEvents() {
  bindCustomSelects();
  document.querySelectorAll("[data-page]").forEach((button) => button.addEventListener("click", () => {
    state.page = button.dataset.page;
    state.modal = null;
    state.accountMenu = false;
    render();
  }));
  document.querySelectorAll("[data-instance]").forEach((button) => button.addEventListener("click", () => {
    state.activeId = button.dataset.instance;
    state.selectedVersion = activeInstance().version;
    state.page = "library";
    state.modal = null;
    state.accountMenu = false;
    render();
  }));
  document.querySelectorAll("[data-instance]").forEach((element) => element.addEventListener("contextmenu", openContextMenu));
  document.querySelectorAll("[data-context-action]").forEach((button) => button.addEventListener("click", handleContextAction));
  document.querySelector("#new-instance")?.addEventListener("click", () => openModal("create"));
  document.querySelector("#sidebar-add")?.addEventListener("click", () => openModal("create"));
  document.querySelector("#edit-instance")?.addEventListener("click", () => openModal("edit"));
  document.querySelectorAll("[data-close-modal]").forEach((button) => button.addEventListener("click", () => openModal(null)));
  document.querySelector("#instance-form")?.addEventListener("submit", saveInstance);
  document.querySelector("#delete-instance")?.addEventListener("click", () => openModal("delete"));
  document.querySelector("#confirm-delete")?.addEventListener("click", deleteInstance);
  document.querySelector("#open-instance")?.addEventListener("click", () => callSimple("open_instance_folder", { instanceId: activeInstance().id }, "Instance folder opened."));
  document.querySelector("#open-data")?.addEventListener("click", () => callSimple("open_data_folder", {}, "Data folder opened."));
  document.querySelector("#signin")?.addEventListener("click", login);
  document.querySelector("#account-trigger")?.addEventListener("click", () => {
    state.accountMenu = !state.accountMenu;
    state.contextMenu = null;
    render();
  });
  document.querySelector("#add-account")?.addEventListener("click", login);
  document.querySelectorAll("[data-select-account]").forEach((button) => button.addEventListener("click", selectAccount));
  document.querySelectorAll("[data-remove-account]").forEach((button) => button.addEventListener("click", removeAccount));
  document.querySelector("#primary-action")?.addEventListener("click", state.data.account ? launch : login);
  document.querySelector("#launch-version")?.addEventListener("change", (event) => {
    state.selectedVersion = event.target.value;
    const label = document.querySelector("#play-version-label");
    if (label && state.data.account) label.textContent = state.selectedVersion;
  });
  document.querySelector("#save-settings")?.addEventListener("click", saveSettings);
  document.querySelector("#ram")?.addEventListener("input", updateRamOutput);
  document.querySelector("#override-ram")?.addEventListener("change", toggleInstanceRam);
  document.querySelector("#instance-ram")?.addEventListener("input", updateInstanceRamOutput);
}

function openModal(modal) {
  if (state.busy) return;
  state.contextMenu = null;
  state.accountMenu = false;
  state.modal = modal;
  render();
}

function openContextMenu(event) {
  event.preventDefault();
  if (state.busy || state.modal) return;
  const instance = state.data.instances.find((item) => item.id === event.currentTarget.dataset.instance);
  if (!instance) return;
  const bounds = event.currentTarget.getBoundingClientRect();
  const menuWidth = 224;
  const menuHeight = 258;
  const requestedX = event.clientX || bounds.right - 8;
  const requestedY = event.clientY || bounds.top + 12;
  state.activeId = instance.id;
  state.selectedVersion = instance.version;
  state.accountMenu = false;
  state.contextMenu = {
    instanceId: instance.id,
    x: Math.max(8, Math.min(requestedX, window.innerWidth - menuWidth - 8)),
    y: Math.max(8, Math.min(requestedY, window.innerHeight - menuHeight - 8)),
  };
  render();
  document.querySelector('[data-context-action="play"]')?.focus();
}

function handleContextAction(event) {
  const action = event.currentTarget.dataset.contextAction;
  const instanceId = state.contextMenu?.instanceId;
  const instance = state.data.instances.find((item) => item.id === instanceId);
  if (!instance) return;
  state.activeId = instance.id;
  state.selectedVersion = instance.version;
  state.contextMenu = null;
  if (action === "edit") {
    state.modal = "edit";
    render();
  } else if (action === "delete") {
    if (state.data.instances.length <= 1) return;
    state.modal = "delete";
    render();
  } else if (action === "folder") {
    render();
    callSimple("open_instance_folder", { instanceId: instance.id }, "Instance folder opened.");
  } else if (action === "play") {
    state.page = "library";
    render();
    state.data.account ? launch() : login();
  }
}

function updateRamOutput(event) {
  document.querySelector("#ram-output").textContent = `${(Number(event.target.value) / 1024).toFixed(Number(event.target.value) % 1024 ? 1 : 0)} GB`;
}

function toggleInstanceRam(event) {
  const input = document.querySelector("#instance-ram");
  input.disabled = !event.target.checked;
  document.querySelector("#instance-ram-wrap").classList.toggle("disabled", !event.target.checked);
}

function updateInstanceRamOutput(event) {
  document.querySelector("#instance-ram-output").textContent = `${(Number(event.target.value) / 1024).toFixed(1)} GB`;
}

async function login() {
  if (state.busy) return;
  state.accountMenu = false;
  state.busy = "login";
  state.status = "Opening Microsoft sign-in in your browser...";
  render();
  try {
    const account = await invoke("sign_in");
    state.data.account = account;
    const index = state.data.accounts.findIndex((item) => item.uuid === account.uuid);
    if (index >= 0) state.data.accounts[index] = account;
    else state.data.accounts.push(account);
    notify("Signed in successfully.", "success");
    state.status = "Signed in. Ready to play.";
  } catch (error) {
    fail("Sign-in failed", error);
  } finally {
    state.busy = null;
    render();
  }
}

async function selectAccount(event) {
  if (state.busy) return;
  const playerUuid = event.currentTarget.dataset.selectAccount;
  if (playerUuid === state.data.account?.uuid) {
    state.accountMenu = false;
    render();
    return;
  }
  state.busy = "account";
  try {
    state.data.account = await invoke("select_account", { playerUuid });
    state.accountMenu = false;
    state.status = `${state.data.account.name} is now active.`;
  } catch (error) {
    fail("Could not switch accounts", error);
  } finally {
    state.busy = null;
    render();
  }
}

async function removeAccount(event) {
  if (state.busy) return;
  event.stopPropagation();
  const playerUuid = event.currentTarget.dataset.removeAccount;
  const removed = state.data.accounts.find((account) => account.uuid === playerUuid);
  state.busy = "account";
  try {
    state.data.account = await invoke("remove_account", { playerUuid });
    state.data.accounts = state.data.accounts.filter((account) => account.uuid !== playerUuid);
    state.accountMenu = Boolean(state.data.account);
    state.status = removed ? `${removed.name} removed.` : "Account removed.";
    notify("Account removed.", "success");
  } catch (error) {
    fail("Could not remove account", error);
  } finally {
    state.busy = null;
    render();
  }
}

async function saveInstance(event) {
  event.preventDefault();
  if (state.busy) return;
  const editing = state.modal === "edit";
  const name = document.querySelector("#instance-name").value.trim();
  const version = document.querySelector("#instance-version").value;
  if (!name) return;
  state.busy = "save";
  try {
    let instance;
    if (editing) {
      const overrideRam = document.querySelector("#override-ram").checked;
      instance = await invoke("update_instance", {
        instanceId: activeInstance().id,
        name,
        version,
        ramMb: overrideRam ? Number(document.querySelector("#instance-ram").value) : null,
        jvmArgs: document.querySelector("#instance-jvm").value || null,
        gameArgs: document.querySelector("#instance-game").value || null,
      });
      const index = state.data.instances.findIndex((item) => item.id === instance.id);
      state.data.instances[index] = instance;
      notify("Instance saved.", "success");
    } else {
      instance = await invoke("create_instance", { name, version });
      state.data.instances.push(instance);
      state.activeId = instance.id;
      notify("Instance created.", "success");
    }
    state.selectedVersion = instance.version;
    state.modal = null;
    state.status = "Ready to play.";
  } catch (error) {
    fail(editing ? "Could not save instance" : "Could not create instance", error);
  } finally {
    state.busy = null;
    render();
  }
}

async function deleteInstance() {
  if (state.busy) return;
  const instance = activeInstance();
  if (isRunning(instance.id)) return;
  state.busy = "delete";
  try {
    await invoke("delete_instance", { instanceId: instance.id });
    state.data.instances = state.data.instances.filter((item) => item.id !== instance.id);
    state.activeId = state.data.instances[0].id;
    state.selectedVersion = state.data.instances[0].version;
    state.modal = null;
    notify("Instance deleted.", "success");
  } catch (error) {
    fail("Could not delete instance", error);
  } finally {
    state.busy = null;
    render();
  }
}

async function saveSettings() {
  if (state.busy) return;
  state.busy = "save";
  try {
    state.data.settings = await invoke("save_launcher_settings", { settings: {
      ramMb: Number(document.querySelector("#ram").value),
      showSnapshots: document.querySelector("#snapshots").checked,
      openConsole: document.querySelector("#console").checked,
      jvmArgs: document.querySelector("#global-jvm").value.trim(),
      gameArgs: document.querySelector("#global-game").value.trim(),
    } });
    state.status = "Settings saved.";
    notify("Settings saved.", "success");
  } catch (error) {
    fail("Could not save settings", error);
  } finally {
    state.busy = null;
    render();
  }
}

async function launch() {
  if (state.busy) return;
  const instance = activeInstance();
  if (isRunning(instance.id)) return;
  const version = state.selectedVersion || instance.version;
  state.busy = `launch:${instance.id}`;
  state.data.runningInstances.push(instance.id);
  state.progress = 0;
  state.status = `Preparing Minecraft ${version}...`;
  render();
  try {
    const updated = await invoke("launch", { instanceId: instance.id, version });
    const index = state.data.instances.findIndex((item) => item.id === updated.id);
    state.data.instances[index] = updated;
    state.selectedVersion = updated.version;
    state.progress = 1;
    state.status = `${updated.name} is running.`;
    notify("Minecraft is running.", "success");
  } catch (error) {
    state.data.runningInstances = state.data.runningInstances.filter((id) => id !== instance.id);
    state.progress = 0;
    fail("Minecraft could not be started", error);
  } finally {
    state.busy = null;
    render();
  }
}

async function callSimple(command, args, successMessage) {
  try {
    await invoke(command, args);
    state.status = successMessage;
    updateStatusDom();
  } catch (error) {
    fail("Action failed", error);
  }
}

function cleanError(error) {
  return String(error || "Unknown error").replace(/^Error:\s*/i, "");
}

function fail(title, error) {
  const message = `${title}: ${cleanError(error)}`;
  state.status = message;
  notify(message, "error");
  updateStatusDom();
}

let toastTimer;
function notify(message, type = "error") {
  state.toast = { message, type };
  document.querySelector(".toast")?.remove();
  document.querySelector(".app-shell")?.insertAdjacentHTML("beforeend", `<div class="toast ${type}">${icon(type === "success" ? "check" : "close")}<span>${escapeHtml(message)}</span></div>`);
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => {
    state.toast = null;
    document.querySelector(".toast")?.remove();
  }, 4500);
}

function updateStatusDom() {
  const text = document.querySelector("#activity-text");
  const progress = document.querySelector("#progress-fill");
  if (text) text.textContent = state.status;
  if (progress) progress.style.width = `${Math.round(state.progress * 100)}%`;
}

async function init() {
  render();
  try {
    await listen("status", (event) => {
      state.status = String(event.payload);
      updateStatusDom();
    });
    await listen("progress", (event) => {
      state.progress = Math.max(0, Math.min(1, Number(event.payload) || 0));
      updateStatusDom();
    });
    await listen("instance-status", (event) => {
      const { instanceId, running } = event.payload || {};
      if (!instanceId || !state.data) return;
      const ids = new Set(state.data.runningInstances);
      if (running) ids.add(instanceId);
      else ids.delete(instanceId);
      state.data.runningInstances = [...ids];
      if (!running) {
        const instance = state.data.instances.find((item) => item.id === instanceId);
        state.status = instance ? `${instance.name} exited.` : "Minecraft exited.";
        state.progress = 0;
      }
      if (state.page === "library" && !state.modal) render();
      else updateStatusDom();
    });
    state.data = await invoke("load_launcher");
    applyAccent(state.data.accentColor);
    state.activeId = state.data.instances[0].id;
    state.selectedVersion = state.data.instances[0].version;
    state.status = "Ready to play.";
    render();
  } catch (error) {
    app.innerHTML = `<div class="fatal"><h1>Wisdom could not start</h1><p>${escapeHtml(cleanError(error))}</p><button id="retry" class="button primary">Try again</button></div>`;
    document.querySelector("#retry").addEventListener("click", () => location.reload());
  }
}

window.addEventListener("focus", async () => {
  if (!state.data) return;
  try {
    const accent = await invoke("get_system_accent");
    state.data.accentColor = accent;
    applyAccent(accent);
  } catch (error) {
    console.warn("Windows accent color could not be refreshed", error);
  }
});

init();
