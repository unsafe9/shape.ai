/**
 * T4.5 — Presentation Template
 *
 * A TemplateContract for slide decks: nested frame per slide, title/body text
 * cards, image/artifact preview cards, optional connector edges, and speaker-note
 * cards. Everything lowers to the same create-group/create-card/create-edge ops
 * as any other shape or note — no presentation runtime, no slide object type.
 *
 * The export outline reader (presentationOutline) reads the produced primitives
 * sorted by meta.slideIndex and emits a design_doc_md outline — additive branch
 * in generateLocalExport, not a new exportType.
 *
 * Real symbols used: TemplateContract, SceneStyleToken (renderScene.ts),
 * SceneGroup, SceneNode, SceneEdge (schema.ts).
 */

import type { SceneGroup, SceneNode, SceneEdge } from "../schema";
import type { SceneStyleToken } from "../renderScene";
import type { TemplateContract } from "./contract";

// ---------------------------------------------------------------------------
// §1  Slide geometry constants
// ---------------------------------------------------------------------------

/** 16:9 slide world-unit dimensions. */
const SLIDE_WIDTH = 1280;
const SLIDE_HEIGHT = 720;

/** Horizontal gap between slides when tiled in a row. */
const SLIDE_GAP = 80;

/** Title card: top of slide. */
const TITLE_X = 60;
const TITLE_Y = 40;
const TITLE_W = 1160;
const TITLE_H = 100;

/** Body card: below title. */
const BODY_X = 60;
const BODY_Y = 180;
const BODY_W = 780;
const BODY_H = 460;

/** Image preview card: right column. */
const IMAGE_X = 880;
const IMAGE_Y = 180;
const IMAGE_W = 360;
const IMAGE_H = 460;

/** Speaker-note card: sits below the slide frame. */
const NOTE_X = 0;
const NOTE_Y = SLIDE_HEIGHT + 20;
const NOTE_W = SLIDE_WIDTH;
const NOTE_H = 120;

// ---------------------------------------------------------------------------
// §2  Two-slide starter recipe
// ---------------------------------------------------------------------------

/**
 * The presentation TemplateContract — a two-slide starter deck.
 *
 * Applying this template produces only SceneGroup / SceneNode / SceneEdge rows.
 * Title/body/notes are text cards edited via the standard edit-card-text op;
 * slides are frames selected/moved/renamed/deleted like any frame.
 *
 * meta.templateKind = "presentation" is written on every created object so the
 * presentationOutline reader and any future "next slide" affordance can identify
 * deck objects without a separate table.
 */
export const presentationTemplate: TemplateContract = {
  metadata: {
    id: "presentation",
    title: "Presentation",
    description:
      "Slide deck template: nested frames per slide with title/body text, " +
      "image/artifact previews, optional flow connectors, and speaker-note cards.",
    category: "presentation",
    templateKind: "presentation"
  },

  recipe: {
    frames: [
      // Deck-level frame — wraps all slides.
      {
        localId: "deck",
        title: "Presentation",
        summary: "Slide deck",
        meta: { templateKind: "presentation", semanticType: "deck" }
      },
      // Slide 1 — title slide.
      {
        localId: "slide-1",
        parentLocalId: "deck",
        title: "Title slide",
        meta: { templateKind: "presentation", semanticType: "slide", slideIndex: 0 }
      },
      // Slide 2 — content slide.
      {
        localId: "slide-2",
        parentLocalId: "deck",
        title: "Slide 2",
        meta: { templateKind: "presentation", semanticType: "slide", slideIndex: 1 }
      }
    ],

    shapes: [
      // ── Slide 1 ──────────────────────────────────────────────────────────
      {
        localId: "s1-title",
        frameLocalId: "slide-1",
        title: "Presentation title",
        summary: "Subtitle or tagline",
        styleKey: "slide-title",
        position: { x: TITLE_X, y: TITLE_Y },
        size: { width: TITLE_W, height: TITLE_H },
        meta: { templateKind: "presentation", semanticType: "slide-title" }
      },
      {
        localId: "s1-body",
        frameLocalId: "slide-1",
        title: "Body",
        summary: "Opening statement\nKey theme\nAgenda overview",
        styleKey: "slide-body",
        position: { x: BODY_X, y: BODY_Y },
        size: { width: BODY_W, height: BODY_H },
        meta: { templateKind: "presentation", semanticType: "slide-body" }
      },
      {
        localId: "s1-image",
        frameLocalId: "slide-1",
        title: "Image placeholder",
        summary: "Replace with artifact image",
        styleKey: "slide-image",
        position: { x: IMAGE_X, y: IMAGE_Y },
        size: { width: IMAGE_W, height: IMAGE_H },
        meta: { templateKind: "presentation", semanticType: "slide-image", artifactId: null }
      },
      {
        localId: "s1-notes",
        frameLocalId: "slide-1",
        title: "Speaker notes",
        summary: "What to say for slide 1…",
        styleKey: "speaker-note",
        position: { x: NOTE_X, y: NOTE_Y },
        size: { width: NOTE_W, height: NOTE_H },
        meta: { templateKind: "presentation", semanticType: "speaker-note" }
      },

      // ── Slide 2 ──────────────────────────────────────────────────────────
      {
        localId: "s2-title",
        frameLocalId: "slide-2",
        title: "Slide 2 title",
        summary: "",
        styleKey: "slide-title",
        position: { x: SLIDE_WIDTH + SLIDE_GAP + TITLE_X, y: TITLE_Y },
        size: { width: TITLE_W, height: TITLE_H },
        meta: { templateKind: "presentation", semanticType: "slide-title" }
      },
      {
        localId: "s2-body",
        frameLocalId: "slide-2",
        title: "Body",
        summary: "First bullet\nSecond bullet\nThird bullet",
        styleKey: "slide-body",
        position: { x: SLIDE_WIDTH + SLIDE_GAP + BODY_X, y: BODY_Y },
        size: { width: BODY_W, height: BODY_H },
        meta: { templateKind: "presentation", semanticType: "slide-body" }
      },
      {
        localId: "s2-image",
        frameLocalId: "slide-2",
        title: "Image placeholder",
        summary: "Replace with artifact image",
        styleKey: "slide-image",
        position: { x: SLIDE_WIDTH + SLIDE_GAP + IMAGE_X, y: IMAGE_Y },
        size: { width: IMAGE_W, height: IMAGE_H },
        meta: { templateKind: "presentation", semanticType: "slide-image", artifactId: null }
      },
      {
        localId: "s2-notes",
        frameLocalId: "slide-2",
        title: "Speaker notes",
        summary: "What to say for slide 2…",
        styleKey: "speaker-note",
        position: { x: SLIDE_WIDTH + SLIDE_GAP + NOTE_X, y: NOTE_Y },
        size: { width: NOTE_W, height: NOTE_H },
        meta: { templateKind: "presentation", semanticType: "speaker-note" }
      }
    ],

    // Optional slide-to-slide flow connector.
    edges: [
      {
        localId: "flow-1-2",
        frameLocalId: "deck",
        sourceLocalId: "s1-title",
        targetLocalId: "s2-title",
        label: "then",
        styleKey: "slide-flow",
        meta: { templateKind: "presentation", semanticType: "slide-flow" }
      }
    ]
  },

  layout: {
    origin: { x: 0, y: 0 },
    defaultShapeSize: { width: TITLE_W, height: TITLE_H }
  },

  exports: {
    allowed: ["design_doc_md", "image_prompt"],
    default: "design_doc_md"
  },

  tags: {
    suggested: [
      { localId: "tag-draft-slide", name: "draft-slide", color: "#f5a623", description: "Slide still in draft" },
      { localId: "tag-needs-image", name: "needs-image", color: "#7ed321", description: "Slide needs an image" },
      { localId: "tag-final", name: "final", color: "#4a9eff", description: "Slide is final" }
    ]
  },

  promptHints: {
    systemHint:
      "This is a slide deck. Each child frame is a slide ordered by meta.slideIndex; " +
      "title/body are text cards (styleKey slide-title/slide-body), speaker-note cards hold narration, " +
      "image cards reference artifacts via meta.artifactId.",
    fieldHints: {
      "slide-title": "One short headline.",
      "slide-body": "2–5 short bullets.",
      "speaker-note": "Narration, not shown on the slide.",
      "slide-image": "Caption for the image; set meta.artifactId to a SceneArtifact id."
    },
    suggestedOperations: ["create", "edit text", "connect", "tag"]
  }
};

// ---------------------------------------------------------------------------
// §3  Presentation style tokens
// ---------------------------------------------------------------------------

/**
 * Five additive SceneStyleToken entries for presentation cards.
 * Appended to defaultStyles in renderScene.ts (additive, no schema change).
 * Until appended, cards fall back gracefully to the "default" token.
 */
export const presentationStyleTokens: SceneStyleToken[] = [
  {
    id: "slide-title",
    fill: "#ffffff",
    stroke: "#2f7ee6",
    text: "#0d1f33",
    mutedText: "#5a7188",
    accent: "#2f7ee6",
    surface: "#f7fbff",
    pastel: "#ebf4ff"
  },
  {
    id: "slide-body",
    fill: "#fafcff",
    stroke: "#7b8794",
    text: "#172026",
    mutedText: "#65717b",
    accent: "#158f83",
    surface: "#f7f9fb",
    pastel: "#f0f7ff"
  },
  {
    id: "slide-image",
    fill: "#f4f8f4",
    stroke: "#26965e",
    text: "#10251a",
    mutedText: "#5b7464",
    accent: "#26965e",
    surface: "#f4fbf6",
    pastel: "#eaf9ef"
  },
  {
    id: "speaker-note",
    fill: "#fffdf0",
    stroke: "#c67914",
    text: "#2a1b0b",
    mutedText: "#80684c",
    accent: "#c67914",
    surface: "#fdf2de",
    pastel: "#fdf2de"
  },
  {
    id: "slide-flow",
    fill: "#ffffff",
    stroke: "#7a68ce",
    text: "#1d1833",
    mutedText: "#675f85",
    accent: "#7a68ce",
    surface: "#f8f7ff",
    pastel: "#f1effd"
  }
];

// ---------------------------------------------------------------------------
// §4  Export outline reader (design_doc_md presentation branch)
// ---------------------------------------------------------------------------

/**
 * Slide object extracted from a flat scene primitive set.
 * All fields are derived from SceneGroup / SceneNode — no separate slide model.
 */
type SlideData = {
  slideIndex: number;
  title: string;
  bodyBullets: string[];
  speakerNotes: string[];
};

/**
 * Read deck-produced primitives and emit a slide-ordered Markdown outline.
 *
 * This is the additive "presentation branch" described in T4.5 §8 option A:
 * called from generateLocalExport when exportType === "design_doc_md" and the
 * group carries meta.templateKind === "presentation".
 *
 * @param deckGroup  The top-level deck SceneGroup.
 * @param allGroups  All SceneGroups in the scene (to find child slides).
 * @param allNodes   All SceneNodes in the scene (to find per-slide cards).
 * @returns Markdown string — heading per slide, bullets, and speaker-notes block.
 */
export function presentationOutline(
  deckGroup: SceneGroup,
  allGroups: SceneGroup[],
  allNodes: SceneNode[]
): string {
  // Collect child slide frames — direct children of the deck ordered by meta.slideIndex.
  const slideFrames = allGroups
    .filter(
      (g) =>
        g.parentGroupId === deckGroup.id &&
        (g.meta as Record<string, unknown> | undefined)?.semanticType === "slide"
    )
    .sort((a, b) => {
      const ai = ((a.meta as Record<string, unknown> | undefined)?.slideIndex as number) ?? 0;
      const bi = ((b.meta as Record<string, unknown> | undefined)?.slideIndex as number) ?? 0;
      return ai - bi;
    });

  // For each slide frame, gather title/body/note cards by semanticType.
  const slides: SlideData[] = slideFrames.map((frame) => {
    const members = allNodes.filter((n) => n.groupId === frame.id);
    const titleCard = members.find(
      (n) => (n.meta as Record<string, unknown> | undefined)?.semanticType === "slide-title"
    );
    const bodyCards = members.filter(
      (n) => (n.meta as Record<string, unknown> | undefined)?.semanticType === "slide-body"
    );
    const noteCards = members.filter(
      (n) => (n.meta as Record<string, unknown> | undefined)?.semanticType === "speaker-note"
    );

    const slideTitle = titleCard?.title || frame.title || "Untitled slide";

    const bodyBullets: string[] = bodyCards.flatMap((card) =>
      (card.summary || "")
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean)
    );

    const speakerNotes: string[] = noteCards.flatMap((card) =>
      (card.summary || "")
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean)
    );

    return {
      slideIndex: ((frame.meta as Record<string, unknown> | undefined)?.slideIndex as number) ?? 0,
      title: slideTitle,
      bodyBullets,
      speakerNotes
    };
  });

  // Render Markdown outline.
  const lines: string[] = [`# ${deckGroup.title || "Presentation"}`, ""];
  for (const slide of slides) {
    lines.push(`## ${slide.title}`);
    if (slide.bodyBullets.length > 0) {
      for (const bullet of slide.bodyBullets) {
        lines.push(`- ${bullet}`);
      }
    }
    if (slide.speakerNotes.length > 0) {
      lines.push("");
      lines.push("> **Speaker notes:** " + slide.speakerNotes.join(" / "));
    }
    lines.push("");
  }

  return lines.join("\n").trimEnd();
}

// ---------------------------------------------------------------------------
// §5  Per-template meta validation helper (T4.5 §Open-questions D1 C4)
// ---------------------------------------------------------------------------

/** Valid semantic types in the presentation meta namespace. */
export type PresentationSemanticType =
  | "deck"
  | "slide"
  | "slide-title"
  | "slide-body"
  | "slide-image"
  | "speaker-note"
  | "slide-flow";

const VALID_SEMANTIC_TYPES = new Set<string>([
  "deck",
  "slide",
  "slide-title",
  "slide-body",
  "slide-image",
  "speaker-note",
  "slide-flow"
]);

/**
 * Validate meta values on a presentation-template object at authoring time.
 * Returns an array of error strings (empty = valid).
 * Never gates the renderer — only advisory at apply/lint time.
 */
export function validatePresentationMeta(meta: Record<string, unknown>): string[] {
  const errors: string[] = [];
  if (meta.templateKind !== "presentation") {
    errors.push(`meta.templateKind must be "presentation", got ${String(meta.templateKind)}`);
  }
  if ("semanticType" in meta && !VALID_SEMANTIC_TYPES.has(meta.semanticType as string)) {
    errors.push(`meta.semanticType "${String(meta.semanticType)}" is not a valid presentation semantic type`);
  }
  if ("slideIndex" in meta && !Number.isInteger(meta.slideIndex)) {
    errors.push(`meta.slideIndex must be an integer, got ${String(meta.slideIndex)}`);
  }
  return errors;
}
