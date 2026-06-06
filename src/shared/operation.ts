/**
 * T2.5 — Collaboration-Ready Operation Model
 *
 * Pure type contract + local operation-log helper.
 * No CRDT, no realtime transport, no remote auth, no remote cursors.
 *
 * DOCUMENT STATE vs EPHEMERAL STATE boundary
 * ------------------------------------------
 * Document state: persisted in Scene, revision-bearing, logged in the `events` table.
 *   - nodes / edges / groups (geometry, identity, text, style)
 *   - comments (Scene.comments), artifacts (Scene.artifacts)
 *   - proposals (Scene.proposals — minimal slot, full UI is P5/T5.4)
 *   - scene revision (sceneVersion / scene_version metadata key)
 *
 * Ephemeral state: never in `events`, resettable without a document revision bump.
 *   - viewport / camera  (SceneSnapshot.camera — render-time only)
 *   - hover / active-tool / follow / companion animation (core + shell)
 *   - selection  — authoritative home is the `selection_json` metadata key;
 *                  `Scene.selection` is kept for renderer back-compat but is NOT a
 *                  document revision (the `select` op must NOT bump sceneVersion).
 *   - read cursor / trace overlay (P5 derived from the events log, not stored)
 */

import type { RenderScenePatch, ExtendedRenderPatch } from "./renderPatch";

// ---------------------------------------------------------------------------
// Actor taxonomy
// ---------------------------------------------------------------------------

/** Who authored an operation. */
export type ActorType = "human" | "mcp" | "system";

// ---------------------------------------------------------------------------
// Operation envelope
// ---------------------------------------------------------------------------

/**
 * Every canonical edit — human or MCP — is wrapped in an OperationEnvelope
 * before being committed.  The envelope carries the metadata fields required
 * for undo / MCP-trace / audit without adding CRDT or transport behavior.
 *
 * Field derivation from real symbols:
 *   operationId   — generated at the funnel; becomes events.id (storage.ts:434)
 *   actorId       — "human" for shell; stable MCP identity is a P5/T5.1 dependency
 *   actorType     — "human" | "mcp" | "system"
 *   clientId      — "local-shell" for the web shell; MCP session id is P5/T5.1
 *   targetIds     — per-kind derivation; guarantees semantic target info (not raw blob)
 *   timestamp     — reuses the existing `now`/`generatedAt` string already threaded
 *                   through applyRenderPatchToShapeScene and commitAppPatch
 *   baseRevision  — scene.sceneVersion read BEFORE apply; collaboration-readiness hook
 *   sourceToolCall — set by the MCP layer per tool; absent for shell ops
 *   patch         — the inner typed RenderScenePatch / extended op member
 */
export type OperationEnvelope = {
  operationId: string;
  actorId: string;
  actorType: ActorType;
  clientId: string;
  targetIds: string[];
  timestamp: string;
  /** sceneVersion read BEFORE apply — the "authored against" revision for future conflict detection. */
  baseRevision: number;
  /** Present only for MCP-originated ops. */
  sourceToolCall?: {
    tool: string;
    callId?: string;
  };
  /** The typed inner operation. */
  patch: ExtendedRenderPatch;
};

// ---------------------------------------------------------------------------
// Synthesise a local-human envelope (used as the default in applyRenderPatchToShapeScene)
// ---------------------------------------------------------------------------

/**
 * Derive the semantic targetIds from a patch so every logged envelope carries
 * explicit target info rather than a raw blob reference.
 *
 * Per T2.5 §2 targetIds-derivation table.
 */
export function deriveTargetIds(patch: ExtendedRenderPatch): string[] {
  switch (patch.kind) {
    case "create-card":
      return [patch.card.id];
    case "create-group":
      return [patch.group.id];
    case "create-edge":
      return [patch.edgeId, patch.source, patch.target];
    case "move-card":
    case "set-card-z-index":
    case "edit-card-text":
    case "delete-card":
      return [patch.id];
    case "move-group":
    case "delete-group":
      return [patch.id];
    case "delete-edge":
      return [patch.id];
    case "select":
      // Selection is ephemeral; targetIds are the selected ids (no document target).
      return patch.selection.kind === "canvas"
        ? []
        : patch.selection.kind === "multi"
          ? patch.selection.ids
          : [patch.selection.id];
    // T2.2 ops
    case "resize-card":
      return [patch.id];
    case "resize-group":
      return [patch.id];
    case "align-cards":
    case "distribute-cards":
      return patch.ids;
    case "duplicate-objects":
      return patch.ids;
    case "batch":
      return patch.ops.flatMap(deriveTargetIds);
    case "add-comment":
      return patch.target.kind === "canvas"
        ? []
        : patch.target.kind === "multi"
          ? patch.target.ids
          : [patch.target.id];
    case "export":
      return patch.scopeIds;
    case "accept-proposal":
    case "reject-proposal":
      return [patch.proposalId];
    // T2.4 ops
    case "group-objects":
      return [patch.frameId, ...patch.ids];
    case "ungroup":
      return [patch.id];
    case "set-object-group":
      return [patch.frameId, ...patch.ids];
    case "set-object-tags":
      return [patch.id];
    case "create-tag":
      return [patch.tag.id];
    default: {
      // Exhaustive fallback — if a new patch kind is added without updating this
      // table the compiler will surface it via the never check below.
      const _exhaustive: never = patch;
      void _exhaustive;
      return [];
    }
  }
}

/**
 * Synthesise a local-human OperationEnvelope from a patch and a base revision.
 * Used as the DEFAULT in applyRenderPatchToShapeScene so all existing callers
 * receive a valid envelope without any source changes.
 */
export function synthesiseLocalEnvelope(
  patch: ExtendedRenderPatch,
  baseRevision: number,
  now: string
): OperationEnvelope {
  return {
    operationId: `op-${now}-${Math.random().toString(36).slice(2, 9)}`,
    actorId: "human",
    actorType: "human",
    clientId: "local-shell",
    targetIds: deriveTargetIds(patch),
    timestamp: now,
    baseRevision,
    patch
  };
}

// ---------------------------------------------------------------------------
// Local append-only operation log
// ---------------------------------------------------------------------------

/**
 * In-memory append-only operation log.
 * Usable for undo/MCP-trace/audit without persisting to storage.
 * The server-side counterpart writes to the existing `events` table (storage.ts).
 *
 * Not a CRDT, not a transport queue — just a typed, ordered list of operations.
 */
export type OperationLog = {
  /** Ordered list of committed envelopes, oldest first. */
  readonly entries: readonly OperationEnvelope[];
};

/** Create an empty operation log. */
export function createOperationLog(): OperationLog {
  return { entries: [] };
}

/**
 * Append an envelope to the log.
 * Returns a NEW log (immutable append — does not mutate the original).
 */
export function appendToOperationLog(
  log: OperationLog,
  envelope: OperationEnvelope
): OperationLog {
  return { entries: [...log.entries, envelope] };
}

/**
 * Return all entries authored by a given actorId, in chronological order.
 * Useful for MCP trace queries.
 */
export function entriesByActor(
  log: OperationLog,
  actorId: string
): readonly OperationEnvelope[] {
  return log.entries.filter((e) => e.actorId === actorId);
}

/**
 * Return all entries that touch a given targetId.
 * Useful for per-object history / undo scope.
 */
export function entriesByTarget(
  log: OperationLog,
  targetId: string
): readonly OperationEnvelope[] {
  return log.entries.filter((e) => e.targetIds.includes(targetId));
}
