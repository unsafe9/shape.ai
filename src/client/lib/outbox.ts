// Durable outbox for unacked client ops (MG4.3).
//
// Every op the shell authors is appended here BEFORE it is sent on the wire and
// is removed only when the server acks its `opId`. This makes the unacked tail
// survive a reload/reconnect: on (re)connect the engine replays every entry in
// `localSeq` order so an op authored offline (or in flight when the socket
// dropped) is re-sent rather than lost. Re-sending the same `opId` is safe — the
// server dedups by it and re-acks the original seq (idempotent).
//
// C11: the eventual durable backing is storage-core wasm sqlite (OPFS);
// IndexedDB is the interim impl, swap behind OutboxStore.

import type { RenderScenePatch } from "../../shared/renderPatch";

/** `(clientId, localSeq)` idempotency key — mirrors the server `OpId`. */
export type OpId = {
  clientId: string;
  localSeq: number;
};

/**
 * One outbox row: an opId-stamped envelope around a whole `RenderScenePatch`.
 * Field-for-field the `ops` envelope the evolved WS protocol carries, so an
 * entry can be re-sent verbatim with no re-encoding.
 */
export type OutboxEntry = {
  opId: OpId;
  baseRevision: number;
  ts: string;
  patch: RenderScenePatch;
};

/**
 * Durable append-only log of unacked ops, keyed by `opId`.
 *
 * `append` persists before send; `remove` drops acked ids; `all` returns the
 * replay set in `localSeq` order. Implementations must keep `localSeq`
 * monotonic per `clientId` and persist that counter alongside the rows so it
 * never repeats across reloads.
 */
export interface OutboxStore {
  /** Persist an entry (call before sending it on the wire). */
  append(entry: OutboxEntry): Promise<void>;
  /** All unacked entries, ascending by `localSeq` (replay order). */
  all(): Promise<OutboxEntry[]>;
  /** Drop the entries whose `opId` is in `opIds` (on ack/rejected). */
  remove(opIds: OpId[]): Promise<void>;
  /** Drop everything (e.g. a hard reset). */
  clear(): Promise<void>;
  /** Next monotonic `localSeq` for this client; advances and persists. */
  nextLocalSeq(): Promise<number>;
}

/** True when two opIds are the same logical op. */
export function opIdEquals(a: OpId, b: OpId): boolean {
  return a.clientId === b.clientId && a.localSeq === b.localSeq;
}

/** Stable string key for an opId (Set/Map membership). */
export function opIdKey(opId: OpId): string {
  return `${opId.clientId}:${opId.localSeq}`;
}

// ---------------------------------------------------------------------------
// In-memory impl — used by tests and as a no-persistence fallback.
// ---------------------------------------------------------------------------

/**
 * Non-durable `OutboxStore` backed by a plain array. Entries are lost on reload;
 * tests simulate a "reconnect" by reusing the SAME instance (the durable case),
 * which is what the IndexedDB impl guarantees across a real reload.
 */
export class InMemoryOutboxStore implements OutboxStore {
  private entries: OutboxEntry[] = [];
  private seq = 0;

  async append(entry: OutboxEntry): Promise<void> {
    this.entries.push(entry);
  }

  async all(): Promise<OutboxEntry[]> {
    return [...this.entries].sort((a, b) => a.opId.localSeq - b.opId.localSeq);
  }

  async remove(opIds: OpId[]): Promise<void> {
    if (opIds.length === 0) return;
    const drop = new Set(opIds.map(opIdKey));
    this.entries = this.entries.filter((e) => !drop.has(opIdKey(e.opId)));
  }

  async clear(): Promise<void> {
    this.entries = [];
  }

  async nextLocalSeq(): Promise<number> {
    this.seq += 1;
    return this.seq;
  }
}

// ---------------------------------------------------------------------------
// IndexedDB impl — the interim durable backing (C11).
// ---------------------------------------------------------------------------

const DB_NAME = "shape-ai-outbox";
const STORE = "ops";
const META = "meta";

/**
 * Durable `OutboxStore` on IndexedDB. One object store keyed by `opIdKey`, plus
 * a `meta` store holding the per-client `localSeq` high-water mark so the
 * counter survives reloads. The `clientId` namespaces this client's rows so two
 * tabs sharing the DB never collide on localSeq.
 */
export class IndexedDbOutboxStore implements OutboxStore {
  private readonly clientId: string;
  private dbPromise: Promise<IDBDatabase> | null = null;

  constructor(clientId: string) {
    this.clientId = clientId;
  }

  private db(): Promise<IDBDatabase> {
    if (this.dbPromise) return this.dbPromise;
    this.dbPromise = new Promise((resolve, reject) => {
      const req = indexedDB.open(DB_NAME, 1);
      req.onupgradeneeded = () => {
        const db = req.result;
        if (!db.objectStoreNames.contains(STORE)) db.createObjectStore(STORE);
        if (!db.objectStoreNames.contains(META)) db.createObjectStore(META);
      };
      req.onsuccess = () => resolve(req.result);
      req.onerror = () => reject(req.error);
    });
    return this.dbPromise;
  }

  private async tx<T>(
    stores: string[],
    mode: IDBTransactionMode,
    run: (tx: IDBTransaction) => Promise<T> | T
  ): Promise<T> {
    const db = await this.db();
    return new Promise<T>((resolve, reject) => {
      const tx = db.transaction(stores, mode);
      let result: T;
      Promise.resolve(run(tx)).then(
        (r) => {
          result = r;
        },
        (e) => reject(e)
      );
      tx.oncomplete = () => resolve(result);
      tx.onerror = () => reject(tx.error);
      tx.onabort = () => reject(tx.error);
    });
  }

  async append(entry: OutboxEntry): Promise<void> {
    await this.tx([STORE], "readwrite", (tx) => {
      tx.objectStore(STORE).put(entry, opIdKey(entry.opId));
    });
  }

  async all(): Promise<OutboxEntry[]> {
    const rows = await this.tx<OutboxEntry[]>([STORE], "readonly", (tx) => {
      return new Promise<OutboxEntry[]>((resolve, reject) => {
        const req = tx.objectStore(STORE).getAll();
        req.onsuccess = () => resolve(req.result as OutboxEntry[]);
        req.onerror = () => reject(req.error);
      });
    });
    return rows
      .filter((r) => r.opId.clientId === this.clientId)
      .sort((a, b) => a.opId.localSeq - b.opId.localSeq);
  }

  async remove(opIds: OpId[]): Promise<void> {
    if (opIds.length === 0) return;
    await this.tx([STORE], "readwrite", (tx) => {
      const store = tx.objectStore(STORE);
      for (const id of opIds) store.delete(opIdKey(id));
    });
  }

  async clear(): Promise<void> {
    await this.tx([STORE], "readwrite", (tx) => {
      tx.objectStore(STORE).clear();
    });
  }

  async nextLocalSeq(): Promise<number> {
    const key = `localSeq:${this.clientId}`;
    return this.tx<number>([META], "readwrite", (tx) => {
      const store = tx.objectStore(META);
      return new Promise<number>((resolve, reject) => {
        const get = store.get(key);
        get.onsuccess = () => {
          const next = ((get.result as number | undefined) ?? 0) + 1;
          const put = store.put(next, key);
          put.onsuccess = () => resolve(next);
          put.onerror = () => reject(put.error);
        };
        get.onerror = () => reject(get.error);
      });
    });
  }
}
