<script lang="ts">
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
  } from "lucide-svelte";
  import type { Scene, SceneSelection } from "../../shared/schema";
  import type { RenderScenePatch } from "../../shared/renderPatch";

  type Props = {
    scene: Scene;
    selection: SceneSelection;
    /** Ids of the multi-select set; may be empty or single when only one item is selected. */
    multiSelectIds: string[];
    onPatch: (patch: RenderScenePatch) => void;
  };

  let { scene, selection, multiSelectIds, onPatch }: Props = $props();

  // Resolve effective ids: a `multi` selection (or the ephemeral multiSelectIds set)
  // drives batch actions, else the single-target id from the selection.
  const effectiveIds = $derived<string[]>(
    selection.kind === "multi"
      ? selection.ids
      : multiSelectIds.length >= 2
        ? multiSelectIds
        : selection.kind === "node"
          ? [selection.id]
          : selection.kind === "group"
            ? [selection.id]
            : selection.kind === "edge"
              ? [selection.id]
              : []
  );

  const nodeIds = $derived(effectiveIds.filter((id) => scene.nodes.some((n) => n.id === id)));
  const groupIds = $derived(effectiveIds.filter((id) => scene.groups.some((g) => g.id === id)));
  const hasMultipleNodes = $derived(nodeIds.length >= 2);
  const hasEnoughForDistribute = $derived(nodeIds.length >= 3);
  const isSingleGroup = $derived(selection.kind === "group" && effectiveIds.length === 1 && groupIds.length === 1);
  const hasDuplicatable = $derived(nodeIds.length >= 1);
  const hasDeletable = $derived(effectiveIds.length >= 1);

  const showAlignDistribute = $derived(hasMultipleNodes);
  const showGroup = $derived(nodeIds.length + groupIds.length >= 2);
  const showUngroup = $derived(isSingleGroup);

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
</script>

{#if selection.kind !== "canvas"}
  <div
    class="canvas-editing-toolbar"
    role="toolbar"
    tabindex="-1"
    aria-label="Editing actions"
    onpointerdown={(event) => event.stopPropagation()}
  >
    {#if hasDuplicatable}
      <button class="icon-button" type="button" title="Duplicate" aria-label="Duplicate" onclick={handleDuplicate}>
        <Copy size={14} />
      </button>
    {/if}

    {#if showGroup}
      <button class="icon-button" type="button" title="Group into frame" aria-label="Group" onclick={handleGroup}>
        <Group size={14} />
      </button>
    {/if}

    {#if showUngroup}
      <button class="icon-button" type="button" title="Ungroup frame" aria-label="Ungroup" onclick={handleUngroup}>
        <Ungroup size={14} />
      </button>
    {/if}

    {#if showAlignDistribute}
      <div class="canvas-editing-toolbar-sep" aria-hidden="true"></div>
      <button class="icon-button" type="button" title="Align left edges" aria-label="Align left" onclick={() => handleAlignCards("x", "start")}>
        <AlignStartHorizontal size={14} />
      </button>
      <button class="icon-button" type="button" title="Center horizontally" aria-label="Center horizontally" onclick={() => handleAlignCards("x", "center")}>
        <AlignCenterHorizontal size={14} />
      </button>
      <button class="icon-button" type="button" title="Align right edges" aria-label="Align right" onclick={() => handleAlignCards("x", "end")}>
        <AlignEndHorizontal size={14} />
      </button>
      <button class="icon-button" type="button" title="Align top edges" aria-label="Align top" onclick={() => handleAlignCards("y", "start")}>
        <AlignStartVertical size={14} />
      </button>
      <button class="icon-button" type="button" title="Center vertically" aria-label="Center vertically" onclick={() => handleAlignCards("y", "center")}>
        <AlignCenterVertical size={14} />
      </button>
      <button class="icon-button" type="button" title="Align bottom edges" aria-label="Align bottom" onclick={() => handleAlignCards("y", "end")}>
        <AlignEndVertical size={14} />
      </button>
      {#if hasEnoughForDistribute}
        <div class="canvas-editing-toolbar-sep" aria-hidden="true"></div>
        <button class="icon-button" type="button" title="Distribute horizontally" aria-label="Distribute horizontally" onclick={() => handleDistribute("x")}>
          <AlignHorizontalDistributeCenter size={14} />
        </button>
        <button class="icon-button" type="button" title="Distribute vertically" aria-label="Distribute vertically" onclick={() => handleDistribute("y")}>
          <AlignVerticalDistributeCenter size={14} />
        </button>
      {/if}
    {/if}

    {#if hasDeletable}
      <div class="canvas-editing-toolbar-sep" aria-hidden="true"></div>
      <button class="icon-button canvas-editing-toolbar-delete" type="button" title="Delete" aria-label="Delete" onclick={handleDelete}>
        <Trash2 size={14} />
      </button>
    {/if}
  </div>
{/if}
