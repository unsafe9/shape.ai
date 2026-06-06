import { applyTemplate, type TemplateContract } from "../../shared/templates/contract";
import { todoBoardTemplate } from "../../shared/templates/todoBoard";
import { wikiNoteTemplate, ideaBoardTemplate } from "../../shared/templates/wikiNote";
import { ADR_TEMPLATE, SERVER_ARCHITECTURE_TEMPLATE } from "../../shared/templates/adrArchitecture";
import { presentationTemplate } from "../../shared/templates/presentation";
import type { Scene, SceneGroup, ScenePatch } from "../../shared/schema";

export type TemplateCatalogEntry = {
  id: string;
  title: string;
  description: string;
  contract: TemplateContract;
};

export const templateCatalog: TemplateCatalogEntry[] = [
  entry(todoBoardTemplate),
  entry(ADR_TEMPLATE),
  entry(SERVER_ARCHITECTURE_TEMPLATE),
  entry(wikiNoteTemplate),
  entry(ideaBoardTemplate),
  entry(presentationTemplate)
];

function entry(contract: TemplateContract): TemplateCatalogEntry {
  return {
    id: contract.metadata.id,
    title: contract.metadata.title,
    description: contract.metadata.description,
    contract
  };
}

/** Build a scene patch that inserts the template into an open area of the current scene. */
export function buildTemplateInsertion(
  scene: Scene | null,
  templateId: string
): { patch: ScenePatch; group: SceneGroup | null; title: string } | null {
  const found = templateCatalog.find((candidate) => candidate.id === templateId);
  if (!found) return null;
  const anchor = openAnchor(scene);
  const idPrefix = `tpl-${crypto.randomUUID().slice(0, 8)}`;
  const applied = applyTemplate(found.contract, anchor, idPrefix, new Date().toISOString());
  const patch: ScenePatch = {
    groups: applied.groups,
    nodes: applied.nodes,
    edges: applied.edges,
    selection: applied.group ? { kind: "group", id: applied.group.id } : undefined
  };
  return { patch, group: applied.group ?? null, title: found.title };
}

/** Place the template to the right of existing content so it lands in view. */
function openAnchor(scene: Scene | null): { x: number; y: number } {
  if (!scene || scene.groups.length === 0) return { x: 120, y: 120 };
  let maxRight = -Infinity;
  let top = Infinity;
  for (const group of scene.groups) {
    maxRight = Math.max(maxRight, group.bounds.x + group.bounds.width);
    top = Math.min(top, group.bounds.y);
  }
  return { x: maxRight + 240, y: Number.isFinite(top) ? top : 120 };
}
