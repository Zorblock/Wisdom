import "./style.css";
import "@fortawesome/fontawesome-free/css/all.min.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const app = document.querySelector("#app");
const customSelectOptionSets = new Map();
const SELECT_ROW_HEIGHT = 34;
const SELECT_OVERSCAN = 5;

const state = {
  data: null,
  activeId: null,
  selectedVersion: null,
  page: "library",
  modal: null,
  contextMenu: null,
  contextTarget: null,
  contextSelection: "",
  accountMenu: false,
  busy: null,
  status: "Loading launcher...",
  progress: 0,
  toast: null,
};

document.addEventListener("contextmenu", openContextMenu);

document.addEventListener("click", (event) => {
  if (!event.target.closest(".custom-select, .version-field")) closeCustomSelects();
  if (state.contextMenu && !event.target.closest(".context-menu")) {
    dismissContextMenu();
  }
  if (state.accountMenu && !event.target.closest(".sidebar-account")) {
    state.accountMenu = false;
    document.querySelector(".account-popover")?.remove();
    document.querySelector("#account-trigger")?.setAttribute("aria-expanded", "false");
  }
});

document.addEventListener("keydown", (event) => {
  const openSelect = document.querySelector(".custom-select.open");
  if (event.key === "Escape" && openSelect) {
    event.preventDefault();
    closeCustomSelect(openSelect, true);
  } else if (event.key === "Escape" && (state.contextMenu || state.accountMenu)) {
    dismissContextMenu();
    state.accountMenu = false;
    document.querySelector(".account-popover")?.remove();
    document.querySelector("#account-trigger")?.setAttribute("aria-expanded", "false");
  } else if ((event.key === "ArrowDown" || event.key === "ArrowUp") && state.contextMenu) {
    event.preventDefault();
    const actions = [...document.querySelectorAll(".context-action:not(:disabled)")];
    const current = actions.indexOf(document.activeElement);
    const direction = event.key === "ArrowDown" ? 1 : -1;
    actions[(current + direction + actions.length) % actions.length]?.focus();
  }
});

window.addEventListener("resize", () => {
  closeCustomSelects();
  dismissContextMenu(false);
});

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
  copy: "fa-solid fa-copy",
  cut: "fa-solid fa-scissors",
  paste: "fa-solid fa-paste",
  select: "fa-solid fa-i-cursor",
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
  customSelectOptionSets.set(id, options);
  return `
    <div class="custom-select" data-custom-select>
      <input id="${escapeHtml(id)}" type="hidden" value="${escapeHtml(current?.value || "")}" />
      <button type="button" class="select-trigger" role="combobox" aria-label="${escapeHtml(ariaLabel)}" aria-expanded="false" aria-controls="${escapeHtml(id)}-menu" aria-haspopup="listbox">
        <span class="select-value">${escapeHtml(current?.label || "Select")}</span>${icon("down")}
      </button>
      <div id="${escapeHtml(id)}-menu" class="select-menu" role="listbox" aria-label="${escapeHtml(ariaLabel)}" hidden>
        ${searchable ? `<div class="select-search-wrap"><i class="fa-solid fa-magnifying-glass" aria-hidden="true"></i><input class="select-search" type="text" placeholder="Search versions" autocomplete="off" spellcheck="false" aria-label="Search versions" /></div>` : ""}
        <div class="select-options" tabindex="-1">
          <div class="select-spacer"><div class="select-window"></div></div>
          <div class="select-empty" hidden>No matching versions</div>
        </div>
      </div>
    </div>`;
}

function closeCustomSelect(root, restoreFocus = false) {
  if (!root || (!root.classList.contains("open") && !root.classList.contains("opening"))) return;
  root.classList.remove("open", "opening");
  const trigger = root.querySelector(".select-trigger");
  const menu = root.querySelector(".select-menu");
  trigger?.setAttribute("aria-expanded", "false");
  if (menu) {
    window.clearTimeout(root._selectCloseTimer);
    menu.classList.add("closing");
    root._selectCloseTimer = window.setTimeout(() => {
      if (root.classList.contains("open")) return;
      menu.hidden = true;
      menu.classList.remove("closing");
      menu.style.removeProperty("left");
      menu.style.removeProperty("top");
      menu.style.removeProperty("width");
      menu.style.removeProperty("max-height");
      delete menu.dataset.placement;
    }, 130);
  }
  const search = root.querySelector(".select-search");
  if (search) search.value = "";
  root._selectFiltered = root._selectOptions || [];
  root._selectFocusIndex = Math.max(0, root._selectFiltered.findIndex((option) => option.value === root._selectValue));
  renderSelectWindow(root);
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
  window.clearTimeout(root._selectCloseTimer);
  root.classList.add("opening");
  root.classList.remove("open");
  trigger.setAttribute("aria-expanded", "true");
  menu.classList.remove("closing");
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
  menu.dataset.placement = openAbove ? "top" : "bottom";
  void menu.offsetWidth;
  root.classList.remove("opening");
  root.classList.add("open");

  requestAnimationFrame(() => {
    const search = root.querySelector(".select-search");
    const options = root.querySelector(".select-options");
    const selectedIndex = Math.max(0, root._selectFiltered.findIndex((option) => option.value === root._selectValue));
    root._selectFocusIndex = selectedIndex;
    if (options && root._selectFiltered.length) {
      options.scrollTop = Math.max(0, selectedIndex * SELECT_ROW_HEIGHT - (options.clientHeight - SELECT_ROW_HEIGHT) / 2);
    }
    renderSelectWindow(root);
    if (search) search.focus({ preventScroll: true });
    else focusSelectIndex(root, selectedIndex);
  });
}

function renderSelectWindow(root) {
  const viewport = root.querySelector(".select-options");
  const spacer = root.querySelector(".select-spacer");
  const windowElement = root.querySelector(".select-window");
  const empty = root.querySelector(".select-empty");
  if (!viewport || !spacer || !windowElement || !empty) return;

  const options = root._selectFiltered || [];
  empty.hidden = options.length !== 0;
  spacer.hidden = options.length === 0;
  if (!options.length) {
    spacer.style.height = "0px";
    windowElement.replaceChildren();
    return;
  }

  const visibleRows = Math.ceil((viewport.clientHeight || 300) / SELECT_ROW_HEIGHT);
  const start = Math.max(0, Math.floor(viewport.scrollTop / SELECT_ROW_HEIGHT) - SELECT_OVERSCAN);
  const end = Math.min(options.length, start + visibleRows + SELECT_OVERSCAN * 2);
  spacer.style.height = `${options.length * SELECT_ROW_HEIGHT}px`;
  windowElement.style.transform = `translateY(${start * SELECT_ROW_HEIGHT}px)`;
  windowElement.innerHTML = options.slice(start, end).map((option, offset) => {
    const index = start + offset;
    const selected = option.value === root._selectValue;
    return `
      <button type="button" class="select-option ${selected ? "selected" : ""}" role="option" aria-selected="${selected}" data-select-index="${index}" data-select-value="${escapeHtml(option.value)}" data-select-label="${escapeHtml(option.label)}">
        <span>${escapeHtml(option.label)}</span>${selected ? icon("check") : ""}
      </button>`;
  }).join("");
  windowElement.querySelectorAll(".select-option").forEach((option) => {
    option.addEventListener("click", () => chooseSelectOption(option));
  });
}

function moveSelectFocus(root, direction) {
  const options = root._selectFiltered || [];
  if (!options.length) return;
  const activeIndex = Number(document.activeElement?.dataset?.selectIndex);
  const current = Number.isInteger(activeIndex) ? activeIndex : root._selectFocusIndex;
  const next = current == null ? (direction > 0 ? 0 : options.length - 1) : (current + direction + options.length) % options.length;
  focusSelectIndex(root, next);
}

function focusSelectIndex(root, index) {
  const options = root._selectFiltered || [];
  const viewport = root.querySelector(".select-options");
  if (!viewport || !options.length) return;
  const next = Math.max(0, Math.min(index, options.length - 1));
  const top = next * SELECT_ROW_HEIGHT;
  const bottom = top + SELECT_ROW_HEIGHT;
  if (top < viewport.scrollTop) viewport.scrollTop = top;
  else if (bottom > viewport.scrollTop + viewport.clientHeight) viewport.scrollTop = bottom - viewport.clientHeight;
  root._selectFocusIndex = next;
  renderSelectWindow(root);
  root.querySelector(`.select-option[data-select-index="${next}"]`)?.focus({ preventScroll: true });
}

function chooseSelectOption(option) {
  const root = option.closest(".custom-select");
  const input = root?.querySelector('input[type="hidden"]');
  if (!root || !input) return;
  const value = option.dataset.selectValue;
  const label = option.dataset.selectLabel;
  input.value = value;
  root._selectValue = value;
  root.querySelector(".select-value").textContent = label;
  closeCustomSelect(root, true);
  input.dispatchEvent(new Event("change", { bubbles: true }));
}

function filterSelectOptions(event) {
  const root = event.currentTarget.closest(".custom-select");
  const query = event.currentTarget.value.trim().toLowerCase();
  root._selectFiltered = query
    ? root._selectOptions.filter((option) => option.label.toLowerCase().includes(query))
    : root._selectOptions;
  root._selectFocusIndex = 0;
  root.querySelector(".select-options").scrollTop = 0;
  renderSelectWindow(root);
}

function bindCustomSelects() {
  document.querySelectorAll(".custom-select").forEach((root) => {
    const trigger = root.querySelector(".select-trigger");
    const input = root.querySelector('input[type="hidden"]');
    root._selectOptions = customSelectOptionSets.get(input.id) || [];
    root._selectFiltered = root._selectOptions;
    root._selectValue = input.value;
    root._selectFocusIndex = Math.max(0, root._selectOptions.findIndex((option) => option.value === input.value));
    renderSelectWindow(root);

    const toggle = () => {
      if (root.classList.contains("open")) closeCustomSelect(root, true);
      else openCustomSelect(root);
    };
    trigger.addEventListener("pointerdown", (event) => {
      if (event.button !== 0) return;
      event.preventDefault();
      toggle();
    });
    trigger.addEventListener("click", (event) => {
      if (event.detail === 0) toggle();
    });
    trigger.addEventListener("keydown", (event) => {
      if (["Enter", " ", "ArrowDown", "ArrowUp"].includes(event.key)) {
        event.preventDefault();
        openCustomSelect(root);
        if (event.key === "ArrowUp") requestAnimationFrame(() => moveSelectFocus(root, -1));
      }
    });
    root.querySelector(".select-search")?.addEventListener("input", filterSelectOptions);
    let scrollFrame = null;
    root.querySelector(".select-options").addEventListener("scroll", () => {
      if (scrollFrame != null) return;
      scrollFrame = requestAnimationFrame(() => {
        scrollFrame = null;
        renderSelectWindow(root);
      });
    }, { passive: true });
    root.querySelector(".select-menu").addEventListener("keydown", (event) => {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        moveSelectFocus(root, event.key === "ArrowDown" ? 1 : -1);
      } else if (event.key === "Home" || event.key === "End") {
        event.preventDefault();
        focusSelectIndex(root, event.key === "Home" ? 0 : root._selectFiltered.length - 1);
      }
    });
  });

  document.querySelectorAll(".version-field").forEach((field) => {
    field.addEventListener("pointerdown", (event) => {
      if (event.button !== 0 || event.target.closest(".select-trigger, .select-menu")) return;
      event.preventDefault();
      const root = field.querySelector(".custom-select");
      if (root?.classList.contains("open")) closeCustomSelect(root, true);
      else if (root) openCustomSelect(root);
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
  if (!instance) {
    return `
      <header class="topbar">
        <h1>Library</h1>
        <button id="new-instance" class="button secondary">${icon("plus")}New instance</button>
      </header>
      <div class="content-scroll empty-library-wrap">
        <section class="empty-library">
          <span class="empty-library-icon">${icon("instance")}</span>
          <h2>No instances</h2>
          <button id="empty-create" class="button primary">${icon("plus")}Create instance</button>
        </section>
      </div>`;
  }
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
  const position = `left:${state.contextMenu.x}px;top:${state.contextMenu.y}px`;
  if (state.contextMenu.type === "instance") {
    const instance = state.data.instances.find((item) => item.id === state.contextMenu.instanceId);
    if (!instance) return "";
    const running = isRunning(instance.id);
    return `
      <div class="context-menu" role="menu" aria-label="Actions for ${escapeHtml(instance.name)}" style="${position}">
        <div class="context-title"><span class="instance-symbol">${icon("instance")}</span><span><strong>${escapeHtml(instance.name)}</strong><small>Minecraft ${escapeHtml(instance.version)}</small></span></div>
        <div class="context-separator"></div>
        <button class="context-action" role="menuitem" data-context-action="play" ${running ? "disabled" : ""}>${icon(running ? "check" : "play")}<span>${running ? "Already running" : "Play"}</span></button>
        <button class="context-action" role="menuitem" data-context-action="edit">${icon("edit")}<span>Edit</span></button>
        <button class="context-action" role="menuitem" data-context-action="folder">${icon("folder")}<span>Open folder</span></button>
        <div class="context-separator"></div>
        <button class="context-action danger-action" role="menuitem" data-context-action="delete" ${running ? "disabled" : ""} title="${running ? "A running instance cannot be deleted" : "Delete instance permanently"}">${icon("trash")}<span>${running ? "Currently running" : "Delete instance"}</span></button>
      </div>`;
  }

  if (state.contextMenu.type === "text") {
    const noSelection = !state.contextSelection;
    return `
      <div class="context-menu compact-context-menu" role="menu" aria-label="Text actions" style="${position}">
        <button class="context-action" role="menuitem" data-context-action="cut" ${noSelection ? "disabled" : ""}>${icon("cut")}<span>Cut</span></button>
        <button class="context-action" role="menuitem" data-context-action="copy" ${noSelection ? "disabled" : ""}>${icon("copy")}<span>Copy</span></button>
        <button class="context-action" role="menuitem" data-context-action="paste">${icon("paste")}<span>Paste</span></button>
        <div class="context-separator"></div>
        <button class="context-action" role="menuitem" data-context-action="select-all">${icon("select")}<span>Select all</span></button>
      </div>`;
  }

  return `
    <div class="context-menu compact-context-menu" role="menu" aria-label="Launcher actions" style="${position}">
      ${state.contextMenu.type === "selection" ? `<button class="context-action" role="menuitem" data-context-action="copy">${icon("copy")}<span>Copy</span></button><div class="context-separator"></div>` : ""}
      <button class="context-action" role="menuitem" data-context-action="new-instance" ${state.busy ? "disabled" : ""}>${icon("plus")}<span>New instance</span></button>
      <button class="context-action" role="menuitem" data-context-action="settings">${icon("settings")}<span>Settings</span></button>
      <button class="context-action" role="menuitem" data-context-action="data-folder">${icon("folder")}<span>Open data folder</span></button>
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
  if ((state.modal === "edit" || state.modal === "delete") && !instance) return "";
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
          <div class="modal-footer">${editing && !isRunning(instance.id) ? `<button id="delete-instance" type="button" class="button text-danger">${icon("trash")}Delete instance</button>` : ""}<span></span><button type="button" data-close-modal class="button secondary">Cancel</button><button type="submit" class="button primary">${editing ? "Save changes" : "Create instance"}</button></div>
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
  document.querySelectorAll("[data-context-action]").forEach((button) => button.addEventListener("click", handleContextAction));
  document.querySelector("#new-instance")?.addEventListener("click", () => openModal("create"));
  document.querySelector("#empty-create")?.addEventListener("click", () => openModal("create"));
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
    dismissContextMenu(false);
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
  dismissContextMenu(false);
  state.accountMenu = false;
  state.modal = modal;
  render();
}

function openContextMenu(event) {
  event.preventDefault();
  if (!state.data || event.target.closest(".context-menu")) return;
  closeCustomSelects();
  state.accountMenu = false;
  document.querySelector(".account-popover")?.remove();
  document.querySelector("#account-trigger")?.setAttribute("aria-expanded", "false");

  const instanceElement = !state.modal ? event.target.closest("[data-instance]") : null;
  const editable = event.target.closest('textarea, [contenteditable="true"], input:not([type="range"]):not([type="checkbox"]):not([type="radio"]):not([type="button"]):not([type="submit"]):not([type="hidden"])');
  const instance = instanceElement
    ? state.data.instances.find((item) => item.id === instanceElement.dataset.instance)
    : null;
  let type = "global";
  state.contextTarget = null;
  state.contextSelection = "";

  if (instance) {
    type = "instance";
  } else if (editable) {
    type = "text";
    state.contextTarget = editable;
    if (typeof editable.selectionStart === "number") {
      state.contextSelection = editable.value.slice(editable.selectionStart, editable.selectionEnd);
    } else {
      state.contextSelection = window.getSelection()?.toString() || "";
    }
  } else {
    state.contextSelection = window.getSelection()?.toString().trim() || "";
    if (state.contextSelection) type = "selection";
  }

  state.contextMenu = {
    type,
    instanceId: instance?.id || null,
    x: event.clientX || 8,
    y: event.clientY || 8,
  };
  mountContextMenu();
}

function mountContextMenu() {
  document.querySelector(".context-menu")?.remove();
  const host = document.querySelector(".app-shell");
  if (!host || !state.contextMenu) return;
  host.insertAdjacentHTML("beforeend", renderContextMenu());
  const menu = host.querySelector(".context-menu");
  if (!menu) return;
  const bounds = menu.getBoundingClientRect();
  const left = Math.max(8, Math.min(state.contextMenu.x, window.innerWidth - bounds.width - 8));
  const top = Math.max(8, Math.min(state.contextMenu.y, window.innerHeight - bounds.height - 8));
  menu.style.left = `${left}px`;
  menu.style.top = `${top}px`;
  menu.querySelectorAll("[data-context-action]").forEach((button) => button.addEventListener("click", handleContextAction));
  void menu.offsetWidth;
  menu.classList.add("open");
}

function dismissContextMenu(animate = true) {
  const menu = document.querySelector(".context-menu");
  state.contextMenu = null;
  state.contextTarget = null;
  state.contextSelection = "";
  if (!menu) return;
  if (!animate) {
    menu.remove();
    return;
  }
  menu.classList.remove("open");
  window.setTimeout(() => menu.remove(), 120);
}

async function writeClipboard(text) {
  if (!text) return;
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const fallback = document.createElement("textarea");
    fallback.value = text;
    fallback.setAttribute("readonly", "");
    fallback.style.position = "fixed";
    fallback.style.opacity = "0";
    document.body.append(fallback);
    fallback.select();
    document.execCommand("copy");
    fallback.remove();
  }
}

async function handleTextContextAction(action, target, selection) {
  if (action === "copy") {
    await writeClipboard(selection);
    return;
  }
  if (action === "select-all") {
    target?.focus();
    if (typeof target?.select === "function") target.select();
    else document.execCommand("selectAll");
    return;
  }
  if (action === "cut") {
    await writeClipboard(selection);
    if (typeof target?.setRangeText === "function") {
      target.setRangeText("", target.selectionStart, target.selectionEnd, "end");
      target.dispatchEvent(new Event("input", { bubbles: true }));
    } else {
      target?.focus();
      document.execCommand("delete");
    }
    return;
  }
  if (action === "paste") {
    try {
      const text = await navigator.clipboard.readText();
      target?.focus();
      if (typeof target?.setRangeText === "function") {
        target.setRangeText(text, target.selectionStart, target.selectionEnd, "end");
        target.dispatchEvent(new Event("input", { bubbles: true }));
      } else {
        document.execCommand("insertText", false, text);
      }
    } catch {
      target?.focus();
      document.execCommand("paste");
    }
  }
}

async function handleContextAction(event) {
  const action = event.currentTarget.dataset.contextAction;
  const menuType = state.contextMenu?.type;
  const target = state.contextTarget;
  const selection = state.contextSelection;
  if (menuType === "text" || (menuType === "selection" && action === "copy")) {
    dismissContextMenu();
    await handleTextContextAction(action, target, selection);
    return;
  }
  if (menuType !== "instance") {
    dismissContextMenu();
    if (action === "new-instance") openModal("create");
    else if (action === "settings") {
      state.page = "settings";
      state.modal = null;
      render();
    } else if (action === "data-folder") {
      callSimple("open_data_folder", {}, "Data folder opened.");
    }
    return;
  }

  const instanceId = state.contextMenu?.instanceId;
  const instance = state.data.instances.find((item) => item.id === instanceId);
  if (!instance) return;
  state.activeId = instance.id;
  state.selectedVersion = instance.version;
  dismissContextMenu(false);
  if (action === "edit") {
    state.modal = "edit";
    render();
  } else if (action === "delete") {
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
  if (!instance) return;
  if (isRunning(instance.id)) return;
  state.busy = "delete";
  try {
    await invoke("delete_instance", { instanceId: instance.id });
    state.data.instances = state.data.instances.filter((item) => item.id !== instance.id);
    const next = state.data.instances[0] || null;
    state.activeId = next?.id || null;
    state.selectedVersion = next?.version || null;
    state.modal = null;
    state.status = next ? "Ready to play." : "No instances.";
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
  if (!instance) return;
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
    const initialInstance = state.data.instances[0] || null;
    state.activeId = initialInstance?.id || null;
    state.selectedVersion = initialInstance?.version || null;
    state.status = initialInstance ? "Ready to play." : "No instances.";
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
