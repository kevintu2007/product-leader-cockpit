import { createHash } from "node:crypto";
import { globSync, readFileSync } from "node:fs";
import { dirname, extname, relative, resolve, sep } from "node:path";

const root = resolve(import.meta.dirname, "..", "..");
const sourceFiles = globSync("{apps,crates}/**/*", {
  cwd: root,
  exclude: ["**/dist/**", "**/target/**", "**/node_modules/**"],
});

const allowedCapabilityKeys = new Set([
  "$schema",
  "identifier",
  "description",
  "windows",
  "permissions",
]);
const allowedCapabilityPermissions = new Set();
const allowedTauriRootKeys = new Set([
  "$schema",
  "productName",
  "version",
  "identifier",
  "build",
  "app",
  "bundle",
]);
const allowedTauriBuildKeys = new Set([
  "beforeDevCommand",
  "devUrl",
  "beforeBuildCommand",
  "frontendDist",
]);
const allowedTauriAppKeys = new Set(["security", "windows"]);
const allowedTauriSecurityKeys = new Set(["csp"]);
// The installer (item ⑩, product owner 2026-09-23): NSIS only, for the
// current user, WebView2 embedded in the installer rather than downloaded
// by it (and no minimum version that could trigger an update), no updater
// artifacts, and the vendored template whose uninstaller removes no
// application data. Every value is pinned, not only the keys.
const expectedTauriBundle = {
  active: true,
  targets: ["nsis"],
  createUpdaterArtifacts: false,
  publisher: "Product Mission Control",
  icon: ["icons/icon.ico"],
  windows: {
    webviewInstallMode: { type: "offlineInstaller", silent: true },
    nsis: {
      template: "nsis/installer.nsi",
      installMode: "currentUser",
      languages: ["English", "TradChinese", "SimpChinese", "Japanese", "Korean", "Spanish"],
      displayLanguageSelector: false,
    },
  },
};
const allowedTauriWindowKeys = new Set([
  "title",
  "width",
  "height",
  "minWidth",
  "minHeight",
  "resizable",
  "url",
  "additionalBrowserArgs",
]);
const expectedAdditionalBrowserArgs =
  "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --disable-background-networking --disable-component-update --disable-domain-reliability --disable-quic --disable-sync --dns-over-https-mode=off --metrics-recording-only --no-pings";
const allowedConnectSources = new Set(["'self'", "ipc:", "http://ipc.localhost"]);
const expectedCspDirectives = new Map([
  ["default-src", ["'self'"]],
  ["base-uri", ["'none'"]],
  ["form-action", ["'none'"]],
  ["object-src", ["'none'"]],
  ["style-src", ["'self'", "'unsafe-inline'"]],
  ["img-src", ["'self'", "data:"]],
  ["connect-src", [...allowedConnectSources]],
]);
const reviewedPackageDependencies = new Map([
  [
    "package.json",
    new Map([
      ["dependencies", new Set()],
      [
        "devDependencies",
        new Set([
          // Tier 1 presentation-contract end-to-end runner, authorized by
          // the product owner on 2026-09-05.
          "@playwright/test",
          "@testing-library/jest-dom",
          "@testing-library/react",
          "@testing-library/user-event",
          "@types/node",
          "@types/react",
          "@types/react-dom",
          "@vitejs/plugin-react",
          "eslint",
          "eslint-plugin-jsx-a11y",
          "eslint-plugin-react-hooks",
          "eslint-plugin-react-refresh",
          "jsdom",
          "prettier",
          "typescript",
          "typescript-eslint",
          "vite",
          "vitest",
        ]),
      ],
    ]),
  ],
  [
    "apps/desktop/package.json",
    new Map([
      ["dependencies", new Set(["@tauri-apps/api", "react", "react-dom"])],
      ["devDependencies", new Set(["@tauri-apps/cli"])],
    ]),
  ],
]);
const reviewedCargoDependencies = new Map([
  [
    "apps/desktop/src-tauri/Cargo.toml",
    new Map([
      // The three workspace crates and serde arrived with the real Ledger
      // IPC wiring (8f68f42). Reviewed and approved by the product owner
      // on 2026-09-05: all three are first-party crates in this workspace,
      // and serde is already reviewed for pmc-platform.
      [
        "dependencies",
        new Set([
          "tauri",
          // Added with the Executive Cockpit route query, reviewed and
          // approved by the product owner.
          "pmc-application",
          "pmc-domain",
          "pmc-ledger",
          "pmc-platform",
          "serde",
          // Host-owned native dialogs (ADR 0011, product owner 2026-09-19 and
          // 2026-09-21). Windows target only, default features off; callable
          // only from the reviewed block in native_dialogs.rs.
          "rfd",
        ]),
      ],
      ["build-dependencies", new Set(["tauri-build"])],
    ]),
  ],
  [
    "crates/pmc-application/Cargo.toml",
    new Map([
      [
        "dependencies",
        new Set([
          "pmc-domain",
          "pmc-knowledge",
          "pmc-ledger",
          "pmc-platform",
          // The sample workspace's seed and its manifest moved here from
          // tools/pmc-seed (item ⑨; product owner 2026-09-23). All three are
          // already reviewed for other workspace crates.
          "serde",
          "serde_json",
          "sha2",
        ]),
      ],
    ]),
  ],
  ["crates/pmc-domain/Cargo.toml", new Map([["dependencies", new Set(["sha2"])]])],
  [
    "crates/pmc-knowledge/Cargo.toml",
    new Map([
      // sha2 became a real dependency when the pmc.projection/v1 generator
      // started hashing published artifacts (a5600ea). Reviewed and
      // approved by the product owner on 2026-09-05; sha2 is already
      // reviewed for pmc-domain and pmc-ledger.
      ["dependencies", new Set(["pmc-domain", "pmc-platform", "sha2"])],
      ["dev-dependencies", new Set(["sha2"])],
    ]),
  ],
  [
    "crates/pmc-ledger/Cargo.toml",
    new Map([["dependencies", new Set(["pmc-domain", "rusqlite", "sha2"])]]),
  ],
  [
    "crates/pmc-platform/Cargo.toml",
    new Map([
      [
        "dependencies",
        new Set([
          // The encrypted Operational Backup archive of ADR 0010, accepted
          // by the product owner on 2026-09-21: age v1 (passphrase, no SSH,
          // plugin or armor), a deterministic tar, zstd compression.
          "age",
          "tar",
          "zstd",
          "atomic-write-file",
          // The recovery passphrase generator (ADR 0010 §7): the OS random
          // source. Already in the tree through age; reviewed 2026-09-21.
          "getrandom",
          "icu_locale_core",
          "jiff",
          "keyring-core",
          "serde",
          "serde_json",
          "sha2",
          "windows-native-keyring-store",
          // The one approved `unsafe` call (windows_names.rs, product owner
          // 2026-09-23): Windows' own name comparison, CompareStringOrdinal.
          "windows-sys",
          "zeroize",
        ]),
      ],
    ]),
  ],
]);

function repositoryLabel(file) {
  return relative(root, file).replaceAll("\\", "/");
}

const forbiddenCapability =
  /^(?:fs|filesystem|shell|http|https|network|sql|uri|opener|process)(?::|$)/i;
const forbiddenDependencyClass = (name) => {
  const normalized = String(name).toLowerCase();
  const networkPackages = new Set([
    "reqwest",
    "ureq",
    "hyper",
    "attohttpc",
    "curl",
    "surf",
    "isahc",
    "awc",
    "axios",
    "got",
    "node-fetch",
    "undici",
  ]);
  if (networkPackages.has(normalized)) return "network-client";
  const classes = [
    "filesystem",
    "fs",
    "shell",
    "https",
    "http",
    "network",
    "sql",
    "uri",
    "opener",
    "process",
  ];
  return classes.find((className) =>
    new RegExp(`(?:^|[-_./@])${className}(?:$|[-_./])`, "i").test(normalized),
  );
};

function objectKeysOutside(value, allowed, path) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    return [`${path}: expected an object`];
  }
  return Object.keys(value)
    .filter((key) => !allowed.has(key))
    .map((key) => `${path}.${key}: unsupported key`);
}

function parseJson(file, label) {
  try {
    return { value: JSON.parse(readFileSync(file, "utf8")), failures: [] };
  } catch (error) {
    return {
      value: undefined,
      failures: [
        `${label}: invalid JSON (${error instanceof Error ? error.message : String(error)})`,
      ],
    };
  }
}

function inspectCapability(file) {
  const label = relative(root, file);
  const parsed = parseJson(file, label);
  if (parsed.value === undefined) return parsed.failures;
  const capability = parsed.value;
  const failures = objectKeysOutside(capability, allowedCapabilityKeys, label);
  if (capability.identifier !== "default") failures.push(`${label}.identifier: must be default`);
  if (
    !Array.isArray(capability.windows) ||
    capability.windows.some((window) => window !== "main")
  ) {
    failures.push(`${label}.windows: only the main window is allowed`);
  }
  if (!Array.isArray(capability.permissions)) {
    failures.push(`${label}.permissions: expected an array`);
  } else {
    for (const permission of capability.permissions) {
      const identifier = typeof permission === "string" ? permission : permission?.identifier;
      if (typeof identifier !== "string") {
        failures.push(`${label}.permissions: permission identifiers must be strings`);
      } else if (
        !allowedCapabilityPermissions.has(identifier) ||
        forbiddenCapability.test(identifier)
      ) {
        failures.push(
          `${label}.permissions.${identifier}: permission is not in the static-shell allowlist`,
        );
      }
    }
  }
  return failures;
}

function inspectTauriConfig(file) {
  const label = relative(root, file);
  const parsed = parseJson(file, label);
  if (parsed.value === undefined) return parsed.failures;
  const config = parsed.value;
  const failures = objectKeysOutside(config, allowedTauriRootKeys, label);
  if (config.build !== undefined) {
    failures.push(...objectKeysOutside(config.build, allowedTauriBuildKeys, `${label}.build`));
    if (
      typeof config.build.devUrl !== "string" ||
      !/^https?:\/\/(?:127\.0\.0\.1|localhost)(?::\d+)?\/?$/.test(config.build.devUrl)
    ) {
      failures.push(`${label}.build.devUrl: only a loopback development URL is allowed`);
    }
    if (
      typeof config.build.frontendDist !== "string" ||
      config.build.frontendDist.includes("://")
    ) {
      failures.push(`${label}.build.frontendDist: must be a relative local path`);
    }
  }
  if (config.app === undefined) {
    failures.push(`${label}.app: static shell requires an app configuration`);
  } else {
    failures.push(...objectKeysOutside(config.app, allowedTauriAppKeys, `${label}.app`));
    if (!Array.isArray(config.app.windows) || config.app.windows.length === 0) {
      failures.push(`${label}.app.windows: static shell requires at least one window`);
    } else {
      config.app.windows.forEach((window, index) => {
        const windowLabel = `${label}.app.windows[${index}]`;
        failures.push(...objectKeysOutside(window, allowedTauriWindowKeys, windowLabel));
        if (window?.url !== undefined) {
          if (
            typeof window.url !== "string" ||
            !/^https?:\/\/(?:127\.0\.0\.1|localhost)(?::\d+)?\/?$/.test(window.url)
          ) {
            failures.push(`${windowLabel}.url: remote window URLs are not allowed`);
          }
        }
        if (window?.additionalBrowserArgs !== expectedAdditionalBrowserArgs) {
          failures.push(
            `${windowLabel}.additionalBrowserArgs: static shell requires the exact no-background-networking argument set`,
          );
        }
      });
    }
    if (config.app.security === undefined) {
      failures.push(`${label}.app.security: static shell requires security configuration`);
    } else {
      failures.push(
        ...objectKeysOutside(
          config.app.security,
          allowedTauriSecurityKeys,
          `${label}.app.security`,
        ),
      );
      const csp = config.app.security.csp;
      if (typeof csp !== "string") {
        failures.push(`${label}.app.security.csp: expected a string`);
      } else {
        const directives = new Map();
        for (const rawDirective of csp.split(";")) {
          const parts = rawDirective.trim().split(/\s+/).filter(Boolean);
          if (parts.length === 0) continue;
          const [name, ...sources] = parts;
          if (!expectedCspDirectives.has(name)) {
            failures.push(`${label}.app.security.csp: directive ${name} is not allowed`);
          } else if (directives.has(name)) {
            failures.push(`${label}.app.security.csp: duplicate directive ${name}`);
          }
          directives.set(name, sources);
        }
        for (const [name, expectedSources] of expectedCspDirectives) {
          const sources = directives.get(name);
          if (!sources) {
            failures.push(`${label}.app.security.csp: required ${name} directive is missing`);
            continue;
          }
          if (name === "connect-src") {
            for (const source of sources) {
              if (!allowedConnectSources.has(source)) {
                failures.push(`${label}.app.security.csp: network source ${source} is not allowed`);
              }
            }
          }
          if (
            sources.length !== expectedSources.length ||
            sources.some((source) => !expectedSources.includes(source))
          ) {
            failures.push(
              `${label}.app.security.csp: ${name} directive does not match the static-shell policy`,
            );
          }
        }
      }
    }
  }
  failures.push(...inspectTauriBundle(config.bundle, file, `${label}.bundle`));
  return failures;
}

/** Every path where `actual` differs from `expected`, named for the report. */
function jsonDifferences(actual, expected, path) {
  if (
    expected !== null &&
    typeof expected === "object" &&
    actual !== null &&
    typeof actual === "object" &&
    Array.isArray(expected) === Array.isArray(actual)
  ) {
    if (Array.isArray(expected)) {
      return JSON.stringify(actual) === JSON.stringify(expected)
        ? []
        : [`${path}: must be exactly ${JSON.stringify(expected)}`];
    }
    const differences = [];
    for (const key of Object.keys(actual)) {
      if (!(key in expected)) differences.push(`${path}.${key}: unsupported key`);
    }
    for (const [key, value] of Object.entries(expected)) {
      differences.push(...jsonDifferences(actual[key], value, `${path}.${key}`));
    }
    return differences;
  }
  return actual === expected ? [] : [`${path}: must be exactly ${JSON.stringify(expected)}`];
}

function inspectTauriBundle(bundle, configFile, label) {
  const failures = jsonDifferences(bundle, expectedTauriBundle, label);
  if (failures.length > 0) return failures;
  return inspectNsisTemplate(
    configFile,
    bundle.windows.nsis.template,
    `${label}.windows.nsis.template`,
  );
}

// The reviewed vendored template, whole (line endings normalised). Any edit
// — an added !include, a macro, a new deletion — fails until this value is
// changed on purpose, in a reviewed commit, after the template was compared
// with upstream again. A Tauri CLI upgrade needs the same: the template names
// the CLI version it came from.
const reviewedNsisTemplateSha256 =
  "a8f10e81cdb76a3ce6a3c3a2d1d254e0c385987ed4932621bb3b9a7f8cdca3b2";

/**
 * The vendored NSIS template: it stays inside src-tauri, it is exactly the
 * reviewed file, it was taken from the Tauri CLI version the workspace builds
 * with, and nothing in it removes an application data folder or offers to —
 * uninstalling never deletes the person's data (item ⑩). The deletion
 * patterns are a second line behind the pinned digest.
 */
function inspectNsisTemplate(configFile, templatePath, label) {
  const base = dirname(configFile);
  const template = resolve(base, templatePath);
  if (!template.startsWith(base + sep)) {
    return [`${label}: must stay inside ${relative(root, base)}`];
  }
  let source;
  try {
    source = readFileSync(template, "utf8");
  } catch {
    return [`${label}: ${relative(root, template)} cannot be read`];
  }
  const failures = [];
  const digest = createHash("sha256").update(source.replace(/\r\n/g, "\n")).digest("hex");
  if (digest !== reviewedNsisTemplateSha256) {
    failures.push(
      `${label}: ${relative(root, template)} is not the reviewed template (SHA-256 ${digest}); review the change and update reviewedNsisTemplateSha256`,
    );
  }
  let cliVersion;
  try {
    cliVersion = JSON.parse(readFileSync(resolve(dirname(base), "package.json"), "utf8"))
      .devDependencies?.["@tauri-apps/cli"];
  } catch {
    cliVersion = undefined;
  }
  if (typeof cliVersion !== "string" || !source.includes(`tag tauri-cli-v${cliVersion}`)) {
    failures.push(
      `${label}: vendored from a different Tauri CLI than the pinned @tauri-apps/cli ${String(cliVersion)}; compare it with the new upstream template`,
    );
  }
  const deletesData = [
    [/DeleteAppData/i, "offers to delete application data"],
    [/\$\(deleteAppData\)/i, "offers to delete application data"],
    [
      /RMDir[^\n]*\$\{?(?:APPDATA|LOCALAPPDATA)\}?/i,
      "removes a folder under the application data roots",
    ],
    [
      /Delete[^\n]*\$\{?(?:APPDATA|LOCALAPPDATA)\}?[\\/]/i,
      "deletes a file under the application data roots",
    ],
    [/ProductMissionControlDesktop/i, "names PMC's data folder"],
  ];
  for (const [pattern, reason] of deletesData) {
    if (pattern.test(source)) failures.push(`${label}: the uninstaller ${reason}`);
  }
  return failures;
}

function inspectPackageManifest(file, reviewedLabel) {
  const label = reviewedLabel ?? repositoryLabel(file);
  const parsed = parseJson(file, label);
  if (parsed.value === undefined) return parsed.failures;
  const manifest = parsed.value;
  const failures = [];
  for (const section of [
    "dependencies",
    "devDependencies",
    "optionalDependencies",
    "peerDependencies",
  ]) {
    const dependencies = manifest[section];
    if (!dependencies || typeof dependencies !== "object" || Array.isArray(dependencies)) continue;
    for (const [name, spec] of Object.entries(dependencies)) {
      const actualNames = [name];
      if (typeof spec === "string" && /^npm:/i.test(spec)) {
        const aliasedName = spec.slice(4);
        const versionSeparator = aliasedName.startsWith("@")
          ? aliasedName.indexOf("@", 1)
          : aliasedName.indexOf("@");
        actualNames.push(
          versionSeparator === -1 ? aliasedName : aliasedName.slice(0, versionSeparator),
        );
      }
      const forbiddenClass = actualNames.map(forbiddenDependencyClass).find(Boolean);
      if (forbiddenClass) {
        failures.push(`${label}.${section}.${name}: forbidden native capability dependency`);
      }
      const reviewed = reviewedPackageDependencies.get(label)?.get(section);
      if (!reviewed?.has(name) || actualNames.at(-1) !== name) {
        failures.push(
          `${label}.${section}.${name}: dependency is not in the exact reviewed manifest`,
        );
      }
    }
  }
  return failures;
}

function inspectCargoManifest(file) {
  const label = repositoryLabel(file);
  const failures = [];
  let section = "";
  let dependencyTable = undefined;
  for (const [lineNumber, rawLine] of readFileSync(file, "utf8").split(/\r?\n/).entries()) {
    const line = rawLine.replace(/#.*/, "").trim();
    const sectionMatch = /^\[([^\]]+)\]$/.exec(line);
    if (sectionMatch) {
      section = sectionMatch[1];
      // Cargo supports both `[dependencies] name = ...` and a table-form
      // alias (`[dependencies.alias] package = "..."`). Target-specific
      // dependency sections have the same suffix, so parse the section path
      // instead of relying on one-line dependency declarations.
      const dependencyTableMatch =
        /(?:^|\.)((?:dev-|build-)?dependencies)(?:\.(?:([A-Za-z0-9_-]+)|["']([^"']+)["']))?$/.exec(
          section,
        );
      dependencyTable = dependencyTableMatch
        ? {
            kind: dependencyTableMatch[1],
            alias: dependencyTableMatch[2] ?? dependencyTableMatch[3],
          }
        : undefined;
      continue;
    }
    if (!dependencyTable || !line.includes("=")) continue;
    const dependencyName = dependencyTable.alias ?? /^([A-Za-z0-9_-]+)\s*=/.exec(line)?.[1];
    const packageAlias = /\bpackage\s*=\s*["']([^"']+)["']/.exec(line)?.[1];
    const forbiddenClass =
      forbiddenDependencyClass(dependencyName) ?? forbiddenDependencyClass(packageAlias);
    if (forbiddenClass) {
      failures.push(
        `${label}:${lineNumber + 1}.${dependencyName}: forbidden native capability dependency`,
      );
    }
    const canonicalName = packageAlias ?? dependencyName;
    const reviewed = reviewedCargoDependencies.get(label)?.get(dependencyTable.kind);
    if (canonicalName && !reviewed?.has(canonicalName)) {
      failures.push(
        `${label}:${lineNumber + 1}.${dependencyName}: dependency is not in the exact reviewed manifest`,
      );
    }
  }
  return failures;
}

/** Any use of the native dialog crate (ADR 0011); only the reviewed block may. */
const NATIVE_DIALOG_API = /\brfd\s*::|\buse\s+rfd\b|\bextern\s+crate\s+rfd\b/;

/**
 * Host files allowed to hold a filesystem path type at all (ADR 0011). A path
 * in the host exists only where a dialog returns one; everywhere else in
 * `src-tauri`, including through aliases, enums, tuple structs or manual
 * serialization, a path type is refused outright.
 */
const hostFilesAllowedPathTypes = new Set(["apps/desktop/src-tauri/src/native_dialogs.rs"]);

/**
 * No path crosses the IPC boundary (ADR 0011). Three rules, over code with
 * comments removed: a path type (`PathBuf`, `Path`, `OsString`, `OsStr`,
 * `std::path`) appears only in the allowed files; no Tauri command argument,
 * whatever its attribute's arguments, is named like a path; no field of a
 * serialized or deserialized struct is named like a path. Returns one finding
 * per offending item.
 */
function pathBearingIpc(content, label) {
  const findings = [];
  const cut = content.search(/^\s*#\[cfg\(test\)\]\s*\n(?:\s*#\[[^\n]*\]\s*\n)*\s*mod /m);
  const production = (cut === -1 ? content : content.slice(0, cut))
    .replace(/\/\*[\s\S]*?\*\//g, "")
    .replace(/\/\/[^\n]*/g, "");
  const pathName = /\b\w*path\w*\s*:/i;
  if (
    !hostFilesAllowedPathTypes.has(label.replaceAll("\\", "/")) &&
    /\b(?:PathBuf|Path|OsString|OsStr)\b|\bstd\s*::\s*path\b/.test(production)
  ) {
    findings.push(`${label}: path type in host code`);
  }
  for (const match of production.matchAll(
    /#\s*\[\s*tauri::command\b[^\]]*\][\s\S]{0,400}?\bfn\s+([a-z_][a-z0-9_]*)\s*(?:<[^>]*>)?\s*\(([\s\S]*?)\)\s*(?:->|\{)/g,
  )) {
    if (pathName.test(match[2])) {
      findings.push(`${label}: path-bearing IPC argument in ${match[1]}`);
    }
  }
  for (const match of production.matchAll(
    /#\s*\[\s*derive\s*\(([^)]*)\)\s*\][\s\S]{0,300}?\b(?:struct|enum)\s+(\w+)[^{;]*\{([\s\S]*?)\n\}/g,
  )) {
    if (/\b(?:Serialize|Deserialize)\b/.test(match[1]) && pathName.test(match[3])) {
      findings.push(`${label}: path-bearing IPC field in ${match[2]}`);
    }
  }
  return findings;
}

const FORBIDDEN_NATIVE_API =
  /\.plugin\s*\(|tauri_plugin_|tauri::api::|std::process|\buse\s+std\s*::\s*\{[^;]*\bprocess\b|Command::new\s*\(/s;

// Every file that legitimately spawns a native process gets a named,
// marker-delimited, SHA-256-pinned block instead of a blanket exemption:
// changing the reviewed code even by one byte changes its hash and fails
// this check again, so review is tied to exact content, not merely to a
// filename. `importStatement` is optional -- present only for files that
// pull the process API in through a standalone `use` line outside the
// marked block itself (a fully qualified `std::process::Command::new(...)`
// inside the block needs no separate import to strip).
/**
 * The exact Tauri IPC command surface this application is reviewed to expose.
 *
 * This replaced a blanket prohibition on `#[tauri::command]`,
 * `generate_handler!` and `.invoke_handler(` on 2026-09-05. That rule sat
 * beside `executeSql`, `readAnyFile` and `runShell`, which are all *generic
 * capability escapes*, and ADR 0009 states the actual intent: "Tauri native
 * permissions remain empty until a separately reviewed typed capability is
 * required." The accepted CSP already provisions `ipc:` and
 * `http://ipc.localhost`, which would be pointless if IPC were forbidden
 * outright. So the rule is enforced as an inventory rather than a ban: a
 * named, enumerable surface is the opposite of a generic escape, and the
 * surface must grow through a visible reviewed edit to this list.
 *
 * Both the declared commands and the registered handler list are checked
 * against this set, in both directions, so neither a command that is never
 * registered nor a registration that was never reviewed can pass.
 */
/** Every `#[tauri::command]` function name declared in one file. */
function declaredTauriCommands(content) {
  const names = new Set();
  for (const match of content.matchAll(
    /#\s*\[\s*tauri::command\s*\][\s\S]{0,400}?\bfn\s+([a-z_][a-z0-9_]*)/g,
  )) {
    names.add(match[1]);
  }
  return names;
}

const reviewedTauriCommands = new Set([
  // Reviewed H0 platform query. A later system-health contract may absorb
  // it into GetSystemHealth; until that contract exists, it stands on its
  // own.
  "get_ledger_status",
  // Reviewed H0 route query for the Executive Cockpit. Read-only: it
  // derives attention, ranks and composes, and mutates nothing. Reads the
  // projection and composition snapshots and answers outOfSync when their
  // revisions differ (DG3 S01 amendment 2026-09-15, §8).
  "get_executive_cockpit",
  // Reviewed H0 route query for the Portfolio overview (S02 list half).
  // Read-only, and shares the Cockpit's ordering rather than defining a
  // second one.
  "get_portfolio_overview",
  // Reviewed H0 route query for S09 People. Read-only; composition folds
  // classification upward so a directory entry cannot leak by aggregation.
  "get_people_directory",
  // Reviewed H0 route query for S03 Work Queue. Read-only: it
  // derives attention, orders by the accepted ranking policy, filters and
  // pages. It offers no write. The intents it returns say what a record's
  // lifecycle admits, which is strictly weaker than what may be executed --
  // preparation, classification, policy and approval all still gate H2, so
  // this command cannot become a write path by being called differently.
  // It reads two snapshots and reports `outOfSync` rather than composing
  // across two Ledger revisions.
  "get_work_queue",
  // Reviewed H0 route query for the S02 detail half and the O01
  // Product-health inspector, per the DG3 O01 amendment of 2026-09-07.
  // Read-only. Work appears only under the person who carries it; the DTO
  // has no field for work the Product owns, so the command cannot become a
  // path to that claim. Reads two snapshots and reports `outOfSync` rather
  // than composing across two Ledger revisions. Offers no write.
  "get_product_detail",
  // ---- S03 write path. Reviewed H1/H2a commands. The
  // webview supplies only the identity it read (id + expected version), the
  // person's own rationale, and one opaque clientRequestId per submitted
  // command (the idempotency id, reused verbatim on retry). The host mints
  // correlation ids, timestamps, audit / receipt / prepared-intent / action
  // ids; the actor is fixed to the Head of Products and the H2a
  // confirmation is the explicit Approve click. No actor, policy, receipt,
  // fingerprint, Vault path or time crosses IPC. Every command runs the
  // domain's own gates (state, version, classification, policy, expiry,
  // digest) inside one immediate SQLite transaction.
  // H2a step 1: persists the canonical Prepared Intent and returns the whole
  // typed contract for O03; executes nothing.
  "prepare_accept_action_request",
  // H2a step 2: the acknowledged digest must match the persisted preview;
  // atomic single-use receipt on execute.
  "approve_and_execute_accept_action_request",
  // H2a rejection (v45): durable, audited, no effect. Closing O03 locally is
  // not a rejection; this command is.
  "reject_prepared_accept_action_request",
  // H1 retreat routes for an Open Action Request, with the person's rationale.
  "decline_action_request",
  "withdraw_action_request",
  // H1: Open -> InProgress.
  "start_action",
  // ---- Action lifecycle. Same boundary as above.
  // H0: the Action as held now plus every Evidence reference as a read
  // record (id, role, verification, pinned, classification, version);
  // never a Vault path. Read-only.
  "get_action_completion_context",
  // H1: link an Evidence reference as completion evidence; the Ledger
  // resolves the reference itself, the webview names only its id.
  "link_action_completion_evidence",
  // H2a step 1 for Complete / Cancel / Reopen. The Ledger prepares with its
  // own persisted Evidence authority (source revisions bound since v45);
  // the optional completion Judgment's disposition is fixed by the host.
  "prepare_complete_action",
  "prepare_cancel_action",
  "prepare_reopen_action",
  // H2a step 2 for each: acknowledged digest, atomic single-use receipt.
  "approve_and_execute_complete_action",
  "approve_and_execute_cancel_action",
  "approve_and_execute_reopen_action",
  // H2a rejection (v45) of any of the three previews.
  "reject_prepared_action_intent",
  // ---- Decision Requests. Same boundary as above.
  // H0: every Evidence reference as a read record; never a Vault path.
  "get_evidence_references",
  // H1 retreat route with the person's rationale.
  "withdraw_decision_request",
  // H2a step 1: the host mints the Decision id, every resulting Action
  // Request id and the Prepared Intent id; Evidence is read from the Ledger
  // by the ids the person chose; the Judgment disposition is host-fixed.
  "prepare_resolve_decision_request",
  // H2a step 2: acknowledged digest, atomic single-use receipt; resulting
  // Action Requests are created inside the same Ledger transaction.
  "approve_and_execute_resolve_decision_request",
  // H2a rejection (v45).
  "reject_prepared_decision_intent",
  // ---- Evidence writes from the inspector. The first commands in this host
  // that read the user's Product Vault rather than only the Ledger.
  //
  // Same boundary, and it holds without an exception: the webview names the
  // Evidence id it read and one opaque request id, and **supplies no path**.
  // Both operations use the path the Evidence record already stores, so no
  // Vault path crosses this boundary in either direction -- the reason
  // relocate, which needs a person to choose a new path, is deliberately
  // absent from this list.
  //
  // H1-User, no H2a review sheet: each is one explicit click whose whole
  // effect is stated before it runs. The Vault root is validated per
  // invocation rather than held from startup, and a workspace with no Vault
  // refuses with a stable code instead of pretending one is missing.
  "pin_evidence_fingerprint",
  "reobserve_evidence_verification",
  // H0: whether the Vault can serve the two operations above right now, and
  // if not, which of the two ordinary reasons applies. Read-only; carries no
  // path. Exists so O01 can show those actions disabled with a reason rather
  // than failing on click (the Vault's visible Degraded Mode).
  "get_vault_status",
  // H1-User: link an Evidence reference to the inspected Product. Ledger
  // only -- no Vault, no observation -- so it stays available in Degraded
  // Mode. Scoped to one target kind on purpose: O01 reads
  // `evidence_links.target_type = 'product'`, and a generic target picker
  // over the domain's thirteen kinds is a different surface.
  "link_evidence_to_product",
  // ---- S10 UI language. The platform settings document, never the Ledger.
  // H0: the stored preference, one of the six UI languages or "und" (follow
  // the system); anything else reads as "und".
  "get_display_locale",
  // H1-User: store a new preference. The webview supplies only that value,
  // which the host checks against the same seven; the write is serialized
  // through the one managed settings store at the revision just read.
  "set_display_locale",
  // H0: the workspace's configured time zone, the zone every entered date is
  // read in (DG3 record-entry amendment §3.5). An IANA name, never a path.
  "get_display_timezone",
  // ---- S7-A backup destination (ADR 0010, ADR 0011). The platform settings
  // document, never the Ledger.
  // H0: whether a destination is set and reachable. Never the path.
  "get_backup_destination",
  // H1-User: opens the host's native folder picker, proves the folder is a
  // real writable directory, stores it. The webview supplies only the
  // dialog title and learns only configured/available/chosen.
  "choose_backup_destination",
  // ---- S7-A recovery passphrase (ADR 0010 §7-8). Never in the Ledger,
  // settings, an archive or a log; no command returns a stored one.
  // Returns a newly generated ten-word passphrase to show once; stores
  // nothing.
  "generate_recovery_passphrase",
  // H1-User: the confirmed passphrase, for this session and, only when the
  // person ticks it, Windows Credential Manager.
  "set_backup_passphrase",
  // H0: available / remembered, never the passphrase.
  "get_backup_passphrase_status",
  // ---- S7-A backup runs and the write gate (DG3 backup-setup amendment).
  // H0: one state, folder name only, never a path or the passphrase.
  "get_backup_status",
  // H1-User: "Back up now"; ends verified and registered, or with a reason.
  "run_backup_now",
  // ---- S7-B1 Operational Restore (H2b; DG3 restore amendment, ADR 0011,
  // ADR 0012). Live only. The picked file stays in the host behind an opaque
  // token; the webview supplies dialog labels, that token, the passphrase
  // for that one file (never stored), and for approval the preview digest,
  // the typed date and one clientRequestId. It learns the file's own name,
  // the preview facts and the outcome -- never a path.
  // H1-User: the host's file picker.
  "choose_restore_archive",
  // H1-User: the recovery backup a failed restore left, from the backup
  // registry, offered first on System Health (restore-unopened amendment §2);
  // no argument, and only its file name comes back.
  "choose_recovery_archive",
  // H0: forget the picked file and its check.
  "discard_restore_selection",
  // H0: decrypt and check every member; nothing is replaced.
  "check_restore_archive",
  // H1-User: the recovery backup of the current workspace, then the exact
  // preview (the H2b Prepared Intent, kept outside the Ledger it replaces).
  "prepare_restore_from_archive",
  // H2b reject: nothing changes; the choice is recorded.
  "reject_prepared_restore",
  // H2b approve and execute: close, replace, reopen, check; put back on
  // failure.
  "approve_and_execute_restore",
  // H0: whether the Ledger is open, and why not (S11).
  "get_system_health",
  "get_workspace_status",
  "choose_first_workspace",
  "switch_workspace",
  "reset_sample_data",
  "prepare_sample_delete",
  "reject_sample_delete",
  "approve_sample_delete",
  // S11 "Quit" when the Ledger cannot be used.
  "quit_pmc",
  // ---- Evidence from a file (item ⑦-3; DG3 Vault-root and Evidence-from-file
  // amendment §4). The picked file stays in the host behind an opaque,
  // expiring token; the webview supplies the dialog title, that token, the
  // chosen classification and one clientRequestId, and learns the file's own
  // name, its observation time and existing references by id and version.
  // H1-User: the host's file picker, opened in the Vault folder.
  "choose_evidence_file",
  // H1-User: observe again, pin, create (reservation-checked).
  "create_evidence_from_file",
  // H0: forget the chosen file.
  "discard_evidence_file_choice",
  // ---- The Live Product Vault folder (item ⑦, H2b; DG3 Vault-root
  // amendment, ADR 0011, ADR 0012). Live only. The picked folder stays in the
  // host behind an opaque token; the webview supplies the dialog title, that
  // token, and for approval the preview digest, the typed code and one
  // clientRequestId. It learns the folder's own name, counts and the
  // outcome -- never a path.
  // H1-User: the host's folder picker; refuses a folder PMC already uses.
  "choose_vault_folder",
  // H1-User: a fresh verified backup, then the exact preview.
  "prepare_vault_root_change",
  // H2b reject: nothing changes; the choice is recorded.
  "reject_vault_root_change",
  // H2b approve: re-check under the gate and the Ledger write lock, then the
  // one settings write.
  "approve_vault_root_change",
  // ---- Record entry, the Portfolio family (slice 6B; DG3 record-entry
  // amendment §2–§4). The webview supplies the fields the person entered,
  // the id and version of a record it read, and one clientRequestId per
  // opened sheet; the host mints every new id through the Ledger's durable
  // reservation (v47), the correlation, audit ids and the instant, and fixes
  // provenance to UserEntered. No path.
  // H0: every record of one kind with its editable fields and version.
  "list_entry_records",
  // H0: one record's editable fields at its current version, for an edit.
  "get_entry_record",
  // H1-User creates and edits, one command per operation (§3.1).
  "create_portfolio_record",
  "update_portfolio_record",
  "create_product_record",
  "update_product_record",
  "create_roadmap_record",
  "update_roadmap_record",
  "create_kpi_definition_record",
  "update_kpi_definition_record",
  "create_kpi_observation_record",
  "update_kpi_observation_record",
  // H1-User links between records the sheet read, at the versions it read.
  "link_portfolio_product_record",
  "link_product_roadmap_record",
  "link_product_kpi_record",
  // ---- Record entry, the Delivery family (slice 6C): Initiative, Project
  // and Milestone creates and edits, and the InitiativeProject /
  // ProjectProduct links. A Milestone is created under the Project the
  // sheet was opened from. Same boundary as above; instants the person
  // entered arrive as UTC milliseconds, never before the epoch.
  "create_initiative_record",
  "update_initiative_record",
  "create_project_record",
  "update_project_record",
  "create_milestone_record",
  "update_milestone_record",
  "link_initiative_project_record",
  "link_project_product_record",
  // ---- Record entry, People (slice 6D): a Stakeholder create and edit, and
  // a Stakeholder's relationship to a subject it read (responsible for it or
  // depending on it), at both versions read. Same boundary as above.
  "create_stakeholder_record",
  "update_stakeholder_record",
  "link_stakeholder_subject_record",
  // ---- Record entry, work (slice 6E): an Action Request draft and its submit
  // (Draft -> Open at the version the row read), a Decision Request draft and
  // its submit, an Issue, a Risk, and a Risk's response. Same boundary as
  // above; a request's intended owner is a Stakeholder id the sheet read.
  "create_action_request_draft_record",
  "submit_action_request_record",
  "create_decision_request_draft_record",
  "submit_decision_request_record",
  "create_issue_record",
  "create_risk_record",
  "update_risk_response_record",
  // ---- The Ledger upgrade gate (DG3 upgrade-gate amendment, ADR 0004).
  // H0: whether an older, unsupported or newer Ledger replaces the shell,
  // with the facts the screen shows. No path.
  "get_upgrade_gate",
  // H1-User "Upgrade": a new verified pre-upgrade backup, then the upgrade
  // in one transaction on its receipt. The webview supplies nothing.
  "run_upgrade",
  // ---- S03 Risk and Issue H2a write path. Reviewed H2a
  // commands on the same boundary as the Action and Decision ones above:
  // the webview supplies the identity and version it read, the person's own
  // rationale, the Evidence ids the person picked, and one opaque
  // clientRequestId per submitted command. The host mints correlation ids,
  // timestamps, prepared-intent / audit / receipt ids, and -- for an
  // occurrence -- the id of the Issue that occurrence creates, which the
  // preview names and the payload digest binds.
  //
  // A PREPARE first asks the Ledger which preview this client request already
  // produced and returns that one, so a retry can never mint a second
  // identity; a preview that has since been approved or refused is reported
  // as consumed rather than silently replaced.
  //
  // The three Issue transitions share one approve command on purpose: which
  // transition is being executed, and which Evidence it binds, are read from
  // the stored preview, never accepted from the webview. Evidence roles are
  // likewise assigned from the operation in the host.
  "prepare_record_risk_occurrence",
  "prepare_close_risk",
  "approve_and_execute_record_risk_occurrence",
  "approve_and_execute_close_risk",
  "reject_prepared_risk_intent",
  "prepare_resolve_issue",
  "prepare_close_issue",
  "prepare_reopen_issue",
  "approve_and_execute_issue_transition",
  "reject_prepared_issue_intent",
]);

const reviewedNativeApiHarnesses = new Map([
  [
    "crates/pmc-platform/tests/settings_store.rs",
    {
      importStatement: "use std::process::Command;",
      harnesses: [
        ["CRASH", "797f4580bcd26449322fb68ffdb69413e82945f95036fedb4f38ee095ed096e5"],
        ["JUNCTION", "4b3a67264ca1e29897dfafcbeae6518665817476588c8635a00d5dd43306eb4a"],
        ["ABORT", "cb569af588a104716b8cb18b5283ee5278944fd75e9cb74745781e13ea9fc377"],
      ],
    },
  ],
  [
    "crates/pmc-platform/tests/workspace_identity.rs",
    {
      importStatement: "use std::process::Command;",
      harnesses: [["JUNCTION", "92f760f34da7d503d7f497fd75abdb7b2ba1dd510495823e6beb02c445bd2f0f"]],
    },
  ],
  [
    "crates/pmc-platform/tests/paths.rs",
    {
      importStatement: "use std::process::Command;",
      harnesses: [["JUNCTION", "92f760f34da7d503d7f497fd75abdb7b2ba1dd510495823e6beb02c445bd2f0f"]],
    },
  ],
  [
    // The same reviewed junction harness as paths.rs, byte for byte, reused
    // for the Vault stage-path TOCTOU hardening tests, as the product owner
    // decided.
    "crates/pmc-platform/tests/managed_publication.rs",
    {
      importStatement: "use std::process::Command;",
      harnesses: [["JUNCTION", "92f760f34da7d503d7f497fd75abdb7b2ba1dd510495823e6beb02c445bd2f0f"]],
    },
  ],
  [
    "crates/pmc-knowledge/tests/vault.rs",
    {
      importStatement: "use std::process::Command;",
      harnesses: [["JUNCTION", "92f760f34da7d503d7f497fd75abdb7b2ba1dd510495823e6beb02c445bd2f0f"]],
    },
  ],
  [
    "crates/pmc-knowledge/tests/evidence.rs",
    {
      importStatement: "use std::process::Command;",
      harnesses: [["JUNCTION", "92f760f34da7d503d7f497fd75abdb7b2ba1dd510495823e6beb02c445bd2f0f"]],
    },
  ],
  [
    // The only call into rfd (ADR 0011).
    "apps/desktop/src-tauri/src/native_dialogs.rs",
    {
      importStatement: undefined,
      harnesses: [
        ["FOLDER-PICKER", "bb656e3d868d7c1f088b092b91cf396abf30aa4f1eabdfd104611e7147777be8"],
        // The restore sheet's backup-file picker (DG3 restore amendment §3.1,
        // accepted 2026-09-22).
        ["FILE-PICKER", "b5b78ccb82d0454e37830db2675ed8c868e8bbda0699ad4de2389a57cfee531a"],
        // The second launch's "already running" message box (item ⑩,
        // single instance; product owner 2026-09-21). One button, no path.
        ["MESSAGE-BOX", "45045ecbab931ee19a0f4b300c6ffc4c26e1a9516d3a8784cc3f83a62abb7db8"],
        // Evidence from a file (DG3 Vault-root and Evidence-from-file amendment
        // §4.1, accepted 2026-09-23): any file, opened in the Vault folder; the
        // host checks the answer against the Vault.
        [
          "EVIDENCE-FILE-PICKER",
          "a9da5fe4a79343016ac716f431e78dbf66ce56bdebf743ee7e69927cce551399",
        ],
      ],
    },
  ],
  [
    "crates/pmc-platform/src/uri_launcher.rs",
    {
      importStatement: undefined,
      harnesses: [
        ["OBSIDIAN-LAUNCH", "112ce1b1d2924fa48563b32af14f8a251904e16e9c537ad810c75d2ecd2f2553"],
      ],
    },
  ],
]);

/**
 * The in-place Ledger upgrade (slice 8d) runs only on a receipt the backup
 * pipeline made for that exact source. Rust cannot seal a public trait across
 * crates, so the boundary is kept here: outside `pmc-ledger` itself, only
 * the backup service may even name `upgrade_in_place` or
 * `VerifiedPreUpgradeBackup`. Banning the names — not just a call or an
 * `impl` shape — also catches `use … as` aliases and fully qualified paths,
 * since each must spell the name once; building an identifier by macro would
 * need a crate the dependency allow-list does not admit. The whole file is
 * scanned, comments and strings included: stripping comments by pattern is
 * not Rust-aware, and a string holding a comment marker could hide live code.
 */
const upgradeEntryAllowed = new Set(["crates/pmc-application/src/backup_service.rs"]);

function upgradeProofBoundary(content, label) {
  const path = label.replaceAll("\\", "/");
  if (path.startsWith("crates/pmc-ledger/") || upgradeEntryAllowed.has(path)) {
    return [];
  }
  const findings = [];
  if (/\bupgrade_in_place\b/.test(content)) {
    findings.push(`${label}: in-place upgrade named outside the backup service`);
  }
  if (/\bVerifiedPreUpgradeBackup\b/.test(content)) {
    findings.push(`${label}: pre-upgrade backup proof named outside the backup service`);
  }
  return findings;
}

/**
 * Record entry (slice 6A; DG3 record-entry amendment §4; product owner
 * 2026-09-22): the desktop host creates records only through the Ledger's
 * reservation-checked writers and the application flows built on them. The
 * plain create and link writers take a caller-supplied id — they exist for the
 * seed tool and the tests — so the host may not name one at all, in any
 * module, comments included: as a method call (`.create_risk(`), a path
 * (`SqliteProductLedger::create_risk`) or a value taken into a binding
 * (`let f = ledger.create_risk;`) — every spelling reaches the writer through
 * `.` or `::`, which is what is matched, with any spacing. A bare word is
 * not matched: the same names are audit effect codes in string literals. A
 * name built by macro would need a crate the dependency allow-list does not
 * admit. Evidence joined on 2026-09-23 (schema v48, Evidence from a file):
 * the host creates a reference only through
 * `create_evidence_reference_from_reservation`, which the trailing `\b` keeps
 * apart from the plain `create_evidence_reference`.
 */
const plainCreateWriters =
  /(?:\.|::)\s*(?:create_(?:portfolio|product|roadmap|kpi_definition|kpi_observation|initiative|project|milestone|stakeholder|risk|issue|action_request_draft|decision_request_draft|evidence_reference)|link_(?:portfolio_product|portfolio_initiative|product_roadmap|product_kpi|initiative_project|project_product|stakeholder_relationship))\b/g;

function recordEntryBoundary(content, label) {
  const path = label.replaceAll("\\", "/");
  if (!/^apps\/[^/]+\/src-tauri\/src\/.*\.rs$/.test(path)) {
    return [];
  }
  const findings = [];
  for (const match of content.matchAll(plainCreateWriters)) {
    findings.push(
      `${label}: host names the plain writer ${match[0].trim()} (record entry goes through a reservation)`,
    );
  }
  return findings;
}

/**
 * The one reviewed `unsafe` call (product owner, 2026-09-23: Windows' own
 * name comparison for Evidence from a file). `pmc-platform` only denies
 * unsafe code, and a deny can be overridden by an `allow`, so the exception
 * is held here: exactly one `unsafe` block and one `allow(unsafe_code)`, in
 * that one file, and none anywhere else. Any further unsafe code needs the
 * product owner again.
 */
const REVIEWED_UNSAFE_FILE = "crates/pmc-platform/src/windows_names.rs";
const unsafeCode = /\bunsafe\s*(?:\{|fn\b|impl\b|extern\b|trait\b)/g;
const unsafeAllowance = /#!?\[\s*allow\s*\([^)]*\bunsafe_code\b/g;

function unsafeBoundary(content, label) {
  const path = label.replaceAll("\\", "/");
  // Shipped code only: integration tests under a crate's `tests/` are their
  // own crates and never reach the product.
  if (!/^(?:crates\/[^/]+|apps\/[^/]+\/src-tauri|tools\/[^/]+)\/src\//.test(path)) {
    return [];
  }
  const code = content.match(unsafeCode)?.length ?? 0;
  const allowances = content.match(unsafeAllowance)?.length ?? 0;
  if (path === REVIEWED_UNSAFE_FILE) {
    return code === 1 && allowances === 1
      ? []
      : [`${label}: the reviewed unsafe exception changed (one unsafe block, one allowance)`];
  }
  return code > 0 || allowances > 0 ? [`${label}: unsafe code outside the reviewed exception`] : [];
}

/**
 * Only the sample-workspace service, its tests, the host's one sample
 * module and the development command may prepare, seed or reset the sample
 * data (the accepted sample-workspace amendment §7; the design's
 * policy boundary). Anything else reaching for them could write synthetic
 * data where it does not belong.
 */
// A lexical guard, not an authority boundary: it catches any file outside
// the allowed ones that names these entry points (an alias still has to name
// them once; a comment mention is flagged too). It does not see direct
// filesystem calls, and inside the allowed files it checks nothing — those
// are reviewed, not verified here.
const SAMPLE_WORKSPACE_ENTRY_POINTS =
  /\b(?:SampleWorkspace|seed_training(?:_in)?|write_seed(?:_at)?|remove_tree)\b/;
const SAMPLE_WORKSPACE_CALLERS = new Set([
  "crates/pmc-application/src/sample_workspace.rs",
  "crates/pmc-application/src/sample_lifecycle.rs",
  "crates/pmc-application/tests/sample_workspace.rs",
  "apps/desktop/src-tauri/src/sample_workspace.rs",
]);

function sampleWorkspaceBoundary(content, label) {
  const path = label.replaceAll("\\", "/");
  if (SAMPLE_WORKSPACE_CALLERS.has(path) || path.startsWith("tools/pmc-seed/")) {
    return [];
  }
  return SAMPLE_WORKSPACE_ENTRY_POINTS.test(content)
    ? [`${label}: prepares or seeds the sample workspace outside its service`]
    : [];
}

/** `tools/` is outside the general source scan; this boundary covers it. */
function inspectToolsForSampleWorkspace() {
  return globSync("tools/**/*.rs", {
    cwd: root,
    exclude: ["**/target/**"],
  }).flatMap((file) => {
    const absolute = resolve(root, file);
    return sampleWorkspaceBoundary(readFileSync(absolute, "utf8"), repositoryLabel(absolute));
  });
}

function inspectSourceFiles() {
  const forbidden = [
    ["generic SQL command", /executeSql/],
    ["arbitrary file command", /readAnyFile/],
    ["generic shell command", /runShell/],
    ["prototype import", /(?:from|import\s*)[\s(]*["'][^"']*prototypes\/dg1/],
    ["native plugin/API", FORBIDDEN_NATIVE_API],
    ["native dialog API", NATIVE_DIALOG_API],
    [
      "direct Rust network API",
      /(?:std::net(?:::|\b)|tokio::net(?:::|\b)|(?:Tcp|Udp)(?:Stream|Socket)|(?:TcpListener|UdpSocket)|(?:reqwest|ureq|hyper|attohttp|curl)::)/,
    ],
    [
      "renderer network/navigation API",
      /(?:\b(?:globalThis|window|self)\.fetch\s*\(|\bfetch\s*\(|\bXMLHttpRequest\b|\bWebSocket\s*\(|\bEventSource\s*\(|\bnavigator\.sendBeacon\s*\(|\bwindow\.open\s*\(|\b(?:window\.)?location(?:\.href)?\s*=|\b(?:window\.)?location\.(?:assign|replace)\s*\(|<\s*(?:form|a)\b[^>]*(?:action|href)\s*=)/i,
    ],
  ];
  const failures = [];
  for (const file of sourceFiles) {
    if (![".js", ".jsx", ".mjs", ".rs", ".ts", ".tsx"].includes(extname(file))) continue;
    const absolute = resolve(root, file);
    const content = readFileSync(absolute, "utf8");
    const label = repositoryLabel(absolute);
    let inspectedContent = content;
    const reviewedHarnessSpec = reviewedNativeApiHarnesses.get(label);
    if (reviewedHarnessSpec) {
      const { importStatement, harnesses: reviewedHarnesses } = reviewedHarnessSpec;
      if (importStatement !== undefined) {
        const importCount = content.split(importStatement).length - 1;
        if (importCount !== 1) {
          failures.push(`${label}: reviewed harness process import changed`);
        }
      }
      for (const [harnessName, expectedHash] of reviewedHarnesses) {
        const startMarker = `// PMC-REVIEWED-${harnessName}-HARNESS-START`;
        const endMarker = `// PMC-REVIEWED-${harnessName}-HARNESS-END`;
        const start = inspectedContent.indexOf(startMarker);
        const end = inspectedContent.indexOf(endMarker);
        if (
          start === -1 ||
          end === -1 ||
          end <= start ||
          start !== inspectedContent.lastIndexOf(startMarker) ||
          end !== inspectedContent.lastIndexOf(endMarker)
        ) {
          failures.push(`${label}: reviewed ${harnessName.toLowerCase()} harness markers changed`);
        } else {
          const blockStart = start + startMarker.length;
          const reviewedBlock = inspectedContent.slice(blockStart, end);
          const reviewedHash = createHash("sha256").update(reviewedBlock).digest("hex");
          if (reviewedHash !== expectedHash) {
            failures.push(`${label}: reviewed ${harnessName.toLowerCase()} harness hash changed`);
          } else {
            inspectedContent = `${inspectedContent.slice(0, start)}${inspectedContent.slice(
              end + endMarker.length,
            )}`;
          }
        }
      }
      if (importStatement !== undefined) {
        inspectedContent = inspectedContent.replace(importStatement, "");
      }
    }
    for (const [name, pattern] of forbidden) {
      if (pattern.test(inspectedContent)) {
        failures.push(`${relative(root, absolute)}: ${name}`);
      }
    }
    if (/^apps\/[^/]+\/src-tauri\/src\/.*\.rs$/.test(label)) {
      failures.push(...pathBearingIpc(content, label));
    }
    if (extname(file) === ".rs") {
      failures.push(...upgradeProofBoundary(content, label));
      failures.push(...recordEntryBoundary(content, label));
      failures.push(...unsafeBoundary(content, label));
      failures.push(...sampleWorkspaceBoundary(content, label));
    }
  }
  return failures;
}

function inspectSourceFile(file) {
  const label = relative(root, file);
  const content = readFileSync(file, "utf8");
  const failures = [...pathBearingIpc(content, label)];
  for (const [name, pattern] of [
    ["generic SQL command", /executeSql/],
    ["arbitrary file command", /readAnyFile/],
    ["generic shell command", /runShell/],
    ["native plugin/API", FORBIDDEN_NATIVE_API],
    ["native dialog API", NATIVE_DIALOG_API],
    [
      "direct Rust network API",
      /(?:std::net(?:::|\b)|tokio::net(?:::|\b)|(?:Tcp|Udp)(?:Stream|Socket)|(?:TcpListener|UdpSocket)|(?:reqwest|ureq|hyper|attohttp|curl)::)/,
    ],
    [
      "renderer network/navigation API",
      /(?:\b(?:globalThis|window|self)\.fetch\s*\(|\bfetch\s*\(|\bXMLHttpRequest\b|\bWebSocket\s*\(|\bEventSource\s*\(|\bnavigator\.sendBeacon\s*\(|\bwindow\.open\s*\(|\b(?:window\.)?location(?:\.href)?\s*=|\b(?:window\.)?location\.(?:assign|replace)\s*\(|<\s*(?:form|a)\b[^>]*(?:action|href)\s*=)/i,
    ],
  ]) {
    if (pattern.test(content)) failures.push(`${label}: ${name}`);
  }
  for (const command of declaredTauriCommands(content)) {
    if (!reviewedTauriCommands.has(command)) {
      failures.push(`${label}: unreviewed Tauri command`);
      break;
    }
  }
  return failures;
}

/**
 * Enforces the reviewed Tauri IPC inventory in both directions.
 *
 * Declared: every `#[tauri::command]` function found in the Tauri crate.
 * Registered: every path listed inside `generate_handler![...]`.
 *
 * A declared command that is not reviewed, a registered command that is not
 * reviewed, a reviewed command that nothing declares, and a declared command
 * that is never registered are all failures. The last two matter because a
 * stale allowlist entry would silently widen what a future edit may add
 * without review.
 */
function inspectTauriIpcSurface() {
  const failures = [];
  const declared = new Set();
  const registered = new Set();
  for (const file of globSync("apps/**/src-tauri/src/**/*.rs", { cwd: root })) {
    const content = readFileSync(resolve(root, file), "utf8");
    for (const name of declaredTauriCommands(content)) {
      declared.add(name);
    }
    for (const handler of content.matchAll(/generate_handler\s*!\s*\[([^\]]*)\]/g)) {
      for (const entry of handler[1].split(",")) {
        const name = entry.trim().split("::").pop();
        if (name) registered.add(name);
      }
    }
  }
  for (const name of declared) {
    if (!reviewedTauriCommands.has(name)) {
      failures.push(
        `apps/desktop/src-tauri: Tauri command ${name} is not in the reviewed IPC inventory`,
      );
    }
  }
  for (const name of registered) {
    if (!reviewedTauriCommands.has(name)) {
      failures.push(
        `apps/desktop/src-tauri: registered handler ${name} is not in the reviewed IPC inventory`,
      );
    }
  }
  for (const name of reviewedTauriCommands) {
    if (!declared.has(name)) {
      failures.push(`reviewed IPC inventory lists ${name}, which no Tauri command declares`);
    }
    if (!registered.has(name)) {
      failures.push(`reviewed IPC inventory lists ${name}, which no invoke handler registers`);
    }
  }
  return failures;
}

function inspectWorkspace() {
  const failures = [
    ...inspectSourceFiles(),
    ...inspectToolsForSampleWorkspace(),
    ...inspectTauriIpcSurface(),
  ];
  for (const file of globSync("apps/**/src-tauri/capabilities/*.json", { cwd: root })) {
    failures.push(...inspectCapability(resolve(root, file)));
  }
  for (const file of globSync("apps/**/src-tauri/tauri.conf.json", { cwd: root })) {
    failures.push(...inspectTauriConfig(resolve(root, file)));
  }
  for (const file of ["package.json", ...globSync("apps/**/package.json", { cwd: root })]) {
    failures.push(...inspectPackageManifest(resolve(root, file)));
  }
  for (const file of globSync("{Cargo.toml,apps/**/Cargo.toml,crates/**/Cargo.toml}", {
    cwd: root,
  })) {
    failures.push(...inspectCargoManifest(resolve(root, file)));
  }
  return failures;
}

function runNegativeFixtureCoverage() {
  const fixtures = [
    ["forbidden-capability.json", inspectCapability, ["shell:allow-open"]],
    [
      "forbidden-classes.json",
      inspectCapability,
      [
        "fs:default",
        "filesystem:default",
        "shell:default",
        "http:default",
        "https:default",
        "network:default",
        "uri:default",
        "sql:default",
        "opener:default",
        "process:default",
      ],
    ],
    [
      "forbidden-tauri.json",
      inspectTauriConfig,
      ["unsupported key", "network source", "remote window URLs"],
    ],
    ["missing-connect-csp.json", inspectTauriConfig, ["required connect-src directive is missing"]],
    ["missing-security.json", inspectTauriConfig, ["requires security configuration"]],
    [
      "missing-browser-args.json",
      inspectTauriConfig,
      ["requires the exact no-background-networking argument set"],
    ],
    ["remote-window-url.json", inspectTauriConfig, ["remote window URLs"]],
    [
      "installer-drift.json",
      inspectTauriConfig,
      [
        "bundle.targets: must be exactly",
        "bundle.windows.webviewInstallMode.type: must be exactly",
        "bundle.windows.nsis.installMode: must be exactly",
        "bundle.windows.nsis.minimumWebview2Version: unsupported key",
      ],
    ],
    [
      "installer-deletes-data.json",
      inspectTauriConfig,
      [
        "offers to delete application data",
        "removes a folder under the application data roots",
        "vendored from a different Tauri CLI",
        "is not the reviewed template",
      ],
    ],
    ["forbidden-package.json", inspectPackageManifest, ["forbidden native capability dependency"]],
    [
      "forbidden-package-alias.json",
      inspectPackageManifest,
      ["forbidden native capability dependency"],
    ],
    [
      "forbidden-package-alias-swap.json",
      inspectPackageManifest,
      ["dependency is not in the exact reviewed manifest"],
      "apps/desktop/package.json",
    ],
    [
      "forbidden-package-classes.json",
      inspectPackageManifest,
      [
        "native-fs",
        "native-filesystem",
        "native-shell",
        "native-http",
        "native-https",
        "native-network",
        "native-sql",
        "native-uri",
        "native-opener",
        "native-process",
      ],
    ],
    ["forbidden-cargo.toml", inspectCargoManifest, ["forbidden native capability dependency"]],
    [
      "forbidden-cargo-alias.toml",
      inspectCargoManifest,
      ["forbidden native capability dependency"],
    ],
    [
      "forbidden-cargo-table-alias.toml",
      inspectCargoManifest,
      ["forbidden native capability dependency"],
    ],
    [
      "forbidden-cargo-quoted-table-alias.toml",
      inspectCargoManifest,
      ["forbidden native capability dependency"],
    ],
    [
      "forbidden-cargo-classes.toml",
      inspectCargoManifest,
      [
        "native_fs",
        "native_filesystem",
        "native_shell",
        "native_http",
        "native_https",
        "native_network",
        "native_sql",
        "native_uri",
        "native_opener",
        "native_process",
      ],
    ],
    ["forbidden-command.rs", inspectSourceFile, ["unreviewed Tauri command"]],
    ["forbidden-native-api.rs", inspectSourceFile, ["native plugin/API"]],
    ["forbidden-native-command-alias.rs", inspectSourceFile, ["native plugin/API"]],
    ["forbidden-native-command-grouped-alias.rs", inspectSourceFile, ["native plugin/API"]],
    ["forbidden-native-changed-executable.rs", inspectSourceFile, ["native plugin/API"]],
    ["forbidden-rust-network.rs", inspectSourceFile, ["direct Rust network API"]],
    ["forbidden-native-dialog.rs", inspectSourceFile, ["native dialog API"]],
    ["forbidden-ipc-path-argument.rs", inspectSourceFile, ["path-bearing IPC argument"]],
    ["forbidden-ipc-path-field.rs", inspectSourceFile, ["path type in host code"]],
    ["forbidden-ipc-path-named-field.rs", inspectSourceFile, ["path-bearing IPC field"]],
    ["forbidden-ipc-path-attribute-args.rs", inspectSourceFile, ["path-bearing IPC argument"]],
    ["forbidden-ipc-path-alias.rs", inspectSourceFile, ["path type in host code"]],
    ["forbidden-renderer-network.fixture", inspectSourceFile, ["renderer network/navigation API"]],
    ["forbidden-renderer-fetch.fixture", inspectSourceFile, ["renderer network/navigation API"]],
    ["forbidden-renderer-form.fixture", inspectSourceFile, ["renderer network/navigation API"]],
    ["forbidden-renderer-anchor.fixture", inspectSourceFile, ["renderer network/navigation API"]],
    [
      "forbidden-renderer-location-href.fixture",
      inspectSourceFile,
      ["renderer network/navigation API"],
    ],
  ];
  const failures = [];
  for (const [name, inspector, expected, reviewedLabel] of fixtures) {
    const file = resolve(root, "scripts/verify/fixtures/policy", name);
    const findings = inspector(file, reviewedLabel);
    for (const expectedFinding of expected) {
      if (!findings.some((finding) => finding.includes(expectedFinding))) {
        failures.push(`policy fixture ${name}: expected finding ${expectedFinding}`);
      }
    }
  }
  return failures;
}

const failures = [...inspectWorkspace(), ...runNegativeFixtureCoverage()];
if (failures.length > 0) {
  console.error(failures.join("\n"));
  process.exit(1);
}

console.log(
  `Policy checks passed for ${String(sourceFiles.length)} workspace entries and static capability manifests.`,
);
