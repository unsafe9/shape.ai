import {
  AlignCenterHorizontal,
  AlignCenterVertical,
  AlignEndHorizontal,
  AlignEndVertical,
  AlignHorizontalDistributeCenter,
  AlignStartHorizontal,
  AlignStartVertical,
  AlignVerticalDistributeCenter,
  Copy,
  Group,
  Trash2,
  Ungroup
} from "lucide-react";
import type { Scene, SceneSelection } from "../../shared/schema";
import type { RenderScenePatch } from "../../shared/renderPatch";

type CanvasEditingToolbarProps = {
  scene: Scene;
  selection: SceneSelection;
  /** Ids of the multi-select set; may be empty or single when only one item is selected. */
  multiSelectIds: string[];
  onPatch: (patch: RenderScenePatch) => void;
};

/**
 * T2.2: Minimal editing toolbar shown for the current selection.
 * Wires group/ungroup/duplicate/align/distribute/delete to the
 * already-implemented renderPatch ops. Shown only when something is selected.
 */
export function CanvasEditingToolbar({ scene, selection, multiSelectIds, onPatch }: CanvasEditingToolbarProps) {
  if (selection.kind === "canvas") return null;

  // Resolve effective ids: multi-select set if present, else single-target from selection.
  const effectiveIds: string[] =
    multiSelectIds.length >= 2
      ? multiSelectIds
      : selection.kind === "node"
        ? [selection.id]
        : selection.kind === "group"
          ? [selection.id]
          : selection.kind === "edge"
            ? [selection.id]
            : [];

  const nodeIds = effectiveIds.filter((id) => scene.nodes.some((n) => n.id === id));
  const groupIds = effectiveIds.filter((id) => scene.groups.some((g) => g.id === id));
  const hasMultipleNodes = nodeIds.length >= 2;
  const hasEnoughForDistribute = nodeIds.length >= 3;
  const isSingleGroup = selection.kind === "group" && effectiveIds.length === 1 && groupIds.length === 1;
  const hasDuplicatable = nodeIds.length >= 1;
  const hasDeletable = effectiveIds.length >= 1;

  function handleDuplicate() {
    if (!hasDuplicatable) return;
    onPatch({ kind: "duplicate-objects", ids: nodeIds, delta: { x: 40, y: 40 } });
  }

  function handleDelete() {
    if (!hasDeletable) return;
    if (effectiveIds.length === 1) {
      const id = effectiveIds[0];
      if (scene.nodes.some((n) => n.id === id)) {
        onPatch({ kind: "delete-card", id });
        return;
      }
      if (scene.groups.some((g) => g.id === id)) {
        onPatch({ kind: "delete-group", id });
        return;
      }
      if (scene.edges.some((e) => e.id === id)) {
        onPatch({ kind: "delete-edge", id });
        return;
      }
      return;
    }
    // Batch delete for multi-select
    const ops: RenderScenePatch[] = [];
    for (const id of effectiveIds) {
      if (scene.nodes.some((n) => n.id === id)) ops.push({ kind: "delete-card", id });
      else if (scene.groups.some((g) => g.id === id)) ops.push({ kind: "delete-group", id });
      else if (scene.edges.some((e) => e.id === id)) ops.push({ kind: "delete-edge", id });
    }
    if (ops.length > 0) onPatch({ kind: "batch", ops });
  }

  function handleAlignCards(axis: "x" | "y", mode: "start" | "center" | "end") {
    if (!hasMultipleNodes) return;
    onPatch({ kind: "align-cards", ids: nodeIds, axis, mode });
  }

  function handleDistribute(axis: "x" | "y") {
    if (!hasEnoughForDistribute) return;
    onPatch({ kind: "distribute-cards", ids: nodeIds, axis });
  }

  function handleGroup() {
    const idsToGroup = [...nodeIds, ...groupIds];
    if (idsToGroup.length < 2) return;
    const frameId = `frame-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
    onPatch({ kind: "group-objects", ids: idsToGroup, frameId });
  }

  function handleUngroup() {
    if (!isSingleGroup) return;
    onPatch({ kind: "ungroup", id: effectiveIds[0] });
  }

  const showAlignDistribute = hasMultipleNodes;
  const showGroup = nodeIds.length + groupIds.length >= 2;
  const showUngroup = isSingleGroup;

  return (
    <div
      className="canvas-editing-toolbar"
      role="toolbar"
      aria-label="Editing actions"
      onPointerDown={(event) => event.stopPropagation()}
    >
      {hasDuplicatable ? (
        <button className="icon-button" type="button" title="Duplicate" aria-label="Duplicate" onClick={handleDuplicate}>
          <Copy size={14} />
        </button>
      ) : null}

      {showGroup ? (
        <button className="icon-button" type="button" title="Group into frame" aria-label="Group" onClick={handleGroup}>
          <Group size={14} />
        </button>
      ) : null}

      {showUngroup ? (
        <button className="icon-button" type="button" title="Ungroup frame" aria-label="Ungroup" onClick={handleUngroup}>
          <Ungroup size={14} />
        </button>
      ) : null}

      {showAlignDistribute ? (
        <>
          <div className="canvas-editing-toolbar-sep" aria-hidden="true" />
          <button
            className="icon-button"
            type="button"
            title="Align left edges"
            aria-label="Align left"
            onClick={() => handleAlignCards("x", "start")}
          >
            <AlignStartHorizontal size={14} />
          </button>
          <button
            className="icon-button"
            type="button"
            title="Center horizontally"
            aria-label="Center horizontally"
            onClick={() => handleAlignCards("x", "center")}
          >
            <AlignCenterHorizontal size={14} />
          </button>
          <button
            className="icon-button"
            type="button"
            title="Align right edges"
            aria-label="Align right"
            onClick={() => handleAlignCards("x", "end")}
          >
            <AlignEndHorizontal size={14} />
          </button>
          <button
            className="icon-button"
            type="button"
            title="Align top edges"
            aria-label="Align top"
            onClick={() => handleAlignCards("y", "start")}
          >
            <AlignStartVertical size={14} />
          </button>
          <button
            className="icon-button"
            type="button"
            title="Center vertically"
            aria-label="Center vertically"
            onClick={() => handleAlignCards("y", "center")}
          >
            <AlignCenterVertical size={14} />
          </button>
          <button
            className="icon-button"
            type="button"
            title="Align bottom edges"
            aria-label="Align bottom"
            onClick={() => handleAlignCards("y", "end")}
          >
            <AlignEndVertical size={14} />
          </button>
          {hasEnoughForDistribute ? (
            <>
              <div className="canvas-editing-toolbar-sep" aria-hidden="true" />
              <button
                className="icon-button"
                type="button"
                title="Distribute horizontally"
                aria-label="Distribute horizontally"
                onClick={() => handleDistribute("x")}
              >
                <AlignHorizontalDistributeCenter size={14} />
              </button>
              <button
                className="icon-button"
                type="button"
                title="Distribute vertically"
                aria-label="Distribute vertically"
                onClick={() => handleDistribute("y")}
              >
                <AlignVerticalDistributeCenter size={14} />
              </button>
            </>
          ) : null}
        </>
      ) : null}

      {hasDeletable ? (
        <>
          <div className="canvas-editing-toolbar-sep" aria-hidden="true" />
          <button
            className="icon-button canvas-editing-toolbar-delete"
            type="button"
            title="Delete"
            aria-label="Delete"
            onClick={handleDelete}
          >
            <Trash2 size={14} />
          </button>
        </>
      ) : null}
    </div>
  );
}
