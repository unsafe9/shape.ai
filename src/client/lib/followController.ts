// T5.3 — Spectator / follow mode controller (ephemeral shell-only state).
//
// Pure, framework-agnostic state machine for watching ONE MCP companion's canvas
// activity. It owns no document state: it reads the followee's live `lastTarget`
// from the T5.1 `/api/mcp/clients` stream and decides *when* a camera command
// should be issued, leaving the actual framing to the existing imperative camera
// surface (`focusBounds`/`setCamera`, via App's `handleFocusTarget`). It never
// touches `Scene`, `EngineEvent`, or the canvas core. Reset-on-reload is correct.
//
// Design: docs/design/ai-companion-canvas/tasks/T5.3.md (§0 state shape, §1 state
// machine, §2 click-to-follow, §3 pinned follow, §4 pause/resume, §5 jump-to-current,
// §7 handoff, §8 "never steal control" guard).

import type { McpClientInfo } from "./api";

export type FollowMode =
  // no followee; camera is fully user-owned
  | "off"
  // actively following: each new target re-frames the camera
  | "pinned"
  // a followee is selected and its trail still records, but the camera is frozen
  | "paused";

export type FollowState = {
  mode: FollowMode;
  // T5.1 McpClientIdentity.clientId; the one agent being watched
  followeeClientId: string | null;
  // key of the target the camera last framed (for jump-to-current change detection)
  lastFramedKey: string | null;
};

export const initialFollowState: FollowState = {
  mode: "off",
  followeeClientId: null,
  lastFramedKey: null
};

// A dock state that may be followed (silenced / absent agents are not followable, §2).
function isFollowable(client: McpClientInfo): boolean {
  return client.dockState === "idle" || client.dockState === "active" || client.dockState === "error";
}

// Resolved followee from the live registry stream, or null if it left/cannot be read.
export function resolveFollowee(
  state: FollowState,
  clients: McpClientInfo[]
): McpClientInfo | null {
  if (state.followeeClientId === null) return null;
  return clients.find((client) => client.clientId === state.followeeClientId) ?? null;
}

// Stable structural key for a CanvasTarget, used to detect "the agent moved".
// `null` for non-spatial / absent targets that should not drive the camera.
export function targetKey(target: unknown): string | null {
  if (!target || typeof target !== "object") return null;
  const t = target as { kind?: string; id?: string; groupId?: string; artifactId?: string; ids?: { kind: string; id: string }[]; rect?: { x: number; y: number; width: number; height: number } };
  switch (t.kind) {
    case "group":
    case "node":
    case "edge":
      return t.id ? `${t.kind}:${t.id}` : null;
    case "artifact":
      return t.artifactId && t.groupId ? `artifact:${t.groupId}:${t.artifactId}` : null;
    case "selection":
      return Array.isArray(t.ids) && t.ids.length > 0
        ? `selection:${t.ids.map((s) => `${s.kind}:${s.id}`).join(",")}`
        : null;
    case "viewport":
      return t.rect ? `viewport:${t.rect.x},${t.rect.y},${t.rect.width},${t.rect.height}` : null;
    // {kind:"canvas"} is non-spatial: never moves the camera in follow mode
    default:
      return null;
  }
}

// ---------------------------------------------------------------------------
// Transitions (§1). Each returns the next state; the caller separately performs
// any camera command described by `decideFollowCommand`. Transitions never issue
// camera commands themselves — they only mutate the ephemeral machine.
// ---------------------------------------------------------------------------

// §2 click-to-follow: clicking a followable chip pins follow to it. Clicking the
// already-followed chip again is a hard handoff back to manual control (§7).
export function toggleFollow(state: FollowState, client: McpClientInfo): FollowState {
  if (state.followeeClientId === client.clientId && state.mode !== "off") {
    return { ...initialFollowState };
  }
  if (!isFollowable(client)) return state;
  return { mode: "pinned", followeeClientId: client.clientId, lastFramedKey: null };
}

// §4 pause: freeze the camera but keep the followee + trail recording.
export function pauseFollow(state: FollowState): FollowState {
  if (state.mode !== "pinned") return state;
  return { ...state, mode: "paused" };
}

// §4 resume: re-pin and snap to the followee's *current* target (handled by caller).
export function resumeFollow(state: FollowState): FollowState {
  if (state.mode !== "paused" || state.followeeClientId === null) return state;
  return { ...state, mode: "pinned", lastFramedKey: null };
}

// §8 guard: a user grab during pinned follow demotes to paused (soft handoff, §7),
// remembering the followee for a one-tap resume. No effect outside pinned.
export function onUserGrab(state: FollowState): FollowState {
  if (state.mode !== "pinned") return state;
  return { ...state, mode: "paused" };
}

// §7 hard handoff: stop following entirely. Camera stays where it is (caller issues
// no command).
export function stopFollow(state: FollowState): FollowState {
  if (state.mode === "off") return state;
  return { ...initialFollowState };
}

// §1/§8: if the followee disconnected or was muted, drop follow with no camera move.
export function reconcileFollowee(state: FollowState, clients: McpClientInfo[]): FollowState {
  if (state.followeeClientId === null) return state;
  const followee = clients.find((client) => client.clientId === state.followeeClientId);
  if (!followee || followee.dockState === "disconnected" || followee.muted) {
    return { ...initialFollowState };
  }
  return state;
}

// ---------------------------------------------------------------------------
// Camera decision (§3 pinned follow, §8 guard). Pure: given the current state,
// the live followee, and whether a user gesture is active, decide whether the
// caller should frame a target now, and update `lastFramedKey` accordingly.
// ---------------------------------------------------------------------------

export type FollowCommand = {
  // the resolved CanvasTarget to frame via the existing focus path, or null for no-op
  target: unknown | null;
  // the state to commit (with lastFramedKey advanced when a frame is issued)
  state: FollowState;
};

// §3: while pinned and no gesture is active, re-frame whenever the followee's
// live `lastTarget` changes to a new spatial target. §8 rule 1: an active gesture
// suspends follow (zero camera commands). §8 rule 4: paused/off never move the camera.
export function decideFollowCommand(
  state: FollowState,
  followee: McpClientInfo | null,
  gestureActive: boolean
): FollowCommand {
  if (state.mode !== "pinned" || gestureActive || followee === null) {
    return { target: null, state };
  }
  const key = targetKey(followee.lastTarget);
  if (key === null || key === state.lastFramedKey) {
    return { target: null, state };
  }
  return { target: followee.lastTarget, state: { ...state, lastFramedKey: key } };
}

// §5 jump-to-current: one-shot frame of the followee's *live* lastTarget without
// changing mode. Returns the target to frame (or null when non-spatial / absent).
// Reads "current" from the registry, never from lastFramedKey.
export function jumpToCurrentTarget(followee: McpClientInfo | null): unknown | null {
  if (followee === null) return null;
  return targetKey(followee.lastTarget) === null ? null : followee.lastTarget;
}
