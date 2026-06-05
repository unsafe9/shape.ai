import { describe, expect, it } from "vitest";
import {
  createHeterogeneousFixture,
  type FixtureCommentMarker,
  type FixtureActorMarker,
  type HeterogeneousFixture
} from "../src/client/renderer/fixtures";

describe("heterogeneous fixture generator", () => {
  it("generates a deterministic heterogeneous 1k+ workspace mix", () => {
    const a = createHeterogeneousFixture({ seed: 21, profile: "mixed-1k" });
    const b = createHeterogeneousFixture({ seed: 21, profile: "mixed-1k" });
    const total = a.snapshot.cards.length + a.snapshot.edges.length + a.snapshot.groups.length;
    expect(total).toBeGreaterThanOrEqual(1_000);
    expect(a.snapshot.cards[100]).toEqual(b.snapshot.cards[100]);
    expect(new Set(a.snapshot.cards.map((c) => c.styleKey)).size).toBeGreaterThan(3);
    expect(a.comments.length).toBeGreaterThan(0);
    expect(a.actorMarkers.length).toBeGreaterThan(0);
  });

  it("generates a deterministic heterogeneous 5k workspace mix", () => {
    const a = createHeterogeneousFixture({ seed: 99, profile: "mixed-5k" });
    const b = createHeterogeneousFixture({ seed: 99, profile: "mixed-5k" });
    const total = a.snapshot.cards.length + a.snapshot.edges.length + a.snapshot.groups.length;
    expect(total).toBeGreaterThanOrEqual(5_000);
    expect(a.snapshot.cards[500]).toEqual(b.snapshot.cards[500]);
    expect(a.comments.length).toBeGreaterThan(0);
    expect(a.actorMarkers.length).toBeGreaterThan(0);
  });

  it("generates a deterministic heterogeneous workspace mix at 10k+ scale", () => {
    const a = createHeterogeneousFixture({ seed: 7, profile: "mixed-workspace" });
    const b = createHeterogeneousFixture({ seed: 7, profile: "mixed-workspace" });
    const total = a.snapshot.cards.length + a.snapshot.edges.length + a.snapshot.groups.length;
    expect(total).toBeGreaterThanOrEqual(10_000);
    expect(a.snapshot.cards[1000]).toEqual(b.snapshot.cards[1000]);
    expect(a.comments.length).toBeGreaterThan(0);
    expect(a.actorMarkers.length).toBeGreaterThan(0);
  });

  it("produces a valid SceneSnapshot (version, sceneId, metadata.source)", () => {
    const fixture = createHeterogeneousFixture({ seed: 42, profile: "mixed-1k" });
    const snap = fixture.snapshot;
    expect(snap.version).toBe(1);
    expect(snap.sceneId).toMatch(/^heterogeneous-/);
    expect(snap.metadata.source).toBe("fixture");
    expect(snap.metadata.fixtureSeed).toBe(42);
    expect(snap.styles.length).toBeGreaterThan(0);
    expect(snap.selection).toEqual({ kind: "canvas" });
  });

  it("covers multiple fixture families (heterogeneous styleKeys)", () => {
    const { snapshot } = createHeterogeneousFixture({ seed: 3, profile: "mixed-1k" });
    const styleKeys = new Set(snapshot.cards.map((c) => c.styleKey));
    // Should include at least: default, task, evidence, decision, risk, artifact
    expect(styleKeys.size).toBeGreaterThanOrEqual(4);
    expect(styleKeys.has("task")).toBe(true);
    expect(styleKeys.has("evidence")).toBe(true);
    expect(styleKeys.has("artifact")).toBe(true);
  });

  it("covers multiple card types", () => {
    const { snapshot } = createHeterogeneousFixture({ seed: 5, profile: "mixed-1k" });
    const types = new Set(snapshot.cards.map((c) => c.type));
    expect(types.size).toBeGreaterThanOrEqual(5);
  });

  it("contains groups representing frame regions", () => {
    const { snapshot } = createHeterogeneousFixture({ seed: 8, profile: "mixed-1k" });
    expect(snapshot.groups.length).toBeGreaterThanOrEqual(4);
    // All cards reference a valid group
    const groupIds = new Set(snapshot.groups.map((g) => g.id));
    for (const card of snapshot.cards) {
      expect(groupIds.has(card.groupId)).toBe(true);
    }
  });

  it("produces edges referencing valid card ids", () => {
    const { snapshot } = createHeterogeneousFixture({ seed: 13, profile: "mixed-1k" });
    const cardIds = new Set(snapshot.cards.map((c) => c.id));
    for (const edge of snapshot.edges) {
      expect(cardIds.has(edge.source)).toBe(true);
      expect(cardIds.has(edge.target)).toBe(true);
    }
  });

  it("includes labeled and label-less edges (exercises compact edge path)", () => {
    const { snapshot } = createHeterogeneousFixture({ seed: 17, profile: "mixed-1k" });
    const hasLabeled = snapshot.edges.some((e) => e.label.length > 0);
    const hasUnlabeled = snapshot.edges.some((e) => e.label === "");
    expect(hasLabeled).toBe(true);
    expect(hasUnlabeled).toBe(true);
  });

  it("sidecar comments are anchored to valid scene objects", () => {
    const { snapshot, comments } = createHeterogeneousFixture({ seed: 22, profile: "mixed-1k" });
    const cardIds = new Set(snapshot.cards.map((c) => c.id));
    const edgeIds = new Set(snapshot.edges.map((e) => e.id));
    const groupIds = new Set(snapshot.groups.map((g) => g.id));
    for (const comment of comments) {
      expect(comment.id).toMatch(/^hf-comment-/);
      expect(comment.body.length).toBeGreaterThan(0);
      expect(typeof comment.resolved).toBe("boolean");
      if (comment.target.kind === "node") expect(cardIds.has(comment.target.id)).toBe(true);
      if (comment.target.kind === "edge") expect(edgeIds.has(comment.target.id)).toBe(true);
      if (comment.target.kind === "group") expect(groupIds.has(comment.target.id)).toBe(true);
    }
  });

  it("sidecar actor markers have valid structure", () => {
    const { actorMarkers } = createHeterogeneousFixture({ seed: 33, profile: "mixed-1k" });
    for (const marker of actorMarkers) {
      expect(marker.clientId.length).toBeGreaterThan(0);
      expect(["companion", "collaborator", "spectator"]).toContain(marker.actorType);
      expect(["active", "idle", "following"]).toContain(marker.activityState);
    }
  });

  it("comment and actor markers are NOT in the snapshot", () => {
    const fixture = createHeterogeneousFixture({ seed: 44, profile: "mixed-1k" });
    const snap = fixture.snapshot as unknown as Record<string, unknown>;
    expect(snap["comments"]).toBeUndefined();
    expect(snap["actorMarkers"]).toBeUndefined();
  });

  it("respects familyWeights to skew distribution toward wiki cards", () => {
    const defaultFixture = createHeterogeneousFixture({ seed: 55, profile: "mixed-1k" });
    const wikiFixture = createHeterogeneousFixture({
      seed: 55,
      profile: "mixed-1k",
      familyWeights: { wiki: 5.0 }
    });
    const defaultWikiCount = defaultFixture.snapshot.cards.filter((c) => c.type === "evidence").length;
    const skewedWikiCount = wikiFixture.snapshot.cards.filter((c) => c.type === "evidence").length;
    expect(skewedWikiCount).toBeGreaterThan(defaultWikiCount);
  });

  it("determinism holds across different seeds", () => {
    const a1 = createHeterogeneousFixture({ seed: 1, profile: "mixed-1k" });
    const a2 = createHeterogeneousFixture({ seed: 1, profile: "mixed-1k" });
    const b = createHeterogeneousFixture({ seed: 2, profile: "mixed-1k" });
    // Same seed → same fixture
    expect(a1.snapshot.cards[0]).toEqual(a2.snapshot.cards[0]);
    // Different seeds → different results
    expect(a1.snapshot.cards[0].bounds.x).not.toEqual(b.snapshot.cards[0].bounds.x);
  });

  it("default profile is mixed-1k when omitted", () => {
    const a = createHeterogeneousFixture({ seed: 77 });
    const b = createHeterogeneousFixture({ seed: 77, profile: "mixed-1k" });
    expect(a.snapshot.cards.length).toEqual(b.snapshot.cards.length);
    expect(a.snapshot.cards[0]).toEqual(b.snapshot.cards[0]);
  });

  it("wiki cards include CJK text content", () => {
    const { snapshot } = createHeterogeneousFixture({ seed: 88, profile: "mixed-1k" });
    const wikiCards = snapshot.cards.filter((c) => c.id.startsWith("hf-wiki-"));
    expect(wikiCards.length).toBeGreaterThan(0);
    const hasCjk = wikiCards.some((c) => /[぀-鿿가-힯]/.test(c.summary + c.detail));
    expect(hasCjk).toBe(true);
  });

  it("mixed geometry: cards have varied dimensions", () => {
    const { snapshot } = createHeterogeneousFixture({ seed: 66, profile: "mixed-1k" });
    const widths = new Set(snapshot.cards.map((c) => c.bounds.width));
    const heights = new Set(snapshot.cards.map((c) => c.bounds.height));
    expect(widths.size).toBeGreaterThan(5);
    expect(heights.size).toBeGreaterThan(5);
  });
});
