// rAF-coalesced UI-model feed. The presence stream (peer cursors) can fire many times per frame — once
// per inbound cursor frame per peer — and each feed is the heavy path (cross-wasm JSON round-trip + full
// widget-tree rebuild + render). Buffering the latest serialized model and flushing it once per animation
// frame collapses that storm to ≤1 feed/frame regardless of peers × cursor-rate (decision #4: presence
// must not drive a per-event wasm round-trip). The model is a pure render-only mirror, so only the latest
// matters — superseded intermediate frames are safely dropped.
//
// The frame scheduler is INJECTED (request/cancel callbacks) so the coalescing is pure/testable, matching
// the ToastChannel timer-injection pattern; the shell wires it to requestAnimationFrame.

export interface FrameScheduler {
  // Schedule `callback` for the next frame, returning an opaque handle.
  request(callback: () => void): unknown;
  // Cancel a handle returned by `request`; tolerates a stale/cleared handle.
  cancel(handle: unknown): void;
}

// At most one frame is queued at a time; a feed scheduled while one is pending only replaces the pending
// model (no second frame). On the frame, the latest model is handed to `flush` exactly once.
export class UiModelFeed {
  private pending: string | null = null;
  private handle: unknown = null;

  constructor(
    private readonly scheduler: FrameScheduler,
    private readonly flush: (modelJson: string) => void
  ) {}

  // Buffer the latest model JSON and ensure exactly one frame is queued. Repeated calls within a frame
  // supersede the buffer and do NOT queue extra frames, so a presence burst is one feed next frame.
  schedule(modelJson: string): void {
    this.pending = modelJson;
    if (this.handle !== null) return;
    this.handle = this.scheduler.request(() => {
      this.handle = null;
      const json = this.pending;
      this.pending = null;
      if (json !== null) this.flush(json);
    });
  }

  // Drop a queued feed (teardown) so it can't fire against a torn-down host.
  dispose(): void {
    if (this.handle !== null) {
      this.scheduler.cancel(this.handle);
      this.handle = null;
    }
    this.pending = null;
  }
}
