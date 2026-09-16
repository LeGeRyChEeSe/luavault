//! A stand-in for GitHub Releases, so the bench never touches the real update
//! host — and so a suite can publish a message of the day.
//!
//! Until this file existed every bench run asked the production server for a
//! manifest at startup. Here `/manifest.json` answers 404, which the client
//! treats as "nothing to update" (`update.rs`: a failed fetch is the nominal
//! offline case), and `/motd.json` + `/motd.json.sig` answer only when a suite
//! seeded a document.
//!
//! The document is SIGNED with the author's real primary key, because the
//! bench binary only accepts `RELEASE_PUBLIC_KEYS` — there is no test key
//! compiled in, and there must not be: a debug-only key would be one `cfg`
//! away from shipping. The key file never leaves the machine; it is read here
//! the same way `lvrelease` reads it (first 64-hex-char line), wrapped in the
//! fixed PKCS#8 prefix for Ed25519, and handed to `node:crypto`.
//!
//! `LV_UPDATE_BASE` and `LV_MOTD_BASE` are honoured in every build — a
//! redirected host still cannot produce a document the compiled-in keys accept —
//! which is what makes this stub possible without a debug-only escape hatch.

import { createServer, type Server } from 'node:http';
import { createPrivateKey, sign } from 'node:crypto';
import { existsSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

const ROOT = join(import.meta.dirname, '..');
const KEY_FILE = join(ROOT, 'release-primary.key');

/// PKCS#8 DER prefix for an Ed25519 private key (RFC 8410): the 32-byte seed
/// follows it verbatim.
const PKCS8_ED25519_PREFIX = Buffer.from('302e020100300506032b657004220420', 'hex');

/// A bare hex key, or the first 64-hex-char line of a labelled key report.
function readSeed(): Buffer {
  if (!existsSync(KEY_FILE)) {
    throw new Error(
      `clé de publication absente : ${KEY_FILE}\n` +
        `    → la suite motd signe son message avec la vraie clé primaire, car le\n` +
        `      binaire n'accepte que RELEASE_PUBLIC_KEYS (et ne doit rien accepter d'autre).`,
    );
  }
  const raw = readFileSync(KEY_FILE, 'utf8');
  const line = raw
    .split(/\r?\n/)
    .map((l) => l.trim())
    .find((l) => /^[0-9a-fA-F]{64}$/.test(l));
  if (!line) throw new Error(`aucune clé hexadécimale de 64 caractères dans ${KEY_FILE}`);
  return Buffer.from(line, 'hex');
}

/// Base64 Ed25519 signature over the exact bytes, the format `motd.json.sig`
/// carries on the real server.
export function signDocument(bytes: Buffer): string {
  const key = createPrivateKey({
    key: Buffer.concat([PKCS8_ED25519_PREFIX, readSeed()]),
    format: 'der',
    type: 'pkcs8',
  });
  return sign(null, bytes, key).toString('base64');
}

export interface UpdateStub {
  url: string;
  /// Every path the application asked for, in order.
  calls: string[];
  close(): Promise<void>;
}

export interface UpdateStubOptions {
  /// The `motd.json` document to serve, signed. Absent → 404 on both routes.
  motd?: unknown;
  /// Serve the document with a signature that does not match — the case that
  /// proves the client verifies rather than trusts.
  motdBadSignature?: boolean;
}

export async function startUpdateStub(options: UpdateStubOptions = {}): Promise<UpdateStub> {
  const calls: string[] = [];

  let motdBytes: Buffer | null = null;
  let motdSig: string | null = null;
  if (options.motd !== undefined) {
    motdBytes = Buffer.from(JSON.stringify(options.motd), 'utf8');
    motdSig = options.motdBadSignature
      ? signDocument(Buffer.from(JSON.stringify({ ...(options.motd as object), tampered: true })))
      : signDocument(motdBytes);
  }

  const server: Server = createServer((req, res) => {
    const path = (req.url ?? '').split('?')[0];
    calls.push(path);
    const send = (body: Buffer | string, code = 200, type = 'text/plain') => {
      const buf = typeof body === 'string' ? Buffer.from(body, 'utf8') : body;
      res.writeHead(code, { 'content-type': type, 'content-length': buf.length });
      res.end(buf);
    };
    if (path === '/motd.json' && motdBytes) return send(motdBytes, 200, 'application/json');
    if (path === '/motd.json.sig' && motdSig) return send(motdSig);
    return send('not found', 404);
  });

  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const addr = server.address();
  if (addr === null || typeof addr === 'string') throw new Error('update-stub: pas de port');

  return {
    url: `http://127.0.0.1:${addr.port}`,
    calls,
    close: () =>
      new Promise<void>((resolve) => {
        server.close(() => resolve());
      }),
  };
}
