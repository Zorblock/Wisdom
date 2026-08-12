import "./mods.css";

export function createModsFeature({ invoke, getInstance, isRunning, icon, escapeHtml, cleanError, customSelect, notify, rerender, setStatus, goBack }) {
  const state = {
    instanceId: null,
    mods: [],
    results: [],
    query: "",
    installedQuery: "",
    installedFilter: "all",
    installedSort: "name",
    sortDirection: "asc",
    enabledFirst: true,
    searchIndex: "relevance",
    category: "all",
    offset: 0,
    totalHits: 0,
    loadingMods: false,
    searching: false,
    action: null,
    error: null,
    request: 0,
  };
  let searchTimer;

  const actionCopy = {
    install: { label: "Installing", detail: "Resolving the compatible version and required dependencies" },
    remove: { label: "Removing", detail: "Removing the mod and unused dependencies" },
    update: { label: "Updating", detail: "Downloading the compatible update and checking dependencies" },
    updateAll: { label: "Updating mods", detail: "Applying every compatible update" },
    enable: { label: "Enabling", detail: "Activating the managed mod file" },
    disable: { label: "Disabling", detail: "Deactivating the managed mod file" },
  };

  const loaderName = (loader) => ({ fabric: "Fabric", quilt: "Quilt", forge: "Forge", neoforge: "NeoForge" })[loader] || "Vanilla";
  const sortOptions = [
    { value: "name", label: "Name" },
    { value: "filename", label: "File name" },
    { value: "size", label: "File size" },
    { value: "version", label: "Version" },
  ];
  const searchSortOptions = [
    { value: "relevance", label: "Relevance" },
    { value: "downloads", label: "Downloads" },
    { value: "follows", label: "Followers" },
    { value: "updated", label: "Recently updated" },
    { value: "newest", label: "Newest" },
  ];
  const categoryOptions = [
    { value: "all", label: "All categories" },
    { value: "optimization", label: "Optimization" },
    { value: "utility", label: "Utility" },
    { value: "library", label: "Library" },
    { value: "technology", label: "Technology" },
    { value: "cursed", label: "Cursed" },
    { value: "adventure", label: "Adventure" },
    { value: "worldgen", label: "World generation" },
    { value: "mobs", label: "Mobs" },
    { value: "decoration", label: "Decoration" },
    { value: "storage", label: "Storage" },
    { value: "equipment", label: "Equipment" },
    { value: "magic", label: "Magic" },
    { value: "transportation", label: "Transportation" },
    { value: "management", label: "Management" },
    { value: "social", label: "Social" },
    { value: "food", label: "Food" },
    { value: "economy", label: "Economy" },
    { value: "minigame", label: "Minigames" },
  ];
  const installedFilters = [
    ["all", "All"], ["enabled", "Enabled"], ["disabled", "Disabled"],
    ["direct", "Direct"], ["dependencies", "Dependencies"], ["updates", "Updates"], ["issues", "Issues"],
  ];

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

  const formatSize = (bytes) => {
    const size = Number(bytes || 0);
    if (!size) return "Unknown size";
    if (size < 1024 * 1024) return `${(size / 1024).toFixed(0)} KB`;
    return `${(size / (1024 * 1024)).toFixed(size < 10 * 1024 * 1024 ? 1 : 0)} MB`;
  };

  const installedIds = () => new Set(state.mods.map((item) => item.projectId));

  function visibleInstalledMods() {
    const query = state.installedQuery.trim().toLowerCase();
    const visible = state.mods.filter((item) => {
      if (query && ![item.title, item.fileName, item.versionNumber].some((value) => String(value || "").toLowerCase().includes(query))) return false;
      if (state.installedFilter === "enabled") return item.enabled;
      if (state.installedFilter === "disabled") return !item.enabled;
      if (state.installedFilter === "direct") return item.explicit;
      if (state.installedFilter === "dependencies") return !item.explicit;
      if (state.installedFilter === "updates") return item.updateAvailable;
      if (state.installedFilter === "issues") return item.missing || !item.compatible;
      return true;
    });
    const direction = state.sortDirection === "asc" ? 1 : -1;
    return visible.sort((left, right) => {
      if (state.enabledFirst && left.enabled !== right.enabled) return left.enabled ? -1 : 1;
      let compared = 0;
      if (state.installedSort === "size") compared = Number(left.fileSize || 0) - Number(right.fileSize || 0);
      else {
        const key = state.installedSort === "filename" ? "fileName" : state.installedSort === "version" ? "versionNumber" : "title";
        compared = String(left[key] || "").localeCompare(String(right[key] || ""), "en", { numeric: true, sensitivity: "base" });
      }
      return compared * direction || left.title.localeCompare(right.title);
    });
  }

  function renderInstalled() {
    const locked = state.action || isRunning(getInstance()?.id);
    if (state.loadingMods) return `<div class="mods-placeholder"><span class="spinner"></span><span>Checking installed mods...</span></div>`;
    const visible = visibleInstalledMods();
    if (!state.mods.length) return `<div class="mods-empty">No managed mods installed.</div>`;
    if (!visible.length) return `<div class="mods-empty">No mods match these filters.</div>`;
    return visible.map((item) => {
      const activeAction = state.action?.projectId === item.projectId ? state.action : null;
      const relation = item.explicit
        ? `${item.dependencyCount ? `${item.dependencyCount} required ${item.dependencyCount === 1 ? "dependency" : "dependencies"}` : "Direct install"}`
        : `Dependency${item.requiredByCount ? ` required by ${item.requiredByCount}` : ""}`;
      return `
        <article class="managed-mod ${item.enabled ? "" : "disabled-mod"} ${item.compatible && !item.missing ? "" : "attention"} ${activeAction ? "mod-action-active" : ""}">
          <span class="mod-project-icon">${projectIcon(item.iconUrl, item.title)}</span>
          <span class="mod-copy">
            <strong>${escapeHtml(item.title)}</strong>
            <small>${escapeHtml(item.versionNumber)} &middot; ${escapeHtml(relation)}</small>
            <span title="${escapeHtml(item.fileName)}">${escapeHtml(item.fileName)} &middot; ${formatSize(item.fileSize)}</span>
          </span>
          <span class="mod-state">
            ${activeAction ? `<span class="mod-tag action-tag"><span class="spinner"></span>${escapeHtml(activeAction.label)}&hellip;</span>` : ""}
            ${!item.enabled ? `<span class="mod-tag">Disabled</span>` : ""}
            ${item.missing ? `<span class="mod-tag warning">Missing file</span>` : ""}
            ${!item.compatible ? `<span class="mod-tag warning">Incompatible</span>` : ""}
            ${item.updateAvailable ? `<span class="mod-tag update">${escapeHtml(item.latestVersionNumber || "Update")}</span>` : ""}
          </span>
          <span class="mod-actions">
            ${activeAction ? `<span class="mod-action-spinner" role="status" aria-label="${escapeHtml(activeAction.label)} ${escapeHtml(item.title)}"><span class="spinner"></span></span>` : `
              <button class="icon-button" data-toggle-mod="${escapeHtml(item.projectId)}" data-enable="${!item.enabled}" title="${item.enabled ? "Disable" : "Enable"} mod" aria-label="${item.enabled ? "Disable" : "Enable"} ${escapeHtml(item.title)}" ${locked || item.missing ? "disabled" : ""}>${icon("power")}</button>
              ${(item.updateAvailable || item.missing) ? `<button class="icon-button" data-update-mod="${escapeHtml(item.projectId)}" title="${item.missing ? "Repair" : "Update"} mod" aria-label="Update ${escapeHtml(item.title)}" ${locked ? "disabled" : ""}>${icon("refresh")}</button>` : ""}
              ${item.explicit ? `<button class="icon-button danger-icon" data-remove-mod="${escapeHtml(item.projectId)}" title="Remove mod and unused dependencies" aria-label="Remove ${escapeHtml(item.title)}" ${locked ? "disabled" : ""}>${icon("trash")}</button>` : ""}
            `}
          </span>
        </article>`;
    }).join("");
  }

  function renderSearchResults() {
    const locked = state.action || isRunning(getInstance()?.id);
    if (state.searching) return `<div class="mods-placeholder"><span class="spinner"></span><span>Searching Modrinth...</span></div>`;
    if (!state.results.length) return `<div class="mods-empty">No compatible mods found.</div>`;
    const installed = installedIds();
    return state.results.map((item) => {
      const isInstalled = installed.has(item.projectId);
      const activeAction = state.action?.projectId === item.projectId ? state.action : null;
      const categories = (item.categories || []).slice(0, 3).map((category) => `<span>${escapeHtml(category.replaceAll("-", " "))}</span>`).join("");
      return `
        <article class="modrinth-result ${activeAction ? "mod-action-active" : ""}">
          <span class="mod-project-icon">${projectIcon(item.iconUrl, item.title)}</span>
          <span class="mod-copy">
            <strong>${escapeHtml(item.title)}</strong>
            <small>${escapeHtml(item.description)}</small>
            <span>by ${escapeHtml(item.author)} &middot; ${Number(item.downloads || 0).toLocaleString("en-US")} downloads</span>
            ${categories ? `<span class="result-categories">${categories}</span>` : ""}
          </span>
          <button class="button ${isInstalled ? "secondary" : "primary"} ${activeAction ? "action-button" : ""}" data-install-mod="${escapeHtml(item.projectId)}" ${isInstalled || locked ? "disabled" : ""} aria-live="polite">
            ${activeAction ? `<span class="spinner"></span>${escapeHtml(activeAction.label)}&hellip;` : `${icon(isInstalled ? "check" : "download")}${isInstalled ? "Installed" : "Install"}`}
          </button>
        </article>`;
    }).join("");
  }

  function render(instance) {
    const updateCount = state.mods.filter((item) => item.updateAvailable).length;
    const enabledCount = state.mods.filter((item) => item.enabled).length;
    const running = isRunning(instance.id);
    return `
      <header class="topbar mods-topbar">
        <div><button id="mods-back" class="icon-button" aria-label="Back to instance" title="Back">${icon("back")}</button><h1>Mods</h1></div>
        <button id="update-all-mods" class="button secondary" ${!updateCount || state.action || running ? "disabled" : ""}>${state.action?.kind === "updateAll" ? `<span class="spinner"></span>Updating&hellip;` : `${icon("refresh")}Update all${updateCount ? ` (${updateCount})` : ""}`}</button>
      </header>
      <div class="content-scroll mods-content">
        <section class="mods-instance-bar">
          <span><strong>${escapeHtml(instance.name)}</strong><small>Minecraft ${escapeHtml(instance.version)} &middot; ${loaderName(instance.loader)} &middot; ${enabledCount}/${state.mods.length} enabled</small></span>
          <button id="refresh-mods" class="icon-button" title="Refresh mods and updates" aria-label="${state.loadingMods ? "Refreshing mods" : "Refresh mods"}" ${state.action || state.loadingMods ? "disabled" : ""}>${state.loadingMods ? `<span class="spinner"></span>` : icon("refresh")}</button>
        </section>
        ${state.action ? `<div class="mods-action-feedback" role="status" aria-live="polite"><span class="spinner"></span><span><strong>${escapeHtml(state.action.label)}${state.action.title ? ` ${escapeHtml(state.action.title)}` : ""}&hellip;</strong><small>${escapeHtml(state.action.detail)}</small></span></div>` : ""}
        ${running ? `<div class="mods-running-note">${icon("warning")}<span>Stop Minecraft to install, update, enable, disable, or remove mods.</span></div>` : ""}
        ${state.error ? `<div class="mods-error">${icon("warning")}<span>${escapeHtml(state.error)}</span></div>` : ""}
        <section class="mods-section">
          <div class="section-heading"><h3>Installed</h3><span class="count-badge">${visibleInstalledMods().length}/${state.mods.length}</span></div>
          <div class="mods-toolbar installed-toolbar">
            <label class="mods-search"><i class="fa-solid fa-magnifying-glass" aria-hidden="true"></i><input id="installed-mod-search" value="${escapeHtml(state.installedQuery)}" placeholder="Search installed mods" autocomplete="off" spellcheck="false" /></label>
            <div class="compact-select">${customSelect("installed-mod-sort", state.installedSort, sortOptions, "Sort installed mods")}</div>
            <button id="sort-direction" class="icon-button" title="Reverse sort order" aria-label="Reverse sort order">${icon(state.sortDirection === "asc" ? "sortAsc" : "sortDesc")}</button>
          </div>
          <div class="mods-filter-row">
            <div class="filter-chips">${installedFilters.map(([value, label]) => `<button class="filter-chip ${state.installedFilter === value ? "active" : ""}" data-installed-filter="${value}">${label}</button>`).join("")}</div>
            <label class="enabled-first"><input id="enabled-first" class="switch" type="checkbox" ${state.enabledFirst ? "checked" : ""} /><span>Enabled first</span></label>
          </div>
          <div class="managed-mod-list">${renderInstalled()}</div>
        </section>
        <section class="mods-section discover-section">
          <div class="section-heading"><h3>Discover on Modrinth</h3><span class="compatibility-note">${loaderName(instance.loader)} &middot; ${escapeHtml(instance.version)}</span></div>
          <div class="mods-toolbar discover-toolbar">
            <label class="mods-search"><i class="fa-solid fa-magnifying-glass" aria-hidden="true"></i><input id="mods-search" value="${escapeHtml(state.query)}" placeholder="Search compatible mods" autocomplete="off" spellcheck="false" /></label>
            <div class="compact-select">${customSelect("modrinth-category", state.category, categoryOptions, "Mod category")}</div>
            <div class="compact-select">${customSelect("modrinth-sort", state.searchIndex, searchSortOptions, "Sort Modrinth results")}</div>
          </div>
          <div class="modrinth-results">${renderSearchResults()}</div>
          ${state.totalHits > 24 ? `<div class="mods-pagination"><button id="mods-previous" class="button secondary" ${state.offset === 0 || state.searching ? "disabled" : ""}>Previous</button><span>${state.offset + 1}&ndash;${Math.min(state.offset + 24, state.totalHits)} of ${state.totalHits}</span><button id="mods-next" class="button secondary" ${state.offset + 24 >= state.totalHits || state.searching ? "disabled" : ""}>Next</button></div>` : ""}
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
      const result = await invoke("search_modrinth", {
        instanceId: instance.id,
        query: state.query,
        index: state.searchIndex,
        category: state.category === "all" ? null : state.category,
        offset,
      });
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

  async function runAction(command, kind, projectId, successMessage, extra = {}) {
    const instance = getInstance();
    if (!instance || state.action) return;
    if (isRunning(instance.id)) {
      notify("Stop Minecraft before changing installed mods.", "error");
      return;
    }
    const copy = actionCopy[kind] || { label: "Working", detail: "Applying changes" };
    const item = state.mods.find((mod) => mod.projectId === projectId)
      || state.results.find((mod) => mod.projectId === projectId);
    state.action = {
      kind,
      projectId: projectId || null,
      title: kind === "updateAll" ? "" : (item?.title || "mod"),
      label: copy.label,
      detail: copy.detail,
    };
    state.error = null;
    setStatus(`${copy.label}${state.action.title ? ` ${state.action.title}` : ""}...`);
    rerender();
    try {
      const args = { instanceId: instance.id, ...extra };
      if (projectId) args.projectId = projectId;
      state.mods = await invoke(command, args);
      notify(successMessage, "success");
      setStatus(successMessage);
    } catch (error) {
      state.error = cleanError(error);
      notify(state.error, "error");
      setStatus(`Action failed: ${state.error}`);
    } finally {
      state.action = null;
      rerender();
    }
  }

  function bind() {
    document.querySelector("#mods-back")?.addEventListener("click", goBack);
    document.querySelector("#refresh-mods")?.addEventListener("click", () => loadMods(true));
    document.querySelector("#update-all-mods")?.addEventListener("click", () => runAction("update_all_modrinth_mods", "updateAll", null, "Mods updated."));
    document.querySelectorAll("[data-install-mod]").forEach((button) => button.addEventListener("click", () => runAction("install_modrinth_mod", "install", button.dataset.installMod, "Mod and required dependencies installed.")));
    document.querySelectorAll("[data-remove-mod]").forEach((button) => button.addEventListener("click", () => runAction("remove_modrinth_mod", "remove", button.dataset.removeMod, "Mod and unused dependencies removed.")));
    document.querySelectorAll("[data-update-mod]").forEach((button) => button.addEventListener("click", () => runAction("update_modrinth_mod", "update", button.dataset.updateMod, "Mod and dependencies updated.")));
    document.querySelectorAll("[data-toggle-mod]").forEach((button) => button.addEventListener("click", () => runAction("set_modrinth_mod_enabled", button.dataset.enable === "true" ? "enable" : "disable", button.dataset.toggleMod, button.dataset.enable === "true" ? "Mod enabled." : "Mod disabled.", { enabled: button.dataset.enable === "true" })));
    document.querySelectorAll("[data-installed-filter]").forEach((button) => button.addEventListener("click", () => { state.installedFilter = button.dataset.installedFilter; rerender(); }));
    document.querySelector("#enabled-first")?.addEventListener("change", (event) => { state.enabledFirst = event.target.checked; rerender(); });
    document.querySelector("#sort-direction")?.addEventListener("click", () => { state.sortDirection = state.sortDirection === "asc" ? "desc" : "asc"; rerender(); });
    document.querySelector("#installed-mod-sort")?.addEventListener("change", (event) => { state.installedSort = event.target.value; rerender(); });
    document.querySelector("#modrinth-category")?.addEventListener("change", (event) => { state.category = event.target.value; search(0); });
    document.querySelector("#modrinth-sort")?.addEventListener("change", (event) => { state.searchIndex = event.target.value; search(0); });
    document.querySelector("#mods-previous")?.addEventListener("click", () => search(Math.max(0, state.offset - 24)));
    document.querySelector("#mods-next")?.addEventListener("click", () => search(state.offset + 24));
    document.querySelector("#installed-mod-search")?.addEventListener("input", (event) => {
      state.installedQuery = event.target.value;
      const cursor = event.target.selectionStart;
      rerender();
      requestAnimationFrame(() => {
        const input = document.querySelector("#installed-mod-search");
        input?.focus({ preventScroll: true });
        input?.setSelectionRange(cursor, cursor);
      });
    });
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
      state.installedQuery = "";
      state.installedFilter = "all";
      state.offset = 0;
      state.totalHits = 0;
      state.error = null;
    }
    await Promise.all([loadMods(true), search(0)]);
  }

  return { render, bind, open };
}
