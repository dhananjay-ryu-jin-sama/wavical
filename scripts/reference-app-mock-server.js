#!/usr/bin/env node
"use strict";

const crypto = require("node:crypto");
const https = require("node:https");
const net = require("node:net");

const STUB_HOSTS = [
  "cdn.tailwindcss.com",
  "cdnjs.cloudflare.com",
  "unpkg.com",
  "fonts.googleapis.com",
  "fonts.gstatic.com",
  "www.designarena.ai",
];

// 1x1 transparent PNG (67 bytes)
const TRANSPARENT_1X1_PNG = Buffer.from(
  "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=",
  "base64"
);

// Build a minimal valid WOFF1 font containing head, hhea, maxp, OS/2, hmtx, cmap, loca, glyf, name, post
// with one empty glyph + space/ASCII coverage so Edge/Chromium loads @font-face cleanly with zero errors.
function buildMinimalWoffFont() {
  const tables = new Map();

  // head (54 bytes)
  const head = Buffer.alloc(54);
  head.writeUInt32BE(0x00010000, 0); // major.minor
  head.writeUInt32BE(0x00010000, 4); // fontRevision
  head.writeUInt32BE(0x5f0f3cf5, 12); // magicNumber
  head.writeUInt16BE(0x000b, 16); // flags
  head.writeUInt16BE(1000, 18); // unitsPerEm
  head.writeInt16BE(1000, 40); // xMax
  head.writeInt16BE(1000, 42); // yMax
  head.writeInt16BE(0, 50); // indexToLocFormat = short (Offset16/2)
  tables.set("head", head);

  // hhea (36 bytes)
  const hhea = Buffer.alloc(36);
  hhea.writeUInt32BE(0x00010000, 0);
  hhea.writeInt16BE(800, 4); // ascender
  hhea.writeInt16BE(-200, 6); // descender
  hhea.writeUInt16BE(500, 10); // advanceWidthMax
  hhea.writeUInt16BE(1, 34); // numberOfHMetrics
  tables.set("hhea", hhea);

  // maxp (32 bytes, version 1.0 for TrueType)
  const maxp = Buffer.alloc(32);
  maxp.writeUInt32BE(0x00010000, 0);
  maxp.writeUInt16BE(2, 4); // numGlyphs = 2 (.notdef + space/A)
  tables.set("maxp", maxp);

  // OS/2 (78 bytes, version 1)
  const os2 = Buffer.alloc(78);
  os2.writeUInt16BE(1, 0);
  os2.writeInt16BE(500, 2); // xAvgCharWidth
  os2.writeUInt16BE(400, 4); // usWeightClass
  os2.writeUInt16BE(5, 6); // usWidthClass
  os2.writeUInt16BE(0x0040, 62); // fsSelection REGULAR
  os2.writeUInt16BE(32, 64); // usFirstCharIndex
  os2.writeUInt16BE(122, 66); // usLastCharIndex
  os2.writeInt16BE(800, 68); // sTypoAscender
  os2.writeInt16BE(-200, 70); // sTypoDescender
  os2.writeUInt16BE(800, 74); // usWinAscent
  os2.writeUInt16BE(200, 76); // usWinDescent
  tables.set("OS/2", os2);

  // hmtx (2 glyphs: 1 longHorMetric (4 bytes) + 1 lsb (2 bytes))
  const hmtx = Buffer.alloc(6);
  hmtx.writeUInt16BE(500, 0);
  hmtx.writeInt16BE(0, 2);
  hmtx.writeInt16BE(0, 4);
  tables.set("hmtx", hmtx);

  // cmap format 4 mapping U+0020..U+007E -> glyph 1
  const cmap = Buffer.alloc(36);
  cmap.writeUInt16BE(0, 0); // version
  cmap.writeUInt16BE(1, 2); // numTables
  cmap.writeUInt16BE(3, 4); // platformID = Windows
  cmap.writeUInt16BE(1, 6); // encodingID = Unicode BMP
  cmap.writeUInt32BE(12, 8); // offset
  // Subtable at offset 12 (24 bytes? format 4 with 2 segments = 14 + 2 + 4*2*2 = 32 -> adjust)
  const sub = Buffer.alloc(32);
  sub.writeUInt16BE(4, 0); // format 4
  sub.writeUInt16BE(32, 2); // length
  sub.writeUInt16BE(0, 4); // language
  sub.writeUInt16BE(4, 6); // segCountX2 = 2*2
  sub.writeUInt16BE(4, 8); // searchRange
  sub.writeUInt16BE(1, 10); // entrySelector
  sub.writeUInt16BE(0, 12); // rangeShift
  sub.writeUInt16BE(0x007e, 14); // endCode[0]
  sub.writeUInt16BE(0xffff, 16); // endCode[1]
  sub.writeUInt16BE(0, 18); // reservedPad
  sub.writeUInt16BE(0x0020, 20); // startCode[0]
  sub.writeUInt16BE(0xffff, 22); // startCode[1]
  sub.writeInt16BE(-31, 24); // idDelta[0] (32 - 31 = 1)
  sub.writeInt16BE(1, 26); // idDelta[1]
  sub.writeUInt16BE(0, 28); // idRangeOffset[0]
  sub.writeUInt16BE(0, 30); // idRangeOffset[1]
  const cmapFull = Buffer.concat([cmap.subarray(0, 12), sub]);
  tables.set("cmap", cmapFull);

  // loca (short format, 3 entries of uint16 = 6 bytes, all 0 -> empty glyphs)
  const loca = Buffer.alloc(6);
  tables.set("loca", loca);

  // glyf (4 bytes padding)
  const glyf = Buffer.alloc(4);
  tables.set("glyf", glyf);

  // name (minimal empty header)
  const nameTbl = Buffer.alloc(6);
  tables.set("name", nameTbl);

  // post (32 bytes, version 3.0)
  const post = Buffer.alloc(32);
  post.writeUInt32BE(0x00030000, 0);
  tables.set("post", post);

  const sortedTags = Array.from(tables.keys()).sort();
  const numTables = sortedTags.length;

  // Build WOFF1 container
  const woffHeaderSize = 44;
  const woffDirSize = numTables * 20;
  let offset = woffHeaderSize + woffDirSize;
  const dirEntries = [];
  const dataChunks = [];
  let totalSfntSize = 12 + numTables * 16;

  for (const tag of sortedTags) {
    const raw = tables.get(tag);
    const padLen = (4 - (raw.length % 4)) % 4;
    const padded = padLen > 0 ? Buffer.concat([raw, Buffer.alloc(padLen)]) : raw;

    // Simple checksum
    let sum = 0;
    for (let i = 0; i < padded.length; i += 4) {
      sum = (sum + padded.readUInt32BE(i)) >>> 0;
    }

    const entry = Buffer.alloc(20);
    entry.write(tag.padEnd(4, " "), 0, 4, "ascii");
    entry.writeUInt32BE(offset, 4);
    entry.writeUInt32BE(raw.length, 8); // compLength == origLength (uncompressed)
    entry.writeUInt32BE(raw.length, 12); // origLength
    entry.writeUInt32BE(sum, 16); // origChecksum
    dirEntries.push(entry);
    dataChunks.push(padded);
    offset += padded.length;
    totalSfntSize += padded.length;
  }

  const woffHeader = Buffer.alloc(44);
  woffHeader.writeUInt32BE(0x774f4646, 0); // 'wOFF'
  woffHeader.writeUInt32BE(0x00010000, 4); // flavor
  woffHeader.writeUInt32BE(offset, 8); // total length
  woffHeader.writeUInt16BE(numTables, 12);
  woffHeader.writeUInt16BE(0, 14);
  woffHeader.writeUInt32BE(totalSfntSize, 16);
  woffHeader.writeUInt16BE(1, 20);
  woffHeader.writeUInt16BE(0, 22);

  return Buffer.concat([woffHeader, ...dirEntries, ...dataChunks]);
}

const STUB_WOFF_FONT = buildMinimalWoffFont();

// --- Minimal ASN.1 DER X.509 v3 Certificate Generator (zero external dependencies) ---

function derLen(len) {
  if (len < 0x80) return Buffer.from([len]);
  const bytes = [];
  let n = len;
  while (n > 0) {
    bytes.unshift(n & 0xff);
    n >>= 8;
  }
  return Buffer.from([0x80 | bytes.length, ...bytes]);
}

function derTag(tag, content) {
  const buf = Buffer.isBuffer(content) ? content : Buffer.concat(content);
  return Buffer.concat([Buffer.from([tag]), derLen(buf.length), buf]);
}

const derSeq = (...items) => derTag(0x30, items);
const derSet = (...items) => derTag(0x31, items);
const derUtf8 = (s) => derTag(0x0c, Buffer.from(s, "utf8"));
const derOctet = (buf) => derTag(0x04, buf);
const derBitString = (buf) => derTag(0x03, Buffer.concat([Buffer.from([0x00]), buf]));
const derInt = (buf) => {
  let b = Buffer.isBuffer(buf) ? buf : Buffer.from([buf]);
  if (b[0] & 0x80) b = Buffer.concat([Buffer.from([0x00]), b]);
  return derTag(0x02, b);
};
const derUtcTime = (date) => {
  const pad = (n) => String(n).padStart(2, "0");
  const s =
    String(date.getUTCFullYear()).slice(-2) +
    pad(date.getUTCMonth() + 1) +
    pad(date.getUTCDate()) +
    pad(date.getUTCHours()) +
    pad(date.getUTCMinutes()) +
    pad(date.getUTCSeconds()) +
    "Z";
  return derTag(0x17, Buffer.from(s, "ascii"));
};

const OID_SHA256_RSA = Buffer.from("06092a864886f70d01010b0500", "hex");
const OID_CN = Buffer.from("0603550403", "hex");
const OID_KEY_USAGE = Buffer.from("0603551d0f", "hex");
const OID_SAN = Buffer.from("0603551d11", "hex");
const OID_BASIC_CONSTRAINTS = Buffer.from("0603551d13", "hex");
const OID_EXT_KEY_USAGE = Buffer.from("0603551d25", "hex");
const OID_SERVER_AUTH = Buffer.from("06082b06010505070301", "hex");

function generateOfflineTestCert(hosts) {
  const { publicKey, privateKey } = crypto.generateKeyPairSync("rsa", {
    modulusLength: 2048,
    publicKeyEncoding: { type: "spki", format: "der" },
    privateKeyEncoding: { type: "pkcs8", format: "pem" },
  });

  const spkiPin = crypto.createHash("sha256").update(publicKey).digest("base64");
  const serial = crypto.randomBytes(16);
  serial[0] &= 0x7f;
  if (serial[0] === 0) serial[0] = 1;

  const now = Date.now();
  const notBefore = new Date(now - 24 * 3600 * 1000);
  const notAfter = new Date(now + 30 * 24 * 3600 * 1000);

  const issuer = derSeq(derSet(derSeq(OID_CN, derUtf8("Wavical Offline Test Server"))));
  const subject = derSeq(derSet(derSeq(OID_CN, derUtf8(hosts[0]))));

  // Extensions
  const extBasicConstraints = derSeq(
    OID_BASIC_CONSTRAINTS,
    derTag(0x01, Buffer.from([0xff])), // critical = TRUE
    derOctet(derSeq()) // cA = FALSE (default)
  );
  // keyUsage: digitalSignature (bit 0) + keyEncipherment (bit 2) = 0xa0, 5 unused bits -> 03 02 05 a0
  const extKeyUsage = derSeq(
    OID_KEY_USAGE,
    derTag(0x01, Buffer.from([0xff])),
    derOctet(derTag(0x03, Buffer.from([0x05, 0xa0])))
  );
  const extExtendedKeyUsage = derSeq(
    OID_EXT_KEY_USAGE,
    derOctet(derSeq(OID_SERVER_AUTH))
  );
  const sanEntries = hosts.map((h) => derTag(0x82, Buffer.from(h, "ascii")));
  const extSan = derSeq(OID_SAN, derOctet(derSeq(...sanEntries)));

  const tbs = derSeq(
    derTag(0xa0, derInt(2)), // v3
    derInt(serial),
    derSeq(OID_SHA256_RSA),
    issuer,
    derSeq(derUtcTime(notBefore), derUtcTime(notAfter)),
    subject,
    publicKey,
    derTag(0xa3, derSeq(extBasicConstraints, extKeyUsage, extExtendedKeyUsage, extSan))
  );

  const signature = crypto.sign("sha256", tbs, privateKey);
  const certDer = derSeq(tbs, derSeq(OID_SHA256_RSA), derBitString(signature));
  const b64 = certDer.toString("base64").match(/.{1,64}/g).join("\n");
  const certPem = `-----BEGIN CERTIFICATE-----\n${b64}\n-----END CERTIFICATE-----\n`;

  return { keyPem: privateKey, certPem, spkiPin };
}

// --- Stand-in payloads for the 6 reference-app origins ---

function tailwindStubJs() {
  return `
window.__stub_tailwind = true;
(function () {
  var css = [
    '.hidden { display: none !important; }',
    '.flex { display: flex; }',
    '.inline-flex { display: inline-flex; }',
    '.items-center { align-items: center; }',
    '.justify-between { justify-content: space-between; }',
    '.justify-center { justify-content: center; }',
    '.flex-col { flex-direction: column; }',
    '.flex-wrap { flex-wrap: wrap; }',
    '.mx-auto { margin-left: auto; margin-right: auto; }',
    '.max-w-7xl { max-width: 80rem; }',
    '.max-w-6xl { max-width: 72rem; }',
    '.grid { display: grid; }',
    '.relative { position: relative; }',
    '.text-center { text-align: center; }',
    '@media (min-width: 768px) { .md\\\\:flex { display: flex !important; } .md\\\\:hidden { display: none !important; } }'
  ].join('\\n');
  var s = document.createElement('style');
  s.id = 'stub-tailwind-css';
  s.textContent = css;
  (document.head || document.documentElement).appendChild(s);
})();
`;
}

function gsapStubJs() {
  return `
window.__stub_gsap = true;
(function () {
  var triggers = [];
  var tickerCbs = [];
  function resolveTargets(target) {
    if (!target) return [];
    if (typeof target === 'string') return Array.prototype.slice.call(document.querySelectorAll(target));
    if (Array.isArray(target)) {
      var out = [];
      target.forEach(function (t) { out = out.concat(resolveTargets(t)); });
      return out;
    }
    return [target];
  }
  function applyVars(target, vars) {
    if (!vars) return;
    var list = resolveTargets(target);
    list.forEach(function (el) {
      if (el && el.style) {
        if (vars.opacity !== undefined) el.style.opacity = String(vars.opacity);
        if (vars.display !== undefined) el.style.display = String(vars.display);
        if (vars.scaleX !== undefined) el.style.transform = 'scaleX(' + vars.scaleX + ')';
      } else if (el && typeof el === 'object') {
        Object.keys(vars).forEach(function (k) {
          if (['duration','ease','delay','repeat','yoyo','stagger','scrollTrigger','onUpdate','onComplete','clearProps','svgOrigin','transformOrigin','force3D'].indexOf(k) === -1) {
            el[k] = vars[k];
          }
        });
      }
    });
  }
  function runTween(target, vars) {
    var st = vars && vars.scrollTrigger ? Object.assign({}, vars.scrollTrigger) : null;
    function fire() {
      applyVars(target, vars);
      if (vars && typeof vars.onUpdate === 'function') {
        try { vars.onUpdate(); } catch (_) {}
      }
      if (vars && typeof vars.onComplete === 'function') {
        try { vars.onComplete(); } catch (_) {}
      }
    }
    if (st) {
      var entry = { cfg: st, fire: fire, fired: false };
      triggers.push(entry);
      checkTriggers();
    } else {
      setTimeout(fire, 10);
    }
    return {
      kill: function () {},
      progress: function () { return this; },
      scrollTrigger: st ? { kill: function () {} } : null
    };
  }
  function checkTriggers() {
    var vh = window.innerHeight || 800;
    triggers.forEach(function (entry) {
      var trig = entry.cfg.trigger;
      var el = typeof trig === 'string' ? document.querySelector(trig) : trig;
      var active = false;
      if (el && typeof el.getBoundingClientRect === 'function') {
        var r = el.getBoundingClientRect();
        active = r.top <= vh * 0.98 && r.bottom >= 0;
      } else {
        active = true;
      }
      if (typeof entry.cfg.onToggle === 'function') {
        try { entry.cfg.onToggle({ isActive: active }); } catch (_) {}
      }
      if (active && !entry.fired && typeof entry.fire === 'function') {
        entry.fired = true;
        entry.fire();
      }
    });
  }
  window.__stub_checkTriggers = checkTriggers;
  window.addEventListener('scroll', checkTriggers, { passive: true });

  window.gsap = {
    registerPlugin: function (plugin) {
      if (plugin && plugin.__registerTriggerHook) {
        plugin.__registerTriggerHook(triggers, checkTriggers);
      }
    },
    set: function (target, vars) { applyVars(target, vars); },
    to: function (target, vars) { return runTween(target, vars); },
    fromTo: function (target, fromVars, toVars) {
      applyVars(target, fromVars);
      return runTween(target, toVars);
    },
    timeline: function () {
      var tl = {
        to: function (t, v) { runTween(t, v); return tl; },
        fromTo: function (t, f, v) { applyVars(t, f); runTween(t, v); return tl; },
        add: function (fn) { if (typeof fn === 'function') setTimeout(fn, 10); return tl; },
        play: function () { return tl; },
        progress: function () { return tl; },
        kill: function () {}
      };
      return tl;
    },
    ticker: {
      add: function (cb) { if (typeof cb === 'function') tickerCbs.push(cb); },
      lagSmoothing: function () {}
    },
    utils: {
      toArray: function (sel) {
        return typeof sel === 'string' ? Array.prototype.slice.call(document.querySelectorAll(sel)) : (Array.isArray(sel) ? sel : [sel]);
      }
    }
  };
})();
`;
}

function scrollTriggerStubJs() {
  return `
window.__stub_scrolltrigger = true;
(function () {
  var triggersRef = null;
  var checkFn = null;
  window.ScrollTrigger = {
    __registerTriggerHook: function (tr, fn) { triggersRef = tr; checkFn = fn; },
    create: function (cfg) {
      if (triggersRef) triggersRef.push({ cfg: cfg || {}, fired: false });
      if (checkFn) checkFn();
      return { kill: function () {} };
    },
    refresh: function () {
      if (checkFn) checkFn();
    },
    update: function () {
      if (checkFn) checkFn();
    }
  };
})();
`;
}

function lenisStubJs() {
  return `
window.__stub_lenis = true;
window.Lenis = function Lenis(opts) {
  this.opts = opts || {};
  this._handlers = {};
};
window.Lenis.prototype.on = function (ev, fn) {
  (this._handlers[ev] = this._handlers[ev] || []).push(fn);
};
window.Lenis.prototype.raf = function () {};
window.Lenis.prototype.start = function () {};
window.Lenis.prototype.stop = function () {};
window.Lenis.prototype.scrollTo = function (target) {
  if (typeof target === 'number') {
    window.scrollTo(0, target);
  } else if (target && typeof target.scrollIntoView === 'function') {
    target.scrollIntoView();
  }
  if (window.__stub_checkTriggers) window.__stub_checkTriggers();
};
`;
}

function googleFontsStubCss() {
  return `
@font-face {
  font-family: 'Syne';
  font-style: normal;
  font-weight: 600 800;
  font-display: swap;
  src: url('https://fonts.gstatic.com/s/syne/v22/stub-syne.woff') format('woff');
}
@font-face {
  font-family: 'Space Grotesk';
  font-style: normal;
  font-weight: 300 700;
  font-display: swap;
  src: url('https://fonts.gstatic.com/s/spacegrotesk/v16/stub-sg.woff') format('woff');
}
@font-face {
  font-family: 'JetBrains Mono';
  font-style: normal;
  font-weight: 400 600;
  font-display: swap;
  src: url('https://fonts.gstatic.com/s/jetbrainsmono/v18/stub-jbm.woff') format('woff');
}
:root { --stub-google-fonts: 1; }
`;
}

async function startMultiOriginTestServer() {
  const { keyPem, certPem, spkiPin } = generateOfflineTestCert(STUB_HOSTS);

  const hits = new Map();
  const connectAttempts = [];
  const unexpectedHosts = [];
  for (const h of STUB_HOSTS) {
    hits.set(h, []);
  }

  function recordHit(host, req) {
    hits.get(host).push({
      method: req.method,
      url: req.url,
      ts: Date.now(),
    });
  }

  const handlers = {
    "cdn.tailwindcss.com": (req, res) => {
      recordHit("cdn.tailwindcss.com", req);
      res.writeHead(200, { "Content-Type": "application/javascript; charset=utf-8" });
      res.end(tailwindStubJs());
    },
    "cdnjs.cloudflare.com": (req, res) => {
      recordHit("cdnjs.cloudflare.com", req);
      if (req.url.includes("ScrollTrigger.min.js")) {
        res.writeHead(200, { "Content-Type": "application/javascript; charset=utf-8" });
        res.end(scrollTriggerStubJs());
      } else if (req.url.includes("gsap.min.js")) {
        res.writeHead(200, { "Content-Type": "application/javascript; charset=utf-8" });
        res.end(gsapStubJs());
      } else {
        res.writeHead(404);
        res.end("Not Found");
      }
    },
    "unpkg.com": (req, res) => {
      recordHit("unpkg.com", req);
      if (req.url.includes("lenis.min.js")) {
        res.writeHead(200, { "Content-Type": "application/javascript; charset=utf-8" });
        res.end(lenisStubJs());
      } else {
        res.writeHead(404);
        res.end("Not Found");
      }
    },
    "fonts.googleapis.com": (req, res) => {
      recordHit("fonts.googleapis.com", req);
      res.writeHead(200, { "Content-Type": "text/css; charset=utf-8" });
      res.end(googleFontsStubCss());
    },
    "fonts.gstatic.com": (req, res) => {
      recordHit("fonts.gstatic.com", req);
      res.writeHead(200, {
        "Content-Type": "font/woff",
        "Access-Control-Allow-Origin": "*",
      });
      res.end(STUB_WOFF_FONT);
    },
    "www.designarena.ai": (req, res) => {
      recordHit("www.designarena.ai", req);
      res.writeHead(200, { "Content-Type": "image/png" });
      res.end(TRANSPARENT_1X1_PNG);
    },
  };

  const originServers = new Map();
  const originPorts = new Map();

  for (const host of STUB_HOSTS) {
    const srv = https.createServer({ key: keyPem, cert: certPem }, handlers[host]);
    await new Promise((resolve, reject) => {
      srv.once("error", reject);
      srv.listen(0, "127.0.0.1", () => resolve());
    });
    originServers.set(host, srv);
    originPorts.set(host, srv.address().port);
  }

  // HTTP CONNECT proxy bound to 127.0.0.1:0 routing each host to its own local origin server
  const proxyServer = net.createServer((clientSocket) => {
    let buffered = Buffer.alloc(0);
    function onData(chunk) {
      buffered = Buffer.concat([buffered, chunk]);
      const headerEnd = buffered.indexOf("\r\n\r\n");
      if (headerEnd === -1) {
        if (buffered.length > 16384) clientSocket.destroy();
        return;
      }
      clientSocket.removeListener("data", onData);

      const headerStr = buffered.subarray(0, headerEnd).toString("latin1");
      const firstLine = headerStr.split("\r\n")[0] || "";
      const rest = buffered.subarray(headerEnd + 4);
      const match = firstLine.match(/^CONNECT\s+([^:\s]+):(\d+)\s+HTTP\/1\.[01]$/i);
      if (!match) {
        clientSocket.end("HTTP/1.1 405 Method Not Allowed\r\n\r\n");
        return;
      }
      const targetHost = match[1].toLowerCase();
      const targetPort = Number(match[2]);
      connectAttempts.push({ host: targetHost, port: targetPort, ts: Date.now() });

      const originPort = originPorts.get(targetHost);
      if (!originPort) {
        unexpectedHosts.push(targetHost);
        clientSocket.end("HTTP/1.1 502 Bad Gateway\r\n\r\n");
        return;
      }

      const upstream = net.connect(originPort, "127.0.0.1", () => {
        clientSocket.write("HTTP/1.1 200 Connection Established\r\n\r\n");
        if (rest.length > 0) upstream.write(rest);
        clientSocket.pipe(upstream);
        upstream.pipe(clientSocket);
      });
      upstream.on("error", () => clientSocket.destroy());
      clientSocket.on("error", () => upstream.destroy());
    }
    clientSocket.on("data", onData);
    clientSocket.on("error", () => {});
  });

  await new Promise((resolve, reject) => {
    proxyServer.once("error", reject);
    proxyServer.listen(0, "127.0.0.1", () => resolve());
  });
  const proxyPort = proxyServer.address().port;

  return {
    hosts: STUB_HOSTS,
    proxyUrl: `http://127.0.0.1:${proxyPort}`,
    spkiPin,
    resetCounts() {
      for (const h of STUB_HOSTS) hits.set(h, []);
      connectAttempts.length = 0;
      unexpectedHosts.length = 0;
    },
    getStats() {
      const counts = {};
      for (const h of STUB_HOSTS) {
        counts[h] = hits.get(h).length;
      }
      return {
        counts,
        connectAttempts: [...connectAttempts],
        unexpectedHosts: [...unexpectedHosts],
      };
    },
    async close() {
      await new Promise((r) => proxyServer.close(() => r()));
      await Promise.all(
        Array.from(originServers.values()).map((s) => new Promise((r) => s.close(() => r())))
      );
    },
  };
}

module.exports = {
  STUB_HOSTS,
  startMultiOriginTestServer,
};
