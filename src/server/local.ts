import { makeMermaid, selectedSubgraph, graphTextDigest } from "../shared/graph";
import type {
  DecisionGraph,
  DesignSeedOutput,
  ExportOutput,
  ExportRequest,
  GraphEdge,
  GraphNode
} from "../shared/schema";

export function seedDesignGraph(prompt: string): DesignSeedOutput {
  const shortTitle = titleFromPrompt(prompt);
  const nodes: GraphNode[] = [
    node("n-proposition", "proposition", shortTitle, "Problem statement and target outcome.", "selected", 0.72),
    node(
      "n-decision-points",
      "decision_point",
      "Decision points",
      "The choice should be driven by feasibility, evidence strength, reversibility, and agent permissions.",
      "draft",
      0.68
    ),
    node(
      "n-option-graph",
      "option",
      "Typed decision graph",
      "Use a typed graph as the source of truth for discussion and exports.",
      "viable",
      0.82
    ),
    node(
      "n-option-freeform",
      "option",
      "Freeform mindmap",
      "Flexible, but weak at enforcing architectural decision quality.",
      "conditional",
      0.54
    ),
    node(
      "n-evidence",
      "evidence",
      "Evidence ledger",
      "Every recommendation should trace to a concrete assumption, source, or probe.",
      "draft",
      0.7
    ),
    node(
      "n-tradeoff",
      "tradeoff",
      "Readable vs complete",
      "Show compact nodes by default and move deep rationale into the inspector.",
      "draft",
      0.75
    ),
    node(
      "n-blocker",
      "blocker",
      "Unbounded local permissions",
      "Shell execution and code editing are out of MVP scope unless a future approval boundary is added.",
      "infeasible",
      0.88
    ),
    node(
      "n-subdecision",
      "subdecision",
      "Export scope",
      "Exports must work for the whole graph and selected subgraphs.",
      "draft",
      0.66
    ),
    node(
      "n-task",
      "task",
      "First vertical slice",
      "Create design, inspect a node, leave comments, and export Markdown.",
      "draft",
      0.61
    ),
    node(
      "n-artifact",
      "artifact",
      "Derived artifacts",
      "AI plan, design document, Confluence draft, Mermaid, and architecture image.",
      "draft",
      0.64
    )
  ];

  const edges: GraphEdge[] = [
    edge("e1", "decomposes_to", "n-proposition", "n-decision-points", "decide by"),
    edge("e2", "chooses_between", "n-decision-points", "n-option-graph", "recommended"),
    edge("e3", "chooses_between", "n-decision-points", "n-option-freeform", "alternative"),
    edge("e4", "supports", "n-evidence", "n-option-graph", "supports"),
    edge("e5", "trades_off_with", "n-option-graph", "n-tradeoff", "accepts"),
    edge("e6", "blocks", "n-blocker", "n-option-freeform", "weakens"),
    edge("e7", "decomposes_to", "n-option-graph", "n-subdecision", "needs"),
    edge("e8", "depends_on", "n-task", "n-option-graph", "builds on"),
    edge("e9", "produces", "n-subdecision", "n-artifact", "exports")
  ];

  return {
    title: shortTitle,
    explanation: "Created a local design graph. Use the MCP server to let an external AI agent refine it.",
    graph: { version: 1, nodes, edges }
  };
}

export function generateLocalExport(graph: DecisionGraph, request: ExportRequest, designTitle: string): ExportOutput {
  const scoped = selectedSubgraph(graph, request.scope);
  if (request.type === "mermaid") {
    return { title: `${designTitle} Mermaid`, content: makeMermaid(scoped) };
  }

  if (request.type === "confluence_html") {
    return {
      title: `${designTitle} Confluence Draft`,
      content: `<h1>${escapeHtml(designTitle)}</h1>${markdownSections(scoped)
        .split("\n")
        .map((line) => (line.startsWith("#") ? `<h2>${escapeHtml(line.replace(/^#+\s*/, ""))}</h2>` : `<p>${escapeHtml(line)}</p>`))
        .join("\n")}`
    };
  }

  if (request.type === "architecture_image") {
    return {
      title: `${designTitle} Architecture Image`,
      content: svgPreview(scoped, designTitle),
      imagePrompt: imagePrompt(scoped, designTitle)
    };
  }

  const title = request.type === "ai_plan_md" ? `${designTitle} AI Task Plan` : `${designTitle} Design Document`;
  return {
    title,
    content: request.type === "ai_plan_md" ? taskPlanSections(scoped, designTitle) : markdownSections(scoped)
  };
}

function node(
  id: string,
  type: GraphNode["type"],
  title: string,
  summary: string,
  status: GraphNode["status"],
  confidence: number
): GraphNode {
  return {
    id,
    type,
    title,
    summary,
    detail: summary,
    status,
    confidence,
    evidenceRefs: [],
    childDecisionIds: []
  };
}

function edge(id: string, type: GraphEdge["type"], source: string, target: string, label: string): GraphEdge {
  return {
    id,
    type,
    source,
    target,
    label,
    rationale: label,
    confidence: 0.7
  };
}

function titleFromPrompt(prompt: string): string {
  const firstLine = prompt.split(/\n/).find((line) => line.trim())?.trim() ?? "Untitled design";
  return firstLine.length > 70 ? `${firstLine.slice(0, 67)}...` : firstLine;
}

function markdownSections(graph: DecisionGraph): string {
  const selected = graph.nodes.find((node) => node.status === "selected");
  const blockers = graph.nodes.filter((node) => node.type === "blocker");
  const options = graph.nodes.filter((node) => node.type === "option");
  const decisions = graph.nodes.filter((node) => node.type === "decision_point" || node.type === "subdecision");
  return [
    `# ${selected?.title ?? "Design Decision"}`,
    "",
    "## Conclusion",
    selected?.summary ?? "Decision graph summary.",
    "",
    "## Decision Points",
    ...decisions.map((node) => `- **${node.title}**: ${node.summary}`),
    "",
    "## Options",
    ...options.map((node) => `- **${node.title}** (${node.status}): ${node.summary}`),
    "",
    "## Blockers",
    ...(blockers.length ? blockers.map((node) => `- **${node.title}**: ${node.summary}`) : ["- None recorded."]),
    "",
    "## Graph Digest",
    "```text",
    graphTextDigest(graph),
    "```"
  ].join("\n");
}

function taskPlanSections(graph: DecisionGraph, title: string): string {
  const tasks = graph.nodes.filter((node) => node.type === "task");
  return [
    `# ${title} AI Task Plan`,
    "",
    "## Objective",
    graph.nodes[0]?.summary ?? "Implement the accepted design direction.",
    "",
    "## Tasks",
    ...(tasks.length
      ? tasks.map((task, index) => `${index + 1}. ${task.title}: ${task.summary}`)
      : ["1. Use an MCP-connected agent to convert accepted decisions into implementation tasks."]),
    "",
    "## Verification",
    "- Validate graph schema.",
    "- Verify exports for the selected graph scope.",
    "- Review blockers before implementation."
  ].join("\n");
}

function imagePrompt(graph: DecisionGraph, title: string): string {
  return `Create a clean architecture diagram for "${title}" using these graph facts:\n${graphTextDigest(graph)}`;
}

function svgPreview(graph: DecisionGraph, title: string): string {
  const rows = graph.nodes.slice(0, 8);
  const height = Math.max(280, 90 + rows.length * 58);
  return [
    `<svg xmlns="http://www.w3.org/2000/svg" width="1280" height="${height}" viewBox="0 0 1280 ${height}">`,
    `<rect width="1280" height="${height}" fill="#f7faf9"/>`,
    `<text x="48" y="52" font-family="Inter, Arial" font-size="30" font-weight="700" fill="#14231f">${escapeHtml(title)}</text>`,
    ...rows.map((node, index) => {
      const y = 88 + index * 58;
      return `<g><rect x="48" y="${y}" width="760" height="40" rx="8" fill="#ffffff" stroke="#d5ded9"/><text x="68" y="${y + 25}" font-family="Inter, Arial" font-size="16" fill="#24312d">${escapeHtml(node.type)}: ${escapeHtml(node.title)}</text></g>`;
    }),
    `</svg>`
  ].join("");
}

function escapeHtml(value: string): string {
  return value.replace(/[&<>"']/g, (char) => {
    const escapes: Record<string, string> = {
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      '"': "&quot;",
      "'": "&#039;"
    };
    return escapes[char] ?? char;
  });
}
