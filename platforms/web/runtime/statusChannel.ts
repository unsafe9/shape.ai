// The transient toast channel (transient action notices that auto-dismiss; persistent hints never flow
// through here). scene-core owns no time, so the timer is INJECTED (set/clear callbacks) for testability;
// each new notice supersedes the previous one and re-arms a single timer, so timers never leak.

export const TOAST_DISMISS_MS = 2500;

// A scheduler the channel uses instead of the global clock, so the dismiss logic stays pure/testable.
export interface ToastTimer {
  // Arm a one-shot timer for `ms`, returning an opaque handle.
  set(callback: () => void, ms: number): unknown;
  // Cancel a handle returned by `set`; tolerates a stale/cleared handle.
  clear(handle: unknown): void;
}

// At most one toast and one timer live at a time; showing a new toast clears the prior timer. The
// current message is exposed via `message` (null = nothing showing) so the shell can mirror it reactively.
export class ToastChannel {
  message: string | null = null;
  private handle: unknown = null;

  constructor(
    private readonly timer: ToastTimer,
    private readonly onChange: (message: string | null) => void,
    private readonly dismissMs: number = TOAST_DISMISS_MS
  ) {}

  // Show a transient notice (auto-dismisses after `dismissMs`); a second `show` before the timer fires supersedes the first.
  show(message: string): void {
    this.disarm();
    this.message = message;
    this.onChange(message);
    this.handle = this.timer.set(() => {
      this.handle = null;
      this.dismiss();
    }, this.dismissMs);
  }

  // Drop the current toast immediately (and cancel its timer); safe to call when nothing is showing.
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
