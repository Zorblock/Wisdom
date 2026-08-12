import "./mods.css";

export function createModsFeature({ invoke, getInstance, isRunning, icon, escapeHtml, cleanError, customSelect, notify, rerender, setStatus, onInstanceCreated, goBack }) {
  const state = {
    instanceId: null,
    contentType: "mod",
    viewMode: "discover",
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
    actions: new Map(),
    actionQueue: [],
    activeActions: 0,
    error: null,
    request: 0,
    loadRequest: 0,
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
  const contentTypes = [
    ["mod", "mods", "Mods"],
    ["modpack", "instance", "Modpacks"],
    ["resourcepack", "image", "Resource packs"],
    ["shader", "shader", "Shaders"],
  ];
  const contentLabel = () => ({ mod: "mod", modpack: "modpack", resourcepack: "resource pack", shader: "shader" })[state.contentType] || "content";
  const supportsInstalled = () => state.contentType !== "modpack";

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
  const actionFor = (projectId) => state.actions.get(`${state.instanceId}:${projectId}`) || null;
  const hasActions = () => state.actions.size > 0;
  const actionLabel = (action) => action.phase === "queued"
    ? "Queued"
    : `${action.label}${action.kind === "install" ? ` ${Math.round((action.progress || 0) * 100)}%` : ""}`;

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
    const locked = isRunning(getInstance()?.id);
    if (state.loadingMods) return `<div class="mods-placeholder"><span class="spinner"></span><span>Checking installed mods...</span></div>`;
    const visible = visibleInstalledMods();
    if (!state.mods.length) return `<div class="mods-empty">No managed mods installed.</div>`;
    if (!visible.length) return `<div class="mods-empty">No mods match these filters.</div>`;
    return visible.map((item) => {
      const activeAction = actionFor(item.projectId);
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
    const locked = isRunning(getInstance()?.id);
    const inventoryPending = state.loadingMods;
    if (state.searching && !state.results.length) return `<div class="mods-placeholder"><span class="spinner"></span><span>Searching Modrinth...</span></div>`;
    if (!state.results.length) return `<div class="mods-empty">No compatible mods found.</div>`;
    const installed = installedIds();
    const cards = state.results.map((item) => {
      const isInstalled = installed.has(item.projectId);
      const activeAction = actionFor(item.projectId);
      const createsInstance = state.contentType === "modpack";
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
          <button class="button ${isInstalled || inventoryPending ? "secondary" : "primary"} ${activeAction ? "action-button" : ""}" data-install-mod="${escapeHtml(item.projectId)}" ${isInstalled || locked || inventoryPending || activeAction ? "disabled" : ""} ${inventoryPending && !isInstalled ? `title="Checking installed mods"` : ""} aria-live="polite">
            ${activeAction ? `<span class="spinner"></span>${escapeHtml(actionLabel(activeAction))}` : isInstalled ? `${icon("check")}Installed` : inventoryPending ? `<span class="spinner"></span>Checking` : `${icon(createsInstance ? "plus" : "download")}${createsInstance ? "Create instance" : "Install"}`}
          </button>
        </article>`;
    }).join("");
    const emptySlots = state.totalHits > 24 ? Math.max(0, 24 - state.results.length) : 0;
    return cards + Array.from({ length: emptySlots }, () => `<div class="modrinth-result modrinth-result-placeholder" aria-hidden="true"></div>`).join("");
  }

  function render(instance) {
    const updateCount = state.mods.filter((item) => item.updateAvailable).length;
    const enabledCount = state.mods.filter((item) => item.enabled).length;
    const running = isRunning(instance.id);
    return `
      <header class="topbar mods-topbar">
        <div><button id="mods-back" class="icon-button" aria-label="Back to instance" title="Back">${icon("back")}</button><h1>Content</h1></div>
        ${state.contentType === "mod" && state.viewMode === "installed" ? `<button id="update-all-mods" class="button secondary" ${!updateCount || hasActions() || running ? "disabled" : ""}>${icon("refresh")}Update all${updateCount ? ` (${updateCount})` : ""}</button>` : ""}
      </header>
      <div class="content-scroll mods-content">
        <section class="mods-instance-bar">
          <span><strong>${escapeHtml(instance.name)}</strong><small>Minecraft ${escapeHtml(instance.version)} &middot; ${loaderName(instance.loader)}${supportsInstalled() ? ` &middot; ${enabledCount}/${state.mods.length} enabled` : ""}</small></span>
          ${supportsInstalled() ? `<button id="refresh-mods" class="icon-button" title="Refresh installed content and updates" aria-label="${state.loadingMods ? "Refreshing content" : "Refresh content"}" ${hasActions() || state.loadingMods ? "disabled" : ""}>${state.loadingMods ? `<span class="spinner"></span>` : icon("refresh")}</button>` : ""}
        </section>
        ${running ? `<div class="mods-running-note">${icon("warning")}<span>Stop Minecraft to change installed content.</span></div>` : ""}
        ${state.error ? `<div class="mods-error">${icon("warning")}<span>${escapeHtml(state.error)}</span></div>` : ""}
        <div class="content-navigation">
          <div class="content-type-tabs">${contentTypes.map(([value, iconName, label]) => `<button class="content-type-tab ${state.contentType === value ? "active" : ""}" data-content-type="${value}" ${value === "mod" && instance.loader === "vanilla" ? `disabled title="Select a mod loader to install mods"` : ""}>${icon(iconName)}<span>${label}</span></button>`).join("")}</div>
          ${supportsInstalled() ? `<div class="content-view-tabs"><button class="content-view-tab ${state.viewMode === "installed" ? "active" : ""}" data-content-view="installed">Installed</button><button class="content-view-tab ${state.viewMode === "discover" ? "active" : ""}" data-content-view="discover">Download</button></div>` : ""}
        </div>
        ${state.viewMode === "installed" && supportsInstalled() ? `<section class="mods-section content-panel">
          <div class="section-heading"><h3>Installed</h3><span class="count-badge">${visibleInstalledMods().length}/${state.mods.length}</span></div>
          <div class="mods-toolbar installed-toolbar">
            <label class="mods-search"><i class="fa-solid fa-magnifying-glass" aria-hidden="true"></i><input id="installed-mod-search" value="${escapeHtml(state.installedQuery)}" placeholder="Search installed mods" autocomplete="off" spellcheck="false" /></label>
            <div class="compact-select">${customSelect("installed-mod-sort", state.installedSort, sortOptions, "Sort installed mods")}</div>
            <button id="sort-direction" class="icon-button" title="Reverse sort order" aria-label="Reverse sort order">${icon(state.sortDirection === "asc" ? "sortAsc" : "sortDesc")}</button>
          </div>
          <div class="mods-filter-row">
            <div class="filter-chips">${installedFilters.filter(([value]) => state.contentType === "mod" || !["direct", "dependencies"].includes(value)).map(([value, label]) => `<button class="filter-chip ${state.installedFilter === value ? "active" : ""}" data-installed-filter="${value}">${label}</button>`).join("")}</div>
            <label class="enabled-first"><input id="enabled-first" class="switch" type="checkbox" ${state.enabledFirst ? "checked" : ""} /><span>Enabled first</span></label>
          </div>
          <div class="managed-mod-list">${renderInstalled()}</div>
        </section>` : `<section class="mods-section discover-section content-panel">
          <div class="section-heading"><h3>Discover ${state.contentType === "mod" ? "mods" : state.contentType === "modpack" ? "modpacks" : state.contentType === "resourcepack" ? "resource packs" : "shaders"}</h3><span class="compatibility-note">Minecraft ${escapeHtml(instance.version)}</span></div>
          <div class="mods-toolbar discover-toolbar ${state.contentType === "mod" ? "" : "without-category"}">
            <label class="mods-search"><i class="fa-solid fa-magnifying-glass" aria-hidden="true"></i><input id="mods-search" value="${escapeHtml(state.query)}" placeholder="Search compatible mods" autocomplete="off" spellcheck="false" /></label>
            ${state.contentType === "mod" ? `<div class="compact-select">${customSelect("modrinth-category", state.category, categoryOptions, "Mod category")}</div>` : ""}
            <div class="compact-select">${customSelect("modrinth-sort", state.searchIndex, searchSortOptions, "Sort Modrinth results")}</div>
          </div>
          <div class="modrinth-results ${state.results.length ? "has-results" : ""} ${state.searching ? "is-loading" : ""}">${renderSearchResults()}</div>
          ${state.totalHits > 24 ? `<div class="mods-pagination"><button id="mods-previous" class="button secondary" ${state.offset === 0 || state.searching ? "disabled" : ""}>Previous</button><span>${state.offset + 1}&ndash;${Math.min(state.offset + 24, state.totalHits)} of ${state.totalHits}</span><button id="mods-next" class="button secondary" ${state.offset + 24 >= state.totalHits || state.searching ? "disabled" : ""}>Next</button></div>` : ""}
        </section>`}
      </div>`;
  }

  async function loadMods(refreshUpdates = true) {
    const instance = getInstance();
    if (!instance) return;
    const request = ++state.loadRequest;
    const contentType = state.contentType;
    state.loadingMods = true;
    state.error = null;
    rerender();
    try {
      const mods = contentType === "mod"
        ? await invoke("list_instance_mods", { instanceId: instance.id, refreshUpdates })
        : contentType === "modpack"
          ? []
          : await invoke("list_instance_content", { instanceId: instance.id, contentType, refreshUpdates });
      if (request === state.loadRequest && state.instanceId === instance.id && state.contentType === contentType) state.mods = mods;
    } catch (error) {
      if (request === state.loadRequest) state.error = cleanError(error);
    } finally {
      if (request === state.loadRequest) {
        state.loadingMods = false;
        rerender();
      }
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
      const command = state.contentType === "mod" ? "search_modrinth" : "search_modrinth_content";
      const result = await invoke(command, {
        instanceId: instance.id,
        ...(state.contentType === "mod" ? {} : { contentType: state.contentType }),
        query: state.query,
        index: state.searchIndex,
        category: state.contentType !== "mod" || state.category === "all" ? null : state.category,
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

  function syncActionButton(projectId) {
    if (!projectId) {
      const action = [...state.actions.values()].find((item) => item.kind === "updateAll" && item.instanceId === state.instanceId);
      const button = document.querySelector("#update-all-mods");
      if (action && button) {
        button.disabled = true;
        button.innerHTML = `<span class="spinner"></span>${escapeHtml(actionLabel(action))}`;
      }
      return;
    }
    const action = actionFor(projectId);
    document.querySelectorAll(`[data-install-mod="${CSS.escape(projectId)}"]`).forEach((button) => {
      if (!action) return;
      button.disabled = true;
      button.classList.add("action-button");
      button.innerHTML = `<span class="spinner"></span>${escapeHtml(actionLabel(action))}`;
    });
    const article = document.querySelector(`[data-install-mod="${CSS.escape(projectId)}"]`)?.closest(".modrinth-result");
    article?.classList.toggle("mod-action-active", Boolean(action));
    const managed = document.querySelector(`[data-update-mod="${CSS.escape(projectId)}"], [data-remove-mod="${CSS.escape(projectId)}"], [data-toggle-mod="${CSS.escape(projectId)}"]`)?.closest(".managed-mod");
    if (managed && action) {
      managed.classList.add("mod-action-active");
      managed.querySelectorAll("button").forEach((button) => { button.disabled = true; });
      const selector = action.kind === "update" ? "[data-update-mod]" : action.kind === "remove" ? "[data-remove-mod]" : "[data-toggle-mod]";
      const button = managed.querySelector(selector);
      if (button) button.innerHTML = `<span class="spinner"></span>`;
    }
  }

  function runAction(command, kind, projectId, successMessage, extra = {}) {
    const instance = getInstance();
    const actionId = `${instance?.id}:${projectId || `global:${kind}`}`;
    if (!instance || state.actions.has(actionId)) return;
    if (isRunning(instance.id)) {
      notify("Stop Minecraft before changing installed content.", "error");
      return;
    }
    if (state.loadingMods && kind === "install") return;
    const copy = actionCopy[kind] || { label: "Working", detail: "Applying changes" };
    const item = state.mods.find((mod) => mod.projectId === projectId)
      || state.results.find((mod) => mod.projectId === projectId);
    const action = {
      kind,
      projectId: actionId,
      title: kind === "updateAll" ? "" : (item?.title || contentLabel()),
      label: copy.label,
      detail: copy.detail,
      phase: "queued",
      progress: 0,
      instanceId: instance.id,
    };
    state.actions.set(actionId, action);
    state.actionQueue.push({ command, projectId, actionId, successMessage, extra, instanceId: instance.id, contentType: state.contentType });
    state.error = null;
    setStatus(`Queued ${action.title || "mod action"}.`);
    syncActionButton(projectId);
    pumpActions();
  }

  function pumpActions() {
    while (state.activeActions < 2 && state.actionQueue.length) {
      const job = state.actionQueue.shift();
      const action = state.actions.get(job.actionId);
      if (!action) continue;
      state.activeActions += 1;
      action.phase = "active";
      syncActionButton(job.projectId);
      void executeAction(job, action);
    }
  }

  function rerenderAnchored(projectId) {
    const selector = projectId ? `[data-install-mod="${CSS.escape(projectId)}"]` : null;
    const before = selector ? document.querySelector(selector)?.closest(".modrinth-result")?.getBoundingClientRect().top : null;
    rerender();
    if (before == null || !selector) return;
    const after = document.querySelector(selector)?.closest(".modrinth-result")?.getBoundingClientRect().top;
    const scroller = document.querySelector(".content-scroll");
    if (after != null && scroller) scroller.scrollTop += after - before;
  }

  async function executeAction(job, action) {
    setStatus(`${action.label}${action.title ? ` ${action.title}` : ""}...`);
    try {
      const args = { instanceId: job.instanceId, ...job.extra };
      if (job.projectId) args.projectId = job.projectId;
      const mods = await invoke(job.command, args);
      if (state.instanceId === job.instanceId && state.contentType === job.contentType) state.mods = mods;
      notify(job.successMessage, "success");
      setStatus(job.successMessage);
    } catch (error) {
      state.error = cleanError(error);
      notify(state.error, "error");
      setStatus(`Action failed: ${state.error}`);
    } finally {
      state.actions.delete(job.actionId);
      state.activeActions -= 1;
      rerenderAnchored(job.projectId);
      pumpActions();
    }
  }

  function updateProgress(payload) {
    if (!payload?.instanceId || !payload?.projectId) return;
    const action = state.actions.get(`${payload.instanceId}:${payload.projectId}`);
    if (!action || !["install", "update"].includes(action.kind)) return;
    action.progress = Math.max(action.progress || 0, Math.min(1, Number(payload.progress) || 0));
    if (state.instanceId === payload.instanceId) syncActionButton(payload.projectId);
  }

  function bind() {
    document.querySelector("#mods-back")?.addEventListener("click", goBack);
    document.querySelector("#refresh-mods")?.addEventListener("click", () => loadMods(true));
    document.querySelector("#update-all-mods")?.addEventListener("click", () => runAction("update_all_modrinth_mods", "updateAll", null, "Mods updated."));
    document.querySelectorAll("[data-content-type]").forEach((button) => button.addEventListener("click", () => switchContentType(button.dataset.contentType)));
    document.querySelectorAll("[data-content-view]").forEach((button) => button.addEventListener("click", () => { state.viewMode = button.dataset.contentView; rerender(); }));
    document.querySelectorAll("[data-install-mod]").forEach((button) => button.addEventListener("click", () => {
      if (state.contentType === "modpack") return installModpack(button.dataset.installMod);
      const isMod = state.contentType === "mod";
      runAction(isMod ? "install_modrinth_mod" : "install_modrinth_content", "install", button.dataset.installMod, isMod ? "Mod and required dependencies installed." : `${contentLabel()[0].toUpperCase()}${contentLabel().slice(1)} installed.`, isMod ? {} : { contentType: state.contentType });
    }));
    document.querySelectorAll("[data-remove-mod]").forEach((button) => button.addEventListener("click", () => {
      const isMod = state.contentType === "mod";
      runAction(isMod ? "remove_modrinth_mod" : "remove_modrinth_content", "remove", button.dataset.removeMod, `${contentLabel()[0].toUpperCase()}${contentLabel().slice(1)} removed.`, isMod ? {} : { contentType: state.contentType });
    }));
    document.querySelectorAll("[data-update-mod]").forEach((button) => button.addEventListener("click", () => {
      const isMod = state.contentType === "mod";
      runAction(isMod ? "update_modrinth_mod" : "install_modrinth_content", "update", button.dataset.updateMod, `${contentLabel()[0].toUpperCase()}${contentLabel().slice(1)} updated.`, isMod ? {} : { contentType: state.contentType });
    }));
    document.querySelectorAll("[data-toggle-mod]").forEach((button) => button.addEventListener("click", () => {
      const enabled = button.dataset.enable === "true";
      const isMod = state.contentType === "mod";
      runAction(isMod ? "set_modrinth_mod_enabled" : "set_modrinth_content_enabled", enabled ? "enable" : "disable", button.dataset.toggleMod, `${contentLabel()[0].toUpperCase()}${contentLabel().slice(1)} ${enabled ? "enabled" : "disabled"}.`, { enabled, ...(isMod ? {} : { contentType: state.contentType }) });
    }));
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

  async function switchContentType(contentType) {
    if (!contentTypes.some(([value]) => value === contentType) || state.contentType === contentType) return;
    if (contentType === "mod" && getInstance()?.loader === "vanilla") return;
    state.contentType = contentType;
    state.viewMode = "discover";
    state.mods = [];
    state.results = [];
    state.offset = 0;
    state.totalHits = 0;
    state.category = "all";
    state.error = null;
    rerender();
    await Promise.all([loadMods(true), search(0)]);
  }

  async function installModpack(projectId) {
    const source = getInstance();
    const actionId = `${source?.id}:${projectId}`;
    if (!source || state.actions.has(actionId)) return;
    const item = state.results.find((result) => result.projectId === projectId);
    const action = {
      kind: "install",
      projectId: actionId,
      title: item?.title || "modpack",
      label: "Creating instance",
      detail: "Resolving the stable release and pack configuration",
      phase: "active",
      progress: 0,
      instanceId: source.id,
    };
    state.actions.set(actionId, action);
    syncActionButton(projectId);
    setStatus(`Preparing ${action.title}...`);
    try {
      const instance = await invoke("install_modrinth_modpack", {
        sourceInstanceId: source.id,
        projectId,
      });
      notify(`${instance.name} is installing in the background.`, "success");
      onInstanceCreated(instance);
    } catch (error) {
      state.error = cleanError(error);
      notify(state.error, "error");
      setStatus(`Modpack installation failed: ${state.error}`);
    } finally {
      state.actions.delete(actionId);
      if (getInstance()?.id === source.id) rerender();
    }
  }

  async function open() {
    const instance = getInstance();
    if (!instance) return;
    if (instance.loader === "vanilla" && state.contentType === "mod") state.contentType = "resourcepack";
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

  return { render, bind, open, updateProgress };
}
