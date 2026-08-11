import "./style.css";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import logo from "../../assets/logo.png";

const app = document.querySelector("#app");
let data;
let activeInstance = 0;
let selectedVersion = "";

app.innerHTML = `<div class="loading"><img src="${logo}" /><strong>WISDOM</strong><span>Loading your launcher…</span></div>`;

function initials(name = "?") { return name.slice(0, 1).toUpperCase(); }
function render() {
  const instance = data.instances[activeInstance];
  const account = data.account;
  app.innerHTML = `
    <div class="shell">
      <header>
        <div class="brand"><img src="${logo}" /><span>WISDOM</span></div>
        <div class="header-actions">
          <button class="ghost">Settings <span>⚙</span></button>
          ${account ? `<button class="account"><span class="avatar">${account.skinUrl ? `<img src="${account.skinUrl}" />` : initials(account.name)}</span>${account.name}</button><button id="logout" class="ghost icon">↪</button>` : `<button id="signin" class="primary">Sign in with Microsoft</button>`}
        </div>
      </header>
      <section class="hero">
        <p class="eyebrow">MINECRAFT JAVA EDITION</p>
        <h1>Play your way.</h1>
        <p class="sub">Clean instances. Smart downloads. Nothing in your way.</p>
        <div class="play-card">
          <div class="instance-line"><span class="dot"></span><div><strong>${instance.name}</strong><small>${instance.version}</small></div><button class="icon-button">⚙</button></div>
          <button id="play" class="play">Play <span>${selectedVersion || instance.version}</span></button>
          <label class="select-label">Minecraft version<select id="version">${data.versions.map(version => `<option ${version === (selectedVersion || instance.version) ? "selected" : ""}>${version}</option>`).join("")}</select></label>
        </div>
      </section>
      <section class="instances"><div class="section-title"><div><p class="eyebrow">YOUR LIBRARY</p><h2>Instances</h2></div><button id="new-instance" class="ghost">+ New instance</button></div>
        <div class="cards">${data.instances.map((item, index) => `<button class="instance-card ${index === activeInstance ? "selected" : ""}" data-index="${index}"><span class="card-icon">${initials(item.name)}</span><span><strong>${item.name}</strong><small>${item.version}</small></span><span class="card-more">•••</span></button>`).join("")}</div>
      </section>
      <footer id="status">Ready when you are.</footer>
    </div>`;
  document.querySelector("#version").addEventListener("change", event => selectedVersion = event.target.value);
  document.querySelectorAll(".instance-card").forEach(card => card.addEventListener("click", () => { activeInstance = Number(card.dataset.index); selectedVersion = ""; render(); }));
  document.querySelector("#signin")?.addEventListener("click", login);
  document.querySelector("#logout")?.addEventListener("click", async () => { await invoke("sign_out"); data.account = null; render(); });
  document.querySelector("#new-instance")?.addEventListener("click", createInstance);
  document.querySelector("#play")?.addEventListener("click", launch);
}
function setStatus(text) { const el = document.querySelector("#status"); if (el) el.textContent = text; }
async function login() { try { setStatus("Finish sign-in in your browser…"); data.account = await invoke("sign_in"); render(); setStatus("Signed in. Ready to play."); } catch (error) { setStatus(`Sign-in failed: ${error}`); } }
async function createInstance() { try { const instance = await invoke("create_instance", { version: selectedVersion || data.latestVersion }); data.instances.push(instance); activeInstance = data.instances.length - 1; selectedVersion = ""; render(); setStatus("New instance created."); } catch (error) { setStatus(`Could not create instance: ${error}`); } }
async function launch() { try { const instance = data.instances[activeInstance]; setStatus("Preparing Minecraft…"); await invoke("launch", { instanceId: instance.id, version: selectedVersion || instance.version }); setStatus("Minecraft started."); } catch (error) { setStatus(`Could not start: ${error}`); } }

async function init() {
  try { await listen("status", event => setStatus(event.payload)); }
  catch (error) { console.warn("Status events unavailable", error); }
  try { data = await invoke("load_launcher"); selectedVersion = data.latestVersion; render(); }
  catch (error) { app.innerHTML = `<div class="error">Could not load Wisdom: ${error}</div>`; }
}

init();
