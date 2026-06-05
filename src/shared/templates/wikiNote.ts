/**
 * T4.3 — Wiki Note Cluster & Idea Board Templates
 *
 * Two TemplateContract instances (wiki-note, idea-board) that compose entirely
 * from existing primitives via the T4.1 applyTemplate lowering pipeline.
 * No new schema fields, op kinds, style tokens, or export presets.
 *
 * Source/evidence cards: RecipeShape with styleKey:"evidence", citation in
 * evidenceRefs[] (business-layer field, not in SceneSnapshot).
 * Reference edges: RecipeEdge with styleKey:"supports" (citation) or default
 * (associative), label editable via T2.3 edit-text.
 *
 * §7 Verify: spatial reorg (move-card / move-group / regroup) touches only
 * geometry; text/tags/comments/source refs key on stable object id — unchanged.
 */

import type { TemplateContract } from "./contract";

// ---------------------------------------------------------------------------
// Wiki Note Cluster  (templateKind: "wiki-note")
// ---------------------------------------------------------------------------

export const wikiNoteTemplate: TemplateContract = {
  metadata: {
    id: "wiki-note",
    title: "Wiki Note Cluster",
    description: "Linked prose notes with sources and references.",
    category: "knowledge",
    templateKind: "wiki-note"
  },

  recipe: {
    frames: [
      {
        localId: "f-cluster",
        title: "Untitled Wiki",
        summary: "A linked cluster of notes with sources and references.",
        meta: { semanticType: "wiki-cluster" }
      },
      {
        localId: "f-section-a",
        title: "Section",
        parentLocalId: "f-cluster",
        meta: { semanticType: "wiki-section" }
      }
    ],

    shapes: [
      // Overview note — lives directly in the root cluster frame.
      {
        localId: "n-overview",
        frameLocalId: "f-cluster",
        styleKey: "proposition",
        title: "Overview",
        summary: "Introduce the topic here.",
        detail: "",
        position: { x: 40, y: 40 },
        size: { width: 390, height: 390 },
        tagLocalIds: ["t-topic"],
        meta: { semanticType: "note" }
      },
      // First note in section A.
      {
        localId: "n-note-1",
        frameLocalId: "f-section-a",
        styleKey: "proposition",
        title: "Note 1",
        summary: "",
        detail: "",
        position: { x: 40, y: 40 },
        size: { width: 390, height: 390 },
        tagLocalIds: ["t-draft"],
        meta: { semanticType: "note" }
      },
      // Second note in section A.
      {
        localId: "n-note-2",
        frameLocalId: "f-section-a",
        styleKey: "proposition",
        title: "Note 2",
        summary: "",
        detail: "",
        position: { x: 470, y: 40 },
        size: { width: 390, height: 390 },
        tagLocalIds: ["t-draft"],
        meta: { semanticType: "note" }
      },
      // Source / evidence card — citation lives in evidenceRefs[] (business layer).
      // styleKey:"evidence" reuses the existing evidence token in defaultStyles.
      {
        localId: "n-source-1",
        frameLocalId: "f-cluster",
        styleKey: "evidence",
        title: "Source Title",
        summary: "Citation summary",
        detail: "Quoted excerpt or abstract.",
        position: { x: 470, y: 40 },
        size: { width: 390, height: 220 },
        tagLocalIds: ["t-source"],
        meta: {
          semanticType: "source",
          citationKind: "url",
          // evidenceRefs is a SceneNode business-layer field (not in SceneSnapshot).
          // Callers should set node.evidenceRefs = ["https://…"] after applyTemplate.
          evidenceRefsHint: ["https://example.com"]
        }
      }
    ],

    edges: [
      // Reference edge: n-note-1 → n-note-2 ("see also")
      {
        localId: "e-ref-1",
        frameLocalId: "f-cluster",
        sourceLocalId: "n-note-1",
        targetLocalId: "n-note-2",
        label: "see also",
        styleKey: "supports",
        meta: { semanticType: "reference" }
      },
      // Citation edge: n-note-1 → n-source-1
      {
        localId: "e-cite-1",
        frameLocalId: "f-cluster",
        sourceLocalId: "n-note-1",
        targetLocalId: "n-source-1",
        label: "cited by",
        styleKey: "supports",
        meta: { semanticType: "citation" }
      }
    ]
  },

  layout: {
    origin: { x: 0, y: 0 },
    defaultShapeSize: { width: 390, height: 390 }
  },

  exports: {
    allowed: ["design_doc_md", "confluence_html", "madr", "mermaid"],
    default: "design_doc_md"
  },

  tags: {
    suggested: [
      { localId: "t-topic", name: "topic", color: "#5B8DEF" },
      { localId: "t-source", name: "source", color: "#8E8E93" },
      { localId: "t-draft", name: "draft", color: "#E0A458" }
    ]
  },

  promptHints: {
    systemHint:
      "This is a wiki note cluster; cards are notes, evidence cards carry sources in evidenceRefs, edges are references.",
    fieldHints: {
      note: "Heading in title, lede in summary, body in detail",
      source: "Put the citation URL/path in evidenceRefs; quote in detail"
    },
    suggestedOperations: ["create", "connect", "tag", "comment"]
  }
};

// ---------------------------------------------------------------------------
// Idea Board  (templateKind: "idea-board")
// ---------------------------------------------------------------------------

export const ideaBoardTemplate: TemplateContract = {
  metadata: {
    id: "idea-board",
    title: "Idea Board",
    description: "Freeform ideation cards grouped into themes.",
    category: "knowledge",
    templateKind: "idea-board"
  },

  recipe: {
    frames: [
      {
        localId: "f-board",
        title: "Idea Board",
        meta: { semanticType: "idea-board" }
      },
      {
        localId: "f-theme-1",
        title: "Theme",
        parentLocalId: "f-board",
        meta: { semanticType: "idea-theme" }
      }
    ],

    shapes: [
      // Idea card in the root board (free-floating, no theme yet).
      {
        localId: "n-idea-1",
        frameLocalId: "f-board",
        styleKey: "option",
        title: "Idea 1",
        summary: "Describe the idea.",
        detail: "",
        position: { x: 40, y: 40 },
        size: { width: 390, height: 390 },
        tagLocalIds: ["t-spark"],
        meta: { semanticType: "idea" }
      },
      // Idea card inside theme sub-frame.
      {
        localId: "n-idea-2",
        frameLocalId: "f-theme-1",
        styleKey: "option",
        title: "Idea 2",
        summary: "Describe the idea.",
        detail: "",
        position: { x: 40, y: 40 },
        size: { width: 390, height: 390 },
        tagLocalIds: ["t-theme"],
        meta: { semanticType: "idea" }
      },
      // Idea card inside theme sub-frame.
      {
        localId: "n-idea-3",
        frameLocalId: "f-theme-1",
        styleKey: "option",
        title: "Idea 3",
        summary: "Describe the idea.",
        detail: "",
        position: { x: 470, y: 40 },
        size: { width: 390, height: 390 },
        tagLocalIds: ["t-theme"],
        meta: { semanticType: "idea" }
      },
      // Optional source / evidence card (same recipe as §4).
      {
        localId: "n-source-1",
        frameLocalId: "f-board",
        styleKey: "evidence",
        title: "Source Title",
        summary: "Citation summary",
        detail: "Quoted excerpt.",
        position: { x: 470, y: 40 },
        size: { width: 390, height: 220 },
        tagLocalIds: ["t-source"],
        meta: {
          semanticType: "source",
          citationKind: "url",
          evidenceRefsHint: ["https://example.com"]
        }
      }
    ],

    edges: [
      // Associative "relates to" edge between two idea cards.
      {
        localId: "e-rel-1",
        frameLocalId: "f-board",
        sourceLocalId: "n-idea-1",
        targetLocalId: "n-idea-2",
        label: "relates to",
        meta: { semanticType: "relates-to" }
      }
    ]
  },

  layout: {
    origin: { x: 0, y: 0 },
    defaultShapeSize: { width: 390, height: 390 }
  },

  exports: {
    allowed: ["design_doc_md", "mermaid", "image_prompt"],
    default: "design_doc_md"
  },

  tags: {
    suggested: [
      { localId: "t-theme", name: "theme", color: "#5B8DEF" },
      { localId: "t-spark", name: "spark", color: "#E0A458" },
      { localId: "t-parked", name: "parked", color: "#8E8E93" },
      { localId: "t-source", name: "source", color: "#636366" }
    ]
  },

  promptHints: {
    systemHint:
      "This is an idea board; cards are ideas grouped into themes, evidence cards carry sources, edges are associative links.",
    fieldHints: {
      idea: "Idea title in title, note in summary, elaboration in detail",
      source: "Put the citation URL/path in evidenceRefs; quote in detail"
    },
    suggestedOperations: ["create", "connect", "tag", "comment"]
  }
};
