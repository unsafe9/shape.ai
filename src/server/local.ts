import { graphTextDigest, makeMermaid, selectedSubgraph } from "../shared/graph";
import type {
  DecisionGraph,
  ExportOutput,
  ExportRequest,
  GraphEdge,
  GraphNode,
  GroupSeedOutput,
  SceneEdge,
  SceneGroup,
  SceneNode
} from "../shared/schema";

const nodeWidth = 390;
const nodeHeight = 390;
export const defaultSeedNodePositions: Record<string, { x: number; y: number }> = {
  "n-proposition": { x: 0, y: 510 },
  "n-decision-points": { x: 450, y: 510 },
  "n-option-graph": { x: 900, y: 80 },
  "n-option-freeform": { x: 900, y: 510 },
  "n-evidence": { x: 1350, y: 80 },
  "n-tradeoff": { x: 1350, y: 510 },
  "n-blocker": { x: 1350, y: 940 },
  "n-subdecision": { x: 1800, y: 80 },
  "n-task": { x: 1800, y: 510 },
  "n-artifact": { x: 1800, y: 940 }
};

export function seedGroupScene(prompt: string, input: { groupId: string; now: string; parentGroupId?: string | null; tagIds?: string[] }): GroupSeedOutput {
  const title = titleFromPrompt(prompt);
  const graphNodes: GraphNode[] = [
    node("n-proposition", "proposition", title, "Problem statement and target outcome.", "selected", 0.72),
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
      "Create a group, inspect a node, leave comments, and export Markdown.",
      "draft",
      0.61
    ),
    node(
      "n-artifact",
      "artifact",
      "Derived artifacts",
      "MADR, YADR, Mermaid, and image-generation prompts.",
      "draft",
      0.64
    )
  ];

  const graphEdges: GraphEdge[] = [
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

  const nodes: SceneNode[] = graphNodes.map((graphNode, index) => {
    const position = defaultSeedNodePositions[graphNode.id] ?? { x: index * 520, y: 480 };
    return {
      ...graphNode,
      id: scopedId(input.groupId, graphNode.id),
      groupId: input.groupId,
      position,
      size: { width: nodeWidth, height: nodeHeight },
      zIndex: index,
      updatedAt: input.now
    };
  });

  const nodeId = (id: string) => scopedId(input.groupId, id);
  const edges: SceneEdge[] = graphEdges.map((graphEdge) => ({
    ...graphEdge,
    id: scopedId(input.groupId, graphEdge.id),
    source: nodeId(graphEdge.source),
    target: nodeId(graphEdge.target),
    groupId: input.groupId,
    updatedAt: input.now
  }));

  const group: SceneGroup = {
    id: input.groupId,
    parentGroupId: input.parentGroupId ?? null,
    title,
    summary: prompt,
    bounds: boundsForNodes(nodes),
    tagIds: input.tagIds ?? [],
    zIndex: 0,
    collapsed: false,
    createdAt: input.now,
    updatedAt: input.now
  };

  return {
    title,
    explanation: "Created a group on the infinite scene canvas.",
    group,
    nodes,
    edges
  };
}

export function generateLocalExport(graph: DecisionGraph, request: ExportRequest, groupTitle: string): ExportOutput {
  const scope = request.scope?.kind === "selection" ? { kind: "group" } : request.scope ?? { kind: "group" };
  const scoped = selectedSubgraph(graph, scope);
  if (request.type === "mermaid") {
    return { title: `${groupTitle} Mermaid`, content: makeMermaid(scoped) };
  }

  if (request.type === "yadr") {
    return { title: `${groupTitle} YADR`, content: yadrSections(scoped, groupTitle) };
  }

  if (request.type === "image_prompt" || request.type === "architecture_image") {
    const prompt = imagePrompt(scoped, groupTitle);
    return {
      title: `${groupTitle} Image Prompt`,
      content: prompt,
      imagePrompt: prompt
    };
  }

  if (request.type === "confluence_html") {
    return {
      title: `${groupTitle} Confluence Draft`,
      content: `<h1>${escapeHtml(groupTitle)}</h1>${madrSections(scoped, groupTitle)
        .split("\n")
        .map((line) => (line.startsWith("#") ? `<h2>${escapeHtml(line.replace(/^#+\s*/, ""))}</h2>` : `<p>${escapeHtml(line)}</p>`))
        .join("\n")}`
    };
  }

  const title = request.type === "ai_plan_md" ? `${groupTitle} AI Task Plan` : `${groupTitle} Group Document`;
  return {
    title: request.type === "ai_plan_md" ? title : `${groupTitle} MADR`,
    content: request.type === "ai_plan_md" ? taskPlanSections(scoped, groupTitle) : madrSections(scoped, groupTitle)
  };
}

export function boundsForNodes(nodes: SceneNode[]) {
  if (nodes.length === 0) return { x: 0, y: 0, width: 1200, height: 800 };
  const minX = Math.min(...nodes.map((node) => node.position.x));
  const minY = Math.min(...nodes.map((node) => node.position.y));
  const maxX = Math.max(...nodes.map((node) => node.position.x + node.size.width));
  const maxY = Math.max(...nodes.map((node) => node.position.y + node.size.height));
  const padding = 160;
  return {
    x: minX - padding,
    y: minY - padding,
    width: maxX - minX + padding * 2,
    height: maxY - minY + padding * 2
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

function scopedId(groupId: string, id: string): string {
  return `${groupId}-${id}`;
}

function titleFromPrompt(prompt: string): string {
  const firstLine = prompt.split(/\n/).find((line) => line.trim())?.trim() ?? "Untitled group";
  return firstLine.length > 70 ? `${firstLine.slice(0, 67)}...` : firstLine;
}

function madrSections(graph: DecisionGraph, groupTitle: string): string {
  const selected = graph.nodes.find((node) => node.status === "selected");
  const decisionTitle = selected?.title ?? groupTitle;
  const options = graph.nodes.filter((node) => node.type === "option");
  const chosen = options.find((node) => node.status === "selected" || node.status === "viable") ?? options[0] ?? selected;
  const blockers = graph.nodes.filter((node) => node.type === "blocker");
  const tradeoffs = graph.nodes.filter((node) => node.type === "tradeoff");
  const evidence = graph.nodes.filter((node) => node.type === "evidence");
  const decisions = graph.nodes.filter((node) => node.type === "decision_point" || node.type === "subdecision");
  const drivers = [...decisions, ...tradeoffs, ...blockers].map((node) => `${node.title}: ${node.summary}`);
  const positives = evidence.length ? evidence.map((node) => `${node.title}: ${node.summary}`) : ["The selected option keeps the decision graph explicit and reviewable."];
  const negatives = blockers.length ? blockers.map((node) => `${node.title}: ${node.summary}`) : ["Additional review is needed before implementation details are final."];
  return [
    "---",
    "status: proposed",
    "date:",
    "decision-makers:",
    "consulted:",
    "informed:",
    "---",
    "",
    `# ${decisionTitle}`,
    "",
    "## Context and Problem Statement",
    selected?.summary ?? graph.nodes[0]?.summary ?? "Decision graph summary.",
    "",
    "## Decision Drivers",
    ...bulletLines(drivers),
    "",
    "## Considered Options",
    ...bulletLines(options.map((node) => `${node.title}: ${node.summary}`)),
    "",
    "## Decision Outcome",
    `Chosen option: "${chosen?.title ?? decisionTitle}", because ${chosen?.summary ?? selected?.summary ?? "it best matches the recorded decision drivers."}`,
    "",
    "### Consequences",
    ...positives.map((line) => `* Good, because ${line}`),
    ...negatives.map((line) => `* Bad, because ${line}`),
    "",
    "### Confirmation",
    "Review the accepted group, scene version, and generated artifacts before implementation.",
    "",
    "## Pros and Cons of the Options",
    ...optionProsAndCons(options, evidence, tradeoffs, blockers),
    "",
    "## More Information",
    "```text",
    graphTextDigest(graph),
    "```"
  ].join("\n");
}

function yadrSections(graph: DecisionGraph, groupTitle: string): string {
  const selected = graph.nodes.find((node) => node.status === "selected");
  const decisionTitle = selected?.title ?? groupTitle;
  const options = graph.nodes.filter((node) => node.type === "option");
  const chosen = options.find((node) => node.status === "selected" || node.status === "viable") ?? options[0] ?? selected;
  const blockers = graph.nodes.filter((node) => node.type === "blocker");
  const tradeoffs = graph.nodes.filter((node) => node.type === "tradeoff");
  const evidence = graph.nodes.filter((node) => node.type === "evidence");
  const drivers = [...graph.nodes.filter((node) => node.type === "decision_point" || node.type === "subdecision"), ...tradeoffs, ...blockers];
  const optionEntries = options.length ? options : chosen ? [chosen] : [];
  const positives = evidence.length ? evidence : [{ title: "Reviewability", summary: "The decision is captured as a typed graph." }];
  const negatives = blockers.length ? blockers : [{ title: "Open review", summary: "Implementation details still need review." }];

  return [
    "---",
    "metadata:",
    "  status: proposed",
    "  date: TODO",
    "  decision-makers: TODO",
    "  consulted: TODO",
    "  informed: TODO",
    "",
    `title: ${yamlString(decisionTitle)}`,
    "",
    "context-and-problem-statement: |",
    ...yamlBlock(selected?.summary ?? graph.nodes[0]?.summary ?? "Decision graph summary.", 2),
    "",
    "decision-drivers:",
    ...yamlList(drivers.map((node) => `${node.title}: ${node.summary}`)),
    "",
    "considered-options:",
    ...yamlList(optionEntries.map((node) => node.title)),
    "",
    "pros-and-cons-of-the-options:",
    ...yadrOptionEntries(optionEntries, evidence, tradeoffs, blockers),
    "",
    "decision-outcome:",
    "  chosen-option:",
    `    link: ${yamlString(chosen?.title ?? decisionTitle)}`,
    "    justification: |",
    ...yamlBlock(chosen?.summary ?? selected?.summary ?? "Chosen according to the recorded decision drivers.", 6),
    "  consequences:",
    "    positive:",
    ...yamlList(positives.map((node) => `${node.title}: ${node.summary}`), 6),
    "    neutral: []",
    "    negative:",
    ...yamlList(negatives.map((node) => `${node.title}: ${node.summary}`), 6),
    "  confirmation: |",
    ...yamlBlock("Review the accepted group, scene version, and generated artifacts before implementation.", 4),
    "",
    "more-information: |",
    ...yamlBlock(graphTextDigest(graph), 2)
  ].join("\n");
}

function taskPlanSections(graph: DecisionGraph, title: string): string {
  const tasks = graph.nodes.filter((node) => node.type === "task");
  return [
    `# ${title} AI Task Plan`,
    "",
    "## Objective",
    graph.nodes[0]?.summary ?? "Implement the accepted group direction.",
    "",
    "## Tasks",
    ...(tasks.length
      ? tasks.map((task, index) => `${index + 1}. ${task.title}: ${task.summary}`)
      : ["1. Use an MCP-connected agent to convert accepted decisions into implementation tasks."]),
    "",
    "## Verification",
    "- Validate scene schema.",
    "- Verify exports for the selected group scope.",
    "- Review blockers before implementation."
  ].join("\n");
}

function imagePrompt(graph: DecisionGraph, title: string): string {
  return [
    `Create a clean architecture decision diagram for "${title}".`,
    "Use labeled boxes for decision points, options, evidence, blockers, tradeoffs, tasks, and artifacts.",
    "Use directional arrows for graph relationships and keep labels short enough to read.",
    "The result should look like a reviewable architecture diagram, not a web UI screenshot.",
    "Prefer a white or very light background, restrained colors, and clear hierarchy.",
    "",
    "Graph facts:",
    graphTextDigest(graph)
  ].join("\n");
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

function bulletLines(lines: string[]): string[] {
  return lines.length ? lines.map((line) => `* ${line}`) : ["* TODO"];
}

function optionProsAndCons(
  options: GraphNode[],
  evidence: GraphNode[],
  tradeoffs: GraphNode[],
  blockers: GraphNode[]
): string[] {
  if (options.length === 0) return ["### TODO", "", "* Good, because TODO", "* Neutral, because TODO", "* Bad, because TODO"];
  return options.flatMap((option) => [
    `### ${option.title}`,
    "",
    option.summary,
    "",
    ...evidence.map((node) => `* Good, because ${node.title}: ${node.summary}`),
    ...(tradeoffs.length ? tradeoffs.map((node) => `* Neutral, because ${node.title}: ${node.summary}`) : ["* Neutral, because additional tradeoffs may emerge during review."]),
    ...(blockers.length ? blockers.map((node) => `* Bad, because ${node.title}: ${node.summary}`) : ["* Bad, because implementation risk has not been fully reviewed."]),
    ""
  ]);
}

function yadrOptionEntries(
  options: GraphNode[],
  evidence: GraphNode[],
  tradeoffs: GraphNode[],
  blockers: GraphNode[]
): string[] {
  if (options.length === 0) return ["  TODO:", "    description: TODO", "    pros: []", "    neutral: []", "    cons: []"];
  return options.flatMap((option, index) => [
    `  option-${index + 1}:`,
    "    description: |",
    ...yamlBlock(option.summary, 6),
    "    pros:",
    ...yamlList(evidence.map((node) => `${node.title}: ${node.summary}`), 6),
    "    neutral:",
    ...yamlList(tradeoffs.map((node) => `${node.title}: ${node.summary}`), 6),
    "    cons:",
    ...yamlList(blockers.map((node) => `${node.title}: ${node.summary}`), 6)
  ]);
}

function yamlList(values: string[], indent = 0): string[] {
  const prefix = " ".repeat(indent);
  return values.length ? values.map((value) => `${prefix}- ${yamlString(value)}`) : [`${prefix}- TODO`];
}

function yamlBlock(value: string, indent: number): string[] {
  const prefix = " ".repeat(indent);
  return value.split("\n").map((line) => `${prefix}${line}`);
}

function yamlString(value: string): string {
  if (/^[a-zA-Z0-9 _.-]+$/.test(value)) return value;
  return JSON.stringify(value);
}
