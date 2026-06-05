import { randomUUID } from "node:crypto";
import type { Implementation } from "@modelcontextprotocol/sdk/types.js";
import type { SceneSelection } from "../shared/schema";
import type { WorldRect } from "../shared/renderScene";
import type { ExtendedRenderPatch } from "../shared/renderPatch";

// ---------------------------------------------------------------------------
// T5.1: Client identity and dock state
// ---------------------------------------------------------------------------

export type McpClientDockState = "idle" | "active" | "error" | "disconnected" | "muted";

// T5.2: Canvas target type (widened from T5.1's SceneSelection | {kind:"viewport"} | null)
export type CanvasTarget =
  | { kind: "canvas" }
  | { kind: "group"; id: string }
  | { kind: "node"; id: string }
  | { kind: "edge"; id: string }
  | { kind: "selection"; ids: SceneSelection[] }
  | { kind: "viewport"; rect: WorldRect }
  | { kind: "artifact"; artifactId: string; groupId: string };

export type McpClientIdentity = {
  clientId: string;
  actorType: "mcp";
  label: string;
  name: string;
  version: string;
  color: string;
  iconRef: string | null;
  transport: "stdio" | "http";
  dockState: McpClientDockState;
  lastTarget: CanvasTarget | null;
  connectedAt: number;
  lastActivityAt: number;
  muted: boolean;
  /** T5.4: bounded ephemeral ring of recent read + error events (never persisted). */
  readRing: ReadTraceEvent[];
};

// ---------------------------------------------------------------------------
// T5.4: Trace model — types
// ---------------------------------------------------------------------------

/**
 * Every trace category named in the T5.4 design fragment.
 * Write/comment/export/proposal-* are projected from the events table on
 * demand; only "read" and "error" require the in-memory readRing.
 */
export type TraceKind =
  | "read"
  | "write"
  | "comment"
  | "export"
  | "proposal-created"
  | "proposal-accepted"
  | "proposal-rejected"
  | "error";

/** A single projected trace event (display-only; never persisted). */
export type TraceEvent = {
  clientId: string;
  kind: TraceKind;
  target: CanvasTarget;
  verb: string;
  /** events.id for logged ops; null for reads / pre-commit errors. */
  operationId: string | null;
  /** OperationEnvelope.sourceToolCall.callId — links trace back to the MCP call. */
  callId: string | null;
  /** Epoch ms. */
  at: number;
  /** Short human-readable summary, recomputed on read, never persisted. */
  summary: string;
  errorMessage: string | null;
};

/**
 * One entry in the per-client read-event ring.
 * Written by the tracking middleware for query_scene / list_groups / get_group.
 */
export type ReadTraceEvent = {
  tool: string;
  target: CanvasTarget;
  at: number;
  errorMessage?: string;
};

/** Maximum number of read/error events kept per client ring. */
export const READ_RING_CAPACITY = 50;

// ---------------------------------------------------------------------------
// T5.4: Ghost write-preview shape
// ---------------------------------------------------------------------------

/**
 * Represents a pending risky-write preview (ghost diff).
 *
 * Built server-side by applying the staged OperationEnvelope.patch to a
 * throwaway copy of the scene via applyRenderPatchToShapeScene (pure, no
 * commit).  Carried as metadata alongside a pending proposal; the actual
 * geometry rendering is deferred to the shell/core overlay layer (§3 of the
 * design fragment).
 */
export type WritePreview = {
  /** The pending proposal this preview corresponds to. */
  proposalId: string;
  clientId: string;
  /** The risk classification that routed this write to staging. */
  riskClass: "destructive" | "wide" | "agent-flagged";
  /** The patch verb (kind) that would be committed on accept. */
  verb: string;
  /** Semantic target ids derived from the staged patch. */
  targetIds: string[];
  /** Epoch ms when the proposal was staged. */
  stagedAt: number;
};

// ---------------------------------------------------------------------------
// Color palette — 12 distinct hues, deterministic per clientId
// ---------------------------------------------------------------------------

const COLOR_PALETTE = [
  "#e05252", "#e0893d", "#d4be34", "#52b765",
  "#3db8b8", "#3d82e0", "#7b52e0", "#c252c2",
  "#e07b7b", "#7be07b", "#7bc4e0", "#c2a852"
];

function hashClientId(clientId: string): number {
  let h = 0;
  for (let i = 0; i < clientId.length; i++) {
    h = (Math.imul(31, h) + clientId.charCodeAt(i)) >>> 0;
  }
  return h;
}

function colorFromClientId(clientId: string): string {
  return COLOR_PALETTE[hashClientId(clientId) % COLOR_PALETTE.length];
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

const _clients = new Map<string, McpClientIdentity>();

export function getClientRegistry(): ReadonlyMap<string, McpClientIdentity> {
  return _clients;
}

export function listClients(): McpClientIdentity[] {
  return Array.from(_clients.values());
}

export function getClient(clientId: string): McpClientIdentity | undefined {
  return _clients.get(clientId);
}

/**
 * Create and register a new McpClientIdentity from an SDK Implementation.
 * Returns the new identity record.
 */
export function registerClient(
  impl: Implementation,
  transport: "stdio" | "http",
  clientId?: string
): McpClientIdentity {
  const id = clientId ?? randomUUID();
  const rawLabel = impl.title ?? impl.name;
  const label = deduplicateLabel(rawLabel, id);
  const now = Date.now();
  const identity: McpClientIdentity = {
    clientId: id,
    actorType: "mcp",
    label,
    name: impl.name,
    version: impl.version,
    color: colorFromClientId(id),
    iconRef: impl.icons?.[0]?.src ?? null,
    transport,
    dockState: "idle",
    lastTarget: null,
    connectedAt: now,
    lastActivityAt: now,
    muted: false,
    readRing: []
  };
  _clients.set(id, identity);
  return identity;
}

/** Mark a client's dock state. */
export function setClientDockState(
  clientId: string,
  state: McpClientDockState,
  errorMessage?: string
): void {
  const client = _clients.get(clientId);
  if (!client) return;
  client.dockState = state;
  client.lastActivityAt = Date.now();
  // errorMessage is informational; stored for future inspection (e.g. tooltip)
  void errorMessage;
}

/** Update the lastTarget for a client. */
export function setClientLastTarget(clientId: string, target: CanvasTarget | null): void {
  const client = _clients.get(clientId);
  if (!client) return;
  client.lastTarget = target;
}

/** Remove a client from the registry (called after disconnect grace). */
export function removeClient(clientId: string): void {
  _clients.delete(clientId);
}

/** For testing: reset the registry. */
export function _resetClientRegistry(): void {
  _clients.clear();
}

// ---------------------------------------------------------------------------
// T5.4: Read-ring mutation helpers
// ---------------------------------------------------------------------------

/**
 * Push a read trace event into the per-client ring (for query_scene /
 * list_groups / get_group).  Oldest entry is evicted when the ring is full.
 */
export function pushReadTrace(clientId: string, event: ReadTraceEvent): void {
  const client = _clients.get(clientId);
  if (!client) return;
  client.readRing.push(event);
  if (client.readRing.length > READ_RING_CAPACITY) {
    client.readRing.shift();
  }
}

/**
 * Push an error trace event into the per-client ring.
 * Uses tool name as `tool` and `errorMessage` for display.
 */
export function pushErrorTrace(
  clientId: string,
  tool: string,
  target: CanvasTarget,
  errorMessage: string
): void {
  pushReadTrace(clientId, {
    tool,
    target,
    at: Date.now(),
    errorMessage
  });
}

// ---------------------------------------------------------------------------
// T5.4: Risk classifier — routes risky writes to staged proposals
// ---------------------------------------------------------------------------

const DESTRUCTIVE_KINDS = new Set<string>([
  "delete-group",
  "delete-card",
  "delete-edge"
]);

/** Wide-write threshold: patch touching this many targets or more is "wide". */
const WIDE_TARGET_THRESHOLD = 5;

/**
 * Classify a patch as risky (destructive or wide) or safe (immediate commit).
 *
 * Returns the risk class if the write should be staged as a proposal, or null
 * if the write should be committed immediately.
 */
export function classifyWriteRisk(
  patch: ExtendedRenderPatch,
  targetIds: string[]
): WritePreview["riskClass"] | null {
  if (DESTRUCTIVE_KINDS.has(patch.kind)) return "destructive";
  if (targetIds.length >= WIDE_TARGET_THRESHOLD) return "wide";
  return null;
}

// ---------------------------------------------------------------------------
// T5.4: Trace query — project events log + read ring per client
// ---------------------------------------------------------------------------

/**
 * Derive a human-readable summary for a trace event.
 * Display-only; recomputed on read, never persisted.
 */
export function deriveTraceSummary(kind: TraceKind, verb: string, target: CanvasTarget): string {
  const targetDesc = targetDescription(target);
  switch (kind) {
    case "read":
      return targetDesc ? `read ${targetDesc}` : `read ${verb}`;
    case "write":
      return targetDesc ? `${verb} ${targetDesc}` : verb;
    case "comment":
      return targetDesc ? `commented on ${targetDesc}` : "commented";
    case "export":
      return targetDesc ? `exported ${targetDesc}` : "exported";
    case "proposal-created":
      return targetDesc ? `proposed: ${verb} ${targetDesc} (pending)` : `proposed: ${verb} (pending)`;
    case "proposal-accepted":
      return targetDesc ? `accepted proposal → ${verb} ${targetDesc}` : `accepted proposal → ${verb}`;
    case "proposal-rejected":
      return "rejected proposal (no change)";
    case "error":
      return `error on ${targetDesc ?? verb}`;
  }
}

function targetDescription(target: CanvasTarget): string | null {
  switch (target.kind) {
    case "group":
      return `group ${target.id}`;
    case "node":
      return `node ${target.id}`;
    case "edge":
      return `edge ${target.id}`;
    case "artifact":
      return `artifact ${target.artifactId}`;
    case "selection":
      return `selection (${target.ids.length} items)`;
    case "viewport":
      return "viewport";
    case "canvas":
      return null;
  }
}

/**
 * Project the per-client read ring into TraceEvent records.
 * Read ring entries with errorMessage become kind:"error"; others are kind:"read".
 */
export function projectReadRing(clientId: string): TraceEvent[] {
  const client = _clients.get(clientId);
  if (!client) return [];
  return client.readRing.map((entry) => {
    const kind: TraceKind = entry.errorMessage ? "error" : "read";
    return {
      clientId,
      kind,
      target: entry.target,
      verb: entry.tool,
      operationId: null,
      callId: null,
      at: entry.at,
      summary: entry.errorMessage
        ? `error: ${entry.errorMessage}`
        : deriveTraceSummary("read", entry.tool, entry.target),
      errorMessage: entry.errorMessage ?? null
    };
  });
}

/**
 * Project one raw events-table row into a TraceEvent.
 * `row` is the deserialized payload_json (OperationEnvelope shape).
 */
export function projectEventRow(clientId: string, row: {
  id: string;
  type: string;
  payloadJson: string;
  createdAt: string;
}): TraceEvent {
  let payload: Record<string, unknown> = {};
  try {
    payload = JSON.parse(row.payloadJson) as Record<string, unknown>;
  } catch {
    // malformed payload — fall through with defaults
  }

  const kind = eventTypeToTraceKind(row.type);
  const targetIds = Array.isArray(payload.targetIds) ? (payload.targetIds as string[]) : [];
  const target = targetIdsToCanvasTarget(targetIds);
  const callId = (payload.sourceToolCall as { callId?: string } | undefined)?.callId ?? null;
  const at = new Date(row.createdAt).getTime();

  return {
    clientId,
    kind,
    target,
    verb: row.type,
    operationId: row.id,
    callId,
    at,
    summary: deriveTraceSummary(kind, row.type, target),
    errorMessage: null
  };
}

function eventTypeToTraceKind(type: string): TraceKind {
  if (type === "add-comment") return "comment";
  if (type === "export") return "export";
  if (type === "accept-proposal") return "proposal-accepted";
  if (type === "reject-proposal") return "proposal-rejected";
  // proposal-created is only surfaced via the staged sceneProposalSchema row,
  // not via an events row, so it won't appear here.
  return "write";
}

function targetIdsToCanvasTarget(targetIds: string[]): CanvasTarget {
  if (targetIds.length === 0) return { kind: "canvas" };
  if (targetIds.length === 1) {
    const id = targetIds[0];
    // Heuristic: prefix-based kind detection matching scene id conventions.
    if (id.startsWith("g")) return { kind: "group", id };
    if (id.startsWith("e")) return { kind: "edge", id };
    return { kind: "node", id };
  }
  // Multiple targets: represent as a selection of node-kind (display heuristic).
  return {
    kind: "selection",
    ids: targetIds.map((id) => {
      if (id.startsWith("g")) return { kind: "group" as const, id };
      if (id.startsWith("e")) return { kind: "edge" as const, id };
      return { kind: "node" as const, id };
    })
  };
}

// ---------------------------------------------------------------------------
// Label collision resolution: append " (2)", " (3)" etc. by connect order
// ---------------------------------------------------------------------------

function deduplicateLabel(base: string, newId: string): string {
  const existing = Array.from(_clients.values()).filter(
    (c) => c.clientId !== newId && (c.label === base || c.label.startsWith(base + " ("))
  );
  if (existing.length === 0) return base;
  return `${base} (${existing.length + 1})`;
}

// ---------------------------------------------------------------------------
// T5.2: Tool → CanvasTarget mapping
// ---------------------------------------------------------------------------

export type ToolName =
  | "query_scene"
  | "list_groups"
  | "get_group"
  | "create_group"
  | "patch_scene"
  | "create_tag"
  | "update_group_tags"
  | "set_selection"
  | "add_comment"
  | "export_group";

export type ToolCallInfo = {
  tool: ToolName;
  /** groupId arg (get_group, update_group_tags, export_group) */
  groupId?: string;
  /** SceneSelection arg (set_selection, add_comment target) */
  selection?: SceneSelection;
  /** tag filter (query_scene) */
  tagIds?: string[];
  /** ids from T2.5 envelope targetIds (patch_scene, create_group result, etc.) */
  targetIds?: SceneSelection[];
  /** artifact result (export_group) */
  artifactId?: string;
};

/**
 * Resolve a tool call to a CanvasTarget.
 *
 * Resolution order (first match wins):
 *   1. envelope targetIds present → spatial CanvasTarget
 *   2. read tool with group arg → group/viewport
 *   3. whole-scene read → scene-fit viewport (represented as {kind:"canvas"})
 *   4. registry/non-spatial write → {kind:"canvas"}
 *
 * Falls back to null when no target can be determined (keeps prior lastTarget).
 */
export function resolveToolTarget(info: ToolCallInfo): CanvasTarget | null {
  const { tool, groupId, selection, targetIds, artifactId } = info;

  switch (tool) {
    case "query_scene":
      // viewport: hull of matched groups (approximated as canvas-wide for now)
      // A full hull computation requires scene data; caller may pass a rect via
      // targetIds[0] of kind "group" for a filtered read. If none, whole-scene.
      return { kind: "canvas" };

    case "list_groups":
      // whole-scene survey
      return { kind: "canvas" };

    case "get_group":
      if (groupId) return { kind: "group", id: groupId };
      return null;

    case "create_group":
      if (targetIds && targetIds.length === 1) return selectionToTarget(targetIds[0]);
      return { kind: "canvas" };

    case "patch_scene": {
      if (!targetIds || targetIds.length === 0) return null;
      if (targetIds.length === 1) return selectionToTarget(targetIds[0]);
      return { kind: "selection", ids: targetIds };
    }

    case "create_tag":
      // registry/non-spatial write — no canvas location
      return { kind: "canvas" };

    case "update_group_tags":
      if (groupId) return { kind: "group", id: groupId };
      return { kind: "canvas" };

    case "set_selection":
      if (selection) return selectionToTarget(selection);
      return null;

    case "add_comment":
      if (selection) return selectionToTarget(selection);
      return { kind: "canvas" };

    case "export_group":
      if (artifactId && groupId) return { kind: "artifact", artifactId, groupId };
      if (groupId) return { kind: "group", id: groupId };
      return null;

    default:
      return null;
  }
}

function selectionToTarget(sel: SceneSelection): CanvasTarget {
  return sel as CanvasTarget;
}
