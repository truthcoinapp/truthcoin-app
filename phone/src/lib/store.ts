// What the phone keeps: its one pairing (the device key P is a non-extractable CryptoKey, stored as is, so the page
// can use it but no script can read its bytes) and the trades still waiting for an answer, so reopening the page asks
// about them again under the same ids. IndexedDB in the browser; a Map in tests.
import type { Request } from './link';
import type { PairAttempt, Pairing } from './pairing';

export interface Kv {
  get<T>(key: string): Promise<T | undefined>;
  set(key: string, value: unknown): Promise<void>;
  del(key: string): Promise<void>;
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
  return {
    get: <T>(key: string) => tx<T>('readonly', (s) => s.get(key)),
    set: async (key, value) => void (await tx('readwrite', (s) => s.put(value, key))),
    del: async (key) => void (await tx('readwrite', (s) => s.delete(key))),
  };
}

export function memoryKv(): Kv {
  const m = new Map<string, unknown>();
  return {
    get: async <T>(key: string) => m.get(key) as T | undefined,
    set: async (key, value) => void m.set(key, value),
    del: async (key) => void m.delete(key),
  };
}

const PAIRING = 'pairing';
const PENDING = 'pending';
const ATTEMPT = 'pair-attempt';

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

/** "Forget this computer": the keys, the desktop's details and the waiting trades all go. */
export async function forgetAll(kv: Kv): Promise<void> {
  await kv.del(PAIRING);
  await kv.del(PENDING);
  await kv.del(ATTEMPT);
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

export async function loadPending(kv: Kv, nowMs = Date.now()): Promise<PendingTrade[]> {
  const list = (await kv.get<PendingTrade[]>(PENDING)) ?? [];
  return Array.isArray(list) ? list.filter((p) => p && p.req && nowMs - p.at < PENDING_KEEP_MS) : [];
}

export async function savePending(kv: Kv, list: PendingTrade[]): Promise<void> {
  await kv.set(PENDING, list);
}
