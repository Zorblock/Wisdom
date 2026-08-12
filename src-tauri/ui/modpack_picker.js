export function createModpackPicker({ invoke, icon, escapeHtml, cleanError, customSelect, getVersionOptions, getLatestVersion, notify, rerender, setStatus, onCreated }) {
  const state = {
    gameVersion: "",
    releaseChannel: "release",
    query: "",
    searchIndex: "downloads",
    offset: 0,
    totalHits: 0,
    results: [],
    searching: false,
    resolvingId: null,
    confirmation: null,
    error: null,
    request: 0,
  };
  let searchTimer;

  const channelOptions = [
    { value: "release", label: "Stable" },
    { value: "beta", label: "Beta" },
    { value: "alpha", label: "Alpha" },
  ];
  const sortOptions = [
    { value: "downloads", label: "Downloads" },
    { value: "relevance", label: "Relevance" },
    { value: "updated", label: "Recently updated" },
    { value: "newest", label: "Newest" },
    { value: "follows", label: "Followers" },
  ];

  function projectIcon(url, title) {
    try {
      const parsed = new URL(String(url || ""));
      if (parsed.protocol === "https:" && (parsed.hostname === "cdn.modrinth.com" || parsed.hostname.endsWith(".modrinth.com"))) {
        return `<img src="${escapeHtml(parsed.href)}" alt="" loading="lazy" />`;
      }
    } catch {
      // Fall through to the local icon.
    }
    return `<span aria-label="${escapeHtml(title)}">${icon("instance")}</span>`;
  }

  function renderResults() {
    if (state.searching && !state.results.length) return `<div class="modpack-picker-empty"><span class="spinner"></span>Searching Modrinth...</div>`;
    if (!state.results.length) return `<div class="modpack-picker-empty">No compatible modpacks found.</div>`;
    return state.results.map((item) => {
      const resolving = state.resolvingId === item.projectId;
      return `<article class="modpack-picker-result">
        <span class="mod-project-icon">${projectIcon(item.iconUrl, item.title)}</span>
        <span><strong>${escapeHtml(item.title)}</strong><small>${escapeHtml(item.description)}</small><em>by ${escapeHtml(item.author)}</em></span>
        <button type="button" class="button ${resolving ? "secondary" : "primary"}" data-create-modpack="${escapeHtml(item.projectId)}" ${state.resolvingId ? "disabled" : ""}>${resolving ? `<span class="spinner"></span>Checking` : `${icon("plus")}Create`}</button>
      </article>`;
    }).join("");
  }

  function renderConfirmation() {
    const pending = state.confirmation;
    if (!pending) return "";
    const channel = pending.choice.versionType === "beta" ? "Beta" : "Alpha";
    return `<div class="modpack-confirmation" role="alertdialog" aria-modal="true" aria-labelledby="modpack-prerelease-title">
      <div class="danger-mark">${icon("warning")}</div>
      <h3 id="modpack-prerelease-title">${channel} version available</h3>
      <p>No stable version is available for Minecraft ${escapeHtml(state.gameVersion)}. Create the instance with ${escapeHtml(pending.choice.versionNumber)} (${channel})?</p>
      <div class="modal-actions"><button id="cancel-modpack-prerelease" type="button" class="button secondary">Cancel</button><button id="confirm-modpack-prerelease" type="button" class="button primary">Use ${channel}</button></div>
    </div>`;
  }

  function render() {
    return `<div class="modpack-create-fields">
      ${state.error ? `<div class="mods-error">${icon("warning")}<span>${escapeHtml(state.error)}</span></div>` : ""}
      <div class="modpack-picker-controls">
        <div class="field"><span>Minecraft version</span>${customSelect("modpack-game-version", state.gameVersion, getVersionOptions(state.gameVersion), "Minecraft version")}</div>
        <div class="field"><span>Release channel</span>${customSelect("modpack-release-channel", state.releaseChannel, channelOptions, "Release channel")}</div>
      </div>
      <div class="modpack-picker-toolbar">
        <label class="mods-search"><i class="fa-solid fa-magnifying-glass" aria-hidden="true"></i><input id="modpack-search" value="${escapeHtml(state.query)}" placeholder="Search modpacks" autocomplete="off" spellcheck="false" /></label>
        <div class="compact-select">${customSelect("modpack-sort", state.searchIndex, sortOptions, "Sort modpacks")}</div>
      </div>
      <div class="modpack-picker-results ${state.searching ? "is-loading" : ""}">${renderResults()}</div>
      ${state.totalHits > 24 ? `<div class="mods-pagination"><button id="modpack-previous" type="button" class="button secondary" ${state.offset === 0 || state.searching ? "disabled" : ""}>Previous</button><span>${state.offset + 1}&ndash;${Math.min(state.offset + 24, state.totalHits)} of ${state.totalHits}</span><button id="modpack-next" type="button" class="button secondary" ${state.offset + 24 >= state.totalHits || state.searching ? "disabled" : ""}>Next</button></div>` : ""}
      ${renderConfirmation()}
    </div>`;
  }

  function bind() {
    document.querySelector("#modpack-game-version")?.addEventListener("change", (event) => {
      state.gameVersion = event.target.value;
      void search(0);
    });
    document.querySelector("#modpack-release-channel")?.addEventListener("change", (event) => { state.releaseChannel = event.target.value; });
    document.querySelector("#modpack-sort")?.addEventListener("change", (event) => { state.searchIndex = event.target.value; void search(0); });
    document.querySelector("#modpack-search")?.addEventListener("input", (event) => {
      state.query = event.target.value;
      clearTimeout(searchTimer);
      searchTimer = setTimeout(() => search(0), 280);
    });
    document.querySelector("#modpack-previous")?.addEventListener("click", () => search(Math.max(0, state.offset - 24)));
    document.querySelector("#modpack-next")?.addEventListener("click", () => search(state.offset + 24));
    document.querySelectorAll("[data-create-modpack]").forEach((button) => button.addEventListener("click", () => resolve(button.dataset.createModpack)));
    document.querySelector("#cancel-modpack-prerelease")?.addEventListener("click", () => { state.confirmation = null; rerender(); });
    document.querySelector("#confirm-modpack-prerelease")?.addEventListener("click", () => {
      const pending = state.confirmation;
      state.confirmation = null;
      if (pending) void install(pending.projectId, pending.choice);
    });
  }

  async function search(offset = 0) {
    if (!state.gameVersion) return;
    const request = ++state.request;
    state.offset = offset;
    state.searching = true;
    state.error = null;
    rerender();
    try {
      const result = await invoke("search_modrinth_modpacks", {
        gameVersion: state.gameVersion,
        query: state.query,
        index: state.searchIndex,
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

  async function resolve(projectId) {
    if (state.resolvingId) return;
    state.resolvingId = projectId;
    state.error = null;
    setStatus("Checking compatible modpack versions...");
    rerender();
    try {
      const choice = await invoke("resolve_modrinth_modpack", {
        gameVersion: state.gameVersion,
        projectId,
        releaseChannel: state.releaseChannel,
      });
      state.resolvingId = null;
      if (choice.requiresConfirmation) {
        state.confirmation = { projectId, choice };
        rerender();
      } else {
        await install(projectId, choice);
      }
    } catch (error) {
      state.resolvingId = null;
      state.error = cleanError(error);
      notify(state.error, "error");
      setStatus(`Could not create modpack instance: ${state.error}`);
      rerender();
    }
  }

  async function install(projectId, choice) {
    state.resolvingId = projectId;
    setStatus(`Creating ${choice.title}...`);
    rerender();
    try {
      const instance = await invoke("install_modrinth_modpack", {
        gameVersion: state.gameVersion,
        projectId,
        versionId: choice.versionId,
      });
      notify(`${instance.name} is installing in the background.`, "success");
      onCreated(instance);
    } catch (error) {
      state.error = cleanError(error);
      notify(state.error, "error");
      setStatus(`Could not create modpack instance: ${state.error}`);
    } finally {
      state.resolvingId = null;
      rerender();
    }
  }

  function open() {
    if (!state.gameVersion) state.gameVersion = getLatestVersion();
    state.error = null;
    state.confirmation = null;
    void search(0);
  }

  function reset() {
    state.gameVersion = getLatestVersion();
    state.releaseChannel = "release";
    state.query = "";
    state.searchIndex = "downloads";
    state.offset = 0;
    state.totalHits = 0;
    state.results = [];
    state.searching = false;
    state.resolvingId = null;
    state.confirmation = null;
    state.error = null;
    state.request += 1;
  }

  return { render, bind, open, reset };
}
