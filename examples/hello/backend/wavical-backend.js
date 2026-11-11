'use strict';

const net = require('net');

// Console Redirection: ensure stdout is never corrupted by console.log
console.log = function(...args) {
  process.stderr.write(args.map(a => typeof a === 'string' ? a : JSON.stringify(a)).join(' ') + '\n');
};
console.info = console.log;
console.warn = console.log;

const MAX_FRAME_SIZE = 4 * 1024 * 1024; // 4 MB

const handlers = new Map();

function handle(name, fn) {
  if (typeof name !== 'string' || name.length === 0) {
    throw new TypeError('Method name must be a non-empty string');
  }
  if (typeof fn !== 'function') {
    throw new TypeError('Handler must be a function');
  }
  handlers.set(name, fn);
}

const pipePath = process.env.WAVICAL_IPC_PIPE;
if (!pipePath) {
  process.stderr.write('wavical-backend: WAVICAL_IPC_PIPE environment variable is missing\n');
  process.exit(1);
}

const client = net.createConnection(pipePath, () => {});

client.on('error', (err) => {
  process.stderr.write(`wavical-backend pipe error: ${err.message}\n`);
  process.exit(1);
});

client.on('close', () => {
  process.exit(0);
});

let buffer = Buffer.alloc(0);

client.on('data', (chunk) => {
  buffer = Buffer.concat([buffer, chunk]);

  while (buffer.length >= 4) {
    const frameLen = buffer.readUInt32BE(0);
    if (frameLen > MAX_FRAME_SIZE) {
      process.stderr.write(`wavical-backend: frame size ${frameLen} exceeds limit\n`);
      client.destroy();
      process.exit(1);
    }

    if (buffer.length < 4 + frameLen) {
      break;
    }

    const frameBytes = buffer.subarray(4, 4 + frameLen);
    buffer = buffer.subarray(4 + frameLen);

    let req;
    try {
      req = JSON.parse(frameBytes.toString('utf8'));
    } catch (e) {
      process.stderr.write('wavical-backend: malformed JSON request frame\n');
      continue;
    }

    dispatchRequest(req);
  }
});

function sendResponse(id, ok, result, error) {
  const resp = { id, ok };
  if (ok) {
    resp.result = result !== undefined ? result : null;
  } else {
    resp.error = typeof error === 'string' ? error : 'Internal error';
  }

  const jsonStr = JSON.stringify(resp);
  const jsonBuf = Buffer.from(jsonStr, 'utf8');
  if (jsonBuf.length > MAX_FRAME_SIZE) {
    const errResp = Buffer.from(JSON.stringify({ id, ok: false, error: 'Internal error' }), 'utf8');
    const hdr = Buffer.alloc(4);
    hdr.writeUInt32BE(errResp.length, 0);
    client.write(Buffer.concat([hdr, errResp]));
    return;
  }

  const header = Buffer.alloc(4);
  header.writeUInt32BE(jsonBuf.length, 0);
  client.write(Buffer.concat([header, jsonBuf]));
}

function dispatchRequest(req) {
  if (!req || typeof req.id !== 'number' || typeof req.method !== 'string') {
    return;
  }

  const fn = handlers.get(req.method);
  if (!fn) {
    sendResponse(req.id, false, null, 'Method not found');
    return;
  }

  Promise.resolve()
    .then(() => fn(req.params))
    .then((result) => {
      sendResponse(req.id, true, result, null);
    })
    .catch(() => {
      sendResponse(req.id, false, null, 'Internal error');
    });
}

module.exports = {
  handle
};

