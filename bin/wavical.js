#!/usr/bin/env node
"use strict";

const { spawn, spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const exeName = process.platform === "win32" ? "wavical.exe" : "wavical";
const bundledBin = path.join(__dirname, exeName);
const command = fs.existsSync(bundledBin) ? bundledBin : exeName;
const args = process.argv.slice(2);

const hasHelpOrVersion = args.some(
  (a) => a === "--help" || a === "-h" || a === "--version" || a === "-V"
);
const firstPositional = args.find((a) => !a.startsWith("-"));
const isDevCommand =
  !hasHelpOrVersion &&
  (firstPositional === "dev" ||
    firstPositional === "dev-safe" ||
    firstPositional === "dev-ultra-safe");

const childEnv = { ...process.env };
if (isDevCommand) {
  process.stdout.write("Starting Wavical dev\u2026\n");
  childEnv.WAVICAL_DEV_BANNER_PRINTED = "1";
  childEnv.WAVICAL_DEV_WRAPPER_START_MS = String(Date.now());

  // Pre-warm wavical-shell-dev concurrently with wavical.exe so cold OS/AV scans overlap
  // instead of stacking sequentially during `wavical dev`.
  if (!process.env.WAVICAL_DEV_SHELL_BIN) {
    const devShellName =
      process.platform === "win32" ? "wavical-shell-dev.exe" : "wavical-shell-dev";
    const devShellBin = path.join(__dirname, devShellName);
    if (fs.existsSync(devShellBin)) {
      try {
        const warmup = spawn(devShellBin, ["--version"], {
          stdio: "ignore",
          windowsHide: true,
        });
        warmup.on("error", () => {});
        warmup.unref();
      } catch (_) {}
    }
  }
}

const result = spawnSync(command, args, {
  stdio: "inherit",
  env: childEnv,
});

if (result.error) {
  if (result.error.code === "ENOENT") {
    console.error(
      `wavical binary not found (${command}). Build or install it with: cargo install --path crates/wavical-cli`
    );
    process.exit(1);
  }
  console.error(result.error.message);
  process.exit(1);
}

process.exit(result.status === null ? 1 : result.status);
