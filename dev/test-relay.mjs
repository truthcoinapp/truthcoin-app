#!/usr/bin/env node
// A tiny Nostr relay (NIP-01) for the phone link's tests and end-to-end checks. No dependencies: Node's http server,
// a minimal WebSocket (RFC 6455) and BIP340 signature checks written here. It listens on 127.0.0.1 only.
//
//   node dev/test-relay.mjs [port]            (or RELAY_PORT=<port>; 0 picks a free port; default 7447)
//
// It prints "test-relay listening on ws://127.0.0.1:<port>" once ready. Stop it by its PID (SIGTERM).
//
// It supports EVENT (answered with OK), REQ (filters: ids, authors, kinds, #<tag>, since, until, limit), CLOSE and
// EOSE. Events are checked (id and signature) like a real relay; ephemeral kinds (20000-29999) are passed on and not
// kept; others are kept in memory (the last 1000). NIP-40 expired events are refused.
//
// Misbehaviour on demand, to test the link's handling of real relays:
//   RELAY_DUP=<n>               send every event n extra times to each subscriber
//   RELAY_REORDER_MS=<ms>       hold every copy for a random 0..ms before sending it (reorders)
//   RELAY_DROP=<fraction>       accept but silently don't pass on this fraction of events (0..1)
//   RELAY_RATELIMIT_EVERY=<n>   refuse every n-th event with OK false "rate-limited: …" (not passed on)
//   RELAY_LOG=1                 log each message to stderr
import http from 'node:http';
import { createHash, randomInt } from 'node:crypto';
import { pathToFileURL } from 'node:url';

// ---------- BIP340 (secp256k1 Schnorr) verification ----------

const P = 0xfffffffffffffffffffffffffffffffffffffffffffffffffffffffefffffc2fn;
const N = 0xfffffffffffffffffffffffffffffffebaaedce6af48a03bbfd25e8cd0364141n;
const G = [
  0x79be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798n,
  0x483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8n,
  1n,
];
const INF = [0n, 1n, 0n];
const mod = (a, m = P) => ((a % m) + m) % m;

function pow(b, e, m = P) {
  let r = 1n;
  b = mod(b, m);
  while (e > 0n) {
    if (e & 1n) r = (r * b) % m;
    b = (b * b) % m;
    e >>= 1n;
  }
  return r;
}

// Jacobian coordinates (curve a = 0).
function dbl([X, Y, Z]) {
  if (Z === 0n || Y === 0n) return INF;
  const A = mod(X * X), B = mod(Y * Y), C = mod(B * B);
  const D = mod(2n * (mod((X + B) * (X + B)) - A - C));
  const E = mod(3n * A), F = mod(E * E);
  const X3 = mod(F - 2n * D);
  return [X3, mod(E * (D - X3) - 8n * C), mod(2n * Y * Z)];
}

function add(p, q) {
  const [X1, Y1, Z1] = p, [X2, Y2, Z2] = q;
  if (Z1 === 0n) return q;
  if (Z2 === 0n) return p;
  const Z1Z1 = mod(Z1 * Z1), Z2Z2 = mod(Z2 * Z2);
  const U1 = mod(X1 * Z2Z2), U2 = mod(X2 * Z1Z1);
  const S1 = mod(Y1 * Z2 * Z2Z2), S2 = mod(Y2 * Z1 * Z1Z1);
  if (U1 === U2) return S1 === S2 ? dbl(p) : INF;
  const H = mod(U2 - U1), I = mod(4n * H * H), J = mod(H * I);
  const r = mod(2n * (S2 - S1)), V = mod(U1 * I);
  const X3 = mod(r * r - J - 2n * V);
  return [X3, mod(r * (V - X3) - 2n * S1 * J), mod((mod((Z1 + Z2) * (Z1 + Z2)) - Z1Z1 - Z2Z2) * H)];
}

function mul(k, p) {
  let r = INF;
  for (let b = BigInt(k.toString(2).length) - 1n; b >= 0n; b--) {
    r = dbl(r);
    if ((k >> b) & 1n) r = add(r, p);
  }
  return r;
}

function affine([X, Y, Z]) {
  const zi = pow(Z, P - 2n);
  const zi2 = mod(zi * zi);
  return [mod(X * zi2), mod(Y * zi2 * zi)];
}

function liftX(x) {
  if (x >= P) return null;
  const c = mod(x * x * x + 7n);
  const y = pow(c, (P + 1n) / 4n);
  if (mod(y * y) !== c) return null;
  return [x, y & 1n ? P - y : y, 1n];
}

const sha256 = (...parts) => {
  const h = createHash('sha256');
  for (const p of parts) h.update(p);
  return h.digest();
};
const big = (b) => BigInt('0x' + Buffer.from(b).toString('hex'));

export function schnorrVerify(sigHex, msg, pubHex) {
  const sig = Buffer.from(sigHex, 'hex');
  const pub = Buffer.from(pubHex, 'hex');
  if (sig.length !== 64 || pub.length !== 32) return false;
  const Pt = liftX(big(pub));
  if (!Pt) return false;
  const r = big(sig.subarray(0, 32));
  const s = big(sig.subarray(32));
  if (r >= P || s >= N) return false;
  const tag = sha256('BIP0340/challenge');
  const e = mod(big(sha256(tag, tag, sig.subarray(0, 32), pub, msg)), N);
  const R = add(mul(s, G), mul(mod(N - e, N), Pt));
  if (R[2] === 0n) return false;
  const [x, y] = affine(R);
  return (y & 1n) === 0n && x === r;
}

// ---------- events and filters ----------

const HEX64 = /^[0-9a-f]{64}$/;

/** Why an event is refused, or null when it is good. */
export function checkEvent(e) {
  if (!e || typeof e !== 'object') return 'invalid: not an event';
  if (typeof e.id !== 'string' || !HEX64.test(e.id)) return 'invalid: bad id';
  if (typeof e.pubkey !== 'string' || !HEX64.test(e.pubkey)) return 'invalid: bad pubkey';
  if (typeof e.sig !== 'string' || !/^[0-9a-f]{128}$/.test(e.sig)) return 'invalid: bad signature';
  if (!Number.isSafeInteger(e.created_at) || !Number.isSafeInteger(e.kind)) return 'invalid: bad fields';
  if (typeof e.content !== 'string') return 'invalid: bad content';
  if (!Array.isArray(e.tags) || !e.tags.every((t) => Array.isArray(t) && t.every((x) => typeof x === 'string'))) {
    return 'invalid: bad tags';
  }
  const id = sha256(JSON.stringify([0, e.pubkey, e.created_at, e.kind, e.tags, e.content]));
  if (id.toString('hex') !== e.id) return 'invalid: event id does not match';
  if (!schnorrVerify(e.sig, id, e.pubkey)) return 'invalid: signature does not verify';
  const exp = e.tags.find((t) => t[0] === 'expiration');
  if (exp && Number(exp[1]) <= Math.floor(Date.now() / 1000)) return 'invalid: event has expired';
  return null;
}

export function matches(f, e) {
  if (!f || typeof f !== 'object') return false;
  if (Array.isArray(f.ids) && !f.ids.includes(e.id)) return false;
  if (Array.isArray(f.authors) && !f.authors.includes(e.pubkey)) return false;
  if (Array.isArray(f.kinds) && !f.kinds.includes(e.kind)) return false;
  if (typeof f.since === 'number' && e.created_at < f.since) return false;
  if (typeof f.until === 'number' && e.created_at > f.until) return false;
  for (const [k, want] of Object.entries(f)) {
    if (k.length !== 2 || k[0] !== '#' || !Array.isArray(want)) continue;
    if (!e.tags.some((t) => t[0] === k[1] && want.includes(t[1]))) return false;
  }
  return true;
}

const ephemeral = (kind) => kind >= 20000 && kind < 30000;

// ---------- the relay ----------

const GUID = '258EAFA5-E914-47DA-95CA-C5AB0DC85B11';
const MAX_MESSAGE = 512 * 1024;

/**
 * Start a relay. Options (each also read from the environment by the command line): port, dup, reorderMs, drop,
 * rateLimitEvery, log. Resolves with { port, url, close }.
 */
export function startRelay(o = {}) {
  const opt = { port: 7447, dup: 0, reorderMs: 0, drop: 0, rateLimitEvery: 0, log: false, ...o };
  const clients = new Set();
  const stored = [];
  const seen = new Map(); // event id -> time seen (duplicates within 10 minutes are not passed on again)
  let received = 0;
  const log = (...a) => opt.log && console.error('[relay]', ...a);

  function send(c, msg) {
    if (c.closed) return;
    const data = Buffer.from(JSON.stringify(msg));
    let head;
    if (data.length < 126) head = Buffer.from([0x81, data.length]);
    else if (data.length < 65536) {
      head = Buffer.alloc(4);
      head[0] = 0x81;
      head[1] = 126;
      head.writeUInt16BE(data.length, 2);
    } else {
      head = Buffer.alloc(10);
      head[0] = 0x81;
      head[1] = 127;
      head.writeBigUInt64BE(BigInt(data.length), 2);
    }
    c.socket.write(Buffer.concat([head, data]));
  }

  function closeClient(c, code = 1000) {
    if (c.closed) return;
    c.closed = true;
    clients.delete(c);
    const b = Buffer.alloc(4);
    b[0] = 0x88;
    b[1] = 2;
    b.writeUInt16BE(code, 2);
    try {
      c.socket.end(b);
    } catch {
      c.socket.destroy();
    }
  }

  function deliver(e) {
    for (const c of clients) {
      for (const [sub, filters] of c.subs) {
        if (!filters.some((f) => matches(f, e))) continue;
        for (let i = 0; i <= opt.dup; i++) {
          const go = () => send(c, ['EVENT', sub, e]);
          if (opt.reorderMs > 0) setTimeout(go, randomInt(0, opt.reorderMs + 1));
          else go();
        }
      }
    }
  }

  function onEvent(c, e) {
    const why = checkEvent(e);
    if (why) {
      send(c, ['OK', typeof e?.id === 'string' ? e.id : '', false, why]);
      return;
    }
    const now = Date.now();
    for (const [id, t] of seen) if (now - t > 600_000) seen.delete(id);
    if (seen.has(e.id)) {
      send(c, ['OK', e.id, true, 'duplicate: already have this event']);
      return;
    }
    received++;
    if (opt.rateLimitEvery > 0 && received % opt.rateLimitEvery === 0) {
      send(c, ['OK', e.id, false, 'rate-limited: slow down there chief']);
      return;
    }
    send(c, ['OK', e.id, true, '']);
    if (opt.drop > 0 && Math.random() < opt.drop) {
      log('dropped', e.id.slice(0, 8));
      return; // lost in transit: not recorded, so the same event sent again can get through
    }
    seen.set(e.id, now);
    if (!ephemeral(e.kind)) {
      stored.push(e);
      if (stored.length > 1000) stored.shift();
    }
    deliver(e);
  }

  function onText(c, text) {
    log('<-', text.length > 300 ? text.slice(0, 300) + '…' : text);
    let m;
    try {
      m = JSON.parse(text);
    } catch {
      send(c, ['NOTICE', 'error: not JSON']);
      return;
    }
    if (!Array.isArray(m)) return send(c, ['NOTICE', 'error: not an array']);
    if (m[0] === 'EVENT') return onEvent(c, m[1]);
    if (m[0] === 'REQ') {
      const sub = m[1];
      if (typeof sub !== 'string' || !sub || sub.length > 64) return send(c, ['NOTICE', 'error: bad subscription id']);
      const filters = m.slice(2).filter((f) => f && typeof f === 'object');
      c.subs.set(sub, filters);
      const limit = Math.min(500, ...filters.map((f) => (typeof f.limit === 'number' ? f.limit : 500)));
      const old = stored.filter((e) => filters.some((f) => matches(f, e))).slice(-limit);
      for (const e of old) send(c, ['EVENT', sub, e]);
      return send(c, ['EOSE', sub]);
    }
    if (m[0] === 'CLOSE') {
      c.subs.delete(m[1]);
      return;
    }
    send(c, ['NOTICE', `error: unknown message ${String(m[0]).slice(0, 20)}`]);
  }

  // RFC 6455 frames from a client: masked; text, close, ping, pong and continuation.
  function onData(c, chunk) {
    c.buf = Buffer.concat([c.buf, chunk]);
    for (;;) {
      if (c.buf.length < 2) return;
      const fin = (c.buf[0] & 0x80) !== 0;
      const op = c.buf[0] & 0x0f;
      const masked = (c.buf[1] & 0x80) !== 0;
      let len = c.buf[1] & 0x7f;
      let at = 2;
      if (len === 126) {
        if (c.buf.length < 4) return;
        len = c.buf.readUInt16BE(2);
        at = 4;
      } else if (len === 127) {
        if (c.buf.length < 10) return;
        const l = c.buf.readBigUInt64BE(2);
        if (l > BigInt(MAX_MESSAGE)) return closeClient(c, 1009);
        len = Number(l);
        at = 10;
      }
      if (!masked) return closeClient(c, 1002);
      if (len > MAX_MESSAGE) return closeClient(c, 1009);
      if (c.buf.length < at + 4 + len) return;
      const mask = c.buf.subarray(at, at + 4);
      const payload = Buffer.from(c.buf.subarray(at + 4, at + 4 + len));
      for (let i = 0; i < payload.length; i++) payload[i] ^= mask[i & 3];
      c.buf = c.buf.subarray(at + 4 + len);
      if (op === 0x8) return closeClient(c);
      if (op === 0x9) {
        c.socket.write(Buffer.concat([Buffer.from([0x8a, payload.length]), payload]));
        continue;
      }
      if (op === 0xa) continue;
      if (op === 0x1 || op === 0x2) c.frag = [payload];
      else if (op === 0x0) c.frag.push(payload);
      else return closeClient(c, 1002);
      if (c.frag.reduce((n, p) => n + p.length, 0) > MAX_MESSAGE) return closeClient(c, 1009);
      if (fin) {
        const text = Buffer.concat(c.frag).toString('utf8');
        c.frag = [];
        onText(c, text);
      }
    }
  }

  const server = http.createServer((req, res) => {
    res.writeHead(426, { 'content-type': 'text/plain' });
    res.end('test relay: connect with a WebSocket\n');
  });

  server.on('upgrade', (req, socket) => {
    const key = req.headers['sec-websocket-key'];
    if (String(req.headers.upgrade).toLowerCase() !== 'websocket' || typeof key !== 'string') {
      socket.end('HTTP/1.1 400 Bad Request\r\n\r\n');
      return;
    }
    const accept = createHash('sha1').update(key + GUID).digest('base64');
    socket.write(
      'HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n' +
        `Sec-WebSocket-Accept: ${accept}\r\n\r\n`,
    );
    socket.setNoDelay(true);
    const c = { socket, buf: Buffer.alloc(0), frag: [], subs: new Map(), closed: false };
    clients.add(c);
    socket.on('data', (d) => {
      try {
        onData(c, d);
      } catch (err) {
        log('error', err);
        closeClient(c, 1011);
      }
    });
    const gone = () => {
      c.closed = true;
      clients.delete(c);
    };
    socket.on('close', gone);
    socket.on('error', gone);
  });

  return new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(opt.port, '127.0.0.1', () => {
      const port = server.address().port;
      resolve({
        port,
        url: `ws://127.0.0.1:${port}`,
        clientCount: () => clients.size,
        close: () =>
          new Promise((done) => {
            for (const c of [...clients]) {
              c.closed = true;
              c.socket.destroy();
            }
            clients.clear();
            server.close(() => done());
          }),
      });
    });
  });
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const env = process.env;
  const relay = await startRelay({
    port: Number(process.argv[2] ?? env.RELAY_PORT ?? 7447),
    dup: Number(env.RELAY_DUP ?? 0),
    reorderMs: Number(env.RELAY_REORDER_MS ?? 0),
    drop: Number(env.RELAY_DROP ?? 0),
    rateLimitEvery: Number(env.RELAY_RATELIMIT_EVERY ?? 0),
    log: env.RELAY_LOG === '1',
  });
  console.log(`test-relay listening on ${relay.url}`);
  const stop = () => relay.close().then(() => process.exit(0));
  process.on('SIGTERM', stop);
  process.on('SIGINT', stop);
}
