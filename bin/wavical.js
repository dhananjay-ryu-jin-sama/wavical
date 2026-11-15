#!/usr/bin/env node
"use strict";

const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const exeName = process.platform === "win32" ? "wavical.exe" : "wavical";
const bundledBin = path.join(__dirname, exeName);
const command = fs.existsSync(bundledBin) ? bundledBin : exeName;

const result = spawnSync(command, process.argv.slice(2), {
  stdio: "inherit",
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

