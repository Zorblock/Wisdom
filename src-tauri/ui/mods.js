import "./mods.css";

export function createModsFeature({ invoke, getInstance, isRunning, icon, escapeHtml, cleanError, notify, rerender, setStatus, goBack }) {
  const state = {
    instanceId: null,
    mods: [],
    results: [],
    query: "",
    offset: 0,
    totalHits: 0,
    loadingMods: false,
    searching: false,
    action: null,
    error: null,
    request: 0,
  };
  let searchTimer;

  const loaderName = (loader) => ({
    fabric: "Fabric",
    quilt: "Quilt",
    forge: "Forge",
    neoforge: "NeoForge",
  })[loader] || "Vanilla";

  const projectIcon = (url, title) => {
    try {
      const parsed = new URL(String(url || ""));
      if (parsed.protocol === "https:" && (parsed.hostname === "cdn.modrinth.com" || parsed.hostname.endsWith(".modrinth.com"))) {
        return `<img src="${escapeHtml(parsed.href)}" alt="" loading="lazy" />`;
      }
    } catch {
      // Use the local icon fallback.
    }
    return `<span aria-label="${escapeHtml(title)}">${icon("mods")}</span>`;
  };

  const installedIds = () => new Set(state.mods.map((item) => item.projectId));

  function renderInstalled() {
    const locked = state.action || isRunning(getInstance()?.id);
    if (state.loadingMods) return `<div class="mods-placeholder"><span class="spinner"></span><span>Checking installed mods...</span></div>`;
    if (!state.mods.length) return `<div class="mods-empty">No managed mods installed.</div>`;
    return state.mods.map((item) => `
      <article class="managed-mod ${item.compatible && !item.missing ? "" : "attention"}">
        <span class="mod-project-icon">${projectIcon(item.iconUrl, item.title)}</span>
        <span class="mod-copy">
          <strong>${escapeHtml(item.title)}</strong>
          <small>${escapeHtml(item.versionNumber)}${item.explicit ? "" : " · Dependency"}</small>
        </span>
        <span class="mod-state">
          ${item.missing ? `<span class="mod-tag warning">Missing file</span>` : ""}
          ${!item.compatible ? `<span class="mod-tag warning">Incompatible</span>` : ""}
          ${item.updateAvailable ? `<span class="mod-tag update">${escapeHtml(item.latestVersionNumber || "Update")}</span>` : ""}
        </span>
        <span class="mod-actions">
          ${(item.updateAvailable || item.missing) ? `<button class="icon-button" data-update-mod="${escapeHtml(item.projectId)}" title="Update mod" aria-label="Update ${escapeHtml(item.title)}" ${locked ? "disabled" : ""}>${icon("refresh")}</button>` : ""}
          ${item.explicit ? `<button class="icon-button danger-icon" data-remove-mod="${escapeHtml(item.projectId)}" title="Remove mod" aria-label="Remove ${escapeHtml(item.title)}" ${locked ? "disabled" : ""}>${icon("trash")}</button>` : ""}
        </span>
      </article>`).join("");
  }

  function renderSearchResults() {
    const locked = state.action || isRunning(getInstance()?.id);
    if (state.searching) return `<div class="mods-placeholder"><span class="spinner"></span><span>Searching Modrinth...</span></div>`;
    if (!state.results.length) return `<div class="mods-empty">No compatible mods found.</div>`;
    const installed = installedIds();
    return state.results.map((item) => {
      const isInstalled = installed.has(item.projectId);
      const isBusy = state.action === item.projectId;
      return `
        <article class="modrinth-result">
          <span class="mod-project-icon">${projectIcon(item.iconUrl, item.title)}</span>
          <span class="mod-copy">
            <strong>${escapeHtml(item.title)}</strong>
            <small>${escapeHtml(item.description)}</small>
            <span>by ${escapeHtml(item.author)} · ${Number(item.downloads || 0).toLocaleString("en-US")} downloads</span>
          </span>
          <button class="button ${isInstalled ? "secondary" : "primary"}" data-install-mod="${escapeHtml(item.projectId)}" ${isInstalled || locked ? "disabled" : ""}>
            ${isBusy ? `<span class="spinner"></span>` : icon(isInstalled ? "check" : "download")}${isInstalled ? "Installed" : "Install"}
          </button>
        </article>`;
    }).join("");
  }

  function render(instance) {
    const updateCount = state.mods.filter((item) => item.updateAvailable).length;
    const running = isRunning(instance.id);
    return `
      <header class="topbar mods-topbar">
        <div><button id="mods-back" class="icon-button" aria-label="Back to instance" title="Back">${icon("back")}</button><h1>Mods</h1></div>
        <button id="update-all-mods" class="button secondary" ${!updateCount || state.action || running ? "disabled" : ""}>${icon("refresh")}Update all${updateCount ? ` (${updateCount})` : ""}</button>
      </header>
      <div class="content-scroll mods-content">
        <section class="mods-instance-bar">
          <span><strong>${escapeHtml(instance.name)}</strong><small>Minecraft ${escapeHtml(instance.version)} · ${loaderName(instance.loader)}</small></span>
          <button id="refresh-mods" class="icon-button" title="Refresh mods" aria-label="Refresh mods" ${state.action ? "disabled" : ""}>${icon("refresh")}</button>
        </section>
        ${running ? `<div class="mods-running-note">${icon("warning")}<span>Stop Minecraft to install, update, or remove mods.</span></div>` : ""}
        ${state.error ? `<div class="mods-error">${icon("warning")}<span>${escapeHtml(state.error)}</span></div>` : ""}
        <section class="mods-section">
          <div class="section-heading"><h3>Installed</h3><span class="count-badge">${state.mods.length}</span></div>
          <div class="managed-mod-list">${renderInstalled()}</div>
        </section>
        <section class="mods-section discover-section">
          <div class="section-heading"><h3>Discover on Modrinth</h3></div>
          <label class="mods-search"><i class="fa-solid fa-magnifying-glass" aria-hidden="true"></i><input id="mods-search" value="${escapeHtml(state.query)}" placeholder="Search compatible mods" autocomplete="off" spellcheck="false" /></label>
          <div class="modrinth-results">${renderSearchResults()}</div>
          ${state.totalHits > 24 ? `<div class="mods-pagination"><button id="mods-previous" class="button secondary" ${state.offset === 0 || state.searching ? "disabled" : ""}>Previous</button><span>${state.offset + 1}–${Math.min(state.offset + 24, state.totalHits)} of ${state.totalHits}</span><button id="mods-next" class="button secondary" ${state.offset + 24 >= state.totalHits || state.searching ? "disabled" : ""}>Next</button></div>` : ""}
        </section>
      </div>`;
  }

  async function loadMods(refreshUpdates = true) {
    const instance = getInstance();
    if (!instance) return;
    state.loadingMods = true;
    state.error = null;
    rerender();
    try {
      state.mods = await invoke("list_instance_mods", { instanceId: instance.id, refreshUpdates });
    } catch (error) {
      state.error = cleanError(error);
    } finally {
      state.loadingMods = false;
      rerender();
    }
  }

  async function search(offset = 0) {
    const instance = getInstance();
    if (!instance) return;
    const request = ++state.request;
    state.offset = offset;
    state.searching = true;
    state.error = null;
    rerender();
    try {
      const result = await invoke("search_modrinth", { instanceId: instance.id, query: state.query, offset });
      if (request !== state.request) return;
      state.results = result.hits;
      state.offset = result.offset;
      state.totalHits = result.totalHits;
    } catch (error) {
      if (request === state.request) state.error = cleanError(error);
    } finally {
      if (request === state.request) {
        state.searching = false;
        rerender();
      }
    }
  }

  async function runAction(command, projectId, successMessage) {
    const instance = getInstance();
    if (!instance || state.action) return;
    if (isRunning(instance.id)) {
      notify("Stop Minecraft before changing installed mods.", "error");
      return;
    }
    state.action = projectId || "all";
    state.error = null;
    setStatus(successMessage.replace(/\.$/, "...").replace(/^Mod /, "Processing mod "));
    rerender();
    try {
      const args = { instanceId: instance.id };
      if (projectId) args.projectId = projectId;
      state.mods = await invoke(command, args);
      notify(successMessage, "success");
      setStatus(successMessage);
    } catch (error) {
      state.error = cleanError(error);
      notify(state.error, "error");
    } finally {
      state.action = null;
      rerender();
    }
  }

  function bind() {
    document.querySelector("#mods-back")?.addEventListener("click", goBack);
    document.querySelector("#refresh-mods")?.addEventListener("click", () => loadMods(true));
    document.querySelector("#update-all-mods")?.addEventListener("click", () => runAction("update_all_modrinth_mods", null, "Mods updated."));
    document.querySelectorAll("[data-install-mod]").forEach((button) => button.addEventListener("click", () => runAction("install_modrinth_mod", button.dataset.installMod, "Mod installed.")));
    document.querySelectorAll("[data-remove-mod]").forEach((button) => button.addEventListener("click", () => runAction("remove_modrinth_mod", button.dataset.removeMod, "Mod removed.")));
    document.querySelectorAll("[data-update-mod]").forEach((button) => button.addEventListener("click", () => runAction("update_modrinth_mod", button.dataset.updateMod, "Mod updated.")));
    document.querySelector("#mods-previous")?.addEventListener("click", () => search(Math.max(0, state.offset - 24)));
    document.querySelector("#mods-next")?.addEventListener("click", () => search(state.offset + 24));
    document.querySelector("#mods-search")?.addEventListener("input", (event) => {
      state.query = event.target.value;
      clearTimeout(searchTimer);
      searchTimer = setTimeout(() => search(0), 320);
    });
  }

  async function open() {
    const instance = getInstance();
    if (!instance) return;
    if (state.instanceId !== instance.id) {
      state.instanceId = instance.id;
      state.mods = [];
      state.results = [];
      state.query = "";
      state.offset = 0;
      state.totalHits = 0;
      state.error = null;
    }
    await Promise.all([loadMods(true), search(0)]);
  }

  return { render, bind, open };
}
