import { spawnSync } from "node:child_process";
import path from "node:path";
import process from "node:process";
import * as tauri from "@tauri-apps/cli";

function prepareRustCache() {
  const wrapper = process.env.RUSTC_WRAPPER?.trim();
  if (!wrapper || !path.basename(wrapper).toLowerCase().startsWith("sccache")) return;

  process.stdout.write("Checking Rust compiler cache... ");
  const result = spawnSync(wrapper, ["--show-stats"], {
    encoding: "utf8",
    stdio: "ignore",
    timeout: 5000,
    windowsHide: true,
  });

  if (!result.error && result.status === 0) {
    process.stdout.write("ready\n");
    return;
  }

  // A stale sccache server must never prevent the launcher from starting.
  // Cargo still uses its normal incremental cache for this dev session.
  delete process.env.RUSTC_WRAPPER;
  process.stdout.write("unavailable, continuing with Cargo cache\n");
}

prepareRustCache();

try {
  await tauri.run(["dev"], "npm run dev");
} catch (error) {
  tauri.logError(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}
