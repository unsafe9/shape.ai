import { nodeTypeLabels } from "../../shared/graph";
import type { GraphNode } from "../../shared/schema";

export function formatNodeMarkdown(node: GraphNode): string {
  const lines = [
    `# ${node.title}`,
    "",
    `- **Type:** ${nodeTypeLabels[node.type]}`,
    `- **Status:** ${node.status}`,
    `- **Confidence:** ${Math.round(node.confidence * 100)}%`,
    "",
    "## Summary",
    node.summary || "_No summary recorded._",
    "",
    "## Detail",
    node.detail || "_No detail recorded._"
  ];

  if (node.evidenceRefs.length > 0) {
    lines.push("", "## Evidence References", ...node.evidenceRefs.map((ref) => `- ${ref}`));
  }

  if (node.childDecisionIds.length > 0) {
    lines.push("", "## Child Decisions", ...node.childDecisionIds.map((id) => `- ${id}`));
  }

  return `${lines.join("\n").replace(/\n{3,}/g, "\n\n").trim()}\n`;
}

export function cloneNodeForPaste(node: GraphNode, id: string): GraphNode {
  return {
    ...node,
    id,
    title: `${node.title} copy`
  };
}
