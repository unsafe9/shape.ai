import { describe, expect, it } from "vitest";
import { UiModelFeed, type FrameScheduler } from "../runtime/uiModelFeed";

// Controllable frame scheduler: request queues a callback under a fresh id, runFrame fires the queued
// callbacks (one animation frame), pending lets the test assert no leaked frames remain.
function fakeScheduler() {
  const queued = new Map<number, () => void>();
  let nextId = 1;
  let requests = 0;
  const scheduler: FrameScheduler = {
    request(callback) {
      requests++;
      const id = nextId++;
      queued.set(id, callback);
      return id;
    },
    cancel(handle) {
      queued.delete(handle as number);
    }
  };
  return {
    scheduler,
    requests: () => requests,
    pending: () => queued.size,
    runFrame() {
      for (const [id, cb] of [...queued]) {
        queued.delete(id);
        cb();
      }
    }
  };
}

function makeFeed() {
  const frames = fakeScheduler();
  const flushed: string[] = [];
  const feed = new UiModelFeed(frames.scheduler, (json) => flushed.push(json));
  return { feed, frames, flushed };
}

describe("UiModelFeed — rAF coalescing", () => {
  it("collapses a presence burst (many schedules in one frame) to exactly one feed", () => {
    const { feed, frames, flushed } = makeFeed();
    // Simulate a presence storm: 25 cursor frames buffered before the frame runs.
    for (let i = 0; i < 25; i++) feed.schedule(`{"peers":${i}}`);
    // Nothing fed yet — the feed is deferred to the frame.
    expect(flushed).toEqual([]);
    // Exactly one frame was requested for the whole burst (NOT one per schedule).
    expect(frames.requests()).toBe(1);
    frames.runFrame();
    // One feed total, carrying the LATEST model (the others are safely dropped).
    expect(flushed).toEqual([`{"peers":24}`]);
    expect(frames.pending()).toBe(0);
  });

  it("a schedule after a flush queues a fresh frame (low-frequency changes still land)", () => {
    const { feed, frames, flushed } = makeFeed();
    feed.schedule(`{"theme":"dark"}`);
    frames.runFrame();
    feed.schedule(`{"theme":"light"}`);
    frames.runFrame();
    expect(flushed).toEqual([`{"theme":"dark"}`, `{"theme":"light"}`]);
    expect(frames.requests()).toBe(2);
  });

  it("dispose drops a queued feed so it can't fire against a torn-down host", () => {
    const { feed, frames, flushed } = makeFeed();
    feed.schedule(`{"peers":1}`);
    feed.dispose();
    frames.runFrame();
    expect(flushed).toEqual([]);
    expect(frames.pending()).toBe(0);
    // A schedule after dispose still works (the feed is reusable).
    feed.schedule(`{"peers":2}`);
    frames.runFrame();
    expect(flushed).toEqual([`{"peers":2}`]);
  });
});
