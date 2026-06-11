import { defaultStyles, type RenderCard, type RenderEdge, type RenderGroup, type SceneSnapshot, type SceneSelection, type WorldRect } from "./scene";

export type FixtureOptions = {
  seed?: number;
  groups?: number;
  cards?: number;
  edges?: number;
};

export type FixtureProfile = "mixed-1k" | "mixed-5k" | "mixed-workspace";

export type FixtureFamily =
  | "note"
  | "shape"
  | "frame"
  | "todo"
  | "wiki"
  | "slide"
  | "architecture"
  | "artifactPreview";

export type HeterogeneousFixtureOptions = FixtureOptions & {
  profile?: FixtureProfile;
  familyWeights?: Partial<Record<FixtureFamily, number>>;
};

export type FixtureCommentMarker = {
  id: string;
  target: SceneSelection;
  body: string;
  author: string;
  resolved: boolean;
};

export type FixtureActorMarker = {
  clientId: string;
  actorType: "companion" | "collaborator" | "spectator";
  target: SceneSelection | WorldRect;
  activityState: "active" | "idle" | "following";
};

export type HeterogeneousFixture = {
  snapshot: SceneSnapshot;
  comments: FixtureCommentMarker[];
  actorMarkers: FixtureActorMarker[];
};

const generatedAt = "2026-06-05T00:00:00.000Z";

export function createBenchmarkFixture(options: FixtureOptions = {}): SceneSnapshot {
  const seed = options.seed ?? 42;
  const groupCount = Math.max(1, options.groups ?? 6);
  const cardCount = Math.max(1, options.cards ?? 1_200);
  const edgeCount = Math.max(0, options.edges ?? cardCount);
  const random = lcg(seed);
  const groups: RenderGroup[] = [];
  const cards: RenderCard[] = [];
  const edges: RenderEdge[] = [];
  const columnsPerGroup = 10;
  const cardWidth = 260;
  const cardHeight = 156;
  const gapX = 118;
  const gapY = 106;
  const groupWidth = columnsPerGroup * (cardWidth + gapX) + 240;
  const rowsPerGroup = Math.ceil(cardCount / groupCount / columnsPerGroup);
  const groupHeight = Math.max(860, rowsPerGroup * (cardHeight + gapY) + 260);

  for (let groupIndex = 0; groupIndex < groupCount; groupIndex += 1) {
    const gx = (groupIndex % 3) * (groupWidth + 460);
    const gy = Math.floor(groupIndex / 3) * (groupHeight + 520);
    const styleKey = groupIndex % 3 === 0 ? "decision" : groupIndex % 3 === 1 ? "evidence" : "risk";
    groups.push({
      id: `fixture-group-${groupIndex}`,
      title: `Engine validation group ${groupIndex + 1}`,
      summary: "Retained world frame for continuous pan and zoom checks.",
      bounds: { x: gx, y: gy, width: groupWidth, height: groupHeight },
      tagIds: [`tag-${groupIndex % 4}`],
      zIndex: groupIndex,
      styleKey
    });
  }

  for (let index = 0; index < cardCount; index += 1) {
    const group = groups[index % groupCount];
    const localIndex = Math.floor(index / groupCount);
    const col = localIndex % columnsPerGroup;
    const row = Math.floor(localIndex / columnsPerGroup);
    const jitterX = Math.round((random() - 0.5) * 32);
    const jitterY = Math.round((random() - 0.5) * 32);
    const type = index % 5 === 0 ? "decision_point" : index % 5 === 1 ? "option" : index % 5 === 2 ? "evidence" : index % 5 === 3 ? "task" : "tradeoff";
    const status = index % 7 === 0 ? "selected" : index % 7 === 1 ? "viable" : index % 7 === 2 ? "conditional" : "draft";
    const title = `${labelForType(type)} ${index + 1}`;
    const summary = `Seeded card ${index + 1} validates text cache, edge route, hit region, and stable id behavior.`;
    cards.push({
      id: `fixture-card-${index}`,
      groupId: group.id,
      title,
      summary,
      detail: `${summary} Detail text remains business-owned in TypeScript and is only projected as render text.`,
      status,
      type,
      bounds: {
        x: group.bounds.x + 120 + col * (cardWidth + gapX) + jitterX,
        y: group.bounds.y + 150 + row * (cardHeight + gapY) + jitterY,
        width: cardWidth,
        height: cardHeight
      },
      zIndex: index,
      styleKey: type === "evidence" ? "evidence" : type === "tradeoff" ? "risk" : "decision",
      accessibilityLabel: `${type} ${title}. ${summary}`
    });
  }

  for (let index = 0; index < edgeCount; index += 1) {
    const sourceIndex = index % cardCount;
    const targetOffset = 1 + Math.floor(random() * Math.min(17, cardCount - 1));
    const targetIndex = (sourceIndex + targetOffset) % cardCount;
    const source = cards[sourceIndex];
    const target = cards[targetIndex];
    edges.push({
      id: `fixture-edge-${index}`,
      groupId: source.groupId === target.groupId ? source.groupId : groups[index % groupCount].id,
      source: source.id,
      target: target.id,
      label: index % 4 === 0 ? "depends on" : index % 4 === 1 ? "supports" : index % 4 === 2 ? "produces" : "blocks",
      type: index % 4 === 3 ? "blocks" : "supports",
      zIndex: index,
      styleKey: index % 4 === 3 ? "risk" : "default"
    });
  }

  return {
    version: 1,
    sceneId: `benchmark-${seed}-${cardCount}`,
    camera: { x: 120, y: 96, zoom: 0.18 },
    groups,
    cards,
    edges,
    styles: defaultStyles,
    selection: { kind: "canvas" },
    metadata: {
      source: "fixture",
      generatedAt,
      fixtureSeed: seed,
      notes: [
        "Deterministic 1,000+ card/edge fixture for frame time, memory, hit-test, and overlay checks.",
        "Business fields are projected into render text only; comments/tags/artifacts remain outside the renderer."
      ]
    }
  };
}

export function createSmallFixture(): SceneSnapshot {
  return createBenchmarkFixture({ seed: 7, groups: 2, cards: 18, edges: 24 });
}

function labelForType(type: string): string {
  if (type === "decision_point") return "Decision";
  if (type === "evidence") return "Evidence";
  if (type === "task") return "Task";
  if (type === "tradeoff") return "Tradeoff";
  return "Option";
}

function lcg(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = Math.imul(1664525, state) + 1013904223;
    return ((state >>> 0) % 1_000_000) / 1_000_000;
  };
}

// -- Heterogeneous fixture generator --

const PROFILE_SCALES: Record<FixtureProfile, { targetObjects: number; regions: number }> = {
  "mixed-1k": { targetObjects: 1_000, regions: 4 },
  "mixed-5k": { targetObjects: 5_000, regions: 8 },
  "mixed-workspace": { targetObjects: 10_000, regions: 12 }
};

const DEFAULT_FAMILY_WEIGHTS: Record<FixtureFamily, number> = {
  note: 0.18,
  shape: 0.12,
  frame: 0.06,
  todo: 0.20,
  wiki: 0.14,
  slide: 0.08,
  architecture: 0.12,
  artifactPreview: 0.10
};

export function createHeterogeneousFixture(options: HeterogeneousFixtureOptions = {}): HeterogeneousFixture {
  const seed = options.seed ?? 42;
  const profile = options.profile ?? "mixed-1k";
  const random = lcg(seed);
  const scale = PROFILE_SCALES[profile];
  const regions = scale.regions;

  // Merge family weights (user-supplied weights override defaults)
  const rawWeights: Record<FixtureFamily, number> = { ...DEFAULT_FAMILY_WEIGHTS };
  if (options.familyWeights) {
    const families = Object.keys(options.familyWeights) as FixtureFamily[];
    for (const f of families) {
      const w = options.familyWeights[f];
      if (w !== undefined) rawWeights[f] = w;
    }
  }
  const totalWeight = (Object.values(rawWeights) as number[]).reduce((a, b) => a + b, 0);

  // Compute per-family card/group count budgets based on target
  const budget = scale.targetObjects;
  // Reserve ~10% of budget for edges (added on top), use rest for cards/groups
  const cardBudget = Math.floor(budget * 0.88);

  const familyCounts: Record<FixtureFamily, number> = {
    note: 0,
    shape: 0,
    frame: 0,
    todo: 0,
    wiki: 0,
    slide: 0,
    architecture: 0,
    artifactPreview: 0
  };
  const families = Object.keys(rawWeights) as FixtureFamily[];
  let allocated = 0;
  for (let i = 0; i < families.length; i++) {
    const f = families[i];
    if (i === families.length - 1) {
      familyCounts[f] = Math.max(1, cardBudget - allocated);
    } else {
      const count = Math.max(1, Math.round((rawWeights[f] / totalWeight) * cardBudget));
      familyCounts[f] = count;
      allocated += count;
    }
  }

  const groups: RenderGroup[] = [];
  const cards: RenderCard[] = [];
  const edges: RenderEdge[] = [];

  // Region layout: place regions in a grid, each region is a large frame
  const regionW = 2400;
  const regionH = 1800;
  const regionCols = Math.ceil(Math.sqrt(regions));
  const regionGapX = 600;
  const regionGapY = 500;

  // We build one top-level group per region (acts as "frame")
  for (let ri = 0; ri < regions; ri++) {
    const rx = (ri % regionCols) * (regionW + regionGapX);
    const ry = Math.floor(ri / regionCols) * (regionH + regionGapY);
    const regionTypes = ["todo", "wiki", "slide", "architecture", "note", "shape", "artifactPreview", "mixed"] as const;
    const regionLabel = regionTypes[ri % regionTypes.length];
    groups.push({
      id: `hf-region-${ri}`,
      title: `${regionLabel.charAt(0).toUpperCase()}${regionLabel.slice(1)} region ${ri + 1}`,
      summary: `Heterogeneous workspace region ${ri + 1} — ${profile}`,
      bounds: { x: rx, y: ry, width: regionW, height: regionH },
      tagIds: [`tag-region-${ri % 5}`],
      zIndex: ri,
      styleKey: ri % 4 === 0 ? "default" : ri % 4 === 1 ? "decision" : ri % 4 === 2 ? "evidence" : "risk"
    });
  }

  let cardZIndex = 0;

  // Each family emitter places cards into whichever region(s) best fit it.
  // To ensure determinism regardless of familyWeights ordering, each emitter
  // consumes a fixed number of PRNG draws per object (4 draws: x, y, w, h or variants).

  // Helper: pick a region for a card family
  function regionFor(familyIndex: number): RenderGroup {
    return groups[familyIndex % regions];
  }

  // --- note family ---
  emitNotes(random, familyCounts.note, regionFor, cards, cardZIndex);
  cardZIndex += familyCounts.note;

  // --- shape family ---
  emitShapes(random, familyCounts.shape, regionFor, cards, cardZIndex);
  cardZIndex += familyCounts.shape;

  // --- todo family ---
  emitTodoCards(random, familyCounts.todo, regionFor, cards, cardZIndex);
  cardZIndex += familyCounts.todo;

  // --- wiki family ---
  emitWikiCards(random, familyCounts.wiki, regionFor, cards, cardZIndex);
  cardZIndex += familyCounts.wiki;

  // --- slide family (slide groups + title/body cards) ---
  emitSlideFrames(random, familyCounts.slide, regionFor, groups, cards, cardZIndex, regions);
  cardZIndex += familyCounts.slide;

  // --- architecture family ---
  emitArchBoxes(random, familyCounts.architecture, regionFor, cards, edges, cardZIndex);
  cardZIndex += familyCounts.architecture;

  // --- artifactPreview family ---
  emitArtifactPreviews(random, familyCounts.artifactPreview, regionFor, cards, cardZIndex);
  cardZIndex += familyCounts.artifactPreview;

  // --- frame (extra nested groups) ---
  emitFrameGroups(random, familyCounts.frame, regionFor, groups, regions);

  // --- edges: cross-card connections for notes/shapes/todos (heterogeneous edge types) ---
  const edgeSourcePool = cards.filter((c) => c.type !== "slide_title" && c.type !== "slide_body");
  const edgeCount = Math.min(Math.floor(budget * 0.12), Math.max(10, edgeSourcePool.length));
  emitMixedEdges(random, edgeCount, edgeSourcePool, groups, edges, regions);

  // --- sidecar: comments anchored on cards/edges/groups ---
  const comments: FixtureCommentMarker[] = emitCommentMarkers(random, Math.max(8, Math.floor(cards.length * 0.04)), cards, edges, groups);

  // --- sidecar: actor markers ---
  const actorMarkers: FixtureActorMarker[] = emitActorMarkers(random, Math.max(4, Math.floor(cards.length * 0.01)), cards, groups);

  const snapshot: SceneSnapshot = {
    version: 1,
    sceneId: `heterogeneous-${seed}-${profile}`,
    camera: { x: 120, y: 96, zoom: 0.08 },
    groups,
    cards,
    edges,
    styles: defaultStyles,
    selection: { kind: "canvas" },
    metadata: {
      source: "fixture",
      generatedAt,
      fixtureSeed: seed,
      notes: [
        `Deterministic heterogeneous fixture — profile: ${profile}, seed: ${seed}.`,
        "Covers notes, shapes, todo cards, wiki cards, slide frames, architecture diagrams, artifact previews, and frame groups.",
        "Sidecar: comment markers and actor markers (not in SceneSnapshot per render contract)."
      ]
    }
  };

  return { snapshot, comments, actorMarkers };
}

// Fixed PRNG budget per object: 4 draws regardless of family, so downstream
// randomness is stable even when familyWeights change the count allocations.
function emitNotes(
  random: () => number,
  count: number,
  regionFor: (i: number) => RenderGroup,
  cards: RenderCard[],
  baseZ: number
): void {
  for (let i = 0; i < count; i++) {
    const r1 = random(); const r2 = random(); const r3 = random(); const r4 = random();
    const region = regionFor(i);
    const w = 180 + Math.round(r3 * 140); // 180–320
    const h = 90 + Math.round(r4 * 170);  // 90–260
    const x = region.bounds.x + 60 + Math.round(r1 * (region.bounds.width - w - 120));
    const y = region.bounds.y + 60 + Math.round(r2 * (region.bounds.height - h - 120));
    const hasBody = i % 3 !== 0;
    cards.push({
      id: `hf-note-${i}`,
      groupId: region.id,
      title: `Note ${i + 1}`,
      summary: hasBody ? `Short note body for item ${i + 1}.` : "",
      detail: "",
      status: "draft",
      type: "note",
      bounds: { x, y, width: w, height: h },
      zIndex: baseZ + i,
      styleKey: "default",
      accessibilityLabel: `Note ${i + 1}`
    });
  }
}

function emitShapes(
  random: () => number,
  count: number,
  regionFor: (i: number) => RenderGroup,
  cards: RenderCard[],
  baseZ: number
): void {
  for (let i = 0; i < count; i++) {
    const r1 = random(); const r2 = random(); const r3 = random(); const r4 = random();
    const region = regionFor(i + 1);
    const isSquare = i % 3 === 0;
    const w = isSquare ? 140 + Math.round(r3 * 80) : 220 + Math.round(r3 * 160);
    const h = isSquare ? w : 100 + Math.round(r4 * 80);
    const x = region.bounds.x + 60 + Math.round(r1 * (region.bounds.width - w - 120));
    const y = region.bounds.y + 60 + Math.round(r2 * (region.bounds.height - h - 120));
    const hasLabel = i % 2 === 0;
    cards.push({
      id: `hf-shape-${i}`,
      groupId: region.id,
      title: hasLabel ? `Shape ${i + 1}` : "",
      summary: "",
      detail: "",
      status: "draft",
      type: "shape",
      bounds: { x, y, width: Math.max(60, w), height: Math.max(60, h) },
      zIndex: baseZ + i,
      styleKey: i % 2 === 0 ? "default" : "option",
      accessibilityLabel: `Shape ${i + 1}`
    });
  }
}

function emitTodoCards(
  random: () => number,
  count: number,
  regionFor: (i: number) => RenderGroup,
  cards: RenderCard[],
  baseZ: number
): void {
  const statuses = ["selected", "viable", "conditional", "draft"] as const;
  const colW = 220;
  const colH = 120;
  const colsPerGroup = 4;
  for (let i = 0; i < count; i++) {
    const r1 = random(); const r2 = random(); const _r3 = random(); const _r4 = random();
    const region = regionFor(i + 2);
    const col = i % colsPerGroup;
    const row = Math.floor(i / colsPerGroup);
    const jx = Math.round((r1 - 0.5) * 16);
    const jy = Math.round((r2 - 0.5) * 16);
    const x = region.bounds.x + 80 + col * (colW + 24) + jx;
    const y = region.bounds.y + 100 + row * (colH + 18) + jy;
    const status = statuses[i % statuses.length];
    cards.push({
      id: `hf-todo-${i}`,
      groupId: region.id,
      title: `Todo item ${i + 1}`,
      summary: `Short task body ${i + 1}.`,
      detail: "",
      status,
      type: "task",
      bounds: { x, y, width: colW, height: colH },
      zIndex: baseZ + i,
      styleKey: "task",
      accessibilityLabel: `Task ${i + 1}: ${status}`
    });
  }
}

const CJK_SAMPLES = [
  "이 카드는 위키 항목입니다. 긴 내용을 담고 있어 텍스트 캐시를 테스트합니다.",
  "架构决策记录：评估多种方案后选择最优解。",
  "ドキュメントの内容は長いテキストとCJK文字のミックスです。",
  "컴포넌트 설계 및 데이터 흐름에 대한 상세 설명입니다."
];
const LONG_NOSPACE = "antidisestablishmentarianismsupercalifragilisticexpialidocious";

function emitWikiCards(
  random: () => number,
  count: number,
  regionFor: (i: number) => RenderGroup,
  cards: RenderCard[],
  baseZ: number
): void {
  for (let i = 0; i < count; i++) {
    const r1 = random(); const r2 = random(); const r3 = random(); const r4 = random();
    const region = regionFor(i + 3);
    const w = 260 + Math.round(r3 * 100);
    const h = 320 + Math.round(r4 * 200); // tall cards
    const x = region.bounds.x + 80 + Math.round(r1 * (region.bounds.width - w - 160));
    const y = region.bounds.y + 80 + Math.round(r2 * (region.bounds.height - h - 160));
    const cjk = CJK_SAMPLES[i % CJK_SAMPLES.length];
    const body = i % 5 === 0 ? LONG_NOSPACE : `${cjk} Additional detail for wiki entry ${i + 1}.`;
    cards.push({
      id: `hf-wiki-${i}`,
      groupId: region.id,
      title: `Wiki entry ${i + 1}`,
      summary: body,
      detail: `Full wiki content for entry ${i + 1}. ${cjk}`,
      status: "viable",
      type: "evidence",
      bounds: { x, y, width: w, height: Math.max(200, h) },
      zIndex: baseZ + i,
      styleKey: "evidence",
      accessibilityLabel: `Wiki entry ${i + 1}`
    });
  }
}

function emitSlideFrames(
  random: () => number,
  count: number,
  regionFor: (i: number) => RenderGroup,
  groups: RenderGroup[],
  cards: RenderCard[],
  baseZ: number,
  existingRegions: number
): void {
  const slidesPerDeck = Math.max(2, Math.floor(count / 4));
  const deckCount = Math.max(1, Math.ceil(count / slidesPerDeck));
  let cardIndex = 0;
  for (let d = 0; d < deckCount; d++) {
    const r1 = random(); const r2 = random(); const _r3 = random(); const _r4 = random();
    const region = regionFor(d + 4);
    const deckX = region.bounds.x + 100 + Math.round(r1 * 400);
    const deckY = region.bounds.y + 100 + Math.round(r2 * 300);
    const slideW = 800;
    const slideH = 480;
    for (let s = 0; s < slidesPerDeck && cardIndex < count; s++, cardIndex++) {
      const _rs1 = random(); const _rs2 = random(); const _rs3 = random(); const _rs4 = random();
      const slideGid = `hf-slide-group-${d}-${s}`;
      groups.push({
        id: slideGid,
        title: `Slide ${s + 1} — Deck ${d + 1}`,
        summary: "",
        bounds: {
          x: deckX + s * (slideW + 80),
          y: deckY,
          width: slideW,
          height: slideH
        },
        tagIds: [`tag-slide-${d}`],
        zIndex: existingRegions + d * slidesPerDeck + s,
        styleKey: "default"
      });
      // title card
      cards.push({
        id: `hf-slide-title-${d}-${s}`,
        groupId: slideGid,
        title: `Slide ${s + 1} title`,
        summary: "",
        detail: "",
        status: "draft",
        type: "slide_title",
        bounds: { x: deckX + s * (slideW + 80) + 40, y: deckY + 40, width: slideW - 80, height: 80 },
        zIndex: baseZ + cardIndex,
        styleKey: "default",
        accessibilityLabel: `Slide ${s + 1} title`
      });
      // body card
      cards.push({
        id: `hf-slide-body-${d}-${s}`,
        groupId: slideGid,
        title: `Body content for slide ${s + 1}`,
        summary: `Slide ${s + 1} body text in deck ${d + 1}.`,
        detail: "",
        status: "draft",
        type: "slide_body",
        bounds: { x: deckX + s * (slideW + 80) + 40, y: deckY + 140, width: slideW - 80, height: 280 },
        zIndex: baseZ + cardIndex + 1,
        styleKey: "default",
        accessibilityLabel: `Slide ${s + 1} body`
      });
    }
  }
}

function emitArchBoxes(
  random: () => number,
  count: number,
  regionFor: (i: number) => RenderGroup,
  cards: RenderCard[],
  edges: RenderEdge[],
  baseZ: number
): void {
  const cols = Math.max(3, Math.ceil(Math.sqrt(count)));
  const boxW = 160;
  const boxH = 100;
  const boxGapX = 80;
  const boxGapY = 60;
  const firstCardIndex = cards.length;
  for (let i = 0; i < count; i++) {
    const r1 = random(); const r2 = random(); const _r3 = random(); const _r4 = random();
    const region = regionFor(i + 5);
    const col = i % cols;
    const row = Math.floor(i / cols);
    const jx = Math.round((r1 - 0.5) * 20);
    const jy = Math.round((r2 - 0.5) * 20);
    const x = region.bounds.x + 120 + col * (boxW + boxGapX) + jx;
    const y = region.bounds.y + 120 + row * (boxH + boxGapY) + jy;
    const isService = i % 3 !== 2;
    cards.push({
      id: `hf-arch-${i}`,
      groupId: region.id,
      title: isService ? `Service ${i + 1}` : `Risk node ${i + 1}`,
      summary: "",
      detail: "",
      status: "draft",
      type: isService ? "decision_point" : "tradeoff",
      bounds: { x, y, width: boxW, height: boxH },
      zIndex: baseZ + i,
      styleKey: isService ? "decision" : "risk",
      accessibilityLabel: isService ? `Service ${i + 1}` : `Risk ${i + 1}`
    });
  }
  // Dense edges between arch boxes
  const archCards = cards.slice(firstCardIndex);
  const archEdgeCount = Math.min(count * 2, archCards.length * 2);
  for (let i = 0; i < archEdgeCount; i++) {
    const r1 = random(); const _r2 = random(); const _r3 = random(); const _r4 = random();
    const sourceIdx = i % archCards.length;
    const targetIdx = (sourceIdx + 1 + Math.floor(r1 * Math.min(4, archCards.length - 1))) % archCards.length;
    if (sourceIdx === targetIdx) { _r2; _r3; _r4; continue; }
    const src = archCards[sourceIdx];
    const tgt = archCards[targetIdx];
    edges.push({
      id: `hf-arch-edge-${i}`,
      groupId: src.groupId,
      source: src.id,
      target: tgt.id,
      label: i % 2 === 0 ? "depends on" : "produces",
      type: i % 3 === 2 ? "blocks" : "supports",
      zIndex: i,
      styleKey: i % 3 === 2 ? "risk" : "default"
    });
  }
}

function emitArtifactPreviews(
  random: () => number,
  count: number,
  regionFor: (i: number) => RenderGroup,
  cards: RenderCard[],
  baseZ: number
): void {
  for (let i = 0; i < count; i++) {
    const r1 = random(); const r2 = random(); const _r3 = random(); const _r4 = random();
    const region = regionFor(i + 6);
    const w = 240;
    const h = 180;
    const x = region.bounds.x + 80 + Math.round(r1 * (region.bounds.width - w - 160));
    const y = region.bounds.y + 80 + Math.round(r2 * (region.bounds.height - h - 160));
    cards.push({
      id: `hf-artifact-${i}`,
      groupId: region.id,
      title: `Artifact ${i + 1}`,
      summary: `Preview caption for artifact ${i + 1}.`,
      detail: "",
      status: "draft",
      type: "artifact",
      bounds: { x, y, width: w, height: h },
      zIndex: baseZ + i,
      styleKey: "artifact",
      accessibilityLabel: `Artifact preview ${i + 1}`
    });
  }
}

function emitFrameGroups(
  random: () => number,
  count: number,
  regionFor: (i: number) => RenderGroup,
  groups: RenderGroup[],
  existingRegions: number
): void {
  for (let i = 0; i < count; i++) {
    const r1 = random(); const r2 = random(); const r3 = random(); const r4 = random();
    const region = regionFor(i + 7);
    const w = 400 + Math.round(r3 * 600);
    const h = 300 + Math.round(r4 * 500);
    const x = region.bounds.x + 60 + Math.round(r1 * (region.bounds.width - w - 120));
    const y = region.bounds.y + 60 + Math.round(r2 * (region.bounds.height - h - 120));
    groups.push({
      id: `hf-frame-${i}`,
      title: `Frame ${i + 1}`,
      summary: `Nested frame region ${i + 1}`,
      bounds: { x, y, width: Math.max(200, w), height: Math.max(150, h) },
      tagIds: [`tag-frame-${i % 3}`],
      zIndex: existingRegions + 1000 + i,
      styleKey: i % 2 === 0 ? "default" : "decision"
    });
  }
}

function emitMixedEdges(
  random: () => number,
  count: number,
  pool: RenderCard[],
  groups: RenderGroup[],
  edges: RenderEdge[],
  regionCount: number
): void {
  if (pool.length < 2) return;
  const edgeLabels = ["supports", "depends on", "blocks", "produces", "relates to", ""] as const;
  const edgeTypes = ["supports", "blocks", "supports", "supports"] as const;
  const edgeStyles = ["default", "risk", "default", "default"] as const;
  for (let i = 0; i < count; i++) {
    const r1 = random(); const r2 = random(); const _r3 = random(); const _r4 = random();
    const srcIdx = Math.floor(r1 * pool.length);
    const tgtOffset = 1 + Math.floor(r2 * Math.min(20, pool.length - 1));
    const tgtIdx = (srcIdx + tgtOffset) % pool.length;
    const src = pool[srcIdx];
    const tgt = pool[tgtIdx];
    const groupId = src.groupId === tgt.groupId ? src.groupId : groups[i % Math.max(1, regionCount)].id;
    edges.push({
      id: `hf-edge-${i}`,
      groupId,
      source: src.id,
      target: tgt.id,
      label: edgeLabels[i % edgeLabels.length],
      type: edgeTypes[i % edgeTypes.length],
      zIndex: edges.length + i,
      styleKey: edgeStyles[i % edgeStyles.length]
    });
  }
}

function emitCommentMarkers(
  random: () => number,
  count: number,
  cards: RenderCard[],
  edges: RenderEdge[],
  groups: RenderGroup[]
): FixtureCommentMarker[] {
  const result: FixtureCommentMarker[] = [];
  const targets: SceneSelection[] = [
    ...cards.slice(0, Math.min(cards.length, Math.ceil(count * 0.6))).map((c): SceneSelection => ({ kind: "node", id: c.id })),
    ...edges.slice(0, Math.min(edges.length, Math.ceil(count * 0.2))).map((e): SceneSelection => ({ kind: "edge", id: e.id })),
    ...groups.slice(0, Math.min(groups.length, Math.ceil(count * 0.2))).map((g): SceneSelection => ({ kind: "group", id: g.id }))
  ];
  for (let i = 0; i < count; i++) {
    const r1 = random(); const _r2 = random(); const _r3 = random(); const _r4 = random();
    const target = targets[Math.floor(r1 * targets.length)] ?? { kind: "canvas" as const };
    result.push({
      id: `hf-comment-${i}`,
      target,
      body: `Comment ${i + 1}: review note for this item.`,
      author: `user-${(i % 5) + 1}`,
      resolved: i % 7 === 0
    });
  }
  return result;
}

function emitActorMarkers(
  random: () => number,
  count: number,
  cards: RenderCard[],
  groups: RenderGroup[]
): FixtureActorMarker[] {
  const result: FixtureActorMarker[] = [];
  const actorTypes: FixtureActorMarker["actorType"][] = ["companion", "collaborator", "spectator"];
  const activityStates: FixtureActorMarker["activityState"][] = ["active", "idle", "following"];
  for (let i = 0; i < count; i++) {
    const r1 = random(); const r2 = random(); const _r3 = random(); const _r4 = random();
    const useCard = i % 3 !== 2 && cards.length > 0;
    const target: SceneSelection | WorldRect = useCard
      ? { kind: "node", id: cards[Math.floor(r1 * cards.length)].id }
      : groups.length > 0
        ? { kind: "group", id: groups[Math.floor(r2 * groups.length)].id }
        : { kind: "canvas" };
    result.push({
      clientId: `client-${i + 1}`,
      actorType: actorTypes[i % actorTypes.length],
      target,
      activityState: activityStates[i % activityStates.length]
    });
  }
  return result;
}
