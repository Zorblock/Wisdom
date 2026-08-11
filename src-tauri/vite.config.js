import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { defineConfig } from "vite";

const tauriDirectory = dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  root: resolve(tauriDirectory, "ui"),
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/target/**"] },
  },
  build: {
    outDir: resolve(tauriDirectory, "dist"),
    emptyOutDir: true,
    target: ["es2021", "chrome105", "safari13"],
  },
});
