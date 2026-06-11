// AP6 (#19) — the transient toast channel.
//
// shape.ai's bottom-center status region has two notice kinds:
//   - PERSISTENT hints ("Drag to create…", "Ready", error text) live in the
//     `status` $state and stay until the tool/state changes — no timer.
//   - TRANSIENT action notices ("Inserted rectangle", "Comment added",
//     "Export ready") auto-dismiss after ~2.5s with a fade-out.
//
// The transient channel is shell-side (scene-core is pure, it owns no time). This
// module is the pure, framework-neutral state machine for it: the timer is
// INJECTED (set/clear callbacks) so the dismiss logic is testable in node with no
// real clock and the shell wires `setTimeout`/`clearTimeout` once. Each new
// transient notice supersedes the previous one and re-arms a single timer, so
// timers never leak. Persistent text never flows through here.

/** Default auto-dismiss window for a transient toast (D8: ~2.5s). */
export const TOAST_DISMISS_MS = 2500;

/** A scheduler the channel uses instead of touching the global clock directly,
 * so the dismiss logic stays pure/testable (the shell injects setTimeout). */
export interface ToastTimer {
  /** Arm a one-shot timer for `ms`, returning an opaque handle. */
  set(callback: () => void, ms: number): unknown;
  /** Cancel a handle returned by `set`. Tolerates a stale/cleared handle. */
  clear(handle: unknown): void;
}

/** Owns the transient toast lifecycle: at most one toast is live at a time, and
 * at most one timer is armed. Showing a new toast clears the prior timer (no
 * leak), and dismissing/superseding marks the toast gone. The current message
 * is exposed via `message` (null = nothing transient showing) so the shell can
 * mirror it into reactive state. */
export class ToastChannel {
  message: string | null = null;
  private handle: unknown = null;

  constructor(
    private readonly timer: ToastTimer,
    private readonly onChange: (message: string | null) => void,
    private readonly dismissMs: number = TOAST_DISMISS_MS
  ) {}

  /** Show a transient notice; it auto-dismisses after `dismissMs`. A second
   * `show` before the timer fires supersedes the first (its timer is cleared and
   * a fresh one armed), so back-to-back actions never leak a timer. */
  show(message: string): void {
    this.disarm();
    this.message = message;
    this.onChange(message);
    this.handle = this.timer.set(() => {
      this.handle = null;
      this.dismiss();
    }, this.dismissMs);
  }

  /** Drop the current transient toast immediately (and cancel its timer). Safe to
   * call when nothing is showing. The shell calls this when a persistent hint
   * supersedes the transient channel, so a stale toast never lingers. */
  dismiss(): void {
    this.disarm();
    if (this.message === null) return;
    this.message = null;
    this.onChange(null);
  }

  private disarm(): void {
    if (this.handle !== null) {
      this.timer.clear(this.handle);
      this.handle = null;
    }
  }
}
