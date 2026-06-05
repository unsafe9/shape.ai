import { defaultStyles, type RenderCard, type RenderEdge, type RenderGroup, type SceneSnapshot } from "./scene";

export type FixtureOptions = {
  seed?: number;
  groups?: number;
  cards?: number;
  edges?: number;
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
