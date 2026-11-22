"use strict";

const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { describe, it } = require("node:test");

const { parseWorkspaceVersion, syncVersion } = require("./sync-version");
const { verifyPackageDir } = require("./verify-package");

const ROOT_DIR = path.resolve(__dirname, "..");

function createFakeExecutable(filePath, nodeScriptSource) {
  if (process.platform === "win32") {
    // Compile a tiny standalone .cmd? No, Windows CreateProcess requires a real PE .exe
    // when invoked directly by node without shell: true. Instead, we can copy node.exe?
    // Wait: node.exe ignores argv0, so better: compile a tiny C# or PowerShell? No, use Rust
    // or copy target/debug/wavical.exe? No, we can test with a real compiled fake helper or
    // check how fake binaries work on Windows!
    throw new Error("createFakeExecutable requires a compiled binary on Windows");
  }
  fs.writeFileSync(filePath, `#!/usr/bin/env node\n${nodeScriptSource}\n`, {
    mode: 0o755,
  });
}

describe("scripts/sync-version.js", () => {
  it("parses [workspace.package] version from Cargo.toml", () => {
    const sample = `
[workspace]
members = ["crates/a"]

[workspace.package]
version = "1.2.3"
edition = "2024"
`;
    assert.equal(parseWorkspaceVersion(sample), "1.2.3");
  });

  it("fails --check when Cargo.toml and package.json differ, and syncs when written", () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "wavical-sync-test-"));
    try {
      const cargoPath = path.join(tmp, "Cargo.toml");
      const pkgPath = path.join(tmp, "package.json");

      fs.writeFileSync(
        cargoPath,
        '[workspace.package]\nversion = "1.1.1"\nedition = "2024"\n',
        "utf8"
      );
      fs.writeFileSync(
        pkgPath,
        JSON.stringify({ name: "wavical", version: "1.1.0" }, null, 2) + "\n",
        "utf8"
      );

      assert.throws(
        () =>
          syncVersion({
            rootDir: tmp,
            cargoTomlPath: cargoPath,
            packageJsonPath: pkgPath,
            check: true,
          }),
        /Version mismatch.*1\.1\.1.*1\.1\.0/
      );

      const synced = syncVersion({
        rootDir: tmp,
        cargoTomlPath: cargoPath,
        packageJsonPath: pkgPath,
        check: false,
      });
      assert.equal(synced.changed, true);
      assert.equal(synced.version, "1.1.1");

      const checked = syncVersion({
        rootDir: tmp,
        cargoTomlPath: cargoPath,
        packageJsonPath: pkgPath,
        check: true,
      });
      assert.equal(checked.changed, false);
      assert.equal(checked.version, "1.1.1");
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  });
});

describe("scripts/verify-package.js", () => {
  it("fails on a fake bin folder with an old --help output", () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "wavical-verify-fake-"));
    try {
      const binDir = path.join(tmp, "bin");
      fs.mkdirSync(binDir, { recursive: true });

      const pkgJson = {
        name: "wavical",
        version: "1.1.1",
        bin: { wavical: "bin/wavical.js", wca: "bin/wavical.js" },
      };
      fs.writeFileSync(
        path.join(tmp, "package.json"),
        JSON.stringify(pkgJson, null, 2),
        "utf8"
      );
      fs.writeFileSync(path.join(binDir, "wavical.js"), "// wrapper\n", "utf8");
      fs.copyFileSync(
        path.join(ROOT_DIR, "README.md"),
        path.join(tmp, "README.md")
      );
      fs.copyFileSync(
        path.join(ROOT_DIR, "CHANGELOG.md"),
        path.join(tmp, "CHANGELOG.md")
      );

      const cliName = process.platform === "win32" ? "wavical.exe" : "wavical";
      const shellName =
        process.platform === "win32" ? "wavical-shell.exe" : "wavical-shell";
      const cliPath = path.join(binDir, cliName);
      const shellPath = path.join(binDir, shellName);

      if (process.platform === "win32") {
        // Build tiny fake Rust/C#? No, use rustc on a 10-line .rs snippet!
        const fakeCliRs = path.join(tmp, "fake_cli.rs");
        const fakeShellRs = path.join(tmp, "fake_shell.rs");
        fs.writeFileSync(
          fakeCliRs,
          `fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version") {
        println!("wavical 1.1.1 (deadbee)");
    } else {
        println!("Usage: wavical <COMMAND>\\n\\nCommands:\\n  init\\n  dev\\n  build\\n  help");
    }
}\n`,
          "utf8"
        );
        fs.writeFileSync(
          fakeShellRs,
          `fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version") {
        println!("wavical-shell 1.1.1 (deadbee)");
    } else {
        println!("Options:\\n  --app-id <APP_ID>");
    }
}\n`,
          "utf8"
        );
        execFileSync("rustc", [fakeCliRs, "-o", cliPath], { stdio: "pipe" });
        execFileSync("rustc", [fakeShellRs, "-o", shellPath], { stdio: "pipe" });
      } else {
        createFakeExecutable(
          cliPath,
          `if (process.argv.includes('--version')) console.log('wavical 1.1.1 (deadbee)');
else console.log('Usage: wavical <COMMAND>\\n\\nCommands:\\n  init\\n  dev\\n  build\\n  help');`
        );
        createFakeExecutable(
          shellPath,
          `if (process.argv.includes('--version')) console.log('wavical-shell 1.1.1 (deadbee)');
else console.log('Options:\\n  --app-id <APP_ID>');`
        );
      }

      assert.throws(
        () =>
          verifyPackageDir({
            packageDir: tmp,
            expectedCommit: "deadbee",
            writeChecksums: false,
            skipPreviousCheck: true,
          }),
        /missing required command "dev-safe"/
      );
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  });

  it("passes on the real built release binaries when present in target/release", () => {
    const releaseCli = path.join(
      ROOT_DIR,
      "target",
      "release",
      process.platform === "win32" ? "wavical.exe" : "wavical"
    );
    const releaseShell = path.join(
      ROOT_DIR,
      "target",
      "release",
      process.platform === "win32" ? "wavical-shell.exe" : "wavical-shell"
    );

    if (!fs.existsSync(releaseCli) || !fs.existsSync(releaseShell)) {
      // Skip only if release binaries have not been built yet in this environment
      return;
    }

    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "wavical-verify-real-"));
    try {
      const binDir = path.join(tmp, "bin");
      fs.mkdirSync(binDir, { recursive: true });
      fs.copyFileSync(
        path.join(ROOT_DIR, "package.json"),
        path.join(tmp, "package.json")
      );
      fs.copyFileSync(
        path.join(ROOT_DIR, "bin", "wavical.js"),
        path.join(binDir, "wavical.js")
      );
      fs.copyFileSync(
        path.join(ROOT_DIR, "README.md"),
        path.join(tmp, "README.md")
      );
      fs.copyFileSync(
        path.join(ROOT_DIR, "CHANGELOG.md"),
        path.join(tmp, "CHANGELOG.md")
      );
      fs.copyFileSync(releaseCli, path.join(binDir, path.basename(releaseCli)));
      fs.copyFileSync(
        releaseShell,
        path.join(binDir, path.basename(releaseShell))
      );

      const commit = execFileSync("git", ["rev-parse", "--short", "HEAD"], {
        cwd: ROOT_DIR,
        encoding: "utf8",
      }).trim();

      const res = verifyPackageDir({
        packageDir: tmp,
        expectedCommit: commit,
        allowDirty: true,
        writeChecksums: true,
        skipPreviousCheck: true,
      });

      assert.equal(res.version, "1.1.1");
      assert.ok(fs.existsSync(path.join(binDir, "checksums.txt")));
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  });
});
