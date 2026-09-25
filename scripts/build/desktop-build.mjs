import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { join, relative, resolve } from "node:path";

/**
 * `npm run desktop:build` — the provenance-bound release executable, no
 * installer (what CI builds and smoke-tests).
 *
 * `npm run desktop:release` (`--release`) — the Windows installer (item ⑩):
 * refused on a working tree with changes, before anything is built; the
 * built executable must report the same clean commit; then exactly one NSIS
 * installer is copied to its distributable name with a SHA-256 file beside
 * it, and the downloaded build inputs (NSIS, the WebView2 offline installer)
 * are recorded with their hashes.
 */
const release = process.argv.includes("--release");
const root = resolve(import.meta.dirname, "..", "..");

function git(...args) {
  const result = spawnSync("git", args, { encoding: "utf8", cwd: root });
  if (result.status !== 0) {
    throw new Error(result.stderr.trim() || `git ${args.join(" ")} failed`);
  }
  return result.stdout.trim();
}

function sha256(file) {
  return createHash("sha256").update(readFileSync(file)).digest("hex");
}

const commit = git("rev-parse", "HEAD");
const dirty = git("status", "--porcelain").length > 0;
if (release && dirty) {
  console.error(
    "desktop:release refuses a working tree with changes: commit or remove them first.",
  );
  process.exit(1);
}
const npmCli = process.env.npm_execpath;
if (!npmCli) throw new Error("npm_execpath is required for a provenance-bound desktop build");
const result = spawnSync(
  process.execPath,
  [
    npmCli,
    "run",
    "tauri:build",
    "--workspace",
    "@pmc/desktop",
    ...(release ? [] : ["--", "--no-bundle"]),
  ],
  {
    stdio: "inherit",
    cwd: root,
    env: {
      ...process.env,
      PMC_BUILD_COMMIT: commit,
      PMC_BUILD_DIRTY: dirty ? "true" : "false",
    },
  },
);
if (result.error) throw result.error;
if (result.status !== 0 || !release) process.exit(result.status ?? 1);

// ---- the installer ------------------------------------------------------

const config = JSON.parse(
  readFileSync(join(root, "apps/desktop/src-tauri/tauri.conf.json"), "utf8"),
);
const version = config.version;
const releaseDir = join(root, "target/release");

// The executable inside the installer is the one just built: it must name
// this commit and a clean tree.
const metadataRun = spawnSync(join(releaseDir, "pmc-desktop.exe"), ["--pmc-build-metadata"], {
  encoding: "utf8",
});
const metadata = JSON.parse(metadataRun.stdout || "{}");
if (metadata.commit !== commit || metadata.dirty !== "false") {
  console.error(
    `desktop:release: the built executable reports ${JSON.stringify(metadata)}, not ${commit} clean.`,
  );
  process.exit(1);
}

const nsisDir = join(releaseDir, "bundle/nsis");
const installers = existsSync(nsisDir)
  ? readdirSync(nsisDir).filter(
      (name) => name.endsWith("-setup.exe") && name.includes(`_${version}_`),
    )
  : [];
if (installers.length !== 1) {
  console.error(
    `desktop:release: expected exactly one ${version} NSIS installer in ${relative(root, nsisDir)}, found ${installers.length}.`,
  );
  process.exit(1);
}

// What the build downloaded and embedded: the WebView2 offline installer
// the rendered script names, and the NSIS compiler and Tauri plugin it ran.
// The installer's own checksum does not say which Microsoft payload is
// inside it; this record does. Each one must exist, or nothing is released.
const rendered = readFileSync(join(releaseDir, "nsis/x64/installer.nsi"), "utf8");
const webview2 = /!define WEBVIEW2INSTALLERPATH "([^"]+)"/.exec(rendered)?.[1];
const cache = process.env.LOCALAPPDATA ? join(process.env.LOCALAPPDATA, "tauri") : undefined;
const required = [
  ["WebView2 offline installer", webview2],
  ["NSIS compiler", cache && join(cache, "NSIS/Bin/makensis.exe")],
  [
    "nsis_tauri_utils plugin",
    cache && join(cache, "NSIS/Plugins/x86-unicode/additional/nsis_tauri_utils.dll"),
  ],
];
const inputs = [];
for (const [role, path] of required) {
  if (path === undefined || !existsSync(path)) {
    console.error(`desktop:release: the ${role} the build used cannot be found (${String(path)}).`);
    process.exit(1);
  }
  inputs.push({
    role,
    file:
      cache !== undefined && path.startsWith(cache)
        ? relative(cache, path).replaceAll("\\", "/")
        : path,
    lengthBytes: statSync(path).size,
    sha256: sha256(path),
  });
}
const outDir = join(root, "target/release-artifacts");
rmSync(outDir, { recursive: true, force: true });
mkdirSync(outDir, { recursive: true });
const name = `product-mission-control_${version}_windows-x86_64_nsis-setup.exe`;
const installer = join(outDir, name);
copyFileSync(join(nsisDir, installers[0]), installer);
const digest = sha256(installer);
writeFileSync(`${installer}.sha256`, `${digest}  ${name}\n`, "utf8");

writeFileSync(
  join(outDir, "build-inputs.json"),
  `${JSON.stringify({ commit, version, installer: { name, sha256: digest }, inputs }, null, 2)}\n`,
  "utf8",
);

console.log(`${relative(root, installer)}\n${digest}`);
