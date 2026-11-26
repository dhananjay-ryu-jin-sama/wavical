#!/usr/bin/env node
"use strict";

const { spawnSync } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");

const { runReferenceAppE2E } = require("./e2e-reference-app");

const REQUIRED_CLI_COMMANDS = [
  "dev",
  "dev-safe",
  "dev-ultra-safe",
  "build",
  "vendor",
  "allowlist",
];

const REQUIRED_PROD_SHELL_FLAGS = [
  "--network-mode",
  "--remote-url",
];

const FORBIDDEN_PROD_SHELL_FLAGS = [
  "--dev",
  "--dev-url",
  "--record",
  "--listen-stdin",
  "--remote-debugging-port",
  "--probe-port-file",
  "--test-proxy",
  "--test-spki",
];

const REQUIRED_DEV_SHELL_FLAGS = [
  "--dev",
  "--dev-url",
  "--record",
  "--network-mode",
  "--remote-url",
  "--remote-debugging-port",
];

const FORBIDDEN_DEV_SHELL_FLAGS = [
  "--test-proxy",
  "--test-spki",
];

const DEV_BINARY_MARKERS = [
  "WAVICAL_DEV_SHELL_CHANNEL_V1_MARKER",
  "WAVICAL_DEV_SESSION_TOKEN",
];

const TEST_PROBE_BINARY_MARKERS = [
  "WAVICAL_TEST_PROBE_V1_MARKER",
  "WAVICAL_TEST_PROBE_PORT_FILE",
];

const TEST_HOOKS_BINARY_MARKERS = [
  "WAVICAL_TEST_HOOKS_V1_MARKER",
  "WAVICAL_DEV_TEST_PROXY",
  "WAVICAL_DEV_TEST_SPKI",
  "--test-proxy",
  "--test-spki",
  "--ignore-certificate-errors-spki-list",
];

function binaryFilename(base) {
  return process.platform === "win32" ? `${base}.exe` : base;
}

function sha256File(filePath) {
  const hash = crypto.createHash("sha256");
  hash.update(fs.readFileSync(filePath));
  return hash.digest("hex");
}

function runCommand(cmd, args, options = {}) {
  const res = spawnSync(cmd, args, {
    encoding: "utf8",
    windowsHide: true,
    ...options,
  });
  return res;
}

function runNpm(args, options = {}) {
  if (process.platform === "win32") {
    return runCommand("cmd.exe", ["/d", "/s", "/c", "npm", ...args], options);
  }
  return runCommand("npm", args, options);
}

function getCurrentGitCommit(cwd) {
  const res = runCommand("git", ["rev-parse", "--short", "HEAD"], { cwd });
  if (res.status !== 0 || !res.stdout) {
    return null;
  }
  return res.stdout.trim();
}

function checkStrictGitReleaseProvenance(gitRoot, expectedVersion) {
  const statusRes = runCommand("git", ["status", "--porcelain"], { cwd: gitRoot });
  if (statusRes.status !== 0) {
    throw new Error(
      `Strict verification failed: git status failed in "${gitRoot}": ${(statusRes.stderr || "").trim()}`
    );
  }
  const dirty = (statusRes.stdout || "").trim();
  if (dirty.length > 0) {
    throw new Error(
      `Strict verification failed: git working tree is dirty:\n${dirty}`
    );
  }

  const headRes = runCommand("git", ["rev-parse", "HEAD"], { cwd: gitRoot });
  if (headRes.status !== 0 || !(headRes.stdout || "").trim()) {
    throw new Error("Strict verification failed: could not resolve git HEAD");
  }
  const headSha = headRes.stdout.trim();

  const tagName = `v${expectedVersion}`;
  const tagRes = runCommand("git", ["rev-parse", "--verify", `refs/tags/${tagName}^{commit}`], {
    cwd: gitRoot,
  });
  if (tagRes.status !== 0 || !(tagRes.stdout || "").trim()) {
    throw new Error(
      `Strict verification failed: required git tag "${tagName}" does not exist (run: git tag ${tagName})`
    );
  }
  const tagSha = tagRes.stdout.trim();
  if (tagSha !== headSha) {
    throw new Error(
      `Strict verification failed: git tag "${tagName}" points at ${tagSha.slice(0, 7)}, expected HEAD (${headSha.slice(0, 7)})`
    );
  }
  console.log(`Strict git tag check OK: ${tagName} -> ${headSha.slice(0, 7)} (clean tree).`);
}

function checkLicenseFilesPresent(packageDir) {
  const entries = fs.readdirSync(packageDir, { withFileTypes: true });
  const licenses = entries
    .filter((e) => e.isFile() && /^LICENSE/i.test(e.name))
    .map((e) => e.name)
    .sort();
  if (licenses.length === 0) {
    throw new Error(
      `Missing LICENSE file in package "${packageDir}" (expected LICENSE, LICENSE-MIT, or LICENSE-APACHE)`
    );
  }
  for (const name of licenses) {
    const stat = fs.statSync(path.join(packageDir, name));
    if (stat.size === 0) {
      throw new Error(`License file "${name}" in "${packageDir}" is empty`);
    }
  }
  console.log(`License check OK: ${licenses.join(", ")}`);
  return licenses;
}

function parseA10HashesFromMarkdown(mdText) {
  const hashes = {};
  const lines = mdText.split(/\r?\n/);
  for (const line of lines) {
    const match = line.match(
      /\|\s*`(?:bin\/)?([^`|]+)`\s*\|\s*[^|]+\|\s*`([0-9a-f]{64})`\s*\|/i
    );
    if (match) {
      const name = path.basename(match[1].trim());
      hashes[name] = match[2].toLowerCase();
    }
  }
  return hashes;
}

function loadA10ReportHashes(auditsDir, expectedVersion) {
  const jsonPath = path.join(auditsDir, `A10_release_${expectedVersion}.json`);
  const mdPath = path.join(auditsDir, `A10_release_${expectedVersion}.md`);

  let jsonHashes = null;
  if (fs.existsSync(jsonPath)) {
    const parsed = JSON.parse(fs.readFileSync(jsonPath, "utf8"));
    if (parsed.version && parsed.version !== expectedVersion) {
      throw new Error(
        `A10 report "${jsonPath}" version mismatch: expected ${expectedVersion}, got ${parsed.version}`
      );
    }
    const rawBins = parsed.binaries || parsed.hashes || {};
    jsonHashes = {};
    for (const [k, v] of Object.entries(rawBins)) {
      if (typeof v === "string" && /^[0-9a-f]{64}$/i.test(v.trim())) {
        jsonHashes[path.basename(k)] = v.trim().toLowerCase();
      }
    }
  }

  let mdHashes = null;
  if (fs.existsSync(mdPath)) {
    mdHashes = parseA10HashesFromMarkdown(fs.readFileSync(mdPath, "utf8"));
  }

  if (!jsonHashes && !mdHashes) {
    return null;
  }

  if (jsonHashes && mdHashes) {
    for (const [name, digest] of Object.entries(jsonHashes)) {
      if (mdHashes[name] && mdHashes[name] !== digest) {
        throw new Error(
          `Inconsistent A10 hashes for ${name} between ${jsonPath} (${digest}) and ${mdPath} (${mdHashes[name]})`
        );
      }
    }
  }

  return {
    source: jsonHashes ? jsonPath : mdPath,
    hashes: jsonHashes || mdHashes,
  };
}

function verifyAgainstA10Report({
  auditsDir,
  expectedVersion,
  binaryNames,
  currentHashes,
  requireReport,
  skipA10Check,
}) {
  if (skipA10Check) {
    return;
  }
  const report = loadA10ReportHashes(auditsDir, expectedVersion);
  if (!report) {
    if (requireReport) {
      throw new Error(
        `Refusing publish/verification: missing A10 audit report for version ${expectedVersion} (expected ${path.join(auditsDir, `A10_release_${expectedVersion}.md`)} or .json)`
      );
    }
    return;
  }

  for (const name of binaryNames) {
    const auditedHash = report.hashes[name];
    if (!auditedHash) {
      throw new Error(
        `A10 report (${report.source}) does not record a SHA-256 hash for ${name}`
      );
    }
    if (auditedHash !== currentHashes[name]) {
      throw new Error(
        `Refusing publish: ${name} SHA-256 (${currentHashes[name]}) differs from A10 audited hash (${auditedHash}) in ${report.source}`
      );
    }
  }
  console.log(`A10 hash provenance check OK (${report.source}).`);
}

function parseHelpCommands(helpOutput) {
  const commands = new Set();
  const lines = helpOutput.split(/\r?\n/);
  let inCommands = false;
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed === "Commands:") {
      inCommands = true;
      continue;
    }
    if (inCommands) {
      if (!trimmed) {
        continue;
      }
      if (!line.startsWith("  ")) {
        inCommands = false;
        continue;
      }
      const match = trimmed.match(/^([a-z][a-z0-9-]*)\b/);
      if (match && match[1] !== "help") {
        commands.add(match[1]);
      }
    }
  }
  return commands;
}

function parseHelpFlags(helpOutput) {
  const flags = new Set();
  const matches = helpOutput.matchAll(/(?:^|[\s,])(--[a-z0-9][a-z0-9-]*)\b/gm);
  for (const m of matches) {
    flags.add(m[1]);
  }
  return flags;
}

/**
 * Extract CLI commands and flags mentioned in README.md and CHANGELOG.md.
 * Skips the "## ... Roadmap" table rows explicitly marked as Planned.
 */
function extractDocCliReferences(markdownText, { skipRoadmap = false } = {}) {
  const invocations = [];
  const standaloneFlags = new Set();
  const lines = markdownText.split(/\r?\n/);

  let inFence = false;
  let inRoadmap = false;

  for (const rawLine of lines) {
    const trimmed = rawLine.trim();

    if (/^##\s+/.test(trimmed)) {
      inRoadmap = skipRoadmap && /roadmap/i.test(trimmed);
    }
    if (inRoadmap && /planned/i.test(trimmed)) {
      continue;
    }

    if (trimmed.startsWith("```")) {
      inFence = !inFence;
      continue;
    }

    if (inFence) {
      if (/^(wavical|wca)\s+/.test(trimmed)) {
        invocations.push(trimmed);
      }
      continue;
    }

    const codeSpans = rawLine.matchAll(/`([^`]+)`/g);
    for (const spanMatch of codeSpans) {
      const span = spanMatch[1].trim();
      if (/^(wavical|wca)\s+/.test(span)) {
        invocations.push(span);
      } else if (/^--[a-z0-9-]+(?:\s|$)/.test(span)) {
        const flagMatch = span.match(/^(--[a-z0-9-]+)/);
        if (flagMatch) {
          standaloneFlags.add(flagMatch[1]);
        }
      }
    }
  }

  return { invocations, standaloneFlags: Array.from(standaloneFlags) };
}

function tokenizeDocInvocation(invocation) {
  // Remove quoted values, <placeholders>, and unwrap [optional] brackets
  const cleaned = invocation
    .replace(/"[^"]*"/g, " ")
    .replace(/'[^']*'/g, " ")
    .replace(/<[^>]+>/g, " ")
    .replace(/[\[\]]/g, " ");
  return cleaned.split(/\s+/).filter(Boolean);
}

function checkDocsParity({ cliBin, packageDir }) {
  const helpCache = new Map();

  function getHelp(cmdPath) {
    const key = cmdPath.join(" ");
    if (helpCache.has(key)) {
      return helpCache.get(key);
    }
    const res = runCommand(cliBin, [...cmdPath, "--help"]);
    if (res.status !== 0) {
      throw new Error(
        `CLI help failed for "${[path.basename(cliBin), ...cmdPath, "--help"].join(" ")}": ${(res.stderr || res.stdout || "").trim()}`
      );
    }
    const text = res.stdout || "";
    const parsed = {
      text,
      commands: parseHelpCommands(text),
      flags: parseHelpFlags(text),
    };
    helpCache.set(key, parsed);
    return parsed;
  }

  const topHelp = getHelp([]);
  const docFiles = [
    { name: "README.md", skipRoadmap: true },
    { name: "CHANGELOG.md", skipRoadmap: false },
  ];

  const allKnownFlags = new Set(topHelp.flags);
  for (const topCmd of topHelp.commands) {
    const subHelp = getHelp([topCmd]);
    for (const f of subHelp.flags) allKnownFlags.add(f);
    for (const nestedCmd of subHelp.commands) {
      const nestedHelp = getHelp([topCmd, nestedCmd]);
      for (const f of nestedHelp.flags) allKnownFlags.add(f);
    }
  }

  for (const { name, skipRoadmap } of docFiles) {
    const filePath = path.join(packageDir, name);
    if (!fs.existsSync(filePath)) {
      throw new Error(`Missing packaged documentation file: ${filePath}`);
    }
    const content = fs.readFileSync(filePath, "utf8");
    const { invocations, standaloneFlags } = extractDocCliReferences(content, {
      skipRoadmap,
    });

    for (const inv of invocations) {
      const tokens = tokenizeDocInvocation(inv);
      if (tokens.length < 2) continue;

      let idx = 1; // skip 'wavical' or 'wca'
      const cmdPath = [];

      if (!tokens[idx].startsWith("-")) {
        const topCmd = tokens[idx];
        if (!topHelp.commands.has(topCmd)) {
          throw new Error(
            `Docs parity mismatch in ${name}: command "wavical ${topCmd}" (from "${inv}") is not listed in "wavical --help"`
          );
        }
        cmdPath.push(topCmd);
        idx += 1;

        const subHelp = getHelp(cmdPath);
        if (idx < tokens.length && !tokens[idx].startsWith("-") && subHelp.commands.size > 0) {
          const subCmd = tokens[idx];
          if (!subHelp.commands.has(subCmd)) {
            throw new Error(
              `Docs parity mismatch in ${name}: subcommand "wavical ${topCmd} ${subCmd}" (from "${inv}") is not listed in "wavical ${topCmd} --help"`
            );
          }
          cmdPath.push(subCmd);
          idx += 1;
        }
      }

      const targetHelp = getHelp(cmdPath);
      for (let j = idx; j < tokens.length; j++) {
        const tok = tokens[j];
        if (tok.startsWith("--")) {
          if (!targetHelp.flags.has(tok)) {
            const cmdDesc = ["wavical", ...cmdPath].join(" ");
            throw new Error(
              `Docs parity mismatch in ${name}: flag "${tok}" (from "${inv}") is not listed in "${cmdDesc} --help"`
            );
          }
        }
      }
    }

    for (const flag of standaloneFlags) {
      if (!allKnownFlags.has(flag)) {
        throw new Error(
          `Docs parity mismatch in ${name}: flag "${flag}" does not appear in "wavical --help" or any subcommand help`
        );
      }
    }
  }
}

function writeOrVerifyChecksums({ binDir, binaryNames, writeChecksums }) {
  const checksumPath = path.join(binDir, "checksums.txt");
  const lines = [];
  const hashes = {};

  for (const name of binaryNames) {
    const filePath = path.join(binDir, name);
    const digest = sha256File(filePath);
    hashes[name] = digest;
    lines.push(`${digest}  ${name}`);
    console.log(`SHA-256 ${name}: ${digest}`);
  }

  const expectedContent = lines.join("\n") + "\n";

  if (writeChecksums) {
    fs.writeFileSync(checksumPath, expectedContent, "utf8");
    console.log(`Wrote checksums to ${checksumPath}`);
  } else {
    if (!fs.existsSync(checksumPath)) {
      throw new Error(`Missing ${checksumPath} inside package`);
    }
    const actualContent = fs.readFileSync(checksumPath, "utf8").replace(/\r\n/g, "\n");
    if (actualContent !== expectedContent) {
      throw new Error(
        `Checksum mismatch in ${checksumPath}.\nExpected:\n${expectedContent}\nActual:\n${actualContent}`
      );
    }
    console.log(`Verified checksums in ${checksumPath}`);
  }

  return hashes;
}

function extractTarball(tarballPath, destDir) {
  const res = runCommand("tar", ["-xzf", tarballPath, "-C", destDir]);
  if (res.status !== 0) {
    throw new Error(
      `Failed to extract tarball "${tarballPath}": ${(res.stderr || res.stdout || "").trim()}`
    );
  }
  const pkgDir = path.join(destDir, "package");
  if (!fs.existsSync(pkgDir)) {
    throw new Error(`Extracted tarball "${tarballPath}" does not contain a "package/" directory`);
  }
  return pkgDir;
}

function checkPreviousPublishedBinaries({
  packageName,
  currentVersion,
  currentHashes,
  binaryNames,
  previousTarball,
  previousVersion,
  skipPreviousCheck,
}) {
  if (skipPreviousCheck) {
    console.log("Skipping previous-version binary comparison (--skip-previous-check).");
    return;
  }

  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "wavical-prev-"));
  try {
    let prevPkgDir;
    let prevLabel;

    if (previousTarball) {
      prevPkgDir = extractTarball(previousTarball, tempDir);
      const prevPkgJson = JSON.parse(fs.readFileSync(path.join(prevPkgDir, "package.json"), "utf8"));
      prevLabel = `${packageName}@${prevPkgJson.version}`;
    } else {
      let targetVersion = previousVersion;
      if (!targetVersion) {
        const viewRes = runNpm(["view", packageName, "versions", "--json"], {
          timeout: 15000,
        });
        if (viewRes.status !== 0 || !viewRes.stdout) {
          console.warn(
            `WARNING: Could not query npm for previous ${packageName} versions (offline or registry unavailable); skipping byte-identical comparison.`
          );
          return;
        }
        let versions;
        try {
          versions = JSON.parse(viewRes.stdout);
        } catch {
          console.warn(
            `WARNING: Unexpected response from npm view; skipping byte-identical comparison.`
          );
          return;
        }
        if (typeof versions === "string") {
          versions = [versions];
        }
        const candidates = Array.isArray(versions)
          ? versions.filter((v) => v !== currentVersion)
          : [];
        if (candidates.length === 0) {
          console.log(`No previously published ${packageName} version found to compare against.`);
          return;
        }
        targetVersion = candidates[candidates.length - 1];
      }

      prevLabel = `${packageName}@${targetVersion}`;
      const packRes = runNpm(["pack", prevLabel, "--silent"], {
        cwd: tempDir,
        timeout: 30000,
      });
      if (packRes.status !== 0 || !packRes.stdout) {
        console.warn(
          `WARNING: Failed to download ${prevLabel} via npm pack (offline?); skipping byte-identical comparison.`
        );
        return;
      }
      const tgzName = packRes.stdout.trim().split(/\r?\n/).pop().trim();
      const tgzPath = path.join(tempDir, tgzName);
      if (!fs.existsSync(tgzPath)) {
        console.warn(
          `WARNING: Downloaded tarball ${tgzPath} not found; skipping byte-identical comparison.`
        );
        return;
      }
      prevPkgDir = extractTarball(tgzPath, tempDir);
    }

    for (const name of binaryNames) {
      const prevBinPath = path.join(prevPkgDir, "bin", name);
      if (!fs.existsSync(prevBinPath)) {
        continue;
      }
      const prevHash = sha256File(prevBinPath);
      if (prevHash === currentHashes[name]) {
        throw new Error(
          `Stale binary detected: bin/${name} is byte-identical (SHA-256 ${prevHash}) to ${prevLabel}!`
        );
      }
      console.log(`Verified bin/${name} differs from ${prevLabel} (${prevHash.slice(0, 12)}...)`);
    }
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true });
  }
}

function verifyPackageDir({
  packageDir,
  expectedCommit,
  allowDirty = process.env.WAVICAL_ALLOW_DIRTY === "1",
  writeChecksums = true,
  previousTarball = null,
  previousVersion = null,
  skipPreviousCheck = false,
  skipDocsCheck = false,
  skipDevMarkerCheck = false,
  strict = false,
  checkA10 = false,
  skipA10Check = false,
  gitDir = null,
  auditsDir = null,
}) {
  if (strict && allowDirty) {
    throw new Error(
      "Refusing strict verification: WAVICAL_ALLOW_DIRTY=1 is not allowed in --strict mode."
    );
  }
  const pkgJsonPath = path.join(packageDir, "package.json");
  if (!fs.existsSync(pkgJsonPath)) {
    throw new Error(`Missing package.json at ${pkgJsonPath}`);
  }
  const pkg = JSON.parse(fs.readFileSync(pkgJsonPath, "utf8"));
  const expectedVersion = pkg.version;
  if (!expectedVersion) {
    throw new Error("package.json is missing version");
  }

  // Verify wca alias in package.json bin and bin/wavical.js
  if (!pkg.bin || !pkg.bin.wavical || !pkg.bin.wca) {
    throw new Error('package.json "bin" must define both "wavical" and "wca"');
  }

  const binDir = path.join(packageDir, "bin");
  const wrapperJs = path.join(binDir, "wavical.js");
  if (!fs.existsSync(wrapperJs)) {
    throw new Error(`Missing CLI wrapper script: ${wrapperJs}`);
  }

  const cliBinName = binaryFilename("wavical");
  const shellBinName = binaryFilename("wavical-shell");
  const devShellBinName = binaryFilename("wavical-shell-dev");
  const cliBin = path.join(binDir, cliBinName);
  const shellBin = path.join(binDir, shellBinName);
  const devShellBin = path.join(binDir, devShellBinName);

  for (const binPath of [cliBin, shellBin, devShellBin]) {
    if (!fs.existsSync(binPath)) {
      throw new Error(`Missing required binary: ${binPath}`);
    }
  }

  const commitToCheck =
    expectedCommit !== undefined
      ? expectedCommit
      : process.env.WAVICAL_GIT_COMMIT || getCurrentGitCommit(packageDir);

  // 1. Check --version on all three binaries
  const cliVerRes = runCommand(cliBin, ["--version"]);
  if (cliVerRes.status !== 0) {
    throw new Error(
      `${cliBinName} --version failed: ${(cliVerRes.stderr || cliVerRes.stdout || "").trim()}`
    );
  }
  const cliVerOut = (cliVerRes.stdout || "").trim();

  const shellVerRes = runCommand(shellBin, ["--version"]);
  if (shellVerRes.status !== 0) {
    throw new Error(
      `${shellBinName} --version failed: ${(shellVerRes.stderr || shellVerRes.stdout || "").trim()}`
    );
  }
  const shellVerOut = (shellVerRes.stdout || "").trim();

  const devShellVerRes = runCommand(devShellBin, ["--version"]);
  if (devShellVerRes.status !== 0) {
    throw new Error(
      `${devShellBinName} --version failed: ${(devShellVerRes.stderr || devShellVerRes.stdout || "").trim()}`
    );
  }
  const devShellVerOut = (devShellVerRes.stdout || "").trim();

  for (const [label, out] of [
    [cliBinName, cliVerOut],
    [shellBinName, shellVerOut],
    [devShellBinName, devShellVerOut],
  ]) {
    if (!out.includes(expectedVersion)) {
      throw new Error(
        `${label} --version "${out}" does not match package.json version "${expectedVersion}"`
      );
    }
    if (commitToCheck && !out.includes(commitToCheck)) {
      throw new Error(
        `${label} --version "${out}" does not contain expected git commit "${commitToCheck}"`
      );
    }
    if (!allowDirty && out.includes("-dirty")) {
      throw new Error(`${label} --version "${out}" indicates a dirty build`);
    }
  }

  // Ensure CLI and both shells report identical version+commit suffixes
  const cliVerTail = cliVerOut.replace(/^\S+\s+/, "");
  const shellVerTail = shellVerOut.replace(/^\S+\s+/, "");
  const devShellVerTail = devShellVerOut.replace(/^\S+\s+/, "");
  if (cliVerTail !== shellVerTail || cliVerTail !== devShellVerTail) {
    throw new Error(
      `CLI and shell versions differ: "${cliVerOut}" vs "${shellVerOut}" vs "${devShellVerOut}"`
    );
  }
  console.log(`Version check OK: ${cliVerOut} / ${shellVerOut} / ${devShellVerOut}`);

  // 2. Check wavical.exe --help commands and wrapper execution
  const cliHelpRes = runCommand(cliBin, ["--help"]);
  if (cliHelpRes.status !== 0) {
    throw new Error(`${cliBinName} --help failed`);
  }
  const cliHelpOut = cliHelpRes.stdout || "";
  const topCommands = parseHelpCommands(cliHelpOut);
  for (const requiredCmd of REQUIRED_CLI_COMMANDS) {
    if (!topCommands.has(requiredCmd)) {
      throw new Error(
        `${cliBinName} --help is missing required command "${requiredCmd}"`
      );
    }
  }

  const wrapperHelpRes = runCommand(process.execPath, [wrapperJs, "--help"]);
  if (wrapperHelpRes.status !== 0) {
    throw new Error(
      `bin/wavical.js --help failed: ${(wrapperHelpRes.stderr || wrapperHelpRes.stdout || "").trim()}`
    );
  }

  // 3. Check wavical-shell.exe --help: positive prod flags + NEGATIVE dev flags check
  const shellHelpRes = runCommand(shellBin, ["--help"]);
  if (shellHelpRes.status !== 0) {
    throw new Error(`${shellBinName} --help failed`);
  }
  const shellFlags = parseHelpFlags(shellHelpRes.stdout || "");
  for (const requiredFlag of REQUIRED_PROD_SHELL_FLAGS) {
    if (!shellFlags.has(requiredFlag)) {
      throw new Error(
        `${shellBinName} --help is missing required production option "${requiredFlag}"`
      );
    }
  }
  for (const forbiddenFlag of FORBIDDEN_PROD_SHELL_FLAGS) {
    if (shellFlags.has(forbiddenFlag)) {
      throw new Error(
        `Security error: production ${shellBinName} --help must NOT expose dev-only option "${forbiddenFlag}"`
      );
    }
  }

  // 3b. Check wavical-shell-dev.exe --help: positive dev flags + negative test-hooks check
  const devShellHelpRes = runCommand(devShellBin, ["--help"]);
  if (devShellHelpRes.status !== 0) {
    throw new Error(`${devShellBinName} --help failed`);
  }
  const devShellFlags = parseHelpFlags(devShellHelpRes.stdout || "");
  for (const requiredFlag of REQUIRED_DEV_SHELL_FLAGS) {
    if (!devShellFlags.has(requiredFlag)) {
      throw new Error(
        `${devShellBinName} --help is missing required dev option "${requiredFlag}"`
      );
    }
  }
  for (const forbiddenDevFlag of FORBIDDEN_DEV_SHELL_FLAGS) {
    if (devShellFlags.has(forbiddenDevFlag)) {
      throw new Error(
        `Security error: shipped ${devShellBinName} --help must NOT expose test-only option "${forbiddenDevFlag}"`
      );
    }
  }

  // 3c. Negative binary string scan on production wavical-shell.exe & shipped wavical-shell-dev.exe
  const prodShellBytes = fs.readFileSync(shellBin);
  for (const marker of DEV_BINARY_MARKERS) {
    if (prodShellBytes.includes(Buffer.from(marker, "utf8"))) {
      throw new Error(
        `Security error: production ${shellBinName} contains dev command-channel marker "${marker}"`
      );
    }
  }
  for (const marker of TEST_PROBE_BINARY_MARKERS) {
    if (prodShellBytes.includes(Buffer.from(marker, "utf8"))) {
      throw new Error(
        `Security error: production ${shellBinName} contains test-probe marker "${marker}"`
      );
    }
  }
  for (const marker of TEST_HOOKS_BINARY_MARKERS) {
    if (prodShellBytes.includes(Buffer.from(marker, "utf8"))) {
      throw new Error(
        `Security error: production ${shellBinName} contains test-hooks marker "${marker}"`
      );
    }
  }
  if (!skipDevMarkerCheck) {
    const devShellBytes = fs.readFileSync(devShellBin);
    if (!devShellBytes.includes(Buffer.from(DEV_BINARY_MARKERS[0], "utf8"))) {
      throw new Error(
        `${devShellBinName} is missing expected dev marker "${DEV_BINARY_MARKERS[0]}"`
      );
    }
    for (const marker of TEST_PROBE_BINARY_MARKERS) {
      if (devShellBytes.includes(Buffer.from(marker, "utf8"))) {
        throw new Error(
          `Security error: ${devShellBinName} contains test-probe marker "${marker}"`
        );
      }
    }
    for (const marker of TEST_HOOKS_BINARY_MARKERS) {
      if (devShellBytes.includes(Buffer.from(marker, "utf8"))) {
        throw new Error(
          `Security error: shipped ${devShellBinName} contains test-hooks marker "${marker}"`
        );
      }
    }
  }
  console.log("Help, option, and dev-marker isolation checks OK.");

  // 3d. Check LICENSE* file(s) exist in package
  checkLicenseFilesPresent(packageDir);

  // 4. Check docs parity (README.md & CHANGELOG.md)
  if (!skipDocsCheck) {
    checkDocsParity({ cliBin, packageDir });
    console.log("Docs parity check OK (README.md & CHANGELOG.md match wavical --help).");
  }

  // 5. Write or verify bin/checksums.txt
  const binaryNames = [cliBinName, shellBinName, devShellBinName];
  const hashes = writeOrVerifyChecksums({
    binDir,
    binaryNames,
    writeChecksums,
  });

  // 5b. Strict git release tag + clean-tree provenance check
  const resolvedGitDir = gitDir || packageDir;
  if (strict) {
    checkStrictGitReleaseProvenance(resolvedGitDir, expectedVersion);
  }

  // 5c. Verify against A10 release audit report hashes
  const resolvedAuditsDir =
    auditsDir || path.join(gitDir || path.resolve(__dirname, ".."), "docs", "audits");
  verifyAgainstA10Report({
    auditsDir: resolvedAuditsDir,
    expectedVersion,
    binaryNames,
    currentHashes: hashes,
    requireReport: strict || checkA10,
    skipA10Check,
  });

  // 6. Check against previously published npm version
  checkPreviousPublishedBinaries({
    packageName: pkg.name || "wavical",
    currentVersion: expectedVersion,
    currentHashes: hashes,
    binaryNames,
    previousTarball,
    previousVersion,
    skipPreviousCheck,
  });

  return {
    version: expectedVersion,
    cliVersion: cliVerOut,
    shellVersion: shellVerOut,
    devShellVersion: devShellVerOut,
    hashes,
  };
}

async function verifyTarball({
  tarballPath,
  expectedCommit,
  allowDirty,
  previousTarball,
  previousVersion,
  skipPreviousCheck,
  skipDocsCheck,
  skipE2e = false,
  e2eOnline = false,
  testProbeBin = null,
  testHooksDevShellBin = null,
  strict = false,
  checkA10 = false,
  skipA10Check = false,
  gitDir = null,
  auditsDir = null,
}) {
  const absTarball = path.resolve(tarballPath);
  if (!fs.existsSync(absTarball)) {
    throw new Error(`Tarball not found: ${absTarball}`);
  }
  const repoRoot = gitDir || path.resolve(__dirname, "..");
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "wavical-verify-tgz-"));
  let pkgResult;
  try {
    const pkgDir = extractTarball(absTarball, tempDir);
    pkgResult = verifyPackageDir({
      packageDir: pkgDir,
      expectedCommit:
        expectedCommit !== undefined
          ? expectedCommit
          : process.env.WAVICAL_GIT_COMMIT || getCurrentGitCommit(repoRoot),
      allowDirty,
      writeChecksums: false,
      previousTarball,
      previousVersion,
      skipPreviousCheck,
      skipDocsCheck,
      strict,
      checkA10,
      skipA10Check,
      gitDir: repoRoot,
      auditsDir,
    });
  } finally {
    fs.rmSync(tempDir, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
  }

  if (!skipE2e) {
    await runReferenceAppE2E({
      tarballPath: absTarball,
      online: e2eOnline,
      testProbeBin,
      testHooksDevShellBin,
    });
  }

  return pkgResult;
}

function parseCliArgs(argv) {
  const opts = {
    dir: path.resolve(__dirname, ".."),
    tarball: null,
    expectedCommit: undefined,
    previousTarball: null,
    previousVersion: null,
    skipPreviousCheck: false,
    writeChecksums: true,
    skipE2e: false,
    e2eOnline: false,
    testProbeBin: null,
    testHooksDevShellBin: null,
    strict: false,
    checkA10: false,
    skipA10Check: false,
  };

  for (let i = 0; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === "--tarball" && i + 1 < argv.length) {
      opts.tarball = argv[++i];
    } else if (arg === "--dir" && i + 1 < argv.length) {
      opts.dir = path.resolve(argv[++i]);
    } else if (arg === "--commit" && i + 1 < argv.length) {
      opts.expectedCommit = argv[++i];
    } else if (arg === "--previous-tarball" && i + 1 < argv.length) {
      opts.previousTarball = path.resolve(argv[++i]);
    } else if (arg === "--previous-version" && i + 1 < argv.length) {
      opts.previousVersion = argv[++i];
    } else if (arg === "--skip-previous-check") {
      opts.skipPreviousCheck = true;
    } else if (arg === "--verify-checksums-only") {
      opts.writeChecksums = false;
    } else if (arg === "--skip-e2e") {
      opts.skipE2e = true;
    } else if (arg === "--online") {
      opts.e2eOnline = true;
    } else if (arg === "--strict") {
      opts.strict = true;
    } else if (arg === "--check-a10") {
      opts.checkA10 = true;
    } else if (arg === "--skip-a10-check") {
      opts.skipA10Check = true;
    } else if (arg === "--test-probe-bin" && i + 1 < argv.length) {
      opts.testProbeBin = path.resolve(argv[++i]);
    } else if (arg === "--test-hooks-dev-shell-bin" && i + 1 < argv.length) {
      opts.testHooksDevShellBin = path.resolve(argv[++i]);
    } else if (!arg.startsWith("-") && arg.endsWith(".tgz")) {
      opts.tarball = arg;
    } else {
      throw new Error(`Unknown argument: ${arg}`);
    }
  }

  return opts;
}

async function main() {
  try {
    const opts = parseCliArgs(process.argv.slice(2));
    if (opts.tarball) {
      await verifyTarball({
        tarballPath: opts.tarball,
        expectedCommit: opts.expectedCommit,
        previousTarball: opts.previousTarball,
        previousVersion: opts.previousVersion,
        skipPreviousCheck: opts.skipPreviousCheck,
        skipE2e: opts.skipE2e,
        e2eOnline: opts.e2eOnline,
        testProbeBin: opts.testProbeBin,
        testHooksDevShellBin: opts.testHooksDevShellBin,
        strict: opts.strict,
        checkA10: opts.checkA10,
        skipA10Check: opts.skipA10Check,
      });
    } else {
      verifyPackageDir({
        packageDir: opts.dir,
        expectedCommit: opts.expectedCommit,
        writeChecksums: opts.writeChecksums,
        previousTarball: opts.previousTarball,
        previousVersion: opts.previousVersion,
        skipPreviousCheck: opts.skipPreviousCheck,
        strict: opts.strict,
        checkA10: opts.checkA10,
        skipA10Check: opts.skipA10Check,
      });
    }
    console.log("Package verification passed.");
  } catch (err) {
    console.error(`error: ${err.message}`);
    process.exit(1);
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  DEV_BINARY_MARKERS,
  FORBIDDEN_DEV_SHELL_FLAGS,
  FORBIDDEN_PROD_SHELL_FLAGS,
  REQUIRED_CLI_COMMANDS,
  REQUIRED_DEV_SHELL_FLAGS,
  REQUIRED_PROD_SHELL_FLAGS,
  TEST_HOOKS_BINARY_MARKERS,
  TEST_PROBE_BINARY_MARKERS,
  checkLicenseFilesPresent,
  checkStrictGitReleaseProvenance,
  extractDocCliReferences,
  loadA10ReportHashes,
  parseA10HashesFromMarkdown,
  parseHelpCommands,
  parseHelpFlags,
  verifyAgainstA10Report,
  verifyPackageDir,
  verifyTarball,
};
