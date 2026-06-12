// Durable persistence for unacked client ops. The collaboration session (wasm) does the bookkeeping
// (minting `localSeq`, append/remove on author/ack, replay order); this module is only the DURABILITY
// port so the unacked tail survives a reload/reconnect. Re-sending the same `opId` is safe — the server
// dedups by it and re-acks the original seq. An entry is a `WireOp`, so a row re-sends verbatim.

import type { WireOp } from "../shared/object";

// `(clientId, localSeq)` idempotency key — mirrors the server `OpId`.
export type OpId = {
  clientId: string;
  localSeq: number;
};

// One outbox row: an opId-stamped `WireOp` envelope, field-for-field the WS `ops` envelope, so it re-sends verbatim.
export type OutboxEntry = WireOp;

// Durable persistence port for the session's unacked-op log, keyed by `opId`. The `localSeq` high-water
// is recovered from the rows themselves (each carries its seq), so the port holds no separate counter.
export interface OutboxStore {
  // Persist an entry (call before sending it on the wire).
  append(entry: OutboxEntry): Promise<void>;
  // All unacked entries, ascending by `localSeq` (replay order).
  all(): Promise<OutboxEntry[]>;
  // Drop the entries whose `opId` is in `opIds` (on ack/rejected).
  remove(opIds: OpId[]): Promise<void>;
  clear(): Promise<void>;
}

export function opIdEquals(a: OpId, b: OpId): boolean {
  return a.clientId === b.clientId && a.localSeq === b.localSeq;
}

// Stable string key for an opId (Set/Map membership).
export function opIdKey(opId: OpId): string {
  return `${opId.clientId}:${opId.localSeq}`;
}

// Non-durable `OutboxStore` backed by a plain array (tests + no-persistence fallback). Tests simulate a
// "reconnect" by reusing the SAME instance, which the IndexedDB impl guarantees across a real reload.
export class InMemoryOutboxStore implements OutboxStore {
  private entries: OutboxEntry[] = [];

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
}

const DB_NAME = "shape-ai-outbox";
const STORE = "ops";

// Durable `OutboxStore` on IndexedDB, one object store keyed by `opIdKey`. The `clientId` namespaces
// this client's rows so two tabs sharing the DB never collide.
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
}
