// What the phone keeps: its one pairing (the device key P is a non-extractable CryptoKey, stored as is, so the page
// can use it but no script can read its bytes) and the trades still waiting for an answer, so reopening the page asks
// about them again under the same ids. IndexedDB in the browser; a Map in tests.
import type { Request } from './link';
import type { PairAttempt, Pairing } from './pairing';

export interface Kv {
  get<T>(key: string): Promise<T | undefined>;
  set(key: string, value: unknown): Promise<void>;
  del(key: string): Promise<void>;
  keys(): Promise<string[]>;
  /** Delete everything kept (IndexedDB: the whole database). Rejects, with words for people, when it can't. */
  wipe(): Promise<void>;
}

const STORE = 'kv';

export function idbKv(name = 'truthcoin-phone'): Kv {
  function open(): Promise<IDBDatabase> {
    return new Promise((resolve, reject) => {
      let req: IDBOpenDBRequest;
      try {
        req = indexedDB.open(name, 1);
      } catch (e) {
        reject(e);
        return;
      }
      req.onupgradeneeded = () => req.result.createObjectStore(STORE);
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error ?? new Error('IndexedDB unavailable'));
      req.onblocked = () => reject(new Error('IndexedDB blocked'));
    });
  }
  async function tx<T>(mode: IDBTransactionMode, fn: (s: IDBObjectStore) => IDBRequest | void): Promise<T | undefined> {
    const db = await open();
    try {
      return await new Promise<T | undefined>((resolve, reject) => {
        const t = db.transaction(STORE, mode);
        const r = fn(t.objectStore(STORE));
        t.oncomplete = () => resolve(r ? (r.result as T) : undefined);
        t.onerror = () => reject(t.error);
        t.onabort = () => reject(t.error ?? new Error('aborted'));
      });
    } finally {
      db.close();
    }
  }
  // Deleting the database, not its records: deleted records can linger in the browser's files until compaction.
  function wipe(): Promise<void> {
    return new Promise((resolve, reject) => {
      let req: IDBOpenDBRequest;
      try {
        req = indexedDB.deleteDatabase(name);
      } catch (e) {
        reject(new Error(`This browser refused to delete the page's storage (${(e as Error).message}).`));
        return;
      }
      let blocked: ReturnType<typeof setTimeout> | undefined;
      req.onsuccess = () => {
        clearTimeout(blocked);
        resolve();
      };
      req.onerror = () => {
        clearTimeout(blocked);
        reject(new Error(`This browser couldn't delete the page's storage (${req.error?.message ?? 'unknown error'}).`));
      };
      req.onblocked = () => {
        blocked = setTimeout(
          () => reject(new Error('The page is open in another tab or window, which keeps its storage in use: close it, then try again.')),
          4000,
        );
      };
    });
  }
  return {
    get: <T>(key: string) => tx<T>('readonly', (s) => s.get(key)),
    set: async (key, value) => void (await tx('readwrite', (s) => s.put(value, key))),
    del: async (key) => void (await tx('readwrite', (s) => s.delete(key))),
    keys: async () => ((await tx<IDBValidKey[]>('readonly', (s) => s.getAllKeys())) ?? []).map(String),
    wipe,
  };
}

export function memoryKv(): Kv {
  const m = new Map<string, unknown>();
  return {
    get: async <T>(key: string) => m.get(key) as T | undefined,
    set: async (key, value) => void m.set(key, value),
    del: async (key) => void m.delete(key),
    keys: async () => [...m.keys()],
    wipe: async () => m.clear(),
  };
}

const PAIRING = 'pairing';
const ATTEMPT = 'pair-attempt';
/** Each pairing's waiting trades apart (by its Nostr key): a trade never comes back under another pairing's keys. */
const pendingKey = (npub: string) => `pending:${npub}`;

/** The pairing under way (one code's keys), so a reload or a retry answers that code with the same keys. */
export async function loadAttempt(kv: Kv): Promise<PairAttempt | null> {
  return (await kv.get<PairAttempt>(ATTEMPT)) ?? null;
}

export async function saveAttempt(kv: Kv, a: PairAttempt): Promise<void> {
  await kv.set(ATTEMPT, a);
}

export async function dropAttempt(kv: Kv): Promise<void> {
  await kv.del(ATTEMPT);
}

export async function loadPairing(kv: Kv): Promise<Pairing | null> {
  const p = await kv.get<Pairing>(PAIRING);
  if (!p || p.v !== 1 || !p.pPriv || !(p.nsec instanceof Uint8Array) || !Array.isArray(p.relays)) return null;
  return p;
}

export async function savePairing(kv: Kv, p: Pairing): Promise<void> {
  await kv.set(PAIRING, p);
}

/** "Forget this computer": the keys, the desktop's details and the waiting trades all go, the database with them. */
export async function forgetAll(kv: Kv): Promise<void> {
  await kv.wipe();
}

/** A trade sent from this phone that has no final answer yet. */
export interface PendingTrade {
  req: Request;
  title: string;
  label: string;
  state: 'sending' | 'held' | 'unconfirmed';
  heldText: string;
  /** When it was made, ms. */
  at: number;
}

/** The desktop keeps request ids for 24 hours; after that, asking again could not be answered from its record. */
export const PENDING_KEEP_MS = 24 * 3600_000;

export async function loadPending(kv: Kv, npub: string, nowMs = Date.now()): Promise<PendingTrade[]> {
  const list = (await kv.get<PendingTrade[]>(pendingKey(npub))) ?? [];
  return Array.isArray(list) ? list.filter((p) => p && p.req && nowMs - p.at < PENDING_KEEP_MS) : [];
}

export async function savePending(kv: Kv, npub: string, list: PendingTrade[]): Promise<void> {
  await kv.set(pendingKey(npub), list);
}

export async function dropPending(kv: Kv, npub: string): Promise<void> {
  await kv.del(pendingKey(npub));
}

/** Every pairing's waiting trades but `npub`'s (left by earlier pairings, or written late by another tab). */
export async function dropOtherPending(kv: Kv, npub: string): Promise<void> {
  for (const k of await kv.keys()) if (k.startsWith('pending:') && k !== pendingKey(npub)) await kv.del(k);
}
