#!/usr/bin/env node
"use strict";

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const { syncVersion } = require("./sync-version");
const { verifyPackageDir } = require("./verify-package");

function binaryFilename(base) {
  return process.platform === "win32" ? `${base}.exe` : base;
}

function runChecked(cmd, args, options = {}) {
  const res = spawnSync(cmd, args, {
    stdio: "inherit",
    windowsHide: true,
    ...options,
  });
  if (res.error) {
    throw new Error(`Failed to execute ${cmd}: ${res.error.message}`);
  }
  if (res.status !== 0) {
    throw new Error(`${cmd} ${args.join(" ")} exited with code ${res.status}`);
  }
}

function getGitStatus(rootDir) {
  const res = spawnSync("git", ["status", "--porcelain"], {
    cwd: rootDir,
    encoding: "utf8",
    windowsHide: true,
  });
  if (res.status !== 0) {
    throw new Error(
      `git status failed (${res.status}): ${(res.stderr || "").trim()}`
    );
  }
  return (res.stdout || "").trim();
}

function getGitShortCommit(rootDir) {
  const res = spawnSync("git", ["rev-parse", "--short", "HEAD"], {
    cwd: rootDir,
    encoding: "utf8",
    windowsHide: true,
  });
  if (res.status !== 0 || !(res.stdout || "").trim()) {
    throw new Error("Could not determine git commit via git rev-parse --short HEAD");
  }
  return res.stdout.trim();
}

function prepack({
  rootDir = path.resolve(__dirname, ".."),
  strict = false,
  allowDirty = process.env.WAVICAL_ALLOW_DIRTY === "1",
} = {}) {
  if (strict && allowDirty) {
    throw new Error(
      "Refusing publish: WAVICAL_ALLOW_DIRTY=1 is only allowed for local dry-run `npm pack`, never for publishing."
    );
  }

  // a. Refuse if the git working tree is dirty
  const dirtyStatus = getGitStatus(rootDir);
  const isDirty = dirtyStatus.length > 0;
  if (isDirty) {
    if (!allowDirty || strict) {
      throw new Error(
        `Refusing pack/publish: git working tree is dirty. Commit or stash changes first (or set WAVICAL_ALLOW_DIRTY=1 for a local dry-run pack only).\n${dirtyStatus}`
      );
    }
    console.warn(
      "========================================================================\n" +
        "WARNING: WAVICAL_ALLOW_DIRTY=1 is set and git working tree is dirty!\n" +
        "Proceeding for LOCAL DRY RUN ONLY. Never publish a dirty package.\n" +
        "========================================================================"
    );
  }

  // Ensure Cargo.toml workspace version and package.json version match
  syncVersion({ rootDir, check: true });

  const commit = getGitShortCommit(rootDir);

  // b. Build release binaries with locked Cargo.lock
  const buildArgs = [
    "build",
    "--release",
    "--locked",
    "--bin",
    "wavical",
    "--bin",
    "wavical-shell",
  ];

  // Include wavical-dev-shell if/when F2 adds it
  const shellCargoToml = fs.readFileSync(
    path.join(rootDir, "crates", "wavical-shell", "Cargo.toml"),
    "utf8"
  );
  const hasDevShell = /name\s*=\s*"wavical-dev-shell"/.test(shellCargoToml);
  if (hasDevShell) {
    buildArgs.push("--bin", "wavical-dev-shell");
  }

  console.log(`Building release binaries (commit ${commit}, dirty=${isDirty})...`);
  runChecked("cargo", buildArgs, {
    cwd: rootDir,
    env: {
      ...process.env,
      WAVICAL_GIT_COMMIT: commit,
      WAVICAL_GIT_DIRTY: isDirty ? "true" : "false",
    },
  });

  // c. Copy fresh outputs into bin/
  const releaseDir = path.join(rootDir, "target", "release");
  const binDir = path.join(rootDir, "bin");
  fs.mkdirSync(binDir, { recursive: true });

  const binaries = ["wavical", "wavical-shell"];
  if (hasDevShell) {
    binaries.push("wavical-dev-shell");
  }

  for (const base of binaries) {
    const filename = binaryFilename(base);
    const src = path.join(releaseDir, filename);
    const dst = path.join(binDir, filename);
    if (!fs.existsSync(src)) {
      throw new Error(`Expected freshly built release binary missing: ${src}`);
    }
    fs.copyFileSync(src, dst);
    console.log(`Copied ${src} -> ${dst}`);
  }

  // d. Run full package verification against bin/
  verifyPackageDir({
    packageDir: rootDir,
    expectedCommit: commit,
    allowDirty: allowDirty && !strict,
    writeChecksums: true,
  });
}

function main() {
  const args = process.argv.slice(2);
  const strict = args.includes("--strict");

  try {
    prepack({ strict });
  } catch (err) {
    console.error(`error: ${err.message}`);
    process.exit(1);
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  prepack,
};
