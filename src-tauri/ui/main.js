import "./style.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

const app = document.querySelector("#app");

const state = {
  data: null,
  activeId: null,
  selectedVersion: null,
  page: "library",
  modal: null,
  busy: null,
  status: "Launcher wird geladen …",
  progress: 0,
  toast: null,
};

const icons = {
  library: `<svg viewBox="0 0 24 24"><path d="M4 5.5A2.5 2.5 0 0 1 6.5 3H20v15H6.5A2.5 2.5 0 0 0 4 20.5z"/><path d="M4 5.5v15A2.5 2.5 0 0 1 6.5 18H20"/></svg>`,
  settings: `<svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .34 1.88l.06.06-2.83 2.83-.06-.06a1.7 1.7 0 0 0-1.88-.34 1.7 1.7 0 0 0-1.03 1.56V21h-4v-.08A1.7 1.7 0 0 0 8.97 19.4a1.7 1.7 0 0 0-1.88.34l-.06.06-2.83-2.83.06-.06A1.7 1.7 0 0 0 4.6 15a1.7 1.7 0 0 0-1.53-1H3v-4h.08A1.7 1.7 0 0 0 4.6 8.97a1.7 1.7 0 0 0-.34-1.88l-.06-.06L7.03 4.2l.06.06A1.7 1.7 0 0 0 8.97 4.6 1.7 1.7 0 0 0 10 3.08V3h4v.08a1.7 1.7 0 0 0 1.03 1.53 1.7 1.7 0 0 0 1.88-.34l.06-.06 2.83 2.83-.06.06a1.7 1.7 0 0 0-.34 1.88A1.7 1.7 0 0 0 20.92 10H21v4h-.08A1.7 1.7 0 0 0 19.4 15z"/></svg>`,
  plus: `<svg viewBox="0 0 24 24"><path d="M12 5v14M5 12h14"/></svg>`,
  play: `<svg viewBox="0 0 24 24" class="fill"><path d="m8 5 11 7-11 7z"/></svg>`,
  folder: `<svg viewBox="0 0 24 24"><path d="M3 6.5h6l2 2h10v10.5a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>`,
  edit: `<svg viewBox="0 0 24 24"><path d="m4 20 4.2-1 10.9-10.9a2.1 2.1 0 0 0-3-3L5.2 16zM14.8 6.4l3 3"/></svg>`,
  chevron: `<svg viewBox="0 0 24 24"><path d="m9 18 6-6-6-6"/></svg>`,
  close: `<svg viewBox="0 0 24 24"><path d="m6 6 12 12M18 6 6 18"/></svg>`,
  check: `<svg viewBox="0 0 24 24"><path d="m5 12 4 4L19 6"/></svg>`,
  logout: `<svg viewBox="0 0 24 24"><path d="M10 17l5-5-5-5M15 12H3M14 4h5a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-5"/></svg>`,
  trash: `<svg viewBox="0 0 24 24"><path d="M4 7h16M9 7V4h6v3M7 7l1 14h8l1-14M10 11v6M14 11v6"/></svg>`,
  spark: `<svg viewBox="0 0 24 24"><path d="m12 3 1.5 5.5L19 10l-5.5 1.5L12 17l-1.5-5.5L5 10l5.5-1.5z"/></svg>`,
};

function icon(name) {
  return `<span class="icon">${icons[name]}</span>`;
}

function escapeHtml(value = "") {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}

function activeInstance() {
  return state.data?.instances.find((instance) => instance.id === state.activeId) || state.data?.instances[0];
}

function initials(name = "?") {
  return name.trim().slice(0, 2).toUpperCase() || "?";
}

function versionList(selected = "") {
  const showSnapshots = state.data.settings.showSnapshots;
  return state.data.versions.filter((version) => version.kind === "release" || showSnapshots || version.id === selected);
}

function versionOptions(selected) {
  return versionList(selected)
    .map((version) => `<option value="${escapeHtml(version.id)}" ${version.id === selected ? "selected" : ""}>${escapeHtml(version.id)}${version.kind === "snapshot" ? " · Snapshot" : ""}</option>`)
    .join("");
}

function formatLastPlayed(value) {
  if (!value) return "Noch nicht gespielt";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "Bereit";
  return `Zuletzt ${new Intl.DateTimeFormat("de-DE", { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" }).format(date)}`;
}

function shell(content) {
  const account = state.data.account;
  return `
    <div class="app-shell">
      <aside class="sidebar">
        <div class="brand"><span class="brand-mark">W</span><span>Wisdom</span></div>
        <nav class="primary-nav" aria-label="Hauptnavigation">
          <button class="nav-item ${state.page === "library" ? "active" : ""}" data-page="library">${icon("library")}<span>Bibliothek</span></button>
          <button class="nav-item ${state.page === "settings" ? "active" : ""}" data-page="settings">${icon("settings")}<span>Einstellungen</span></button>
        </nav>
        <div class="sidebar-section">
          <div class="sidebar-label"><span>Instanzen</span><button id="sidebar-add" class="mini-button" aria-label="Instanz erstellen">${icon("plus")}</button></div>
          <div class="instance-nav">
            ${state.data.instances.map((instance) => `
              <button class="instance-nav-item ${instance.id === state.activeId && state.page === "library" ? "active" : ""}" data-instance="${escapeHtml(instance.id)}">
                <span class="instance-avatar small">${escapeHtml(initials(instance.name))}</span>
                <span class="instance-nav-copy"><strong>${escapeHtml(instance.name)}</strong><small>${escapeHtml(instance.version)}</small></span>
              </button>`).join("")}
          </div>
        </div>
        <div class="sidebar-account">
          ${account ? `
            <div class="account-row"><span class="account-avatar">${escapeHtml(initials(account.name))}</span><span><strong>${escapeHtml(account.name)}</strong><small>Microsoft-Konto</small></span><button id="logout" class="mini-button" aria-label="Abmelden">${icon("logout")}</button></div>
          ` : `
            <button id="signin" class="signin-card"><span class="account-avatar muted">?</span><span><strong>Microsoft anmelden</strong><small>Zum Spielen erforderlich</small></span>${icon("chevron")}</button>
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
      ${renderModal()}
      ${state.toast ? `<div class="toast ${state.toast.type}">${icon(state.toast.type === "success" ? "check" : "close")}<span>${escapeHtml(state.toast.message)}</span></div>` : ""}
    </div>`;
}

function renderLibrary() {
  const instance = activeInstance();
  const selectedVersion = state.selectedVersion || instance.version;
  const account = state.data.account;
  return `
    <header class="topbar">
      <div><span class="overline">MINECRAFT JAVA</span><h1>Bibliothek</h1></div>
      <button id="new-instance" class="button secondary">${icon("plus")}Neue Instanz</button>
    </header>
    <div class="content-scroll">
      <section class="launch-surface">
        <div class="instance-summary">
          <div class="instance-avatar large">${escapeHtml(initials(instance.name))}</div>
          <div class="instance-heading"><span class="overline">AUSGEWÄHLTE INSTANZ</span><h2>${escapeHtml(instance.name)}</h2><p>${escapeHtml(formatLastPlayed(instance.lastPlayed))}</p></div>
          <div class="instance-actions">
            <button id="open-instance" class="icon-button" aria-label="Instanzordner öffnen" title="Instanzordner öffnen">${icon("folder")}</button>
            <button id="edit-instance" class="icon-button" aria-label="Instanz bearbeiten" title="Instanz bearbeiten">${icon("edit")}</button>
          </div>
        </div>
        <div class="launch-controls">
          <label class="version-field"><span>Version</span><select id="launch-version">${versionOptions(selectedVersion)}</select></label>
          <button id="primary-action" class="button play-button" ${state.busy ? "disabled" : ""}>
            ${state.busy === "launch" ? `<span class="spinner"></span><span><strong>Wird vorbereitet</strong><small id="play-version-label">${escapeHtml(selectedVersion)}</small></span>` : `${icon(account ? "play" : "spark")}<span><strong>${account ? "Spielen" : "Microsoft anmelden"}</strong><small id="play-version-label">${account ? escapeHtml(selectedVersion) : "Danach kannst du direkt starten"}</small></span>`}
          </button>
        </div>
        <p class="launch-note">${account ? "Java und fehlende Spieldateien werden automatisch verwaltet." : "Wisdom speichert deine Anmeldung sicher in der Windows-Anmeldeinformationsverwaltung."}</p>
      </section>

      <section class="library-section">
        <div class="section-heading"><div><span class="overline">DEINE WELTEN, GETRENNT ORGANISIERT</span><h3>Instanzen</h3></div><span class="count-badge">${state.data.instances.length}</span></div>
        <div class="instance-grid">
          ${state.data.instances.map((item) => `
            <button class="instance-card ${item.id === instance.id ? "selected" : ""}" data-instance="${escapeHtml(item.id)}">
              <span class="instance-avatar">${escapeHtml(initials(item.name))}</span>
              <span class="card-copy"><strong>${escapeHtml(item.name)}</strong><small>Minecraft ${escapeHtml(item.version)}</small><em>${escapeHtml(formatLastPlayed(item.lastPlayed))}</em></span>
              ${icon("chevron")}
            </button>`).join("")}
        </div>
      </section>
    </div>`;
}

function renderSettings() {
  const settings = state.data.settings;
  const ramGb = (settings.ramMb / 1024).toFixed(settings.ramMb % 1024 ? 1 : 0);
  return `
    <header class="topbar"><div><span class="overline">WISDOM</span><h1>Einstellungen</h1></div><button id="save-settings" class="button primary" ${state.busy ? "disabled" : ""}>${icon("check")}Speichern</button></header>
    <div class="content-scroll settings-content">
      <section class="settings-group">
        <div class="settings-intro"><h2>Spiel</h2><p>Diese Werte gelten für alle Instanzen, sofern dort nichts anderes gewählt ist.</p></div>
        <div class="settings-card">
          <label class="setting-row range-row"><span><strong>Arbeitsspeicher</strong><small>Für Vanilla sind 4 GB ein guter Startwert.</small></span><span class="range-control"><output id="ram-output">${ramGb} GB</output><input id="ram" type="range" min="1024" max="16384" step="512" value="${settings.ramMb}" /></span></label>
          <label class="setting-row"><span><strong>Snapshots anzeigen</strong><small>Entwicklungsversionen in der Versionsauswahl einblenden.</small></span><input id="snapshots" class="switch" type="checkbox" ${settings.showSnapshots ? "checked" : ""} /></label>
          <label class="setting-row"><span><strong>Java-Konsole öffnen</strong><small>Nützlich zur Fehlersuche; beim normalen Spielen nicht nötig.</small></span><input id="console" class="switch" type="checkbox" ${settings.openConsole ? "checked" : ""} /></label>
        </div>
      </section>
      <section class="settings-group">
        <div class="settings-intro"><h2>Erweitert</h2><p>Nur ändern, wenn du die Argumente wirklich benötigst.</p></div>
        <div class="settings-card form-card">
          <label class="field"><span>Zusätzliche JVM-Argumente</span><input id="global-jvm" value="${escapeHtml(settings.jvmArgs)}" placeholder='z. B. -XX:+UseG1GC' /></label>
          <label class="field"><span>Zusätzliche Spielargumente</span><input id="global-game" value="${escapeHtml(settings.gameArgs)}" placeholder="Optional" /></label>
        </div>
      </section>
      <section class="settings-group">
        <div class="settings-intro"><h2>Speicherort</h2><p>Instanzen, Java-Laufzeiten und Downloads liegen in einem gemeinsamen Wisdom-Ordner.</p></div>
        <div class="settings-card storage-row"><code>${escapeHtml(state.data.dataDirectory)}</code><button id="open-data" class="button secondary">${icon("folder")}Ordner öffnen</button></div>
      </section>
    </div>`;
}

function renderModal() {
  if (!state.modal) return "";
  const instance = activeInstance();
  if (state.modal === "delete") {
    return `<div class="modal-backdrop"><section class="modal compact" role="dialog" aria-modal="true"><div class="danger-mark">${icon("trash")}</div><h2>Instanz löschen?</h2><p>„${escapeHtml(instance.name)}“ und alle darin gespeicherten Welten werden dauerhaft entfernt.</p><div class="modal-actions"><button data-close-modal class="button secondary">Abbrechen</button><button id="confirm-delete" class="button danger">Endgültig löschen</button></div></section></div>`;
  }
  const editing = state.modal === "edit";
  const selected = editing ? instance.version : state.data.latestVersion;
  return `
    <div class="modal-backdrop">
      <section class="modal" role="dialog" aria-modal="true" aria-labelledby="modal-title">
        <div class="modal-header"><div><span class="overline">${editing ? "INSTANZ KONFIGURIEREN" : "NEUE INSTANZ"}</span><h2 id="modal-title">${editing ? escapeHtml(instance.name) : "Minecraft einrichten"}</h2></div><button data-close-modal class="icon-button">${icon("close")}</button></div>
        <form id="instance-form">
          <label class="field"><span>Name</span><input id="instance-name" maxlength="48" value="${editing ? escapeHtml(instance.name) : "Neue Instanz"}" required autofocus /></label>
          <label class="field"><span>Minecraft-Version</span><select id="instance-version">${versionOptions(selected)}</select></label>
          ${editing ? `
            <label class="setting-row inline-setting"><span><strong>Eigener Arbeitsspeicher</strong><small>Überschreibt die globale Einstellung für diese Instanz.</small></span><input id="override-ram" class="switch" type="checkbox" ${instance.ramMb ? "checked" : ""} /></label>
            <label id="instance-ram-wrap" class="field ${instance.ramMb ? "" : "disabled"}"><span>Arbeitsspeicher <output id="instance-ram-output">${((instance.ramMb || state.data.settings.ramMb) / 1024).toFixed(1)} GB</output></span><input id="instance-ram" type="range" min="1024" max="16384" step="512" value="${instance.ramMb || state.data.settings.ramMb}" ${instance.ramMb ? "" : "disabled"} /></label>
            <details class="advanced"><summary>Erweiterte Startoptionen</summary><div class="advanced-fields"><label class="field"><span>JVM-Argumente</span><input id="instance-jvm" value="${escapeHtml(instance.jvmArgs || "")}" placeholder="Globale Einstellung verwenden" /></label><label class="field"><span>Spielargumente</span><input id="instance-game" value="${escapeHtml(instance.gameArgs || "")}" placeholder="Globale Einstellung verwenden" /></label></div></details>
          ` : `<p class="form-hint">Vanilla, sauber getrennt von deinen anderen Instanzen. Java und alle nötigen Dateien richtet Wisdom beim ersten Start ein.</p>`}
          <div class="modal-footer">${editing && state.data.instances.length > 1 ? `<button id="delete-instance" type="button" class="button text-danger">${icon("trash")}Instanz löschen</button>` : ""}<span></span><button type="button" data-close-modal class="button secondary">Abbrechen</button><button type="submit" class="button primary">${editing ? "Änderungen speichern" : "Instanz erstellen"}</button></div>
        </form>
      </section>
    </div>`;
}

function render() {
  if (!state.data) {
    app.innerHTML = `<div class="boot"><span class="brand-mark large-mark">W</span><div><strong>Wisdom</strong><span>${escapeHtml(state.status)}</span></div><span class="boot-line"></span></div>`;
    return;
  }
  const content = state.page === "settings" ? renderSettings() : renderLibrary();
  app.innerHTML = shell(content);
  bindEvents();
}

function bindEvents() {
  document.querySelectorAll("[data-page]").forEach((button) => button.addEventListener("click", () => {
    state.page = button.dataset.page;
    state.modal = null;
    render();
  }));
  document.querySelectorAll("[data-instance]").forEach((button) => button.addEventListener("click", () => {
    state.activeId = button.dataset.instance;
    state.selectedVersion = activeInstance().version;
    state.page = "library";
    state.modal = null;
    render();
  }));
  document.querySelector("#new-instance")?.addEventListener("click", () => openModal("create"));
  document.querySelector("#sidebar-add")?.addEventListener("click", () => openModal("create"));
  document.querySelector("#edit-instance")?.addEventListener("click", () => openModal("edit"));
  document.querySelectorAll("[data-close-modal]").forEach((button) => button.addEventListener("click", () => openModal(null)));
  document.querySelector("#instance-form")?.addEventListener("submit", saveInstance);
  document.querySelector("#delete-instance")?.addEventListener("click", () => openModal("delete"));
  document.querySelector("#confirm-delete")?.addEventListener("click", deleteInstance);
  document.querySelector("#open-instance")?.addEventListener("click", () => callSimple("open_instance_folder", { instanceId: activeInstance().id }, "Instanzordner geöffnet."));
  document.querySelector("#open-data")?.addEventListener("click", () => callSimple("open_data_folder", {}, "Datenordner geöffnet."));
  document.querySelector("#signin")?.addEventListener("click", login);
  document.querySelector("#logout")?.addEventListener("click", logout);
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
  state.modal = modal;
  render();
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
  state.busy = "login";
  state.status = "Microsoft-Anmeldung wird im Browser geöffnet …";
  render();
  try {
    state.data.account = await invoke("sign_in");
    notify("Anmeldung erfolgreich.", "success");
    state.status = "Angemeldet. Bereit zum Spielen.";
  } catch (error) {
    fail("Anmeldung fehlgeschlagen", error);
  } finally {
    state.busy = null;
    render();
  }
}

async function logout() {
  if (state.busy) return;
  try {
    await invoke("sign_out");
    state.data.account = null;
    state.status = "Abgemeldet.";
    notify("Du wurdest abgemeldet.", "success");
    render();
  } catch (error) {
    fail("Abmelden fehlgeschlagen", error);
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
      notify("Instanz gespeichert.", "success");
    } else {
      instance = await invoke("create_instance", { name, version });
      state.data.instances.push(instance);
      state.activeId = instance.id;
      notify("Instanz erstellt.", "success");
    }
    state.selectedVersion = instance.version;
    state.modal = null;
    state.status = "Bereit zum Spielen.";
  } catch (error) {
    fail(editing ? "Speichern fehlgeschlagen" : "Erstellen fehlgeschlagen", error);
  } finally {
    state.busy = null;
    render();
  }
}

async function deleteInstance() {
  if (state.busy) return;
  const instance = activeInstance();
  state.busy = "delete";
  try {
    await invoke("delete_instance", { instanceId: instance.id });
    state.data.instances = state.data.instances.filter((item) => item.id !== instance.id);
    state.activeId = state.data.instances[0].id;
    state.selectedVersion = state.data.instances[0].version;
    state.modal = null;
    notify("Instanz wurde gelöscht.", "success");
  } catch (error) {
    fail("Löschen fehlgeschlagen", error);
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
    state.status = "Einstellungen gespeichert.";
    notify("Einstellungen gespeichert.", "success");
  } catch (error) {
    fail("Speichern fehlgeschlagen", error);
  } finally {
    state.busy = null;
    render();
  }
}

async function launch() {
  if (state.busy) return;
  const instance = activeInstance();
  const version = state.selectedVersion || instance.version;
  state.busy = "launch";
  state.progress = 0;
  state.status = `Minecraft ${version} wird vorbereitet …`;
  render();
  try {
    const updated = await invoke("launch", { instanceId: instance.id, version });
    const index = state.data.instances.findIndex((item) => item.id === updated.id);
    state.data.instances[index] = updated;
    state.selectedVersion = updated.version;
    state.progress = 1;
    state.status = "Minecraft wurde gestartet.";
    notify("Minecraft läuft. Viel Spaß!", "success");
  } catch (error) {
    state.progress = 0;
    fail("Minecraft konnte nicht gestartet werden", error);
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
    fail("Aktion fehlgeschlagen", error);
  }
}

function cleanError(error) {
  return String(error || "Unbekannter Fehler").replace(/^Error:\s*/i, "");
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
    state.data = await invoke("load_launcher");
    state.activeId = state.data.instances[0].id;
    state.selectedVersion = state.data.instances[0].version;
    state.status = "Bereit zum Spielen.";
    render();
  } catch (error) {
    app.innerHTML = `<div class="fatal"><span class="brand-mark large-mark">W</span><h1>Wisdom konnte nicht starten</h1><p>${escapeHtml(cleanError(error))}</p><button id="retry" class="button primary">Erneut versuchen</button></div>`;
    document.querySelector("#retry").addEventListener("click", () => location.reload());
  }
}

init();
