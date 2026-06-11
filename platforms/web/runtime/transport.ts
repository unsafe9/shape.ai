// Transport-agnostic object scene sync seam (OB4.3).
//
// This is the TS mirror of the Rust WS wire protocol implemented by the server's
// `crates/server/src/ws.rs` (`WsClientMessage`/`WsServerMessage`). One socket,
// JSON TEXT frames, internally tagged on `type`, all keys camelCase. Two logical
// channels (channel == message type, not a separate stream):
//   - reliable_ordered: hello, welcome, ops, ack, rejected, patch, feature
//   - ephemeral_besteffort: presence
//
// `ops`/`patch` carry `WireOp[]` — each a wire envelope whose `propDelta` is the
// `ObjectOp` delta JSON the object-native canvas actor applies. `welcome` carries
// an `ObjectScene` snapshot. The single `feature` request/response RPC frame
// replaces the retired bespoke REST surface (OB4.5).

import type { ObjectScene, WireOp, FeatureRequest, FeatureResponse } from "../shared/object";
import type { OpId } from "./outbox";

export type { OpId };

// ---------------------------------------------------------------------------
// Subscription region. `bbox` omitted == whole canvas (matches serde
// skip_serializing_if = "Option::is_none").
// ---------------------------------------------------------------------------

export type Bbox = { x: number; y: number; width: number; height: number };

export type Region = {
  canvasId: string;
  bbox?: Bbox;
};

// ---------------------------------------------------------------------------
// Client -> Server messages.
// ---------------------------------------------------------------------------

/**
 * First frame; `region` optional, `lastAckSeq` defaults 0 server-side. `userId`
 * is the connection's attributed author AND its self-skip identity: the server
 * tags client-attributed ops/presence with it and never echoes them back to the
 * sender. Permissive (no auth, C13); omitted == anonymous connection.
 */
export type HelloMessage = {
  type: "hello";
  canvasId: string;
  region?: Region;
  lastAckSeq: number;
  userId?: string;
};

/**
 * A batch of `WireOp`s to apply, in order. Each carries its own `opId`
 * (`clientId` + `localSeq`) for idempotent dedup, the `baseRevision` it was
 * authored against, and the object-op delta in `propDelta`.
 */
export type OpsMessage = {
  type: "ops";
  ops: WireOp[];
};

/** A Feature request/response RPC frame (canvas switch, comment, template apply, export). */
export type FeatureClientMessage = {
  type: "feature";
  request: FeatureRequest;
};

/** (Re)subscribe to a region. `region` is REQUIRED here. */
export type SubscribeMessage = {
  type: "subscribe";
  canvasId: string;
  region: Region;
};

/** Best-effort presence frame; `payload` is opaque JSON passed through verbatim. */
export type PresenceClientMessage = {
  type: "presence";
  canvasId: string;
  payload: unknown;
};

/** Resume after a disconnect; replies with a fresh welcome snapshot. */
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

// ---------------------------------------------------------------------------
// Server -> Client messages.
// ---------------------------------------------------------------------------

/** Handshake reply: full ObjectScene snapshot + server seq/revision. */
export type WelcomeMessage = {
  type: "welcome";
  scene: ObjectScene;
  seq: number;
  revision: number;
};

/**
 * One applied op: the `opIds` it resolved (one per op) plus the server seq +
 * revision after apply. A duplicate op re-acks its ORIGINAL seq/revision
 * (idempotent), so the client can always drop the matching outbox entries.
 */
export type AckMessage = {
  type: "ack";
  opIds: OpId[];
  seq: number;
  revision: number;
};

/**
 * One op rejected by scene-core; nothing applied/persisted/broadcast. `opIds`
 * echoes the rejected op's id(s) so the client fails the matching outbox entry;
 * it is omitted (serde skip) when empty.
 */
export type RejectedMessage = {
  type: "rejected";
  opIds?: OpId[];
  errors: string[];
};

/** A peer's (or self-echo) applied op fanned out in seq order, as `WireOp`s. */
export type PatchMessage = {
  type: "patch";
  ops: WireOp[];
  seq: number;
};

/** A Feature response to a `feature` request. */
export type FeatureServerMessage = {
  type: "feature";
  response: FeatureResponse;
};

/** A peer's presence frame (ephemeral/best-effort). */
export type PresenceServerMessage = {
  type: "presence";
  payload: unknown;
};

/** Transport- or protocol-level error (bad frame, premature ops, …). */
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

// ---------------------------------------------------------------------------
// Transport interface.
// ---------------------------------------------------------------------------

/** Result of a successful `connect()` handshake (decoded `welcome`). */
export type WelcomeResult = {
  scene: ObjectScene;
  seq: number;
  revision: number;
};

/** An applied `ack`: the resolved opIds plus the server seq/revision after apply. */
export type Ack = {
  opIds: OpId[];
  seq: number;
  revision: number;
};

export type Unsubscribe = () => void;

/**
 * Transport-agnostic object scene sync handle. A WS implementation lives in
 * `wsTransport.ts`; a future WebTransport/gRPC impl maps onto the same channels.
 */
export interface SceneTransport {
  /** Open the session: connect, send `hello`, resolve on `welcome`. */
  connect(canvasId: string, region?: Region): Promise<WelcomeResult>;
  /**
   * Re-aim the connection's window to a new region. The server replies with a
   * region-filtered `welcome` snapshot on the welcome stream (the resnapshot
   * frame), which an attached engine reconciles. `region.bbox` omitted/null =
   * whole canvas.
   */
  subscribe(region: Region): void;
  /** Send a Feature request RPC frame on the reliable channel (OB4.5). */
  sendFeature(request: FeatureRequest): void;
  /** Send a best-effort presence frame on the ephemeral channel. */
  sendPresence(payload: unknown): void;
  /** Subscribe to peer/self applied patches; returns an unsubscribe. */
  onPatch(cb: (patch: PatchMessage) => void): Unsubscribe;
  /** Subscribe to peer presence frames; returns an unsubscribe. */
  onPresence(cb: (presence: PresenceServerMessage) => void): Unsubscribe;
  /** Subscribe to Feature response frames; returns an unsubscribe. */
  onFeature(cb: (feature: FeatureServerMessage) => void): Unsubscribe;
  /** Subscribe to ack frames; returns an unsubscribe. */
  onAck(cb: (ack: AckMessage) => void): Unsubscribe;
  /** Subscribe to rejected frames; returns an unsubscribe. */
  onRejected(cb: (rejected: RejectedMessage) => void): Unsubscribe;
  /** Subscribe to protocol/transport error frames; returns an unsubscribe. */
  onError(cb: (error: ErrorMessage) => void): Unsubscribe;
  /** Close the socket. */
  close(): void;
}
