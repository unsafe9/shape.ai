/**
 * T4.4 — ADR & Architecture Diagram Templates
 *
 * Five engineering TemplateContracts — ADR, decision map, server architecture,
 * dependency diagram, investigation map — as pure primitive recipes over the
 * existing op funnel (create-group / create-card / create-edge / tags).
 *
 * Engineering semantics live in template metadata, meta.semanticType, labels,
 * comments, and export rules. No structured-ADR core component; no new op or
 * object kind. After apply the scene holds only ordinary primitives.
 *
 * Spec: docs/design/ai-companion-canvas/tasks/T4.4.md
 * Contract shape: src/shared/templates/contract.ts (T4.1)
 */

import type { TemplateContract } from "./contract";

// ---------------------------------------------------------------------------
// ADR template (T4.4 §2)
// Mechanically transcribed from seedGroupScene (local.ts).
// ---------------------------------------------------------------------------

export const ADR_TEMPLATE: TemplateContract = {
  metadata: {
    id: "adr",
    title: "ADR / Design Decision",
    description: "Architecture Decision Record — full ADR scaffold with proposition, options, evidence, tradeoffs, blockers, and derived artifacts.",
    category: "engineering",
    templateKind: "adr"
  },
  recipe: {
    frames: [
      {
        localId: "root",
        title: "ADR",
        summary: "Architecture Decision Record",
        meta: { templateKind: "adr" }
      }
    ],
    shapes: [
      {
        localId: "proposition",
        frameLocalId: "root",
        title: "Proposition",
        summary: "Problem statement and target outcome.",
        detail: "Problem statement and target outcome.",
        styleKey: "proposition",
        position: { x: 0, y: 300 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "proposition", status: "selected" }
      },
      {
        localId: "decision-points",
        frameLocalId: "root",
        title: "Decision points",
        summary: "The choice should be driven by feasibility, evidence strength, reversibility, and agent permissions.",
        detail: "The choice should be driven by feasibility, evidence strength, reversibility, and agent permissions.",
        styleKey: "decision_point",
        position: { x: 360, y: 300 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "decision_point", status: "draft" }
      },
      {
        localId: "option-graph",
        frameLocalId: "root",
        title: "Typed decision graph",
        summary: "Use a typed graph as the source of truth for discussion and exports.",
        detail: "Use a typed graph as the source of truth for discussion and exports.",
        styleKey: "option",
        position: { x: 720, y: 120 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "option", status: "viable" }
      },
      {
        localId: "option-freeform",
        frameLocalId: "root",
        title: "Freeform mindmap",
        summary: "Flexible, but weak at enforcing architectural decision quality.",
        detail: "Flexible, but weak at enforcing architectural decision quality.",
        styleKey: "option",
        position: { x: 720, y: 480 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "option", status: "conditional" }
      },
      {
        localId: "evidence",
        frameLocalId: "root",
        title: "Evidence ledger",
        summary: "Every recommendation should trace to a concrete assumption, source, or probe.",
        detail: "Every recommendation should trace to a concrete assumption, source, or probe.",
        styleKey: "evidence",
        position: { x: 1080, y: 40 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "evidence", status: "draft" }
      },
      {
        localId: "tradeoff",
        frameLocalId: "root",
        title: "Readable vs complete",
        summary: "Show compact nodes by default and move deep rationale into the inspector.",
        detail: "Show compact nodes by default and move deep rationale into the inspector.",
        styleKey: "tradeoff",
        position: { x: 1080, y: 300 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "tradeoff", status: "draft" }
      },
      {
        localId: "blocker",
        frameLocalId: "root",
        title: "Unbounded local permissions",
        summary: "Shell execution and code editing are out of MVP scope unless a future approval boundary is added.",
        detail: "Shell execution and code editing are out of MVP scope unless a future approval boundary is added.",
        styleKey: "blocker",
        position: { x: 1080, y: 560 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "blocker", status: "infeasible" }
      },
      {
        localId: "subdecision",
        frameLocalId: "root",
        title: "Export scope",
        summary: "Exports must work for the whole graph and selected subgraphs.",
        detail: "Exports must work for the whole graph and selected subgraphs.",
        styleKey: "subdecision",
        position: { x: 1440, y: 120 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "subdecision", status: "draft" }
      },
      {
        localId: "task",
        frameLocalId: "root",
        title: "First vertical slice",
        summary: "Create a group, inspect a node, leave comments, and export Markdown.",
        detail: "Create a group, inspect a node, leave comments, and export Markdown.",
        styleKey: "task",
        position: { x: 1440, y: 380 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "task", status: "draft" }
      },
      {
        localId: "artifact",
        frameLocalId: "root",
        title: "Derived artifacts",
        summary: "MADR, YADR, Mermaid, and image-generation prompts.",
        detail: "MADR, YADR, Mermaid, and image-generation prompts.",
        styleKey: "artifact",
        position: { x: 1440, y: 640 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "artifact", status: "draft" }
      }
    ],
    edges: [
      {
        localId: "e1",
        frameLocalId: "root",
        sourceLocalId: "proposition",
        targetLocalId: "decision-points",
        label: "decide by",
        styleKey: "decomposes_to",
        meta: { semanticType: "decomposes_to" }
      },
      {
        localId: "e2",
        frameLocalId: "root",
        sourceLocalId: "decision-points",
        targetLocalId: "option-graph",
        label: "recommended",
        styleKey: "chooses_between",
        meta: { semanticType: "chooses_between" }
      },
      {
        localId: "e3",
        frameLocalId: "root",
        sourceLocalId: "decision-points",
        targetLocalId: "option-freeform",
        label: "alternative",
        styleKey: "chooses_between",
        meta: { semanticType: "chooses_between" }
      },
      {
        localId: "e4",
        frameLocalId: "root",
        sourceLocalId: "evidence",
        targetLocalId: "option-graph",
        label: "supports",
        styleKey: "supports",
        meta: { semanticType: "supports" }
      },
      {
        localId: "e5",
        frameLocalId: "root",
        sourceLocalId: "option-graph",
        targetLocalId: "tradeoff",
        label: "accepts",
        styleKey: "trades_off_with",
        meta: { semanticType: "trades_off_with" }
      },
      {
        localId: "e6",
        frameLocalId: "root",
        sourceLocalId: "blocker",
        targetLocalId: "option-freeform",
        label: "weakens",
        styleKey: "blocks",
        meta: { semanticType: "blocks" }
      },
      {
        localId: "e7",
        frameLocalId: "root",
        sourceLocalId: "option-graph",
        targetLocalId: "subdecision",
        label: "needs",
        styleKey: "decomposes_to",
        meta: { semanticType: "decomposes_to" }
      },
      {
        localId: "e8",
        frameLocalId: "root",
        sourceLocalId: "task",
        targetLocalId: "option-graph",
        label: "builds on",
        styleKey: "depends_on",
        meta: { semanticType: "depends_on" }
      },
      {
        localId: "e9",
        frameLocalId: "root",
        sourceLocalId: "subdecision",
        targetLocalId: "artifact",
        label: "exports",
        styleKey: "produces",
        meta: { semanticType: "produces" }
      }
    ]
  },
  layout: {
    origin: { x: 0, y: 0 },
    defaultShapeSize: { width: 270, height: 178 }
  },
  exports: {
    allowed: ["madr", "yadr", "design_doc_md", "confluence_html", "mermaid"],
    default: "madr"
  },
  tags: {
    suggested: [
      { localId: "tag-viable", name: "viable", color: "#22c55e", description: "Option is viable" },
      { localId: "tag-conditional", name: "conditional", color: "#f59e0b", description: "Option is conditional" },
      { localId: "tag-infeasible", name: "infeasible", color: "#ef4444", description: "Option is infeasible" },
      { localId: "tag-selected", name: "selected", color: "#6366f1", description: "Option is selected" }
    ]
  },
  promptHints: {
    systemHint: "An ADR (Architecture Decision Record): a proposition, decision points, competing options, evidence, tradeoffs, blockers, subdecisions, tasks, and derived artifacts. The choice should be driven by feasibility, evidence strength, reversibility, and agent permissions.",
    fieldHints: {
      proposition: "State the problem and target outcome clearly.",
      decision_point: "Name the key decision drivers that guide option selection.",
      option: "Describe each option with enough detail to evaluate feasibility and tradeoffs.",
      evidence: "Cite concrete assumptions, sources, or probes that support an option.",
      tradeoff: "State what is accepted or sacrificed by choosing an option.",
      blocker: "Identify hard constraints that rule out an option.",
      subdecision: "Decompose the main decision into scoped sub-decisions.",
      task: "List the first implementation steps once an option is selected.",
      artifact: "Name the derived export artifacts (MADR, YADR, Mermaid, image prompt)."
    }
  }
};

// ---------------------------------------------------------------------------
// Decision map template (T4.4 §3)
// Lighter decision-graph surface: decision/options/subdecisions spine only.
// ---------------------------------------------------------------------------

export const DECISION_MAP_TEMPLATE: TemplateContract = {
  metadata: {
    id: "decision-map",
    title: "Decision Map",
    description: "Lightweight decision map — one decision point, competing options, optional sub-decisions.",
    category: "engineering",
    templateKind: "decision-map"
  },
  recipe: {
    frames: [
      {
        localId: "root",
        title: "Decision Map",
        summary: "Decision spine: decision point, options, sub-decisions.",
        meta: { templateKind: "decision-map" }
      }
    ],
    shapes: [
      {
        localId: "decision",
        frameLocalId: "root",
        title: "Decision point",
        summary: "What needs to be decided?",
        detail: "What needs to be decided?",
        styleKey: "decision_point",
        position: { x: 0, y: 200 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "decision_point", status: "draft" }
      },
      {
        localId: "option-a",
        frameLocalId: "root",
        title: "Option A",
        summary: "First competing option.",
        detail: "First competing option.",
        styleKey: "option",
        position: { x: 360, y: 80 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "option", status: "viable" }
      },
      {
        localId: "option-b",
        frameLocalId: "root",
        title: "Option B",
        summary: "Second competing option.",
        detail: "Second competing option.",
        styleKey: "option",
        position: { x: 360, y: 320 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "option", status: "conditional" }
      },
      {
        localId: "subdecision",
        frameLocalId: "root",
        title: "Sub-decision",
        summary: "A decision that depends on the chosen option.",
        detail: "A decision that depends on the chosen option.",
        styleKey: "subdecision",
        position: { x: 720, y: 80 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "subdecision", status: "draft" }
      }
    ],
    edges: [
      {
        localId: "e1",
        frameLocalId: "root",
        sourceLocalId: "decision",
        targetLocalId: "option-a",
        label: "considers",
        styleKey: "chooses_between",
        meta: { semanticType: "chooses_between" }
      },
      {
        localId: "e2",
        frameLocalId: "root",
        sourceLocalId: "decision",
        targetLocalId: "option-b",
        label: "considers",
        styleKey: "chooses_between",
        meta: { semanticType: "chooses_between" }
      },
      {
        localId: "e3",
        frameLocalId: "root",
        sourceLocalId: "option-a",
        targetLocalId: "subdecision",
        label: "leads to",
        styleKey: "decomposes_to",
        meta: { semanticType: "decomposes_to" }
      }
    ]
  },
  layout: {
    origin: { x: 0, y: 0 },
    defaultShapeSize: { width: 270, height: 178 }
  },
  exports: {
    allowed: ["mermaid", "madr", "design_doc_md"],
    default: "mermaid"
  },
  tags: {
    suggested: [
      { localId: "tag-viable", name: "viable", color: "#22c55e", description: "Option is viable" },
      { localId: "tag-conditional", name: "conditional", color: "#f59e0b", description: "Option is conditional" }
    ]
  },
  promptHints: {
    systemHint: "A decision map: one decision point, competing options, optional sub-decisions. Edges show choice and decomposition."
  }
};

// ---------------------------------------------------------------------------
// Server architecture diagram template (T4.4 §4)
// Box-and-arrow component diagram with deployment zone frames.
// ---------------------------------------------------------------------------

export const SERVER_ARCHITECTURE_TEMPLATE: TemplateContract = {
  metadata: {
    id: "server-architecture",
    title: "Server Architecture",
    description: "Server architecture diagram — components as boxes, deployment zones as frames, dependencies as labeled edges.",
    category: "engineering",
    templateKind: "server-architecture"
  },
  recipe: {
    frames: [
      {
        localId: "root",
        title: "Server Architecture",
        summary: "Component diagram with deployment zones.",
        meta: { templateKind: "server-architecture" }
      },
      {
        localId: "zone-edge",
        parentLocalId: "root",
        title: "Edge",
        summary: "Edge / ingress tier — load balancers, API gateways, CDN.",
        meta: { semanticType: "zone" }
      },
      {
        localId: "zone-service",
        parentLocalId: "root",
        title: "Service",
        summary: "Service tier — application servers, workers.",
        meta: { semanticType: "zone" }
      },
      {
        localId: "zone-data",
        parentLocalId: "root",
        title: "Data",
        summary: "Data tier — databases, caches, message queues.",
        meta: { semanticType: "zone" }
      }
    ],
    shapes: [
      {
        localId: "component-gateway",
        frameLocalId: "zone-edge",
        title: "API Gateway",
        summary: "Entry point for all inbound requests.",
        detail: "Entry point for all inbound requests.",
        styleKey: "artifact",
        position: { x: 0, y: 0 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "component" }
      },
      {
        localId: "component-server",
        frameLocalId: "zone-service",
        title: "App Server",
        summary: "Core application logic and request handling.",
        detail: "Core application logic and request handling.",
        styleKey: "artifact",
        position: { x: 400, y: 0 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "component" }
      },
      {
        localId: "component-worker",
        frameLocalId: "zone-service",
        title: "Worker",
        summary: "Background job processing.",
        detail: "Background job processing.",
        styleKey: "artifact",
        position: { x: 400, y: 260 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "component" }
      },
      {
        localId: "component-db",
        frameLocalId: "zone-data",
        title: "Database",
        summary: "Primary persistent store.",
        detail: "Primary persistent store.",
        styleKey: "artifact",
        position: { x: 800, y: 0 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "component" }
      },
      {
        localId: "component-cache",
        frameLocalId: "zone-data",
        title: "Cache",
        summary: "In-memory cache for hot reads.",
        detail: "In-memory cache for hot reads.",
        styleKey: "artifact",
        position: { x: 800, y: 260 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "component" }
      }
    ],
    edges: [
      {
        localId: "e1",
        frameLocalId: "root",
        sourceLocalId: "component-gateway",
        targetLocalId: "component-server",
        label: "HTTP/REST",
        styleKey: "depends_on",
        meta: { semanticType: "depends_on" }
      },
      {
        localId: "e2",
        frameLocalId: "root",
        sourceLocalId: "component-server",
        targetLocalId: "component-db",
        label: "SQL",
        styleKey: "depends_on",
        meta: { semanticType: "depends_on" }
      },
      {
        localId: "e3",
        frameLocalId: "root",
        sourceLocalId: "component-server",
        targetLocalId: "component-cache",
        label: "Redis",
        styleKey: "depends_on",
        meta: { semanticType: "depends_on" }
      },
      {
        localId: "e4",
        frameLocalId: "root",
        sourceLocalId: "component-server",
        targetLocalId: "component-worker",
        label: "enqueue",
        styleKey: "produces",
        meta: { semanticType: "produces" }
      },
      {
        localId: "e5",
        frameLocalId: "root",
        sourceLocalId: "component-worker",
        targetLocalId: "component-db",
        label: "writes",
        styleKey: "produces",
        meta: { semanticType: "produces" }
      }
    ]
  },
  layout: {
    origin: { x: 0, y: 0 },
    defaultShapeSize: { width: 270, height: 178 }
  },
  exports: {
    allowed: ["mermaid", "architecture_image", "image_prompt"],
    default: "mermaid"
  },
  tags: {
    suggested: [
      { localId: "tag-edge", name: "Edge", color: "#6366f1", description: "Edge / ingress tier" },
      { localId: "tag-service", name: "Service", color: "#22c55e", description: "Service tier" },
      { localId: "tag-data", name: "Data", color: "#f59e0b", description: "Data tier" }
    ]
  },
  promptHints: {
    systemHint: "A server architecture diagram: boxes are components, frames are deployment zones/tiers, edges are dependencies labeled with the protocol or call."
  }
};

// ---------------------------------------------------------------------------
// Dependency diagram template (T4.4 §5)
// Units + dependency edges; no zone frames required.
// ---------------------------------------------------------------------------

export const DEPENDENCY_DIAGRAM_TEMPLATE: TemplateContract = {
  metadata: {
    id: "dependency-diagram",
    title: "Dependency Diagram",
    description: "Dependency diagram — units (modules, services, packages) with depends-on and blocking edges.",
    category: "engineering",
    templateKind: "dependency-diagram"
  },
  recipe: {
    frames: [
      {
        localId: "root",
        title: "Dependency Diagram",
        summary: "Module/service dependency graph.",
        meta: { templateKind: "dependency-diagram" }
      }
    ],
    shapes: [
      {
        localId: "unit-a",
        frameLocalId: "root",
        title: "Unit A",
        summary: "First unit (module, service, or package).",
        detail: "First unit (module, service, or package).",
        styleKey: "artifact",
        position: { x: 0, y: 0 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "unit" }
      },
      {
        localId: "unit-b",
        frameLocalId: "root",
        title: "Unit B",
        summary: "Second unit.",
        detail: "Second unit.",
        styleKey: "artifact",
        position: { x: 400, y: 0 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "unit" }
      },
      {
        localId: "unit-c",
        frameLocalId: "root",
        title: "Unit C",
        summary: "Third unit.",
        detail: "Third unit.",
        styleKey: "artifact",
        position: { x: 800, y: 0 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "unit" }
      },
      {
        localId: "unit-d",
        frameLocalId: "root",
        title: "Unit D",
        summary: "Fourth unit — may block C.",
        detail: "Fourth unit — may block C.",
        styleKey: "task",
        position: { x: 800, y: 260 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "unit" }
      }
    ],
    edges: [
      {
        localId: "e1",
        frameLocalId: "root",
        sourceLocalId: "unit-a",
        targetLocalId: "unit-b",
        label: "depends on",
        styleKey: "depends_on",
        meta: { semanticType: "depends_on" }
      },
      {
        localId: "e2",
        frameLocalId: "root",
        sourceLocalId: "unit-b",
        targetLocalId: "unit-c",
        label: "depends on",
        styleKey: "depends_on",
        meta: { semanticType: "depends_on" }
      },
      {
        localId: "e3",
        frameLocalId: "root",
        sourceLocalId: "unit-d",
        targetLocalId: "unit-c",
        label: "blocks",
        styleKey: "blocks",
        meta: { semanticType: "blocks" }
      }
    ]
  },
  layout: {
    origin: { x: 0, y: 0 },
    defaultShapeSize: { width: 270, height: 178 }
  },
  exports: {
    allowed: ["mermaid", "architecture_image"],
    default: "mermaid"
  },
  tags: {
    suggested: [
      { localId: "tag-blocking", name: "blocking", color: "#ef4444", description: "Blocking dependency" },
      { localId: "tag-external", name: "external", color: "#6366f1", description: "External dependency" }
    ]
  },
  promptHints: {
    systemHint: "A dependency diagram: boxes are units (modules/services/packages), edges are 'depends on'; mark blocking dependencies with a blocks edge."
  }
};

// ---------------------------------------------------------------------------
// Investigation map template (T4.4 §6 / D1 C2)
// Question → evidence → hypotheses → findings → next steps.
// Discharges D1 condition C2: explicit primitive composition, reusing only
// existing primitives and ops, with zero new core/renderer work.
// ---------------------------------------------------------------------------

export const INVESTIGATION_MAP_TEMPLATE: TemplateContract = {
  metadata: {
    id: "investigation-map",
    title: "Investigation Map",
    description: "Investigation map — a question, evidence cards, competing hypotheses, findings, and next steps.",
    category: "engineering",
    templateKind: "investigation-map"
  },
  recipe: {
    frames: [
      {
        localId: "root",
        title: "Investigation Map",
        summary: "Question → evidence → hypotheses → findings → next steps.",
        meta: { templateKind: "investigation-map" }
      }
    ],
    shapes: [
      {
        localId: "question",
        frameLocalId: "root",
        title: "Investigation question",
        summary: "What are we trying to find out?",
        detail: "What are we trying to find out?",
        styleKey: "proposition",
        position: { x: 0, y: 260 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "proposition", status: "draft" }
      },
      {
        localId: "evidence-1",
        frameLocalId: "root",
        title: "Evidence 1",
        summary: "A concrete observation, log entry, or data point.",
        detail: "A concrete observation, log entry, or data point.",
        styleKey: "evidence",
        position: { x: 360, y: 40 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "evidence", status: "draft" }
      },
      {
        localId: "evidence-2",
        frameLocalId: "root",
        title: "Evidence 2",
        summary: "A second observation or data point.",
        detail: "A second observation or data point.",
        styleKey: "evidence",
        position: { x: 360, y: 300 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "evidence", status: "draft" }
      },
      {
        localId: "hypothesis-a",
        frameLocalId: "root",
        title: "Hypothesis A",
        summary: "First competing explanation.",
        detail: "First competing explanation.",
        styleKey: "option",
        position: { x: 720, y: 40 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "option", status: "viable" }
      },
      {
        localId: "hypothesis-b",
        frameLocalId: "root",
        title: "Hypothesis B",
        summary: "Second competing explanation.",
        detail: "Second competing explanation.",
        styleKey: "option",
        position: { x: 720, y: 300 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "option", status: "conditional" }
      },
      {
        localId: "finding",
        frameLocalId: "root",
        title: "Finding",
        summary: "Concluded finding from evidence and hypotheses.",
        detail: "Concluded finding from evidence and hypotheses.",
        styleKey: "decision_point",
        position: { x: 1080, y: 160 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "decision_point", status: "draft" }
      },
      {
        localId: "next-step",
        frameLocalId: "root",
        title: "Next step",
        summary: "Action item driven by the finding.",
        detail: "Action item driven by the finding.",
        styleKey: "task",
        position: { x: 1440, y: 160 },
        size: { width: 270, height: 178 },
        meta: { semanticType: "task", status: "draft" }
      }
    ],
    edges: [
      {
        localId: "e1",
        frameLocalId: "root",
        sourceLocalId: "question",
        targetLocalId: "hypothesis-a",
        label: "considers",
        styleKey: "chooses_between",
        meta: { semanticType: "chooses_between" }
      },
      {
        localId: "e2",
        frameLocalId: "root",
        sourceLocalId: "question",
        targetLocalId: "hypothesis-b",
        label: "considers",
        styleKey: "chooses_between",
        meta: { semanticType: "chooses_between" }
      },
      {
        localId: "e3",
        frameLocalId: "root",
        sourceLocalId: "evidence-1",
        targetLocalId: "hypothesis-a",
        label: "supports",
        styleKey: "supports",
        meta: { semanticType: "supports" }
      },
      {
        localId: "e4",
        frameLocalId: "root",
        sourceLocalId: "evidence-2",
        targetLocalId: "hypothesis-b",
        label: "supports",
        styleKey: "supports",
        meta: { semanticType: "supports" }
      },
      {
        localId: "e5",
        frameLocalId: "root",
        sourceLocalId: "hypothesis-a",
        targetLocalId: "finding",
        label: "informs",
        styleKey: "depends_on",
        meta: { semanticType: "depends_on" }
      },
      {
        localId: "e6",
        frameLocalId: "root",
        sourceLocalId: "finding",
        targetLocalId: "next-step",
        label: "drives",
        styleKey: "depends_on",
        meta: { semanticType: "depends_on" }
      }
    ]
  },
  layout: {
    origin: { x: 0, y: 0 },
    defaultShapeSize: { width: 270, height: 178 }
  },
  exports: {
    allowed: ["design_doc_md", "mermaid", "ai_plan_md"],
    default: "design_doc_md"
  },
  tags: {
    suggested: [
      { localId: "tag-confirmed", name: "confirmed", color: "#22c55e", description: "Hypothesis or finding confirmed" },
      { localId: "tag-refuted", name: "refuted", color: "#ef4444", description: "Hypothesis refuted" },
      { localId: "tag-open", name: "open", color: "#6366f1", description: "Still open / unresolved" }
    ]
  },
  promptHints: {
    systemHint: "An investigation map: a question, evidence cards, competing hypotheses, findings, and next steps. Edges show which evidence supports which hypothesis and which findings drive next steps."
  }
};

// ---------------------------------------------------------------------------
// Registry — all five ADR/architecture templates keyed by id
// ---------------------------------------------------------------------------

export const ADR_ARCHITECTURE_TEMPLATES: Record<string, TemplateContract> = {
  adr: ADR_TEMPLATE,
  "decision-map": DECISION_MAP_TEMPLATE,
  "server-architecture": SERVER_ARCHITECTURE_TEMPLATE,
  "dependency-diagram": DEPENDENCY_DIAGRAM_TEMPLATE,
  "investigation-map": INVESTIGATION_MAP_TEMPLATE
};
