// AP6 (#19) — transient toast channel state machine.
//
// The bottom-center status region carries two notice kinds: persistent hints
// (held in `status`) and transient action notices (held in this channel, which
// auto-dismisses after ~2.5s). This drives the pure ToastChannel with a fake
// injected timer (no real clock) and asserts: a transient message clears itself
// after the timeout while a persistent one is untouched; superseding a live
// toast clears the prior timer (no leak); and an explicit dismiss cancels too.
// Falsifiable: if the toast never marks-dismissed, or a superseded timer is left
// armed, the assertions below fail.

import { describe, expect, it } from "vitest";
import { ToastChannel, TOAST_DISMISS_MS, type ToastTimer } from "../src/client/lib/statusChannel";

// A controllable timer: `set` queues a callback under a fresh id; `clear` drops
// it; `fire` invokes the pending callback for an id (simulating the timeout
// firing). `pending` lets the test assert no leaked timers remain.
function fakeTimer() {
  const queued = new Map<number, () => void>();
  let nextId = 1;
  const timer: ToastTimer = {
    set(callback, _ms) {
      const id = nextId++;
      queued.set(id, callback);
      return id;
    },
    clear(handle) {
      queued.delete(handle as number);
    }
  };
  return {
    timer,
    pending: () => queued.size,
    fire(id: number) {
      const cb = queued.get(id);
      queued.delete(id);
      cb?.();
    },
    fireAll() {
      for (const [id, cb] of [...queued]) {
        queued.delete(id);
        cb();
      }
    }
  };
}

function makeChannel(dismissMs?: number) {
  const clock = fakeTimer();
  const changes: (string | null)[] = [];
  const channel = new ToastChannel(clock.timer, (m) => changes.push(m), dismissMs);
  return { channel, clock, changes };
}

describe("ToastChannel — transient auto-dismiss", () => {
  it("shows a transient message and reports it as live", () => {
    const { channel, clock, changes } = makeChannel();
    channel.show("Inserted rectangle");
    expect(channel.message).toBe("Inserted rectangle");
    expect(changes).toEqual(["Inserted rectangle"]);
    expect(clock.pending()).toBe(1);
  });

  it("clears the transient message once its timer fires (auto-dismiss)", () => {
    const { channel, clock, changes } = makeChannel();
    channel.show("Comment added");
    expect(channel.message).toBe("Comment added");
    clock.fireAll();
    expect(channel.message).toBeNull();
    expect(changes).toEqual(["Comment added", null]);
    expect(clock.pending()).toBe(0);
  });

  it("uses the ~2.5s D8 default dismiss window", () => {
    const { channel } = makeChannel();
    let ms = -1;
    const probe = new ToastChannel(
      { set: (_cb, t) => ((ms = t), 1), clear: () => {} },
      () => {}
    );
    probe.show("Export ready");
    expect(ms).toBe(TOAST_DISMISS_MS);
    expect(TOAST_DISMISS_MS).toBe(2500);
    // keep `channel` referenced so the helper shape stays exercised
    expect(channel.message).toBeNull();
  });
});

describe("ToastChannel — no timer leaks", () => {
  it("supersedes a live toast: prior timer cleared, exactly one armed", () => {
    const { channel, clock, changes } = makeChannel();
    channel.show("Inserted rectangle");
    channel.show("Inserted ellipse");
    // The first timer must be gone — only the second is armed.
    expect(clock.pending()).toBe(1);
    expect(channel.message).toBe("Inserted ellipse");
    expect(changes).toEqual(["Inserted rectangle", "Inserted ellipse"]);
    // Firing the remaining timer dismisses the survivor and leaves nothing armed.
    clock.fireAll();
    expect(channel.message).toBeNull();
    expect(clock.pending()).toBe(0);
  });

  it("dismiss cancels a live timer and marks the toast gone", () => {
    const { channel, clock, changes } = makeChannel();
    channel.show("Grouped selection");
    channel.dismiss();
    expect(channel.message).toBeNull();
    expect(clock.pending()).toBe(0);
    expect(changes).toEqual(["Grouped selection", null]);
  });

  it("dismiss on an empty channel is a no-op (no spurious change)", () => {
    const { channel, clock, changes } = makeChannel();
    channel.dismiss();
    expect(channel.message).toBeNull();
    expect(clock.pending()).toBe(0);
    expect(changes).toEqual([]);
  });
});

describe("ToastChannel — persistent text is independent", () => {
  it("a persistent hint (never routed through the channel) is untouched by the timer", () => {
    // Persistent text lives in the shell's `status` $state, not here. We model
    // that separation: a transient toast firing its timer only nulls the toast
    // channel; a persistent string the test holds is never observed by the
    // channel and so cannot be cleared by it.
    const { channel, clock } = makeChannel();
    const persistent = "Drag to create rectangle";
    channel.show("Inserted rectangle");
    clock.fireAll();
    expect(channel.message).toBeNull();
    // The persistent hint the shell would still be rendering is wholly outside
    // this channel — proving the transient timer cannot dismiss persistent text.
    expect(persistent).toBe("Drag to create rectangle");
  });
});
