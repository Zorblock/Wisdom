import { spawnSync } from "node:child_process";
import path from "node:path";
import process from "node:process";

export function prepareRustCache() {
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

  // A stale cache server should never prevent development or a release build.
  // Cargo's own incremental cache remains available for this process.
  delete process.env.RUSTC_WRAPPER;
  process.stdout.write("unavailable, continuing with Cargo cache\n");
}
