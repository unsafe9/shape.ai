// TS mirror of the Rust WS wire protocol (server `crates/server/src/ws.rs`). One socket, JSON TEXT
// frames internally tagged on `type`, all keys camelCase. `ops`/`patch` carry `WireOp[]` (each a
// wire envelope whose `propDelta` is the `ObjectOp` delta); `welcome` carries an `ObjectScene` snapshot.

import type { ObjectScene, WireOp, FeatureRequest, FeatureResponse } from "../shared/object";
import type { OpId } from "./outbox";

export type { OpId };

// `bbox` omitted == whole canvas (matches serde skip_serializing_if = "Option::is_none").
export type Bbox = { x: number; y: number; width: number; height: number };

export type Region = {
  canvasId: string;
  bbox?: Bbox;
};

// First frame; `region` optional, `lastAckSeq` defaults 0 server-side. `userId` is the connection's
// attributed author AND its self-skip identity: the server tags its ops/presence and never echoes them
// back. Permissive (no auth); omitted == anonymous connection.
export type HelloMessage = {
  type: "hello";
  canvasId: string;
  region?: Region;
  lastAckSeq: number;
  userId?: string;
};

// A batch of `WireOp`s to apply, in order. Each carries its `opId` (clientId + localSeq) for idempotent
// dedup, the `baseRevision` it was authored against, and the object-op delta in `propDelta`.
export type OpsMessage = {
  type: "ops";
  ops: WireOp[];
};

// A Feature request/response RPC frame (canvas switch, comment, template apply, export).
export type FeatureClientMessage = {
  type: "feature";
  request: FeatureRequest;
};

// (Re)subscribe to a region. `region` is REQUIRED here.
export type SubscribeMessage = {
  type: "subscribe";
  canvasId: string;
  region: Region;
};

// Best-effort presence frame; `payload` is opaque JSON passed through verbatim.
export type PresenceClientMessage = {
  type: "presence";
  canvasId: string;
  payload: unknown;
};

// Resume after a disconnect; replies with a fresh welcome snapshot.
export type ResumeMessage = {
  type: "resume";
  canvasId: string;
  lastAckSeq: number;
};

export type ClientMessage =
  | HelloMessage
  | OpsMessage
  | FeatureClientMessage
  | SubscribeMessage
  | PresenceClientMessage
  | ResumeMessage;

// Handshake reply: full ObjectScene snapshot + server seq/revision.
export type WelcomeMessage = {
  type: "welcome";
  scene: ObjectScene;
  seq: number;
  revision: number;
};

// One applied op: the `opIds` it resolved plus the server seq + revision after apply. A duplicate op
// re-acks its ORIGINAL seq/revision (idempotent), so the client can always drop the matching outbox entries.
export type AckMessage = {
  type: "ack";
  opIds: OpId[];
  seq: number;
  revision: number;
};

// One op rejected by scene-core; nothing applied/persisted/broadcast. `opIds` echoes the rejected id(s);
// omitted (serde skip) when empty.
export type RejectedMessage = {
  type: "rejected";
  opIds?: OpId[];
  errors: string[];
};

// A peer's (or self-echo) applied op fanned out in seq order, as `WireOp`s.
export type PatchMessage = {
  type: "patch";
  ops: WireOp[];
  seq: number;
};

export type FeatureServerMessage = {
  type: "feature";
  response: FeatureResponse;
};

// A peer's presence frame (ephemeral/best-effort).
export type PresenceServerMessage = {
  type: "presence";
  payload: unknown;
};

// Transport- or protocol-level error (bad frame, premature ops, …).
export type ErrorMessage = {
  type: "error";
  message: string;
};

export type ServerMessage =
  | WelcomeMessage
  | AckMessage
  | RejectedMessage
  | PatchMessage
  | FeatureServerMessage
  | PresenceServerMessage
  | ErrorMessage;

// Result of a successful `connect()` handshake (decoded `welcome`).
export type WelcomeResult = {
  scene: ObjectScene;
  seq: number;
  revision: number;
};

export type Ack = {
  opIds: OpId[];
  seq: number;
  revision: number;
};

export type Unsubscribe = () => void;

// Transport-agnostic object scene sync handle. A WS implementation lives in `wsTransport.ts`.
export interface SceneTransport {
  // Open the session: connect, send `hello`, resolve on `welcome`.
  connect(canvasId: string, region?: Region): Promise<WelcomeResult>;
  // Re-aim the connection's window to a new region; the server replies with a region-filtered `welcome`
  // snapshot an attached engine reconciles. `region.bbox` omitted/null = whole canvas.
  subscribe(region: Region): void;
  sendFeature(request: FeatureRequest): void;
  sendPresence(payload: unknown): void;
  onPatch(cb: (patch: PatchMessage) => void): Unsubscribe;
  onPresence(cb: (presence: PresenceServerMessage) => void): Unsubscribe;
  onFeature(cb: (feature: FeatureServerMessage) => void): Unsubscribe;
  onAck(cb: (ack: AckMessage) => void): Unsubscribe;
  onRejected(cb: (rejected: RejectedMessage) => void): Unsubscribe;
  onError(cb: (error: ErrorMessage) => void): Unsubscribe;
  close(): void;
}
