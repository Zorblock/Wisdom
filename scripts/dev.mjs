import process from "node:process";
import * as tauri from "@tauri-apps/cli";
import { prepareRustCache } from "./rust-cache.mjs";

prepareRustCache();

try {
  await tauri.run(["dev"], "npm run dev");
} catch (error) {
  tauri.logError(error instanceof Error ? error.message : String(error));
  process.exitCode = 1;
}
