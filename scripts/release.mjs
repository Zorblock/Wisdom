import { createHash } from "node:crypto";
import {
  copyFileSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";
import * as tauri from "@tauri-apps/cli";
import { prepareRustCache } from "./rust-cache.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const rustTarget = "x86_64-pc-windows-msvc";
const releasePlatform = "windows-x64";

function readJson(relativePath) {
  return JSON.parse(readFileSync(path.join(root, relativePath), "utf8"));
}

function cargoPackageVersion() {
  const cargo = readFileSync(path.join(root, "src-tauri", "Cargo.toml"), "utf8");
  const packageStart = cargo.indexOf("[package]");
  if (packageStart === -1) return undefined;
  const packageEnd = cargo.indexOf("\n[", packageStart + "[package]".length);
  const packageSection = cargo.slice(
    packageStart + "[package]".length,
    packageEnd === -1 ? undefined : packageEnd,
  );
  return /^version\s*=\s*"([^"]+)"/m.exec(packageSection || "")?.[1];
}

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

function newestInstaller(directory) {
  const installers = readdirSync(directory)
    .filter((name) => /setup\.exe$/i.test(name))
    .map((name) => ({ name, path: path.join(directory, name) }))
    .sort((left, right) => statSync(right.path).mtimeMs - statSync(left.path).mtimeMs);
  if (!installers.length) throw new Error(`No NSIS installer was created in ${directory}`);
  return installers[0].path;
}

function assertSafeOutput(output, releaseRoot) {
  const relative = path.relative(releaseRoot, output);
  if (!relative || relative.startsWith("..") || path.isAbsolute(relative)) {
    throw new Error(`Refusing to replace unsafe release path: ${output}`);
  }
}

async function main() {
  if (process.platform !== "win32") {
    throw new Error("The current release pipeline supports Windows only.");
  }

  process.chdir(root);
  const packageJson = readJson("package.json");
  const tauriConfig = readJson(path.join("src-tauri", "tauri.conf.json"));
  const cargoVersion = cargoPackageVersion();
  const versions = [packageJson.version, tauriConfig.version, cargoVersion];
  if (versions.some((version) => !version) || new Set(versions).size !== 1) {
    throw new Error(
      `Release versions do not match (package.json=${packageJson.version}, tauri.conf.json=${tauriConfig.version}, Cargo.toml=${cargoVersion}).`,
    );
  }

  const version = packageJson.version;
  console.log(`Creating Wisdom ${version} for Windows x64...`);
  prepareRustCache();
  await tauri.run(
    ["build", "--bundles", "nsis", "--target", rustTarget],
    "npm run release",
  );

  const targetDirectory = path.join(root, "src-tauri", "target", rustTarget, "release");
  const executable = path.join(targetDirectory, "wisdom.exe");
  const installer = newestInstaller(path.join(targetDirectory, "bundle", "nsis"));
  const releaseRoot = path.join(root, "release");
  const versionRoot = path.join(releaseRoot, version);
  const output = path.join(versionRoot, releasePlatform);
  assertSafeOutput(output, releaseRoot);
  rmSync(output, { recursive: true, force: true });
  mkdirSync(output, { recursive: true });

  const files = [
    { source: executable, name: "Wisdom.exe", type: "portable" },
    { source: installer, name: "Wisdom Installer.exe", type: "installer" },
  ].map((artifact) => {
    const destination = path.join(output, artifact.name);
    copyFileSync(artifact.source, destination);
    return {
      name: artifact.name,
      type: artifact.type,
      size: statSync(destination).size,
      sha256: sha256(destination),
    };
  });

  writeFileSync(
    path.join(output, "SHA256SUMS.txt"),
    `${files.map((file) => `${file.sha256}  ${file.name}`).join("\n")}\n`,
  );
  writeFileSync(
    path.join(versionRoot, "release.json"),
    `${JSON.stringify({ product: "Wisdom", version, platform: "windows", architecture: "x64", files }, null, 2)}\n`,
  );

  console.log(`\nRelease ready: ${path.relative(root, output)}`);
  for (const file of files) {
    console.log(`  ${file.name} (${(file.size / 1024 / 1024).toFixed(1)} MB)`);
  }
}

try {
  await main();
} catch (error) {
  console.error(`\nRelease failed: ${error instanceof Error ? error.message : String(error)}`);
  process.exitCode = 1;
}
