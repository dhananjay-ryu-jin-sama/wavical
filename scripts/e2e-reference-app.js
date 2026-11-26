#!/usr/bin/env node
"use strict";

const assert = require("node:assert/strict");
const { spawn, spawnSync } = require("node:child_process");
const crypto = require("node:crypto");
const fs = require("node:fs");
const http = require("node:http");
const net = require("node:net");
const os = require("node:os");
const path = require("node:path");

const { STUB_HOSTS, startMultiOriginTestServer } = require("./reference-app-mock-server");

const ROOT_DIR = path.resolve(__dirname, "..");
const FIXTURE_DIR = path.join(ROOT_DIR, "tests", "fixtures", "reference-app");

function sleep(ms) {
  return new Promise((r) => setTimeout(r, ms));
}

function getFreePort() {
  return new Promise((resolve, reject) => {
    const srv = net.createServer();
    srv.once("error", reject);
    srv.listen(0, "127.0.0.1", () => {
      const port = srv.address().port;
      srv.close((err) => (err ? reject(err) : resolve(port)));
    });
  });
}

function runNpmSync(args, options = {}) {
  if (process.platform === "win32") {
    return spawnSync("cmd.exe", ["/d", "/s", "/c", "npm", ...args], {
      encoding: "utf8",
      windowsHide: true,
      ...options,
    });
  }
  return spawnSync("npm", args, {
    encoding: "utf8",
    windowsHide: true,
    ...options,
  });
}

function runNpxWavicalSync(args, options = {}) {
  const env = {
    NPM_CONFIG_UPDATE_NOTIFIER: "false",
    ...process.env,
    ...(options.env || {}),
  };
  if (process.platform === "win32") {
    return spawnSync("cmd.exe", ["/d", "/s", "/c", "npx", "--no-install", "wavical", ...args], {
      encoding: "utf8",
      windowsHide: true,
      ...options,
      env,
    });
  }
  return spawnSync("npx", ["--no-install", "wavical", ...args], {
    encoding: "utf8",
    windowsHide: true,
    ...options,
    env,
  });
}

function spawnNpxWavical(args, options = {}) {
  if (process.platform === "win32") {
    return spawn("cmd.exe", ["/d", "/s", "/c", "npx", "--no-install", "wavical", ...args], {
      windowsHide: true,
      ...options,
    });
  }
  return spawn("npx", ["--no-install", "wavical", ...args], {
    windowsHide: true,
    ...options,
  });
}

async function killProcessTree(child) {
  if (!child || !child.pid) return;
  if (process.platform === "win32") {
    spawnSync("taskkill", ["/F", "/T", "/PID", String(child.pid)], {
      stdio: "ignore",
      windowsHide: true,
    });
  } else {
    try {
      child.kill("SIGKILL");
    } catch (_) {}
  }
  await new Promise((resolve) => {
    if (child.exitCode !== null || child.signalCode !== null) return resolve();
    const timer = setTimeout(resolve, 3000);
    child.once("exit", () => {
      clearTimeout(timer);
      resolve();
    });
  });
  await sleep(200);
}

function httpGetJson(url, timeoutMs = 3000) {
  return new Promise((resolve, reject) => {
    const req = http.get(url, { timeout: timeoutMs }, (res) => {
      let data = "";
      res.setEncoding("utf8");
      res.on("data", (chunk) => (data += chunk));
      res.on("end", () => {
        if (res.statusCode < 200 || res.statusCode >= 300) {
          return reject(new Error(`HTTP ${res.statusCode} from ${url}: ${data}`));
        }
        try {
          resolve(JSON.parse(data));
        } catch (err) {
          reject(new Error(`Invalid JSON from ${url}: ${err.message}`));
        }
      });
    });
    req.on("error", reject);
    req.on("timeout", () => {
      req.destroy(new Error(`Timeout fetching ${url}`));
    });
  });
}

function httpPostJson(url, payload, timeoutMs = 10000) {
  const body = JSON.stringify(payload);
  const parsed = new URL(url);
  return new Promise((resolve, reject) => {
    const req = http.request(
      {
        hostname: parsed.hostname,
        port: Number(parsed.port),
        path: parsed.pathname,
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          "Content-Length": Buffer.byteLength(body),
        },
        timeout: timeoutMs,
      },
      (res) => {
        let data = "";
        res.setEncoding("utf8");
        res.on("data", (chunk) => (data += chunk));
        res.on("end", () => {
          try {
            const json = JSON.parse(data);
            if (res.statusCode < 200 || res.statusCode >= 300) {
              return reject(new Error(json.error || `HTTP ${res.statusCode}`));
            }
            resolve(json);
          } catch (err) {
            reject(new Error(`Invalid JSON response: ${err.message} (${data})`));
          }
        });
      }
    );
    req.on("error", reject);
    req.on("timeout", () => req.destroy(new Error(`Timeout posting ${url}`)));
    req.write(body);
    req.end();
  });
}

// Minimal RFC 6455 WebSocket client for Chrome DevTools Protocol (Node >= 20 compatible, zero deps)
class CdpClient {
  constructor(wsUrl) {
    this.wsUrl = wsUrl;
    this.socket = null;
    this.nextId = 1;
    this.pending = new Map();
    this.buffer = Buffer.alloc(0);
    this.closed = false;
  }

  async connect() {
    const u = new URL(this.wsUrl);
    const key = crypto.randomBytes(16).toString("base64");
    const port = Number(u.port || 80);
    const reqPath = `${u.pathname}${u.search}`;

    await new Promise((resolve, reject) => {
      const sock = net.connect(port, u.hostname, () => {
        const headers = [
          `GET ${reqPath} HTTP/1.1`,
          `Host: ${u.host}`,
          "Upgrade: websocket",
          "Connection: Upgrade",
          `Sec-WebSocket-Key: ${key}`,
          "Sec-WebSocket-Version: 13",
          "",
          "",
        ].join("\r\n");
        sock.write(headers);
      });
      this.socket = sock;

      let handshakeBuf = Buffer.alloc(0);
      const onHandshake = (chunk) => {
        handshakeBuf = Buffer.concat([handshakeBuf, chunk]);
        const idx = handshakeBuf.indexOf("\r\n\r\n");
        if (idx === -1) return;
        sock.removeListener("data", onHandshake);
        const head = handshakeBuf.subarray(0, idx).toString("latin1");
        if (!head.startsWith("HTTP/1.1 101")) {
          return reject(new Error(`CDP WebSocket handshake failed: ${head.split("\r\n")[0]}`));
        }
        const leftover = handshakeBuf.subarray(idx + 4);
        sock.on("data", (d) => this._onData(d));
        sock.on("close", () => this._onClose());
        sock.on("error", () => this._onClose());
        if (leftover.length > 0) this._onData(leftover);
        resolve();
      };
      sock.on("data", onHandshake);
      sock.once("error", reject);
    });
  }

  _sendFrame(opcode, payloadBuf) {
    if (this.closed || !this.socket) throw new Error("CDP socket closed");
    const len = payloadBuf.length;
    const mask = crypto.randomBytes(4);
    let header;
    if (len < 126) {
      header = Buffer.alloc(6);
      header[0] = 0x80 | opcode;
      header[1] = 0x80 | len;
      mask.copy(header, 2);
    } else if (len < 65536) {
      header = Buffer.alloc(8);
      header[0] = 0x80 | opcode;
      header[1] = 0x80 | 126;
      header.writeUInt16BE(len, 2);
      mask.copy(header, 4);
    } else {
      header = Buffer.alloc(14);
      header[0] = 0x80 | opcode;
      header[1] = 0x80 | 127;
      header.writeBigUInt64BE(BigInt(len), 2);
      mask.copy(header, 10);
    }
    const masked = Buffer.alloc(len);
    for (let i = 0; i < len; i++) {
      masked[i] = payloadBuf[i] ^ mask[i & 3];
    }
    this.socket.write(Buffer.concat([header, masked]));
  }

  _onData(chunk) {
    this.buffer = Buffer.concat([this.buffer, chunk]);
    while (this.buffer.length >= 2) {
      const b0 = this.buffer[0];
      const b1 = this.buffer[1];
      const opcode = b0 & 0x0f;
      const isMasked = (b1 & 0x80) !== 0;
      let len = b1 & 0x7f;
      let offset = 2;
      if (len === 126) {
        if (this.buffer.length < 4) return;
        len = this.buffer.readUInt16BE(2);
        offset = 4;
      } else if (len === 127) {
        if (this.buffer.length < 10) return;
        len = Number(this.buffer.readBigUInt64BE(2));
        offset = 10;
      }
      const maskLen = isMasked ? 4 : 0;
      if (this.buffer.length < offset + maskLen + len) return;

      let payload = this.buffer.subarray(offset + maskLen, offset + maskLen + len);
      if (isMasked) {
        const m = this.buffer.subarray(offset, offset + 4);
        const unmasked = Buffer.alloc(len);
        for (let i = 0; i < len; i++) unmasked[i] = payload[i] ^ m[i & 3];
        payload = unmasked;
      }
      this.buffer = this.buffer.subarray(offset + maskLen + len);

      if (opcode === 0x8) {
        this.close();
        return;
      }
      if (opcode === 0x9) {
        this._sendFrame(0x0a, payload);
        continue;
      }
      if (opcode === 0x1 || opcode === 0x0) {
        let msg;
        try {
          msg = JSON.parse(payload.toString("utf8"));
        } catch (_) {
          continue;
        }
        if (typeof msg.id === "number" && this.pending.has(msg.id)) {
          const { resolve, reject } = this.pending.get(msg.id);
          this.pending.delete(msg.id);
          if (msg.error) {
            reject(new Error(`CDP error (${msg.error.code}): ${msg.error.message}`));
          } else {
            resolve(msg.result);
          }
        }
      }
    }
  }

  _onClose() {
    this.closed = true;
    for (const { reject } of this.pending.values()) {
      reject(new Error("CDP connection closed"));
    }
    this.pending.clear();
  }

  send(method, params = {}, timeoutMs = 10000) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        if (this.pending.has(id)) {
          this.pending.delete(id);
          reject(new Error(`CDP timeout waiting for ${method}`));
        }
      }, timeoutMs);
      this.pending.set(id, {
        resolve: (val) => {
          clearTimeout(timer);
          resolve(val);
        },
        reject: (err) => {
          clearTimeout(timer);
          reject(err);
        },
      });
      this._sendFrame(0x1, Buffer.from(JSON.stringify({ id, method, params }), "utf8"));
    });
  }

  async evaluate(expr) {
    const res = await this.send("Runtime.evaluate", {
      expression: expr,
      returnByValue: true,
      awaitPromise: true,
    });
    if (res.exceptionDetails) {
      const detail =
        res.exceptionDetails.exception?.description ||
        res.exceptionDetails.text ||
        "JS exception";
      throw new Error(`Page JS error: ${detail}`);
    }
    return res.result ? res.result.value : undefined;
  }

  close() {
    this.closed = true;
    if (this.socket) {
      try {
        this.socket.destroy();
      } catch (_) {}
      this.socket = null;
    }
  }
}

async function connectCdpForPort(cdpPort, timeoutMs = 15000) {
  const deadline = Date.now() + timeoutMs;
  let lastErr = null;
  while (Date.now() < deadline) {
    try {
      const targets = await httpGetJson(`http://127.0.0.1:${cdpPort}/json/list`, 1500);
      if (Array.isArray(targets)) {
        const page =
          targets.find(
            (t) =>
              t.type === "page" &&
              typeof t.webSocketDebuggerUrl === "string" &&
              !String(t.url || "").startsWith("devtools://")
          ) || targets.find((t) => typeof t.webSocketDebuggerUrl === "string");
        if (page) {
          const client = new CdpClient(page.webSocketDebuggerUrl);
          await client.connect();
          await client.send("Runtime.enable");
          return client;
        }
      }
    } catch (err) {
      lastErr = err;
    }
    await sleep(200);
  }
  throw new Error(
    `Timed out waiting for DevTools CDP on 127.0.0.1:${cdpPort}: ${lastErr ? lastErr.message : "no targets"}`
  );
}

function makeTestProbeEval(probePort) {
  return async function evalProbe(expr) {
    const wrapped = `(function(){ return JSON.stringify((${expr})); })()`;
    const resp = await httpPostJson(`http://127.0.0.1:${probePort}/eval`, { script: wrapped });
    if (!resp.ok) {
      throw new Error(resp.error || "test-probe eval failed");
    }
    let val = JSON.parse(resp.result);
    if (typeof val === "string") {
      val = JSON.parse(val);
    }
    return val;
  };
}

async function waitUntil(fn, timeoutMs, intervalMs = 150, desc = "condition") {
  const deadline = Date.now() + timeoutMs;
  let lastVal;
  let lastErr;
  while (Date.now() < deadline) {
    try {
      lastVal = await fn();
      if (lastVal) return lastVal;
    } catch (err) {
      lastErr = err;
    }
    await sleep(intervalMs);
  }
  throw new Error(
    `Timed out after ${timeoutMs}ms waiting for ${desc}${lastErr ? `: ${lastErr.message}` : ` (last=${JSON.stringify(lastVal)})`}`
  );
}

async function assertFullPageWorks(evalFn, { offlineServer, onlineMode, modeLabel }) {
  // 1. Wait for DOM + libraries + images + fonts
  const state = await waitUntil(
    async () => {
      const s = await evalFn(`(() => {
        if (document.readyState !== 'complete') return null;
        if (!window.gsap || !window.ScrollTrigger || !window.Lenis) return null;
        var nav = document.querySelector('#site-nav nav');
        var imgs = Array.prototype.slice.call(document.querySelectorAll('img[data-remove-on-error]'));
        if (imgs.length < 5) return null;
        var allImgsLoaded = imgs.every(function (i) { return i.complete && i.naturalWidth > 0; });
        if (!allImgsLoaded) return null;
        return {
          navDisplay: nav ? getComputedStyle(nav).display : null,
          hasGsap: Boolean(window.gsap),
          hasScrollTrigger: Boolean(window.ScrollTrigger),
          hasLenis: Boolean(window.Lenis),
          imgCount: imgs.length,
          loadedFontCount: document.fonts ? Array.from(document.fonts).filter(function (f) { return f.status === 'loaded'; }).length : 0,
          stubGoogleFontsVar: getComputedStyle(document.documentElement).getPropertyValue('--stub-google-fonts').trim(),
          stubTailwind: Boolean(window.__stub_tailwind),
          stubGsap: Boolean(window.__stub_gsap),
          stubScrollTrigger: Boolean(window.__stub_scrolltrigger),
          stubLenis: Boolean(window.__stub_lenis)
        };
      })()`);
      return s && s.navDisplay === "flex" ? s : null;
    },
    onlineMode ? 20000 : 12000,
    200,
    `${modeLabel}: page assets, Tailwind nav flex, GSAP, Lenis, and logo images`
  );

  assert.equal(state.navDisplay, "flex", `${modeLabel}: nav display must be flex`);
  assert.equal(state.hasGsap, true, `${modeLabel}: GSAP must be present`);
  assert.equal(state.hasScrollTrigger, true, `${modeLabel}: ScrollTrigger must be present`);
  assert.equal(state.hasLenis, true, `${modeLabel}: Lenis must be present`);
  assert.equal(state.imgCount, 5, `${modeLabel}: all 5 logo images must remain loaded`);

  if (offlineServer) {
    assert.equal(state.stubTailwind, true, `${modeLabel}: stub Tailwind marker missing`);
    assert.equal(state.stubGsap, true, `${modeLabel}: stub GSAP marker missing`);
    assert.equal(state.stubScrollTrigger, true, `${modeLabel}: stub ScrollTrigger marker missing`);
    assert.equal(state.stubLenis, true, `${modeLabel}: stub Lenis marker missing`);
    assert.equal(state.stubGoogleFontsVar, "1", `${modeLabel}: stub Google Fonts CSS missing`);

    await waitUntil(
      async () => {
        const stats = offlineServer.getStats();
        return STUB_HOSTS.every((h) => stats.counts[h] >= 1) ? stats : null;
      },
      6000,
      150,
      `${modeLabel}: all 6 offline origin servers receiving hits`
    );
  } else {
    assert.ok(
      state.loadedFontCount >= 1,
      `${modeLabel}: expected >= 1 Google Font loaded in online mode, got ${state.loadedFontCount}`
    );
  }

  // 2. Skip intro to unlock body scrolling, scroll to stats section, verify counters reach targets
  await evalFn(`(() => {
    var skip = document.getElementById('skip-intro');
    if (skip) skip.click();
    document.body.classList.remove('locked');
    var stats = document.querySelector('.stats-grid');
    if (stats) stats.scrollIntoView({ behavior: 'instant', block: 'center' });
    window.dispatchEvent(new Event('scroll'));
    if (window.ScrollTrigger) window.ScrollTrigger.refresh();
    return true;
  })()`);

  const statsTexts = await waitUntil(
    async () => {
      const vals = await evalFn(
        `Array.prototype.slice.call(document.querySelectorAll('.stat-num')).map(function (el) { return el.textContent.trim(); })`
      );
      if (
        Array.isArray(vals) &&
        vals.length === 4 &&
        vals[0] === "50+" &&
        vals[1] === "30+" &&
        vals[2] === "99.9%" &&
        vals[3] === "24/7"
      ) {
        return vals;
      }
      return null;
    },
    8000,
    200,
    `${modeLabel}: stats counters reaching 50+, 30+, 99.9%, 24/7 after scrolling`
  );
  assert.deepEqual(statsTexts, ["50+", "30+", "99.9%", "24/7"]);

  // 3. Submit quote form and verify success panel shows without page reload
  await evalFn(`(() => {
    window.__f6_no_reload = 'ALIVE';
    document.getElementById('q-name').value = 'Alice Tester';
    document.getElementById('q-email').value = 'alice@example.com';
    document.getElementById('q-company').value = 'Wavical Corp';
    document.getElementById('q-service').value = 'Cyber Security';
    document.getElementById('q-desc').value = 'Automated verification of quote form without page reload';
    document.getElementById('q-submit').click();
    return true;
  })()`);

  const quoteResult = await waitUntil(
    async () => {
      const res = await evalFn(`(() => {
        var success = document.getElementById('quote-success');
        if (!success) return null;
        var hidden = success.classList.contains('hidden');
        var disp = getComputedStyle(success).display;
        if (hidden || disp === 'none') return null;
        return {
          hidden: hidden,
          display: disp,
          heading: (success.querySelector('h3') || {}).textContent || '',
          canary: window.__f6_no_reload || null,
          search: window.location.search || ''
        };
      })()`);
      return res;
    },
    8000,
    200,
    `${modeLabel}: quote form success panel visible without reload`
  );
  assert.equal(quoteResult.canary, "ALIVE", `${modeLabel}: page reloaded during quote submit`);
  assert.equal(quoteResult.search, "", `${modeLabel}: unexpected query string from native GET submit`);
  assert.ok(
    quoteResult.heading.includes("Request Received"),
    `${modeLabel}: expected Request Received heading`
  );
}

async function assertUltraSafeFallback(evalFn, { offlineServer, modeLabel }) {
  const fb = await waitUntil(
    async () => {
      const res = await evalFn(`(() => {
        var pre = document.getElementById('preloader');
        var nav = document.getElementById('site-nav');
        var tag = document.getElementById('tagline-text');
        if (!pre || pre.style.display !== 'none') return null;
        if (document.body.classList.contains('locked')) return null;
        if (!nav || nav.style.opacity !== '1') return null;
        if (!tag || !tag.textContent) return null;
        return {
          hasGsap: typeof window.gsap !== 'undefined',
          hasLenis: typeof window.Lenis !== 'undefined',
          preDisplay: pre.style.display,
          locked: document.body.classList.contains('locked'),
          navOpacity: nav.style.opacity,
          tagline: tag.textContent
        };
      })()`);
      return res;
    },
    10000,
    200,
    `${modeLabel}: hardReveal fallback`
  );

  assert.equal(fb.hasGsap, false, `${modeLabel}: GSAP must be blocked in ultra-safe`);
  assert.equal(fb.hasLenis, false, `${modeLabel}: Lenis must be blocked in ultra-safe`);
  assert.equal(fb.preDisplay, "none", `${modeLabel}: preloader must be hidden by hardReveal`);
  assert.equal(fb.locked, false, `${modeLabel}: body.locked must be removed by hardReveal`);
  assert.equal(fb.navOpacity, "1", `${modeLabel}: #site-nav opacity must be 1`);
  assert.ok(
    fb.tagline.includes("Engineering intelligent, connected & secure products"),
    `${modeLabel}: fallback tagline missing`
  );

  if (offlineServer) {
    const stats = offlineServer.getStats();
    const totalHits = Object.values(stats.counts).reduce((a, b) => a + b, 0);
    assert.equal(
      totalHits,
      0,
      `${modeLabel}: expected 0 external HTTP hits in ultra-safe, got ${JSON.stringify(stats.counts)}`
    );
  }
}

function buildTestProbeBinaryIfNeeded(explicitPath) {
  if (explicitPath) {
    const abs = path.resolve(explicitPath);
    if (!fs.existsSync(abs)) {
      throw new Error(`Test probe binary not found at ${abs}`);
    }
    return abs;
  }
  const exe = process.platform === "win32" ? "wavical-shell-test-probe.exe" : "wavical-shell-test-probe";
  const releaseBin = path.join(ROOT_DIR, "target", "test-probe", "release", exe);
  const debugBin = path.join(ROOT_DIR, "target", "test-probe", "debug", exe);
  if (fs.existsSync(releaseBin)) return releaseBin;
  if (fs.existsSync(debugBin)) return debugBin;

  console.log("Building test-only built-app probe (wavical-shell-test-probe)...");
  const res = spawnSync(
    "cargo",
    [
      "build",
      "--release",
      "--locked",
      "-p",
      "wavical-shell",
      "--bin",
      "wavical-shell-test-probe",
      "--features",
      "test-probe",
      "--target-dir",
      "target/test-probe",
    ],
    { cwd: ROOT_DIR, stdio: "inherit", windowsHide: true }
  );
  if (res.status !== 0 || !fs.existsSync(releaseBin)) {
    throw new Error(`Failed to build wavical-shell-test-probe (exit ${res.status})`);
  }
  return releaseBin;
}

function buildTestHooksDevShellIfNeeded(explicitPath) {
  if (explicitPath) {
    const abs = path.resolve(explicitPath);
    if (!fs.existsSync(abs)) {
      throw new Error(`Test-hooks dev shell binary not found at ${abs}`);
    }
    return abs;
  }
  const exe = process.platform === "win32" ? "wavical-shell-dev.exe" : "wavical-shell-dev";
  const releaseBin = path.join(ROOT_DIR, "target", "dev-shell-test-hooks", "release", exe);
  const debugBin = path.join(ROOT_DIR, "target", "dev-shell-test-hooks", "debug", exe);
  if (fs.existsSync(releaseBin)) return releaseBin;
  if (fs.existsSync(debugBin)) return debugBin;

  console.log("Building test-hooks dev shell (wavical-shell-dev with test-hooks)...");
  const res = spawnSync(
    "cargo",
    [
      "build",
      "--release",
      "--locked",
      "-p",
      "wavical-shell",
      "--bin",
      "wavical-shell-dev",
      "--features",
      "dev-shell,test-hooks",
      "--target-dir",
      "target/dev-shell-test-hooks",
    ],
    { cwd: ROOT_DIR, stdio: "inherit", windowsHide: true }
  );
  if (res.status !== 0 || !fs.existsSync(releaseBin)) {
    throw new Error(`Failed to build test-hooks wavical-shell-dev (exit ${res.status})`);
  }
  return releaseBin;
}

async function runDevCommandSession({
  appDir,
  subcommand,
  localAppData,
  offlineServer,
  testHooksDevShellBin,
  onlineMode,
  fn,
}) {
  const cdpPort = await getFreePort();
  const env = {
    ...process.env,
    LOCALAPPDATA: localAppData,
    WAVICAL_HEADLESS_TEST: "1",
    WAVICAL_DEV_REMOTE_DEBUGGING_PORT: String(cdpPort),
  };
  if (offlineServer && testHooksDevShellBin) {
    env.WAVICAL_DEV_SHELL_BIN = testHooksDevShellBin;
    env.WAVICAL_DEV_TEST_PROXY = offlineServer.proxyUrl;
    env.WAVICAL_DEV_TEST_SPKI = offlineServer.spkiPin;
  }

  const child = spawnNpxWavical([subcommand], { cwd: appDir, env });
  let stdoutBuf = "";
  let stderrBuf = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (d) => (stdoutBuf += d));
  child.stderr.on("data", (d) => (stderrBuf += d));

  const expectedMode =
    subcommand === "dev" ? "open" : subcommand === "dev-safe" ? "safe" : "ultra-safe";

  let cdp = null;
  try {
    await waitUntil(
      () => {
        if (child.exitCode !== null) {
          throw new Error(
            `npx wavical ${subcommand} exited early (${child.exitCode}):\nstdout: ${stdoutBuf}\nstderr: ${stderrBuf}`
          );
        }
        return (
          stdoutBuf.includes(`mode=${expectedMode}`) &&
          stdoutBuf.includes("Dev server running (PID:")
        );
      },
      15000,
      100,
      `npx wavical ${subcommand} startup`
    );

    cdp = await connectCdpForPort(cdpPort, 15000);
    await fn({
      cdp,
      getStdout: () => stdoutBuf,
      getStderr: () => stderrBuf,
    });
  } finally {
    if (cdp) cdp.close();
    await killProcessTree(child);
  }
}

async function runBuiltAppProbe({
  testProbeBin,
  distDir,
  localAppData,
  offlineServer,
  fn,
}) {
  const portFile = path.join(localAppData, "probe-port.txt");
  if (fs.existsSync(portFile)) fs.rmSync(portFile);

  const env = {
    ...process.env,
    LOCALAPPDATA: localAppData,
    WAVICAL_HEADLESS_TEST: "1",
    WAVICAL_TEST_PROBE_PORT_FILE: portFile,
  };
  if (offlineServer) {
    env.WAVICAL_TEST_PROBE_PROXY = offlineServer.proxyUrl;
    env.WAVICAL_TEST_PROBE_SPKI = offlineServer.spkiPin;
  }

  const child = spawn(
    testProbeBin,
    ["--asset-root", distDir, "--single-instance", "false", "--persist-state", "false"],
    { cwd: distDir, env, windowsHide: true }
  );
  let stderrBuf = "";
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (d) => (stderrBuf += d));

  try {
    const probePort = await waitUntil(
      () => {
        if (child.exitCode !== null) {
          throw new Error(`test-probe exited early (${child.exitCode}): ${stderrBuf}`);
        }
        if (!fs.existsSync(portFile)) return null;
        const raw = fs.readFileSync(portFile, "utf8").trim();
        const p = Number(raw);
        return Number.isInteger(p) && p > 0 ? p : null;
      },
      15000,
      100,
      "wavical-shell-test-probe port file"
    );

    await waitUntil(
      async () => {
        try {
          const h = await httpGetJson(`http://127.0.0.1:${probePort}/health`, 1000);
          return h && h.ok;
        } catch (_) {
          return false;
        }
      },
      10000,
      100,
      "wavical-shell-test-probe /health"
    );

    await fn(makeTestProbeEval(probePort));
  } finally {
    await killProcessTree(child);
  }
}

function assertNoShellInDist(distDir) {
  const stack = [distDir];
  while (stack.length > 0) {
    const cur = stack.pop();
    for (const entry of fs.readdirSync(cur, { withFileTypes: true })) {
      const full = path.join(cur, entry.name);
      if (entry.isDirectory()) {
        stack.push(full);
      } else if (entry.name.toLowerCase().startsWith("wavical-shell")) {
        throw new Error(`Security error: built dist/ must not contain ${full}`);
      }
    }
  }
}

async function assertRealNetworkReachable() {
  const checkUrls = [
    "https://cdn.tailwindcss.com",
    "https://cdnjs.cloudflare.com/ajax/libs/gsap/3.12.5/gsap.min.js",
    "https://unpkg.com/lenis@1.1.18/dist/lenis.min.js",
  ];
  for (const u of checkUrls) {
    let resp;
    try {
      resp = await fetch(u, { signal: AbortSignal.timeout(8000) });
    } catch (err) {
      throw new Error(
        `Real-network E2E check failed (network unavailable or blocked fetching ${u}): ${err.message}`
      );
    }
    if (!resp.ok) {
      throw new Error(`Real-network E2E check failed: ${u} returned HTTP ${resp.status}`);
    }
  }
}

async function runReferenceAppE2E({
  tarballPath,
  online = false,
  testProbeBin = null,
  testHooksDevShellBin = null,
} = {}) {
  const absTarball = path.resolve(tarballPath);
  if (!fs.existsSync(absTarball)) {
    throw new Error(`Tarball not found: ${absTarball}`);
  }
  if (!fs.existsSync(FIXTURE_DIR)) {
    throw new Error(`Reference app fixture missing at ${FIXTURE_DIR}`);
  }

  if (online) {
    await assertRealNetworkReachable();
  }

  const probeBin = buildTestProbeBinaryIfNeeded(testProbeBin);
  const devHooksBin = online ? null : buildTestHooksDevShellIfNeeded(testHooksDevShellBin);
  const offlineServer = online ? null : await startMultiOriginTestServer();
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "wavical-e2e-ref-"));
  const appDir = path.join(tempRoot, "app");

  const results = [];
  async function runStep(name, fn) {
    const t0 = Date.now();
    try {
      await fn();
      const ms = Date.now() - t0;
      console.log(`[PASS] ${name} (${ms}ms)`);
      results.push({ name, passed: true, ms });
    } catch (err) {
      const ms = Date.now() - t0;
      console.error(`[FAIL] ${name} (${ms}ms): ${err.message}`);
      results.push({ name, passed: false, ms, error: err.message });
    }
  }

  try {
    fs.cpSync(FIXTURE_DIR, appDir, { recursive: true });
    fs.writeFileSync(
      path.join(appDir, "package.json"),
      JSON.stringify({ name: "reference-app-e2e", version: "1.0.0", private: true }, null, 2),
      "utf8"
    );

    const installRes = runNpmSync(["install", "--no-audit", "--no-fund", absTarball], {
      cwd: appDir,
      timeout: 60000,
    });
    if (installRes.status !== 0) {
      throw new Error(
        `npm install ${absTarball} failed:\n${installRes.stderr || installRes.stdout}`
      );
    }

    const installedPkg = JSON.parse(
      fs.readFileSync(path.join(appDir, "node_modules", "wavical", "package.json"), "utf8")
    );
    const expectedVersion = installedPkg.version;
    const modeTag = online ? "online" : "offline";
    console.log(
      `Running F6 reference-app E2E (${modeTag}) against ${path.basename(absTarball)} (package version ${expectedVersion})...`
    );

    await runStep("cli: npx wavical --version matches tarball version", async () => {
      const verRes = runNpxWavicalSync(["--version"], { cwd: appDir, timeout: 25000 });
      assert.equal(
        verRes.status,
        0,
        `npx wavical --version failed (${verRes.error ? verRes.error.message : verRes.status}): ${verRes.stderr}`
      );
      const out = (verRes.stdout || "").trim();
      assert.ok(
        out.includes(expectedVersion),
        `Expected version '${expectedVersion}' in '${out}'`
      );

      const devShellBin = path.join(
        appDir,
        "node_modules",
        "wavical",
        "bin",
        process.platform === "win32" ? "wavical-shell-dev.exe" : "wavical-shell-dev"
      );
      const devHelpRes = spawnSync(devShellBin, ["--help"], { encoding: "utf8", windowsHide: true });
      assert.equal(devHelpRes.status, 0, "installed wavical-shell-dev --help failed");
      assert.ok(
        devHelpRes.stdout.includes("--remote-debugging-port"),
        "installed wavical-shell-dev --help must keep --remote-debugging-port"
      );
      assert.ok(
        !devHelpRes.stdout.includes("--test-proxy") && !devHelpRes.stdout.includes("--test-spki"),
        `installed wavical-shell-dev --help must NOT expose --test-proxy or --test-spki:\n${devHelpRes.stdout}`
      );
    });

    await runStep(`dev (open) + hot reload (${modeTag})`, async () => {
      if (offlineServer) offlineServer.resetCounts();
      const localAppData = path.join(tempRoot, "lad-dev-open");
      fs.mkdirSync(localAppData, { recursive: true });

      const indexPath = path.join(appDir, "src", "index.html");
      const origHtml = fs.readFileSync(indexPath, "utf8");
      try {
        await runDevCommandSession({
          appDir,
          subcommand: "dev",
          localAppData,
          offlineServer,
          testHooksDevShellBin: devHooksBin,
          onlineMode: online,
          fn: async ({ cdp, getStdout }) => {
            const startupOut = getStdout();
            assert.ok(
              startupOut.includes("Starting Wavical dev"),
              `Expected 'Starting Wavical dev' banner in stdout, got: ${startupOut}`
            );
            assert.match(
              startupOut,
              /startup=\d+ms/,
              `Expected timing breakdown in F3 startup line, got: ${startupOut}`
            );
            await assertFullPageWorks((expr) => cdp.evaluate(expr), {
              offlineServer,
              onlineMode: online,
              modeLabel: "dev (open)",
            });

            // Hot reload check
            await sleep(300);
            const markerId = "f6-hot-reload-probe";
            const modified = origHtml.replace(
              "</body>",
              `<div id="${markerId}">HOT_RELOADED_OK</div>\n</body>`
            );
            assert.notEqual(modified, origHtml, "index.html must contain </body>");
            fs.writeFileSync(indexPath, modified, "utf8");

            const reloadLine = await waitUntil(
              () => {
                const m = getStdout().match(/Reloaded in (\d+)ms/);
                return m ? m : null;
              },
              10000,
              100,
              "CLI Reloaded timing line"
            );
            const elapsedMs = Number(reloadLine[1]);
            assert.ok(elapsedMs >= 1, `Expected real reload time >= 1ms, got ${elapsedMs}ms`);

            const probeText = await waitUntil(
              async () => {
                try {
                  return await cdp.evaluate(
                    `(document.getElementById('${markerId}') || {}).textContent || null`
                  );
                } catch (_) {
                  return null;
                }
              },
              8000,
              150,
              "hot-reloaded DOM marker"
            );
            assert.equal(probeText, "HOT_RELOADED_OK");
          },
        });
      } finally {
        fs.writeFileSync(indexPath, origHtml, "utf8");
      }
    });

    await runStep(`dev-safe (${modeTag})`, async () => {
      if (offlineServer) offlineServer.resetCounts();
      const localAppData = path.join(tempRoot, "lad-dev-safe");
      fs.mkdirSync(localAppData, { recursive: true });

      await runDevCommandSession({
        appDir,
        subcommand: "dev-safe",
        localAppData,
        offlineServer,
        testHooksDevShellBin: devHooksBin,
        onlineMode: online,
        fn: async ({ cdp }) => {
          await assertFullPageWorks((expr) => cdp.evaluate(expr), {
            offlineServer,
            onlineMode: online,
            modeLabel: "dev-safe",
          });
        },
      });
    });

    await runStep(`dev-ultra-safe (${modeTag})`, async () => {
      if (offlineServer) offlineServer.resetCounts();
      const localAppData = path.join(tempRoot, "lad-dev-ultra");
      fs.mkdirSync(localAppData, { recursive: true });

      // dev-ultra-safe blocks all external origins at CSP, so it runs directly against the shipped wavical-shell-dev.exe
      await runDevCommandSession({
        appDir,
        subcommand: "dev-ultra-safe",
        localAppData,
        offlineServer,
        testHooksDevShellBin: null,
        onlineMode: online,
        fn: async ({ cdp, getStderr }) => {
          await assertUltraSafeFallback((expr) => cdp.evaluate(expr), {
            offlineServer,
            modeLabel: "dev-ultra-safe",
          });

          const expectedBlockedHosts = [
            "cdn.tailwindcss.com",
            "cdnjs.cloudflare.com",
            "unpkg.com",
            "fonts.googleapis.com",
            "www.designarena.ai",
          ];
          const diagText = await waitUntil(
            () => {
              const s = getStderr();
              const hasAll = expectedBlockedHosts.every((h) => s.includes(h));
              const hasHint =
                s.includes("allowlist") || s.includes("vendor") || s.includes("dev-safe");
              return hasAll && hasHint ? s : null;
            },
            6000,
            150,
            "F3 blocked diagnostics with hints on dev-ultra-safe stderr"
          );
          assert.ok(diagText.includes("blocked"), "expected 'blocked' diagnostic lines");
        },
      });
    });

    await runStep(`build --allow-open (${modeTag})`, async () => {
      if (offlineServer) offlineServer.resetCounts();
      const outDir = path.join(appDir, "dist-open");
      const buildRes = runNpxWavicalSync(
        ["build", "--allow-open", "--outdir", outDir, appDir],
        { cwd: appDir, timeout: 60000 }
      );
      assert.equal(
        buildRes.status,
        0,
        `build --allow-open failed:\n${buildRes.stderr || buildRes.stdout}`
      );
      assertNoShellInDist(outDir);
      const manifest = JSON.parse(
        fs.readFileSync(path.join(outDir, "wavical-manifest.json"), "utf8")
      );
      assert.equal(manifest.mode, "open");

      const localAppData = path.join(tempRoot, "lad-build-open");
      fs.mkdirSync(localAppData, { recursive: true });
      await runBuiltAppProbe({
        testProbeBin: probeBin,
        distDir: outDir,
        localAppData,
        offlineServer,
        fn: async (evalFn) => {
          await assertFullPageWorks(evalFn, {
            offlineServer,
            onlineMode: online,
            modeLabel: "build --allow-open",
          });
        },
      });
    });

    await runStep(`build-safe (${modeTag})`, async () => {
      if (offlineServer) offlineServer.resetCounts();
      const outDir = path.join(appDir, "dist-safe");
      const buildRes = runNpxWavicalSync(
        ["build-safe", "--outdir", outDir, appDir],
        { cwd: appDir, timeout: 60000 }
      );
      assert.equal(
        buildRes.status,
        0,
        `build-safe failed:\n${buildRes.stderr || buildRes.stdout}`
      );
      assertNoShellInDist(outDir);
      const manifest = JSON.parse(
        fs.readFileSync(path.join(outDir, "wavical-manifest.json"), "utf8")
      );
      assert.equal(manifest.mode, "safe");
      assert.ok(manifest.allowlistSha256, "build-safe manifest must include allowlistSha256");
      assert.ok(
        fs.existsSync(path.join(outDir, "wavical.allowlist.json")),
        "build-safe must copy wavical.allowlist.json into dist"
      );

      const localAppData = path.join(tempRoot, "lad-build-safe");
      fs.mkdirSync(localAppData, { recursive: true });
      await runBuiltAppProbe({
        testProbeBin: probeBin,
        distDir: outDir,
        localAppData,
        offlineServer,
        fn: async (evalFn) => {
          await assertFullPageWorks(evalFn, {
            offlineServer,
            onlineMode: online,
            modeLabel: "build-safe",
          });
        },
      });
    });

    await runStep(`build-ultra-safe (${modeTag})`, async () => {
      if (offlineServer) offlineServer.resetCounts();
      const outDir = path.join(appDir, "dist-ultra");
      const buildRes = runNpxWavicalSync(
        ["build-ultra-safe", "--outdir", outDir, appDir],
        { cwd: appDir, timeout: 60000 }
      );
      assert.equal(
        buildRes.status,
        0,
        `build-ultra-safe failed:\n${buildRes.stderr || buildRes.stdout}`
      );
      assertNoShellInDist(outDir);
      const manifest = JSON.parse(
        fs.readFileSync(path.join(outDir, "wavical-manifest.json"), "utf8")
      );
      assert.equal(manifest.mode, "ultra-safe");

      const localAppData = path.join(tempRoot, "lad-build-ultra");
      fs.mkdirSync(localAppData, { recursive: true });
      await runBuiltAppProbe({
        testProbeBin: probeBin,
        distDir: outDir,
        localAppData,
        offlineServer,
        fn: async (evalFn) => {
          await assertUltraSafeFallback(evalFn, {
            offlineServer,
            modeLabel: "build-ultra-safe",
          });
        },
      });
    });

    const failed = results.filter((r) => !r.passed);
    if (failed.length > 0) {
      throw new Error(
        `Reference app E2E failed (${failed.length}/${results.length} steps failed):\n` +
          failed.map((f) => `- ${f.name}: ${f.error}`).join("\n")
      );
    }

    console.log(`All ${results.length} reference-app E2E checks passed (${modeTag}).`);
    return results;
  } finally {
    if (offlineServer) await offlineServer.close();
    fs.rmSync(tempRoot, { recursive: true, force: true, maxRetries: 5, retryDelay: 250 });
  }
}

function parseArgs(argv) {
  const opts = {
    tarball: null,
    online: false,
    testProbeBin: null,
    testHooksDevShellBin: null,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--tarball" && i + 1 < argv.length) {
      opts.tarball = argv[++i];
    } else if (a === "--online") {
      opts.online = true;
    } else if (a === "--test-probe-bin" && i + 1 < argv.length) {
      opts.testProbeBin = argv[++i];
    } else if (a === "--test-hooks-dev-shell-bin" && i + 1 < argv.length) {
      opts.testHooksDevShellBin = argv[++i];
    } else if (!a.startsWith("-") && a.endsWith(".tgz")) {
      opts.tarball = a;
    } else {
      throw new Error(`Unknown argument: ${a}`);
    }
  }
  if (!opts.tarball) {
    throw new Error("Usage: node scripts/e2e-reference-app.js --tarball <path.tgz> [--online]");
  }
  return opts;
}

if (require.main === module) {
  (async () => {
    try {
      const opts = parseArgs(process.argv.slice(2));
      await runReferenceAppE2E({
        tarballPath: opts.tarball,
        online: opts.online,
        testProbeBin: opts.testProbeBin,
        testHooksDevShellBin: opts.testHooksDevShellBin,
      });
    } catch (err) {
      console.error(`error: ${err.message}`);
      process.exit(1);
    }
  })();
}

module.exports = {
  CdpClient,
  runReferenceAppE2E,
};
