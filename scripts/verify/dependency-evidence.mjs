import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, relative, resolve } from "node:path";

const root = resolve(import.meta.dirname, "..", "..");
const lock = JSON.parse(readFileSync(resolve(root, "package-lock.json"), "utf8"));
const cargoLockText = readFileSync(resolve(root, "Cargo.lock"), "utf8");
const APPROVED_NPM_REGISTRY_PREFIX = "https://registry.npmjs.org/";
const APPROVED_CARGO_SOURCE = "registry+https://github.com/rust-lang/crates.io-index";

const executableSignalDispositions = new Map(
  Object.entries({
    "anyhow@1.0.104/build-script-build": [
      "a536d05773b0009ce577aecf64d9a6fb7d759b314df51abff30a6ef08ffc46a7",
      "local-compiler-probe",
      "Invokes the configured local rustc or wrapper for version/cfg probing.",
    ],
    "camino@1.2.5/build-script-build": [
      "77def090bf284e7f566f06bf0cfe7a5072bfc227e018610766f3c44fb8176f5f",
      "local-compiler-probe",
      "Invokes local rustc --version.",
    ],
    "crc32fast@1.5.0/build-script-build": [
      "9bbf26556dd68bbb80db5a82488bde2755926a190389bc76e7857fa19fb72ebb",
      "local-compiler-probe",
      "Invokes the configured local rustc.",
    ],
    "crossbeam-utils@0.8.22/build-script-build": [
      "80d723a2a722212c4c91451fcbe8b65a75724f92c17eff40406051c3d31be271",
      "literal-include-resolved",
      "Both include! paths are literal files captured in the source closure.",
    ],
    "erased-serde@0.4.10/build-script-build": [
      "cc116385d09168fe537a5db06f9e726808379814e088eb9ac9e34e05e2cd3aee",
      "local-compiler-probe",
      "Invokes local rustc --version.",
    ],
    "getrandom@0.3.4/build-script-build": [
      "dcf567e6fdb396eab568a2e21108557ccfdf454418d9edd5042f495303764fbb",
      "local-compiler-probe",
      "Invokes the configured local rustc or wrapper.",
    ],
    "httparse@1.10.1/build-script-build": [
      "b1e84e67daf6769130d634bb3f5f2e115123608fff13efdedf38650889854067",
      "local-compiler-probe",
      "Invokes local rustc for cfg probing.",
    ],
    "libc@0.2.189/build-script-build": [
      "f97102185d49c9915e0cba011a778597436c00d4a8fe9d3f50adc9d9ec3733b6",
      "local-target-probe",
      "Invokes configured compiler and target-local version tools; no network command.",
    ],
    "libdbus-sys@0.2.7/build-script-build": [
      "0ca4e5d756148d716cbe488998e29c3fe23af48014fbc4dc87d5286d580f9bf9",
      "feature-not-reachable",
      "git submodule fallback is inside vendored support; resolved features are default,pkg-config and exclude vendored.",
    ],
    "libsqlite3-sys@0.38.2/build-script-build": [
      "fed6512138575066c946bc2333c37f40ec5ab79c75f7d18b35416c91c28937a7",
      "local-native-compiler",
      "Invokes the local C compiler through cc::Build to compile the locked vendored SQLite source; the reviewed closure contains no network, telemetry, updater, or downloader API.",
    ],
    "iana-time-zone-haiku@0.1.2/build-script-build": [
      "f4147c4414021f4eaa362f2d616f4917af5711ce10d5bc4bdc1af2e90a33e85d",
      "target-not-reachable",
      "Compiles a local Haiku C++ adapter only for the Haiku target; it is not reachable from the supported Windows desktop target and contains no network, telemetry, updater, or downloader API.",
    ],
    "objc2-exception-helper@0.1.1/build-script-build": [
      "d2f33d95574006ad24eace93858efc01a4c64672ae7f99aa62d955fba2cf0eff",
      "target-not-reachable",
      "Compiles a local Objective-C exception helper only for Apple targets; it is not reachable from the supported Windows desktop target and contains no network, telemetry, updater, or downloader API.",
    ],
    "portable-atomic-util@0.2.7/build-script-build": [
      "a6eb66b819e7d463d1d05a02d779acb510c4affa94614db243415af8b6aa2db3",
      "local-compiler-probe",
      "Invokes the configured local rustc or wrapper.",
    ],
    "zerocopy@0.8.57/build-script-build": [
      "54739124673fd8587346ebc13c8399a0b8d545ecc3d2ddb7d74011838c27cf14",
      "local-compiler-probe",
      "Invokes the configured local rustc --version to select cfgs.",
    ],
    "zstd-sys@2.1.0+zstd.1.5.7/build-script-build": [
      "08807e3d32da630ed3a967017590a59ccdd4f7b833d6b0e39771315628395ee7",
      "local-native-compiler",
      "Compiles the locked vendored zstd C source through cc::Build; the pkg-config path runs the local pkg-config only when ZSTD_SYS_USE_PKG_CONFIG is set. No network, telemetry, updater, or downloader API.",
    ],
    "vswhom-sys@0.1.3/build-script-build": [
      "d9487aeaf3e69d56ec5d4aebb0958b1ca57496188e18750c56297711ca2b5592",
      "local-native-compiler",
      "Invokes the local C++ compiler to build the locked Windows Visual Studio discovery helper; the reviewed closure contains no network, telemetry, updater, or downloader API.",
    ],
    "portable-atomic@1.15.0/build-script-build": [
      "4ea0d0cab5f9f666c7391758219e9e093bb787f7ea30524ea5c5a74fa1ec969f",
      "local-compiler-probe",
      "Invokes the configured local rustc or wrapper.",
    ],
    "proc-macro2@1.0.107/build-script-build": [
      "9118143f253423118a39537ad55dc5048bac6ddf8607e0ba138cdcedb531644d",
      "local-compiler-probe",
      "Invokes configured local rustc for version/cfg probing.",
    ],
    "quote@1.0.47/build-script-build": [
      "d1046430c7010bd01d62fb437a33c21a9e87285f77f2da1d508902f6021d5608",
      "local-compiler-probe",
      "Invokes local rustc --version.",
    ],
    "ref-cast@1.0.26/build-script-build": [
      "a2b08b7cdc453f16b69a1932607c6cd8a284cae8cab21044e53720aa33202c96",
      "local-compiler-probe",
      "Invokes local rustc --version.",
    ],
    "rustversion@1.0.23/rustversion": [
      "981e179e13c886b9df1f9b181c15fc47da5f843f5f18961677a21ca5962d63ec",
      "generated-output-reviewed",
      "Includes version.expr generated by the separately reviewed rustversion build target.",
    ],
    "rustversion@1.0.23/build-script-build": [
      "c2b64f5822c72d5f93669aa4ca21db771dce07a3549ee5a6586952ec1ce12f93",
      "local-compiler-probe",
      "Invokes the configured local rustc and writes version.expr under OUT_DIR.",
    ],
    "serde_core@1.0.229/build-script-build": [
      "fd02131d66894a563aed60d8bc7359d3dc669d84fc4a925d8dcac7dba4e2ad3b",
      "local-compiler-probe",
      "Invokes local rustc --version.",
    ],
    "serde@1.0.229/build-script-build": [
      "3c1602bf97c4b59e5d997ef0a7f839957dbfda8d8e11c599990c7e6d3165e328",
      "local-compiler-probe",
      "Invokes local rustc --version.",
    ],
    "swift-rs@1.0.7/build-script-test-build": [
      "f1b26cff4798fb87d4284348642125272b0e13f0694ce40235595df80a1f34eb",
      "target-not-reachable",
      "Test build helper invokes Apple-local swift/xcrun/clang tools and is not reachable on the Windows/Linux production targets.",
    ],
    "syn@1.0.109/build-script-build": [
      "f5f38a36ae6ee55ec0f6ef6d231ce2fba2d6ab60af11f85a1743dd8929c3ad04",
      "local-compiler-probe",
      "Invokes local rustc for cfg probing.",
    ],
    "target-lexicon@0.12.16/build-script-build": [
      "f5ad69b3b8bc34d39cce634cde6f37929c68d4c14c70fb831da5ba0a7b77d687",
      "literal-include-and-compiler-probe",
      "Literal include files are closure-hashed; the only process is local rustc --version.",
    ],
    "tauri-macros@2.6.3/tauri_macros": [
      "b32735576f7002974f48e9dd2ee1e58b36d7e7b7513e5b516dbf62aa106c8ab9",
      "local-compiler-probe",
      "Production proc-macro wrapper invokes only local rustc -V to resolve the compiler host triple.",
    ],
    "thiserror@1.0.69/build-script-build": [
      "0ff063a455c9d66a339c0ae57e3c16c6d90e2a9815c40218831dfffdd2a8d375",
      "local-compiler-probe",
      "Invokes the configured local rustc or wrapper.",
    ],
    "thiserror@2.0.20/build-script-build": [
      "dc22d0da5d4cf72182323216b02c8c4a4579a13d7a327c2f068f9d7a95d72648",
      "local-compiler-probe",
      "Invokes configured local rustc for version/cfg probing.",
    ],
    "typeid@1.0.3/build-script-build": [
      "269a595cf79983856aa225f7adc9e02de02bd4144829355d7d59240d6d0575ad",
      "local-compiler-probe",
      "Invokes local rustc --version.",
    ],
    "wasm-bindgen-shared@0.2.127/build-script-build": [
      "17cf93d0b0acf5a4fac94b52879e4fb62fb9a820c29918b22242d1b848934c26",
      "local-vcs-metadata",
      "Invokes local git rev-parse only; no fetch, pull, submodule, or remote operation.",
    ],
    "windows-implement@0.60.2/windows_implement": [
      "375593c6208a1cc9f2e98b6d508f43c1e308a98d1711bc91fc1eb8e56a970a46",
      "test-only",
      "Invokes local rustfmt only from the crate test module.",
    ],
    "zmij@1.0.23/build-script-build": [
      "c1c14ac38e8c2d6bde487b3b5db698a42831e7297e4c1ca7db8a4feec9f79398",
      "local-compiler-probe",
      "Invokes local rustc --version.",
    ],
  }).map(([key, [sourceClosureSha256, classification, rationale]]) => [
    key,
    { sourceClosureSha256, classification, rationale },
  ]),
);

function parseCargoLockPackages(contents) {
  const packages = new Map();
  for (const block of contents.split("[[package]]").slice(1)) {
    const value = (field) => new RegExp(`^${field} = "([^"]+)"$`, "m").exec(block)?.[1];
    const name = value("name");
    const version = value("version");
    const source = value("source");
    const checksum = value("checksum");
    if (name && version && source) {
      packages.set(`${name}@${version}@${source}`, { source, checksum: checksum ?? null });
    }
  }
  return packages;
}

const cargoLockPackages = parseCargoLockPackages(cargoLockText);

const npmPackages = Object.entries(lock.packages)
  .filter(([path, entry]) => path.includes("node_modules/") && entry.link !== true)
  .map(([path, entry]) => {
    const packageJsonPath = resolve(root, path, "package.json");
    const packageJson = existsSync(packageJsonPath)
      ? JSON.parse(readFileSync(packageJsonPath, "utf8"))
      : {};
    const inferredName = path.slice(path.lastIndexOf("node_modules/") + "node_modules/".length);
    return {
      name: packageJson.name ?? inferredName,
      version: entry.version,
      license: packageJson.license ?? entry.license ?? "UNKNOWN",
      provenance: entry.resolved?.startsWith(APPROVED_NPM_REGISTRY_PREFIX)
        ? "registry.npmjs.org"
        : (entry.resolved ?? "workspace/optional-platform"),
      integrity: entry.integrity ?? null,
      lifecycleScripts: entry.hasInstallScript
        ? Object.keys(packageJson.scripts ?? { install: "lockfile-declared" }).filter((name) =>
            ["preinstall", "install", "postinstall"].includes(name),
          )
        : [],
      optional: entry.optional === true,
      development: entry.dev === true,
    };
  })
  .sort((left, right) =>
    `${left.name}@${left.version}`.localeCompare(`${right.name}@${right.version}`),
  );

const cargo = JSON.parse(
  execFileSync("cargo", ["metadata", "--locked", "--format-version", "1"], {
    cwd: root,
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
    stdio: ["ignore", "pipe", "ignore"],
  }),
);

const workspaceMembers = new Set(cargo.workspace_members);
const cargoPackageById = new Map(cargo.packages.map((entry) => [entry.id, entry]));
const cargoResolveNodeById = new Map(cargo.resolve.nodes.map((entry) => [entry.id, entry]));

function collectRustSourceClosure(entrypoint) {
  const pending = [entrypoint];
  const visited = new Set();
  while (pending.length > 0) {
    const file = resolve(pending.pop());
    if (visited.has(file) || !existsSync(file)) continue;
    visited.add(file);
    const source = readFileSync(file, "utf8");
    const base = dirname(file);
    for (const match of source.matchAll(
      /(?:^|\n)\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z0-9_]+)\s*;/g,
    )) {
      const sibling = resolve(base, `${match[1]}.rs`);
      const nested = resolve(base, match[1], "mod.rs");
      if (existsSync(sibling)) pending.push(sibling);
      else if (existsSync(nested)) pending.push(nested);
    }
    for (const match of source.matchAll(/include!\s*\(\s*"([^"]+)"\s*\)/g)) {
      const included = resolve(base, match[1]);
      if (existsSync(included)) pending.push(included);
    }
  }
  return [...visited].sort();
}

function relevantExecutableDependencies(packageId, targetKind) {
  const node = cargoResolveNodeById.get(packageId);
  if (!node) return [];
  return node.deps
    .filter((dependency) =>
      dependency.dep_kinds.some(({ kind }) =>
        targetKind.includes("custom-build") ? kind === "build" : kind === null,
      ),
    )
    .map((dependency) => cargoPackageById.get(dependency.pkg))
    .filter(Boolean)
    .map((dependency) => `${dependency.name}@${dependency.version}`)
    .sort();
}

const executableSignalPatterns = [
  ["process-spawn", /\b(?:std::process::Command|Command::new)\b/g],
  ["native-compiler", /\bcc::Build::new\b/g],
  ["dynamic-include", /\binclude!\s*\(/g],
];

function executableSignalOccurrences(sourceFiles, sources, packageRoot) {
  const occurrences = [];
  sourceFiles.forEach((file, fileIndex) => {
    const lines = sources[fileIndex].split(/\r?\n/);
    lines.forEach((line, lineIndex) => {
      for (const [signal, pattern] of executableSignalPatterns) {
        pattern.lastIndex = 0;
        if (pattern.test(line)) {
          occurrences.push({
            signal,
            file: relative(packageRoot, file).replaceAll("\\", "/"),
            line: lineIndex + 1,
            source: line.trim().slice(0, 240),
          });
        }
      }
    });
  });
  return occurrences;
}

const cargoPackages = cargo.packages
  .filter((entry) => !workspaceMembers.has(entry.id))
  .map((entry) => {
    const lockEntry = cargoLockPackages.get(`${entry.name}@${entry.version}@${entry.source}`);
    const executableTargets = entry.targets
      .filter(
        (target) => target.kind.includes("custom-build") || target.kind.includes("proc-macro"),
      )
      .map((target) => {
        const sourceFiles = collectRustSourceClosure(target.src_path);
        const sources = sourceFiles.map((file) => readFileSync(file, "utf8"));
        const source = sources.join("\n");
        const prohibitedBuildSignals = [
          [
            "network-api",
            /\b(?:std::net|tokio::net|TcpStream|TcpListener|UdpSocket|reqwest|ureq|hyper::client)\b/,
          ],
          ["telemetry", /\btelemetry\b/i],
          ["updater", /\b(?:auto.?update|self.?update|updater)\b/i],
        ]
          .filter(([, pattern]) => pattern.test(source))
          .map(([name]) => name);
        const packageRoot = dirname(entry.manifest_path);
        const executionSignalOccurrences = executableSignalOccurrences(
          sourceFiles,
          sources,
          packageRoot,
        );
        const executionSignals = [
          ...new Set(executionSignalOccurrences.map(({ signal }) => signal)),
        ];
        const sourceClosure = sourceFiles.map((file) =>
          relative(packageRoot, file).replaceAll("\\", "/"),
        );
        const closureSha256 = createHash("sha256")
          .update(
            sourceFiles
              .map((file, index) => `${sourceClosure[index]}\0${sources[index]}\0`)
              .join(""),
          )
          .digest("hex");
        const dispositionKey = `${entry.name}@${entry.version}/${target.name}`;
        const reviewedDisposition = executableSignalDispositions.get(dispositionKey);
        const disposition =
          executionSignals.length === 0
            ? {
                classification: "no-execution-signal",
                rationale:
                  "No process-spawn, native-compiler, or include execution signal in the hashed source closure.",
              }
            : reviewedDisposition?.sourceClosureSha256 === closureSha256
              ? reviewedDisposition
              : null;
        return {
          name: target.name,
          kinds: target.kind,
          sourceClosure,
          sourceClosureSha256: closureSha256,
          relevantDependencies: relevantExecutableDependencies(entry.id, target.kind),
          prohibitedBuildSignals,
          executionSignals,
          executionSignalOccurrences,
          disposition,
        };
      });
    return {
      name: entry.name,
      version: entry.version,
      license: entry.license ?? "UNKNOWN",
      provenance: entry.source === APPROVED_CARGO_SOURCE ? "crates.io" : (entry.source ?? "path"),
      checksum: lockEntry?.checksum ?? null,
      executableTargets,
    };
  })
  .sort((left, right) =>
    `${left.name}@${left.version}`.localeCompare(`${right.name}@${right.version}`),
  );

const acceptedLicenseExpressions = new Set([
  "(MIT OR Apache-2.0) AND Unicode-3.0",
  "0BSD OR MIT OR Apache-2.0",
  "Apache-2.0",
  "Apache-2.0 / MIT",
  "Apache-2.0 AND MIT",
  // Each of the next three offers Apache-2.0 or MIT, which PMC elects (the
  // age archive dependencies of ADR 0010, accepted 2026-09-21).
  "Apache-2.0 OR GPL-2.0-only",
  "BSD-2-Clause OR Apache-2.0 OR MIT",
  "MIT OR Apache-2.0 OR BSD-1-Clause",
  "Apache-2.0 OR MIT",
  "Apache-2.0 WITH LLVM-exception",
  "Apache-2.0 WITH LLVM-exception OR Apache-2.0 OR MIT",
  "Apache-2.0/MIT",
  "BlueOak-1.0.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "BSD-3-Clause AND MIT",
  "BSD-3-Clause OR MIT OR Apache-2.0",
  "BSD-3-Clause/MIT",
  "CC-BY-4.0",
  "CC0-1.0",
  "CC0-1.0 OR MIT-0 OR Apache-2.0",
  "ISC",
  "MIT",
  "MIT OR Apache-2.0",
  "MIT OR Apache-2.0 OR LGPL-2.1-or-later",
  "MIT OR Apache-2.0 OR Zlib",
  "MIT OR Zlib OR Apache-2.0",
  "MIT-0",
  "MIT/Apache-2.0",
  "MPL-2.0",
  "Python-2.0",
  "Unicode-3.0",
  "Unlicense OR MIT",
  "Unlicense/MIT",
  "Zlib",
  "Zlib OR Apache-2.0 OR MIT",
]);

const unknownLicenses = [
  ...npmPackages
    .filter((entry) => entry.license === "UNKNOWN")
    .map((entry) => `npm:${entry.name}@${entry.version}`),
  ...cargoPackages
    .filter((entry) => entry.license === "UNKNOWN")
    .map((entry) => `cargo:${entry.name}@${entry.version}`),
];
const unapprovedLicenses = [...npmPackages, ...cargoPackages]
  .filter((entry) => !acceptedLicenseExpressions.has(entry.license))
  .map((entry) => `${entry.name}@${entry.version}:${entry.license}`);
const invalidNpmProvenance = npmPackages
  .filter(
    (entry) =>
      entry.provenance !== "registry.npmjs.org" ||
      typeof entry.integrity !== "string" ||
      !entry.integrity.startsWith("sha512-"),
  )
  .map((entry) => `${entry.name}@${entry.version}`);
const invalidCargoProvenance = cargoPackages
  .filter(
    (entry) => entry.provenance !== "crates.io" || !/^[a-f0-9]{64}$/.test(entry.checksum ?? ""),
  )
  .map((entry) => `${entry.name}@${entry.version}`);
const prohibitedCargoBuildSignals = cargoPackages.flatMap((entry) =>
  entry.executableTargets
    .filter((target) => target.prohibitedBuildSignals.length > 0)
    .map(
      (target) =>
        `${entry.name}@${entry.version}:${target.name}:${target.prohibitedBuildSignals.join("+")}`,
    ),
);
const missingExecutableDispositions = cargoPackages.flatMap((entry) =>
  entry.executableTargets
    .filter((target) => target.executionSignals.length > 0 && target.disposition === null)
    .map((target) => `${entry.name}@${entry.version}/${target.name}@${target.sourceClosureSha256}`),
);
const cargoCustomBuildCount = cargoPackages.reduce(
  (count, entry) =>
    count +
    entry.executableTargets.filter((target) => target.kinds.includes("custom-build")).length,
  0,
);
const cargoProcMacroCount = cargoPackages.reduce(
  (count, entry) =>
    count + entry.executableTargets.filter((target) => target.kinds.includes("proc-macro")).length,
  0,
);

const evidence = {
  schemaVersion: 3,
  generatedFrom: ["package-lock.json", "Cargo.lock"],
  policy: {
    acceptedLicenseExpressions: [...acceptedLicenseExpressions].sort(),
    unknownLicenses,
    unapprovedLicenses,
    invalidNpmProvenance,
    invalidCargoProvenance,
    prohibitedCargoBuildSignals,
    missingExecutableDispositions,
    cargoExecutableTargetDisposition: {
      customBuildCount: cargoCustomBuildCount,
      procMacroCount: cargoProcMacroCount,
      review:
        "Every executable target is inventory-listed below. Acceptance requires locked crates.io provenance, Cargo.lock checksum, approved license, and no network, telemetry, or updater API signal across its statically reachable Rust source closure. Each target records a closure hash, execution signals, and relevant build/runtime dependencies.",
    },
    lifecycleDisposition:
      "Only optional development dependency fsevents may declare an install lifecycle script; it is not installed on Windows and is accepted as a macOS file-watcher adapter.",
  },
  npm: npmPackages,
  cargo: cargoPackages,
};

if (unknownLicenses.length > 0) {
  throw new Error(`Unknown dependency licenses: ${unknownLicenses.join(", ")}`);
}

if (unapprovedLicenses.length > 0) {
  throw new Error(`Unapproved dependency licenses: ${unapprovedLicenses.join(", ")}`);
}

if (invalidNpmProvenance.length > 0 || invalidCargoProvenance.length > 0) {
  throw new Error(
    `Unapproved dependency provenance: npm=${invalidNpmProvenance.join(",")} cargo=${invalidCargoProvenance.join(",")}`,
  );
}

if (prohibitedCargoBuildSignals.length > 0) {
  throw new Error(
    `Cargo executable target review failed: ${prohibitedCargoBuildSignals.join(", ")}`,
  );
}

if (missingExecutableDispositions.length > 0) {
  throw new Error(
    `Missing or stale executable dispositions: ${missingExecutableDispositions.join(", ")}`,
  );
}

const lifecyclePackages = npmPackages.filter((entry) => entry.lifecycleScripts.length > 0);
if (
  lifecyclePackages.some(
    (entry) => entry.name !== "fsevents" || !entry.optional || !entry.development,
  )
) {
  throw new Error(`Unreviewed lifecycle scripts: ${JSON.stringify(lifecyclePackages)}`);
}

const serializedEvidence = `${JSON.stringify(evidence, null, 2)}\n`;
if (process.argv.includes("--write")) {
  writeFileSync(resolve(root, "docs/evidence/s1/dependency-evidence.json"), serializedEvidence);
  process.stdout.write("Updated docs/evidence/s1/dependency-evidence.json\n");
} else {
  process.stdout.write(serializedEvidence);
}
