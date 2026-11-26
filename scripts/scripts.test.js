"use strict";

const assert = require("node:assert/strict");
const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { describe, it } = require("node:test");

const {
  getPrepackBuildInvocations,
  prepack,
  validatePrepackBuildInvocations,
} = require("./prepack");
const { parseWorkspaceVersion, syncVersion } = require("./sync-version");
const {
  DEV_BINARY_MARKERS,
  TEST_HOOKS_BINARY_MARKERS,
  TEST_PROBE_BINARY_MARKERS,
  parseA10HashesFromMarkdown,
  verifyPackageDir,
} = require("./verify-package");

const ROOT_DIR = path.resolve(__dirname, "..");

function createFakeExecutable(filePath, nodeScriptSource) {
  if (process.platform === "win32") {
    throw new Error("createFakeExecutable requires a compiled binary on Windows");
  }
  fs.writeFileSync(filePath, `#!/usr/bin/env node\n${nodeScriptSource}\n`, {
    mode: 0o755,
  });
}

function buildFakeBinSet(
  tmp,
  binDir,
  { cliHelp, prodShellHelp, devShellHelp, prodMarker = "", devMarker = DEV_BINARY_MARKERS[0] }
) {
  const cliName = process.platform === "win32" ? "wavical.exe" : "wavical";
  const shellName =
    process.platform === "win32" ? "wavical-shell.exe" : "wavical-shell";
  const devShellName =
    process.platform === "win32" ? "wavical-shell-dev.exe" : "wavical-shell-dev";

  const cliPath = path.join(binDir, cliName);
  const shellPath = path.join(binDir, shellName);
  const devShellPath = path.join(binDir, devShellName);

  if (process.platform === "win32") {
    const fakeCliRs = path.join(tmp, "fake_cli.rs");
    const fakeShellRs = path.join(tmp, "fake_shell.rs");
    const fakeDevShellRs = path.join(tmp, "fake_dev_shell.rs");

    fs.writeFileSync(
      fakeCliRs,
      `fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--version") {
        println!("wavical 1.2.0 (deadbee)");
    } else {
        println!(${JSON.stringify(cliHelp)});
    }
}\n`,
      "utf8"
    );
    fs.writeFileSync(
      fakeShellRs,
      `fn main() {
    let args: Vec<String> = std::env::args().collect();
    let _marker = ${JSON.stringify(prodMarker)};
    if !_marker.is_empty() && args.iter().any(|a| a == "--never") {
        println!("{}", _marker);
    }
    if args.iter().any(|a| a == "--version") {
        println!("wavical-shell 1.2.0 (deadbee)");
    } else {
        println!(${JSON.stringify(prodShellHelp)});
    }
}\n`,
      "utf8"
    );
    fs.writeFileSync(
      fakeDevShellRs,
      `fn main() {
    let args: Vec<String> = std::env::args().collect();
    let _marker = ${JSON.stringify(devMarker)};
    if args.iter().any(|a| a == "--never") {
        println!("{}", _marker);
    }
    if args.iter().any(|a| a == "--version") {
        println!("wavical-shell-dev 1.2.0 (deadbee)");
    } else {
        println!(${JSON.stringify(devShellHelp)});
    }
}\n`,
      "utf8"
    );

    execFileSync("rustc", [fakeCliRs, "-o", cliPath], { stdio: "pipe" });
    execFileSync("rustc", [fakeShellRs, "-o", shellPath], { stdio: "pipe" });
    execFileSync("rustc", [fakeDevShellRs, "-o", devShellPath], { stdio: "pipe" });
  } else {
    createFakeExecutable(
      cliPath,
      `if (process.argv.includes('--version')) console.log('wavical 1.2.0 (deadbee)');
else console.log(${JSON.stringify(cliHelp)});`
    );
    createFakeExecutable(
      shellPath,
      `const _m = ${JSON.stringify(prodMarker)};
if (process.argv.includes('--version')) console.log('wavical-shell 1.2.0 (deadbee)');
else console.log(${JSON.stringify(prodShellHelp)});`
    );
    createFakeExecutable(
      devShellPath,
      `const _m = ${JSON.stringify(devMarker)};
if (process.argv.includes('--version')) console.log('wavical-shell-dev 1.2.0 (deadbee)');
else console.log(${JSON.stringify(devShellHelp)});`
    );
  }

  return { cliPath, shellPath, devShellPath };
}

function setupTempPkgDir(tmp, { includeLicense = true } = {}) {
  const binDir = path.join(tmp, "bin");
  fs.mkdirSync(binDir, { recursive: true });
  const pkgJson = {
    name: "wavical",
    version: "1.2.0",
    bin: { wavical: "bin/wavical.js", wca: "bin/wavical.js" },
  };
  fs.writeFileSync(
    path.join(tmp, "package.json"),
    JSON.stringify(pkgJson, null, 2),
    "utf8"
  );
  fs.writeFileSync(path.join(binDir, "wavical.js"), "// wrapper\n", "utf8");
  fs.copyFileSync(path.join(ROOT_DIR, "README.md"), path.join(tmp, "README.md"));
  fs.copyFileSync(
    path.join(ROOT_DIR, "CHANGELOG.md"),
    path.join(tmp, "CHANGELOG.md")
  );
  if (includeLicense) {
    fs.copyFileSync(path.join(ROOT_DIR, "LICENSE-MIT"), path.join(tmp, "LICENSE-MIT"));
    fs.copyFileSync(path.join(ROOT_DIR, "LICENSE-APACHE"), path.join(tmp, "LICENSE-APACHE"));
  }
  return binDir;
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
        '[workspace.package]\nversion = "1.2.0"\nedition = "2024"\n',
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
        /Version mismatch.*1\.2\.0.*1\.1\.0/
      );

      const synced = syncVersion({
        rootDir: tmp,
        cargoTomlPath: cargoPath,
        packageJsonPath: pkgPath,
        check: false,
      });
      assert.equal(synced.changed, true);
      assert.equal(synced.version, "1.2.0");

      const checked = syncVersion({
        rootDir: tmp,
        cargoTomlPath: cargoPath,
        packageJsonPath: pkgPath,
        check: true,
      });
      assert.equal(checked.changed, false);
      assert.equal(checked.version, "1.2.0");
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  });
});

describe("scripts/prepack.js", () => {
  it("builds production and dev shells in separate cargo runs with separate target dirs", () => {
    const plan = getPrepackBuildInvocations();
    validatePrepackBuildInvocations(plan);

    const prod = plan.find((p) => p.binBase === "wavical-shell");
    const dev = plan.find((p) => p.binBase === "wavical-shell-dev");
    assert.ok(prod);
    assert.ok(dev);
    assert.ok(!prod.args.includes("dev-shell"));
    assert.ok(!prod.args.includes("wavical-shell-dev"));
    assert.ok(dev.args.includes("--features"));
    assert.ok(dev.args.includes("dev-shell"));
    assert.ok(dev.args.includes("--target-dir"));
    assert.ok(dev.args.includes("target/dev-shell"));
  });

  it("rejects any build plan that combines wavical-shell and wavical-shell-dev in one cargo invocation", () => {
    assert.throws(
      () =>
        validatePrepackBuildInvocations([
          {
            cmd: "cargo",
            args: [
              "build",
              "--release",
              "--bin",
              "wavical-shell",
              "--bin",
              "wavical-shell-dev",
              "--features",
              "dev-shell",
            ],
          },
        ]),
      /must not be built in one cargo invocation/
    );

    assert.throws(
      () =>
        validatePrepackBuildInvocations([
          {
            cmd: "cargo",
            args: ["build", "--release", "--bin", "wavical-shell", "--features", "dev-shell"],
          },
          {
            cmd: "cargo",
            args: [
              "build",
              "--release",
              "--bin",
              "wavical-shell-dev",
              "--features",
              "dev-shell",
              "--target-dir",
              "target/dev-shell",
            ],
          },
        ]),
      /must not enable dev-shell feature/
    );
  });

  it("invokes prepack with mocked runner and verifies separate shell builds", () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "wavical-prepack-mock-"));
    try {
      fs.writeFileSync(
        path.join(tmp, "Cargo.toml"),
        '[workspace.package]\nversion = "1.2.0"\nedition = "2024"\n',
        "utf8"
      );
      fs.writeFileSync(
        path.join(tmp, "package.json"),
        JSON.stringify({ name: "wavical", version: "1.2.0" }),
        "utf8"
      );
      execFileSync("git", ["init"], { cwd: tmp, stdio: "pipe" });
      execFileSync("git", ["config", "user.email", "test@example.com"], {
        cwd: tmp,
        stdio: "pipe",
      });
      execFileSync("git", ["config", "user.name", "Test"], {
        cwd: tmp,
        stdio: "pipe",
      });
      execFileSync("git", ["add", "."], { cwd: tmp, stdio: "pipe" });
      execFileSync("git", ["commit", "-m", "init"], { cwd: tmp, stdio: "pipe" });

      const calls = [];
      prepack({
        rootDir: tmp,
        allowDirty: true,
        runCheckedFn: (cmd, args) => {
          calls.push({ cmd, args });
          const binIdx = args.indexOf("--bin");
          const binName = args[binIdx + 1];
          const exe = process.platform === "win32" ? `${binName}.exe` : binName;
          const targetDirIdx = args.indexOf("--target-dir");
          const outDir =
            targetDirIdx >= 0
              ? path.join(tmp, args[targetDirIdx + 1], "release")
              : path.join(tmp, "target", "release");
          fs.mkdirSync(outDir, { recursive: true });
          fs.writeFileSync(path.join(outDir, exe), "stub");
        },
        verifyPackageFn: () => ({}),
      });

      assert.equal(calls.length, 3);
      assert.ok(calls[1].args.includes("wavical-shell"));
      assert.ok(!calls[1].args.includes("wavical-shell-dev"));
      assert.ok(calls[2].args.includes("wavical-shell-dev"));
      assert.ok(calls[2].args.includes("target/dev-shell"));
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  });
});

describe("scripts/verify-package.js", () => {
  it("fails on a fake bin folder with an old --help output", () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "wavical-verify-fake-"));
    try {
      const binDir = setupTempPkgDir(tmp);
      buildFakeBinSet(tmp, binDir, {
        cliHelp: "Usage: wavical <COMMAND>\n\nCommands:\n  init\n  dev\n  build\n  help",
        prodShellHelp: "Options:\n  --network-mode <M>\n  --remote-url <U>",
        devShellHelp:
          "Options:\n  --dev\n  --dev-url <U>\n  --record\n  --network-mode <M>\n  --remote-url <U>\n  --remote-debugging-port <P>",
      });

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

  it("fails if production wavical-shell exposes dev flags or contains the dev marker", () => {
    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "wavical-verify-neg-"));
    try {
      const binDir = setupTempPkgDir(tmp);
      const validCliHelp =
        "Usage: wavical <COMMAND>\n\nCommands:\n  init\n  dev\n  dev-safe\n  dev-ultra-safe\n  allowlist\n  vendor\n  build\n  help";
      const validDevHelp =
        "Options:\n  --dev\n  --dev-url <U>\n  --record\n  --network-mode <M>\n  --remote-url <U>\n  --remote-debugging-port <P>";

      // 1. Production shell lists --dev in --help -> rejected
      buildFakeBinSet(tmp, binDir, {
        cliHelp: validCliHelp,
        prodShellHelp: "Options:\n  --network-mode <M>\n  --remote-url <U>\n  --dev",
        devShellHelp: validDevHelp,
      });

      assert.throws(
        () =>
          verifyPackageDir({
            packageDir: tmp,
            expectedCommit: "deadbee",
            writeChecksums: false,
            skipPreviousCheck: true,
            skipDocsCheck: true,
          }),
        /must NOT expose dev-only option "--dev"/
      );

      // 2. Production shell contains dev binary marker -> rejected
      buildFakeBinSet(tmp, binDir, {
        cliHelp: validCliHelp,
        prodShellHelp: "Options:\n  --network-mode <M>\n  --remote-url <U>",
        devShellHelp: validDevHelp,
        prodMarker: DEV_BINARY_MARKERS[0],
      });

      assert.throws(
        () =>
          verifyPackageDir({
            packageDir: tmp,
            expectedCommit: "deadbee",
            writeChecksums: false,
            skipPreviousCheck: true,
            skipDocsCheck: true,
          }),
        /contains dev command-channel marker/
      );

      // 2b. Production shell lists --remote-debugging-port or contains test-probe marker -> rejected
      buildFakeBinSet(tmp, binDir, {
        cliHelp: validCliHelp,
        prodShellHelp:
          "Options:\n  --network-mode <M>\n  --remote-url <U>\n  --remote-debugging-port <P>",
        devShellHelp: validDevHelp,
      });
      assert.throws(
        () =>
          verifyPackageDir({
            packageDir: tmp,
            expectedCommit: "deadbee",
            writeChecksums: false,
            skipPreviousCheck: true,
            skipDocsCheck: true,
          }),
        /must NOT expose dev-only option "--remote-debugging-port"/
      );

      buildFakeBinSet(tmp, binDir, {
        cliHelp: validCliHelp,
        prodShellHelp: "Options:\n  --network-mode <M>\n  --remote-url <U>",
        devShellHelp: validDevHelp,
        prodMarker: TEST_PROBE_BINARY_MARKERS[0],
      });
      assert.throws(
        () =>
          verifyPackageDir({
            packageDir: tmp,
            expectedCommit: "deadbee",
            writeChecksums: false,
            skipPreviousCheck: true,
            skipDocsCheck: true,
          }),
        /contains test-probe marker/
      );

      // 3. Missing wavical-shell-dev -> rejected
      const devExe =
        process.platform === "win32" ? "wavical-shell-dev.exe" : "wavical-shell-dev";
      fs.rmSync(path.join(binDir, devExe));
      assert.throws(
        () =>
          verifyPackageDir({
            packageDir: tmp,
            expectedCommit: "deadbee",
            writeChecksums: false,
            skipPreviousCheck: true,
            skipDocsCheck: true,
          }),
        /Missing required binary/
      );
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  });

  it("passes on the real built release binaries when present in target/release and target/dev-shell/release", () => {
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
    const releaseDevShell = path.join(
      ROOT_DIR,
      "target",
      "dev-shell",
      "release",
      process.platform === "win32" ? "wavical-shell-dev.exe" : "wavical-shell-dev"
    );

    if (
      !fs.existsSync(releaseCli) ||
      !fs.existsSync(releaseShell) ||
      !fs.existsSync(releaseDevShell)
    ) {
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
      fs.copyFileSync(path.join(ROOT_DIR, "LICENSE-MIT"), path.join(tmp, "LICENSE-MIT"));
      fs.copyFileSync(path.join(ROOT_DIR, "LICENSE-APACHE"), path.join(tmp, "LICENSE-APACHE"));
      fs.copyFileSync(releaseCli, path.join(binDir, path.basename(releaseCli)));
      fs.copyFileSync(
        releaseShell,
        path.join(binDir, path.basename(releaseShell))
      );
      fs.copyFileSync(
        releaseDevShell,
        path.join(binDir, path.basename(releaseDevShell))
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
        skipA10Check: true,
      });

      const currentPkg = JSON.parse(
        fs.readFileSync(path.join(ROOT_DIR, "package.json"), "utf8")
      );
      assert.equal(res.version, currentPkg.version);
      assert.ok(fs.existsSync(path.join(binDir, "checksums.txt")));
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true });
    }
  });

  it("enforces package.json prepublishOnly never builds, refuses prepack during publish, and rejects test-hooks/missing LICENSE/untagged/A10 mismatch", () => {
    const pkg = JSON.parse(fs.readFileSync(path.join(ROOT_DIR, "package.json"), "utf8"));
    assert.ok(
      pkg.scripts.prepublishOnly.includes("verify-package.js") &&
        pkg.scripts.prepublishOnly.includes("--strict") &&
        pkg.scripts.prepublishOnly.includes("--verify-checksums-only") &&
        !pkg.scripts.prepublishOnly.includes("prepack") &&
        !pkg.scripts.prepublishOnly.includes("cargo"),
      `prepublishOnly must only run verify-package.js --strict --verify-checksums-only, got: ${pkg.scripts.prepublishOnly}`
    );

    assert.throws(
      () => prepack({ npmCommand: "publish" }),
      /Refusing to rebuild during "npm publish"/
    );

    assert.throws(
      () =>
        validatePrepackBuildInvocations([
          {
            cmd: "cargo",
            args: ["build", "--release", "--bin", "wavical-shell"],
          },
          {
            cmd: "cargo",
            args: [
              "build",
              "--release",
              "--bin",
              "wavical-shell-dev",
              "--features",
              "dev-shell,test-hooks",
              "--target-dir",
              "target/dev-shell",
            ],
          },
        ]),
      /must NEVER enable test-hooks or test-probe/
    );

    const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "wavical-strict-test-"));
    try {
      const binDir = setupTempPkgDir(tmp, { includeLicense: false });
      const validCliHelp =
        "Usage: wavical <COMMAND>\n\nCommands:\n  init\n  dev\n  dev-safe\n  dev-ultra-safe\n  allowlist\n  vendor\n  build\n  help";
      const validProdHelp = "Options:\n  --network-mode <M>\n  --remote-url <U>";
      const validDevHelp =
        "Options:\n  --dev\n  --dev-url <U>\n  --record\n  --network-mode <M>\n  --remote-url <U>\n  --remote-debugging-port <P>";

      // 1. Missing LICENSE rejected
      buildFakeBinSet(tmp, binDir, {
        cliHelp: validCliHelp,
        prodShellHelp: validProdHelp,
        devShellHelp: validDevHelp,
      });
      assert.throws(
        () =>
          verifyPackageDir({
            packageDir: tmp,
            expectedCommit: "deadbee",
            writeChecksums: false,
            skipPreviousCheck: true,
            skipDocsCheck: true,
            skipA10Check: true,
          }),
        /Missing LICENSE file/
      );

      fs.copyFileSync(path.join(ROOT_DIR, "LICENSE-MIT"), path.join(tmp, "LICENSE-MIT"));

      // 2. Shipped wavical-shell-dev exposing --test-proxy or marker rejected
      buildFakeBinSet(tmp, binDir, {
        cliHelp: validCliHelp,
        prodShellHelp: validProdHelp,
        devShellHelp: `${validDevHelp}\n  --test-proxy <U>`,
      });
      assert.throws(
        () =>
          verifyPackageDir({
            packageDir: tmp,
            expectedCommit: "deadbee",
            writeChecksums: false,
            skipPreviousCheck: true,
            skipDocsCheck: true,
            skipA10Check: true,
          }),
        /must NOT expose test-only option "--test-proxy"/
      );

      buildFakeBinSet(tmp, binDir, {
        cliHelp: validCliHelp,
        prodShellHelp: validProdHelp,
        devShellHelp: validDevHelp,
        devMarker: `${DEV_BINARY_MARKERS[0]} ${TEST_HOOKS_BINARY_MARKERS[0]}`,
      });
      assert.throws(
        () =>
          verifyPackageDir({
            packageDir: tmp,
            expectedCommit: "deadbee",
            writeChecksums: false,
            skipPreviousCheck: true,
            skipDocsCheck: true,
            skipA10Check: true,
          }),
        /contains test-hooks marker/
      );

      // 3. Clean git repo + tag v1.2.0 + A10 hash check
      buildFakeBinSet(tmp, binDir, {
        cliHelp: validCliHelp,
        prodShellHelp: validProdHelp,
        devShellHelp: validDevHelp,
      });
      const initialRes = verifyPackageDir({
        packageDir: tmp,
        expectedCommit: "deadbee",
        writeChecksums: true,
        skipPreviousCheck: true,
        skipDocsCheck: true,
        skipA10Check: true,
      });

      const auditsDir = path.join(tmp, "docs", "audits");
      fs.mkdirSync(auditsDir, { recursive: true });
      const a10MdPath = path.join(auditsDir, "A10_release_1.2.0.md");
      fs.writeFileSync(
        a10MdPath,
        `| File | Size (bytes) | SHA-256 |\n|---|---:|---|\n` +
          `| \`bin/wavical.exe\` | 1 | \`${initialRes.hashes["wavical.exe"] || initialRes.hashes["wavical"]}\` |\n` +
          `| \`bin/wavical-shell.exe\` | 1 | \`${initialRes.hashes["wavical-shell.exe"] || initialRes.hashes["wavical-shell"]}\` |\n` +
          `| \`bin/wavical-shell-dev.exe\` | 1 | \`${"0".repeat(64)}\` |\n`,
        "utf8"
      );

      const parsedA10 = parseA10HashesFromMarkdown(fs.readFileSync(a10MdPath, "utf8"));
      assert.equal(parsedA10["wavical-shell-dev.exe"], "0".repeat(64));

      execFileSync("git", ["init"], { cwd: tmp, stdio: "pipe" });
      execFileSync("git", ["config", "user.email", "test@example.com"], { cwd: tmp, stdio: "pipe" });
      execFileSync("git", ["config", "user.name", "Test"], { cwd: tmp, stdio: "pipe" });
      execFileSync("git", ["add", "."], { cwd: tmp, stdio: "pipe" });
      execFileSync("git", ["commit", "-m", "release"], { cwd: tmp, stdio: "pipe" });

      // Missing tag -> fails strict
      assert.throws(
        () =>
          verifyPackageDir({
            packageDir: tmp,
            expectedCommit: "deadbee",
            allowDirty: false,
            writeChecksums: false,
            skipPreviousCheck: true,
            skipDocsCheck: true,
            strict: true,
            auditsDir,
          }),
        /required git tag "v1\.2\.0" does not exist/
      );

      execFileSync("git", ["tag", "v1.2.0"], { cwd: tmp, stdio: "pipe" });

      // Tag exists, but A10 hash mismatches -> refused
      assert.throws(
        () =>
          verifyPackageDir({
            packageDir: tmp,
            expectedCommit: "deadbee",
            allowDirty: false,
            writeChecksums: false,
            skipPreviousCheck: true,
            skipDocsCheck: true,
            strict: true,
            auditsDir,
          }),
        /differs from A10 audited hash/
      );

      // Update A10 report with matching hash, commit, re-tag -> passes strict!
      fs.writeFileSync(
        a10MdPath,
        `| File | Size (bytes) | SHA-256 |\n|---|---:|---|\n` +
          Object.entries(initialRes.hashes)
            .map(([n, h]) => `| \`bin/${n}\` | 1 | \`${h}\` |`)
            .join("\n") +
          "\n",
        "utf8"
      );
      execFileSync("git", ["add", "."], { cwd: tmp, stdio: "pipe" });
      execFileSync("git", ["commit", "-m", "a10"], { cwd: tmp, stdio: "pipe" });
      execFileSync("git", ["tag", "-f", "v1.2.0"], { cwd: tmp, stdio: "pipe" });

      const strictOk = verifyPackageDir({
        packageDir: tmp,
        expectedCommit: "deadbee",
        allowDirty: false,
        writeChecksums: false,
        skipPreviousCheck: true,
        skipDocsCheck: true,
        strict: true,
        auditsDir,
      });
      assert.equal(strictOk.version, "1.2.0");
    } finally {
      fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
    }
  });
});
