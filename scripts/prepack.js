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

/**
 * Return the separate cargo build invocations required so Cargo feature unification
 * never leaks `dev-shell` into the production `wavical-shell` binary.
 */
function getPrepackBuildInvocations() {
  return [
    {
      label: "CLI (wavical)",
      cmd: "cargo",
      args: ["build", "--release", "--locked", "-p", "wavical-cli", "--bin", "wavical"],
      srcRelDir: path.join("target", "release"),
      binBase: "wavical",
    },
    {
      label: "Production shell (wavical-shell)",
      cmd: "cargo",
      args: ["build", "--release", "--locked", "-p", "wavical-shell", "--bin", "wavical-shell"],
      srcRelDir: path.join("target", "release"),
      binBase: "wavical-shell",
    },
    {
      label: "Dev shell (wavical-shell-dev)",
      cmd: "cargo",
      args: [
        "build",
        "--release",
        "--locked",
        "-p",
        "wavical-shell",
        "--bin",
        "wavical-shell-dev",
        "--features",
        "dev-shell",
        "--target-dir",
        "target/dev-shell",
      ],
      srcRelDir: path.join("target", "dev-shell", "release"),
      binBase: "wavical-shell-dev",
    },
  ];
}

/**
 * Guard verifying that production and dev shells are never built in the same cargo invocation,
 * that production does not enable `dev-shell`, and that dev shell uses `--target-dir target/dev-shell`.
 */
function validatePrepackBuildInvocations(invocations) {
  let sawProdShell = false;
  let sawDevShell = false;

  for (const inv of invocations) {
    const args = inv.args || [];
    const hasProd = args.some((a, i) => a === "--bin" && args[i + 1] === "wavical-shell");
    const hasDev = args.some((a, i) => a === "--bin" && args[i + 1] === "wavical-shell-dev");
    const hasDevFeature = args.some(
      (a, i) => a === "--features" && typeof args[i + 1] === "string" && args[i + 1].split(",").includes("dev-shell")
    );
    const hasForbiddenTestFeature = args.some(
      (a, i) =>
        a === "--features" &&
        typeof args[i + 1] === "string" &&
        args[i + 1]
          .split(",")
          .some((f) => f === "test-hooks" || f === "test-probe")
    );
    const targetDirIdx = args.indexOf("--target-dir");
    const targetDir = targetDirIdx >= 0 ? args[targetDirIdx + 1] : null;

    if (hasForbiddenTestFeature) {
      throw new Error(
        "Invalid build plan: shipped binaries must NEVER enable test-hooks or test-probe features"
      );
    }

    if (hasProd && hasDev) {
      throw new Error(
        "Invalid build plan: wavical-shell and wavical-shell-dev must not be built in one cargo invocation (would unify dev-shell feature)"
      );
    }
    if (hasProd) {
      if (hasDevFeature) {
        throw new Error(
          "Invalid build plan: production wavical-shell must not enable dev-shell feature"
        );
      }
      sawProdShell = true;
    }
    if (hasDev) {
      if (!hasDevFeature) {
        throw new Error(
          "Invalid build plan: wavical-shell-dev must enable --features dev-shell"
        );
      }
      if (!targetDir || targetDir.replace(/\\/g, "/") !== "target/dev-shell") {
        throw new Error(
          `Invalid build plan: wavical-shell-dev must use --target-dir target/dev-shell (got ${targetDir})`
        );
      }
      sawDevShell = true;
    }
  }

  if (!sawProdShell || !sawDevShell) {
    throw new Error(
      "Invalid build plan: must include separate builds for wavical-shell and wavical-shell-dev"
    );
  }
}

function prepack({
  rootDir = path.resolve(__dirname, ".."),
  strict = false,
  allowDirty = process.env.WAVICAL_ALLOW_DIRTY === "1",
  npmCommand = process.env.npm_command,
  runCheckedFn = runChecked,
  verifyPackageFn = verifyPackageDir,
} = {}) {
  if (npmCommand === "publish") {
    throw new Error(
      'Refusing to rebuild during "npm publish". Build once with "npm pack", run A10 on that .tgz, and then publish with "npm publish <that>.tgz".'
    );
  }

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

  // b. Build release binaries in separate cargo invocations with separate target dirs
  const invocations = getPrepackBuildInvocations();
  validatePrepackBuildInvocations(invocations);

  for (const inv of invocations) {
    console.log(`Building ${inv.label} (commit ${commit}, dirty=${isDirty})...`);
    runCheckedFn(inv.cmd, inv.args, {
      cwd: rootDir,
      env: {
        ...process.env,
        WAVICAL_GIT_COMMIT: commit,
        WAVICAL_GIT_DIRTY: isDirty ? "true" : "false",
      },
    });
  }

  // c. Copy fresh outputs into bin/
  const binDir = path.join(rootDir, "bin");
  fs.mkdirSync(binDir, { recursive: true });

  for (const inv of invocations) {
    const filename = binaryFilename(inv.binBase);
    const src = path.join(rootDir, inv.srcRelDir, filename);
    const dst = path.join(binDir, filename);
    if (!fs.existsSync(src)) {
      throw new Error(`Expected freshly built release binary missing: ${src}`);
    }
    fs.copyFileSync(src, dst);
    console.log(`Copied ${src} -> ${dst}`);
  }

  // d. Run full package verification against bin/ (A10 report does not exist until after npm pack)
  verifyPackageFn({
    packageDir: rootDir,
    expectedCommit: commit,
    allowDirty: allowDirty && !strict,
    writeChecksums: true,
    skipA10Check: true,
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
  getPrepackBuildInvocations,
  prepack,
  validatePrepackBuildInvocations,
};
