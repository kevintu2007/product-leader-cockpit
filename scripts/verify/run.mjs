import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const checks = [
  ["foundation", "python", ["scripts/validate-foundation.py", "."]],
  ["failure-probe", "node", ["scripts/verify/failure-probe.mjs"]],
  ["prettier", "npm", ["run", "format:check"]],
  ["eslint", "npm", ["run", "lint"]],
  ["typescript", "npm", ["run", "typecheck"]],
  ["typescript-tests", "npm", ["run", "test"]],
  ["frontend-build", "npm", ["run", "build"]],
  ["cargo-lock", "cargo", ["metadata", "--locked", "--no-deps", "--format-version", "1"]],
  ["rustfmt", "cargo", ["fmt", "--check"]],
  ["cargo-fetch", "cargo", ["fetch", "--locked"]],
  ["cargo-offline-build", "cargo", ["build", "--workspace", "--locked", "--offline"]],
  [
    "clippy",
    "cargo",
    ["clippy", "--workspace", "--locked", "--all-targets", "--", "-D", "warnings"],
  ],
  ["rust-tests", "cargo", ["test", "--workspace", "--locked"]],
  ["policy", "npm", ["run", "policy:check"]],
];

function run(command, args) {
  if (process.platform !== "win32") {
    return spawnSync(command, args, { stdio: "inherit" });
  }

  const commandLine = [command, ...args].join(" ");
  return spawnSync(process.env.ComSpec ?? "cmd.exe", ["/d", "/s", "/c", commandLine], {
    stdio: "inherit",
  });
}

function canonicalJson(value) {
  if (Array.isArray(value)) {
    return value.map(canonicalJson);
  }
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, entry]) => [key, canonicalJson(entry)]),
    );
  }
  return value;
}

function verifyDependencyEvidence() {
  console.log("\n[verify:dependency-evidence] node scripts/verify/dependency-evidence.mjs");
  const result = spawnSync(process.execPath, ["scripts/verify/dependency-evidence.mjs"], {
    cwd: resolve(import.meta.dirname, "..", ".."),
    encoding: "utf8",
    maxBuffer: 32 * 1024 * 1024,
  });
  if (result.error) {
    console.error(`[verify:dependency-evidence] could not start: ${result.error.message}`);
    return false;
  }
  if (result.status !== 0) {
    if (result.stderr) {
      process.stderr.write(result.stderr);
    }
    console.error(`[verify:dependency-evidence] failed with status ${String(result.status)}`);
    return false;
  }

  const evidencePath = resolve(
    import.meta.dirname,
    "..",
    "..",
    "docs",
    "evidence",
    "s1",
    "dependency-evidence.json",
  );
  try {
    const generated = canonicalJson(JSON.parse(result.stdout));
    const committed = canonicalJson(JSON.parse(readFileSync(evidencePath, "utf8")));
    if (JSON.stringify(generated) !== JSON.stringify(committed)) {
      console.error(`[verify:dependency-evidence] generated evidence differs from ${evidencePath}`);
      return false;
    }
  } catch (error) {
    console.error(`[verify:dependency-evidence] invalid evidence: ${String(error)}`);
    return false;
  }
  console.log("[verify:dependency-evidence] passed");
  return true;
}

for (const [name, command, args] of checks) {
  if (name === "policy") {
    if (!verifyDependencyEvidence()) {
      process.exit(1);
    }
  }
  console.log(`\n[verify:${name}] ${command} ${args.join(" ")}`);
  const result = run(command, args);
  if (result.error) {
    console.error(`[verify:${name}] could not start: ${result.error.message}`);
    process.exit(1);
  }
  if (result.status !== 0) {
    console.error(`[verify:${name}] failed with status ${String(result.status)}`);
    process.exit(result.status ?? 1);
  }
  console.log(`[verify:${name}] passed`);
}

console.log("\n[verify] all required checks passed");
