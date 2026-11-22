#!/usr/bin/env node
"use strict";

const fs = require("node:fs");
const path = require("node:path");

/**
 * Extract `version = "..."` from the `[workspace.package]` section of Cargo.toml.
 */
function parseWorkspaceVersion(cargoTomlText) {
  const lines = cargoTomlText.split(/\r?\n/);
  let inWorkspacePackage = false;

  for (const rawLine of lines) {
    const line = rawLine.replace(/#.*$/, "").trim();
    if (!line) continue;

    if (line.startsWith("[")) {
      inWorkspacePackage = line === "[workspace.package]";
      continue;
    }

    if (inWorkspacePackage) {
      const match = line.match(/^version\s*=\s*"([^"]+)"\s*$/);
      if (match) {
        return match[1].trim();
      }
    }
  }

  throw new Error("Could not find [workspace.package].version in Cargo.toml");
}

/**
 * Synchronize or check `package.json` version against `Cargo.toml` `[workspace.package].version`.
 */
function syncVersion({
  rootDir = path.resolve(__dirname, ".."),
  check = false,
  cargoTomlPath,
  packageJsonPath,
} = {}) {
  const cargoPath = cargoTomlPath || path.join(rootDir, "Cargo.toml");
  const pkgPath = packageJsonPath || path.join(rootDir, "package.json");

  if (!fs.existsSync(cargoPath)) {
    throw new Error(`Missing Cargo.toml at ${cargoPath}`);
  }
  if (!fs.existsSync(pkgPath)) {
    throw new Error(`Missing package.json at ${pkgPath}`);
  }

  const cargoText = fs.readFileSync(cargoPath, "utf8");
  const cargoVersion = parseWorkspaceVersion(cargoText);

  const pkgText = fs.readFileSync(pkgPath, "utf8");
  const pkg = JSON.parse(pkgText);
  const pkgVersion = pkg.version;

  if (!pkgVersion || typeof pkgVersion !== "string") {
    throw new Error(`package.json at ${pkgPath} is missing a string "version" field`);
  }

  if (check) {
    if (pkgVersion !== cargoVersion) {
      throw new Error(
        `Version mismatch: Cargo.toml workspace.package.version is "${cargoVersion}", but package.json version is "${pkgVersion}". Run "node scripts/sync-version.js" to update package.json.`
      );
    }
    return {
      version: cargoVersion,
      previousPackageVersion: pkgVersion,
      changed: false,
    };
  }

  if (pkgVersion === cargoVersion) {
    return {
      version: cargoVersion,
      previousPackageVersion: pkgVersion,
      changed: false,
    };
  }

  pkg.version = cargoVersion;
  const eol = pkgText.includes("\r\n") ? "\r\n" : "\n";
  const formatted = JSON.stringify(pkg, null, 2).replace(/\n/g, eol) + eol;
  fs.writeFileSync(pkgPath, formatted, "utf8");

  return {
    version: cargoVersion,
    previousPackageVersion: pkgVersion,
    changed: true,
  };
}

function main() {
  const args = process.argv.slice(2);
  const check = args.includes("--check") || args.includes("--verify");

  try {
    const result = syncVersion({ check });
    if (check) {
      console.log(`Version check passed: ${result.version} (Cargo.toml == package.json)`);
    } else if (result.changed) {
      console.log(
        `Updated package.json version: ${result.previousPackageVersion} -> ${result.version}`
      );
    } else {
      console.log(`package.json version already matches Cargo.toml (${result.version})`);
    }
  } catch (err) {
    console.error(`error: ${err.message}`);
    process.exit(1);
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  parseWorkspaceVersion,
  syncVersion,
};
