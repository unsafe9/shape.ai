<script lang="ts">
  import { onDestroy } from "svelte";
  import { Activity, BrainCircuit, LayoutTemplate, Layers, Loader2, Maximize2, Minus, PanelLeft, Plus, X } from "lucide-svelte";
  import { createGroup, createTag, fetchScene, saveScenePatch, updateGroupTags } from "../lib/api";
  import { applyRenderPatchToShapeScene, type RenderScenePatch } from "../../shared/renderPatch";
  import type { CameraState } from "../../shared/renderScene";
  import type { Scene, SceneGroup, SceneSelection, Tag } from "../../shared/schema";
  import { ShapeCanvasHost, type RendererHealth, type RendererStats, type ShapeCanvasHostCallbacks } from "../lib/canvasHost";
  import { createPatchSaver, isContinuousRendererPatch } from "../lib/patchSaver";
  import { buildTemplateInsertion, templateCatalog } from "../lib/templates";
  import Sidebar from "./Sidebar.svelte";
  import TemplatePicker from "./TemplatePicker.svelte";
  import CanvasHost from "./ShapeCanvasHost.svelte";

  const tagColors = ["#6b8df2", "#12a594", "#d17b31", "#b65fcf", "#d84d66", "#6f7a86"];
  const seedPrompt =
    "Draft an AI-assisted architecture decision tool that extracts propositions, decision points, options, evidence, blockers, tradeoffs, subdecisions, tasks, and exports.";

  // ----- document projection (read of server scene) -----
  let scene = $state<Scene | null>(null);

  // ----- ephemeral selection / viewport / chrome -----
  let selection = $state<SceneSelection>({ kind: "canvas" });
  let camera = $state<CameraState>({ x: 140, y: 120, zoom: 0.28 });
  let activeTagIds = $state<string[]>([]);
  let currentGroupId = $state<string | undefined>(undefined);

  let prompt = $state(seedPrompt);
  let tagName = $state("");
  let status = $state("Ready");
  let busy = $state(false);
  let groupPanelOpen = $state(false);
  let templatePickerOpen = $state(false);
  let diagnosticsOpen = $state(false);

  // ----- ephemeral renderer readout -----
  let rendererStats = $state<RendererStats | null>(null);
  let rendererHealth = $state<RendererHealth | null>(null);
  let rendererStatus = $state("No renderer status yet");

  // ----- non-reactive refs (last-write-wins guards + gesture gate) -----
  // selectionRef mirrors the reactive `selection` so the loadScene effect can
  // read the current selection WITHOUT subscribing to it (matching App.tsx,
  // where the load effect deps excluded selection and used selectionRef).
  let host: ShapeCanvasHost | null = null;
  let canvasWrap: HTMLDivElement;
  let sceneRequest = 0;
  let gestureActive = false;
  let selectionRef: SceneSelection = { kind: "canvas" };

  const activeGroupId = $derived(activeGroupIdForSelection(scene, selection) ?? currentGroupId ?? scene?.groups[0]?.id);
  const activeGroup = $derived(scene?.groups.find((group) => group.id === activeGroupId) ?? null);
  const readyState = $derived(rendererHealth?.state ?? "wasm-unavailable");
  const rendererDetail = $derived(rendererHealth?.detail ?? "Detecting Rust/WASM package.");
  const hasRenderableScene = $derived(Boolean(scene?.groups.length));

  // T6.2 §1: debounced, gesture-gated renderer-patch save lives in the
  // framework-neutral patchSaver module; the shell only feeds it.
  const patchSaver = createPatchSaver({
    isGestureActive: () => gestureActive,
    onSaved: (savedScene, savedSelection) => {
      const validated = validSelection(savedScene, savedSelection);
      sceneRequest += 1;
      scene = savedScene;
      selection = validated;
      const savedGroupId = activeGroupIdForSelection(savedScene, validated);
      if (savedGroupId) currentGroupId = savedGroupId;
    },
    onError: (rollbackScene, rollbackSelection, message) => {
      const restored = validSelection(rollbackScene, rollbackSelection);
      scene = rollbackScene;
      selection = restored;
      const restoredGroupId = activeGroupIdForSelection(rollbackScene, restored);
      if (restoredGroupId) currentGroupId = restoredGroupId;
      status = message;
      void refreshScene().catch((error) => (status = error instanceof Error ? error.message : "Scene refresh failed"));
    }
  });

  const hostCallbacks: ShapeCanvasHostCallbacks = {
    onCameraChange: (next) => (camera = next),
    onSelectionChange: handleRendererSelection,
    onPatch: handleRendererPatch,
    onGestureChange: handleRendererGesture,
    onStats: (stats) => (rendererStats = stats),
    onStatus: handleRendererStatus,
    onHealthChange: (health) => {
      rendererHealth = health;
      if (health.state === "ready" && isDiagnosticsOnlyRendererStatus(status)) status = "Ready";
    }
  };

  // Initial fetch + reactive reload-on-filter, mirroring the App.tsx
  // batched/debounced load. Document writes flow ONLY through patchSaver, so the
  // scene store is never re-derived per keystroke (T6.2 §1 batch-aware wiring).
  void refreshScene().catch((error) => (status = error instanceof Error ? error.message : "Scene load failed"));

  $effect(() => {
    const tagIds = activeTagIds;
    const id = window.setTimeout(() => {
      void refreshScene(tagIds).catch((error) => (status = error instanceof Error ? error.message : "Scene load failed"));
    }, 120);
    return () => window.clearTimeout(id);
  });

  // Push the document scene into the engine whenever it or the tag filter
  // changes. Selection is read non-reactively (selectionRef) so a selection
  // change alone does NOT trigger a scene reload — mirroring App.tsx.
  $effect(() => {
    const currentScene = scene;
    const tagIds = activeTagIds;
    if (!host || !currentScene) return;
    host.loadScene(currentScene, tagIds, selectionRef);
  });

  // Keep selectionRef in sync with the reactive selection and push selection to
  // the engine. This effect intentionally only commands syncSelection.
  $effect(() => {
    selectionRef = selection;
    host?.syncSelection(selection);
  });

  onDestroy(() => patchSaver.dispose());

  async function refreshScene(tagIds = activeTagIds): Promise<void> {
    const requestId = ++sceneRequest;
    const nextScene = await fetchScene({ tagIds });
    if (requestId !== sceneRequest) return;
    scene = nextScene;
    const serverSelection = validSelection(nextScene, nextScene.selection);
    const localSelection = validSelection(nextScene, selection);
    selection = serverSelection.kind === "canvas" && localSelection.kind !== "canvas" ? localSelection : serverSelection;
  }

  function handleHost(next: ShapeCanvasHost): void {
    host = next;
    if (scene) host.loadScene(scene, activeTagIds, selection);
    host.syncSelection(selection);
  }

  function handleRendererSelection(next: SceneSelection): void {
    if (!scene) return;
    const valid = validSelection(scene, next);
    selection = valid;
    const nextGroupId = activeGroupIdForSelection(scene, valid);
    if (nextGroupId) currentGroupId = nextGroupId;
    void saveScenePatch({ selection: valid }).catch((error) => {
      status = error instanceof Error ? error.message : "Selection save failed";
    });
  }

  function handleRendererPatch(patch: RenderScenePatch): void {
    if (!scene) return;
    const previousScene = scene;
    const previousSelection = selection;
    const applied = applyRenderPatchToShapeScene(scene, patch, new Date().toISOString());
    if (applied.errors.length > 0) {
      status = applied.errors.join("; ");
      return;
    }
    const optimisticSelection = validSelection(applied.scene, applied.scene.selection);
    sceneRequest += 1;
    if (isContinuousRendererPatch(patch)) {
      if (!gestureActive) commitScene(applied.scene, optimisticSelection);
      patchSaver.queue(applied.appPatch, previousScene, previousSelection, patch.kind, gestureActive);
      return;
    }
    commitScene(applied.scene, optimisticSelection);
    patchSaver.flush();
    patchSaver.saveNow(applied.appPatch, previousScene, previousSelection, patch.kind);
  }

  function handleRendererGesture(active: boolean): void {
    gestureActive = active;
    if (active) return;
    if (scene) commitScene(scene, selection);
    patchSaver.flush();
  }

  function commitScene(nextScene: Scene, nextSelection: SceneSelection): void {
    const valid = validSelection(nextScene, nextSelection);
    scene = nextScene;
    selection = valid;
    const nextGroupId = activeGroupIdForSelection(nextScene, valid);
    if (nextGroupId) currentGroupId = nextGroupId;
  }

  function handleRendererStatus(message: string): void {
    rendererStatus = message;
    if (isDiagnosticsOnlyRendererStatus(message)) return;
    status = message;
  }

  function handleContextMenuRequest(_point: { x: number; y: number }): void {
    // Net-new node context menu is owned by the full inspector slice (out of the
    // T6.2 working-slice scope); ignored here.
  }

  async function runCreateGroup(): Promise<void> {
    await withBusy("Creating group", async () => {
      sceneRequest += 1;
      const response = await createGroup(prompt, undefined, activeTagIds);
      scene = response.scene;
      groupPanelOpen = false;
      status = response.message;
      currentGroupId = response.group.id;
      const group = response.scene.groups.find((candidate) => candidate.id === response.group.id);
      if (group) focusGroup(group, { fit: true });
      await selectGroup(response.group.id, response.scene);
    });
  }

  async function applyTemplateById(templateId: string): Promise<void> {
    await withBusy("Inserting template", async () => {
      const built = buildTemplateInsertion(scene, templateId);
      if (!built) return;
      sceneRequest += 1;
      const response = await saveScenePatch(built.patch);
      scene = response.scene;
      if (built.group) {
        const created = response.scene.groups.find((group) => group.id === built.group!.id) ?? built.group;
        currentGroupId = created.id;
        focusGroup(created, { fit: true });
        await selectGroup(created.id, response.scene);
      }
      status = `Inserted ${built.title}`;
    });
  }

  async function runCreateTag(): Promise<void> {
    if (!tagName.trim()) return;
    await withBusy("Creating tag", async () => {
      const color = tagColors[(scene?.tags.length ?? 0) % tagColors.length];
      const response = await createTag(tagName.trim(), color);
      scene = response.scene;
      tagName = "";
      status = `Tag created: ${response.tag.name}`;
    });
  }

  async function toggleGroupTag(tag: Tag): Promise<void> {
    if (!activeGroup || !scene) return;
    const group = activeGroup;
    const nextTagIds = group.tagIds.includes(tag.id)
      ? group.tagIds.filter((tagId) => tagId !== tag.id)
      : [...group.tagIds, tag.id];
    scene = { ...scene, groups: scene.groups.map((candidate) => (candidate.id === group.id ? { ...group, tagIds: nextTagIds } : candidate)) };
    try {
      const response = await updateGroupTags(group.id, nextTagIds);
      scene = response.scene;
      status = "Group tags updated";
    } catch (error) {
      status = error instanceof Error ? error.message : "Tag update failed";
    }
  }

  async function selectGroup(groupId: string, sourceScene: Scene | null = scene): Promise<void> {
    if (!sourceScene) return;
    const valid = validSelection(sourceScene, { kind: "group", id: groupId });
    selection = valid;
    const nextGroupId = activeGroupIdForSelection(sourceScene, valid);
    if (nextGroupId) currentGroupId = nextGroupId;
    host?.syncSelection(valid);
    try {
      await saveScenePatch({ selection: valid });
    } catch (error) {
      status = error instanceof Error ? error.message : "Selection save failed";
    }
  }

  function onSelectGroupFromSidebar(group: SceneGroup): void {
    groupPanelOpen = false;
    focusGroup(group, { fit: true });
    void selectGroup(group.id);
  }

  function toggleTagFilter(tagId: string): void {
    activeTagIds = activeTagIds.includes(tagId)
      ? activeTagIds.filter((id) => id !== tagId)
      : [...activeTagIds, tagId];
  }

  function focusGroup(group: SceneGroup, options: { zoom?: number; fit?: boolean } = {}): void {
    const rect = canvasWrap?.getBoundingClientRect();
    if (!rect || !host) return;
    host.focusBounds(group.bounds, {
      screen: { x: rect.width / 2, y: rect.height / 2 },
      zoom: options.fit ? undefined : options.zoom ?? Math.min(0.65, Math.max(0.18, camera.zoom)),
      padding: options.fit ? { x: 110, y: 140 } : undefined,
      minZoom: options.fit ? 0.36 : undefined,
      maxZoom: options.fit ? 0.58 : undefined
    });
  }

  function zoomAtCenter(deltaY: number): void {
    const rect = canvasWrap?.getBoundingClientRect();
    if (!rect || !host) return;
    host.wheelAtScreen({ x: rect.width / 2, y: rect.height / 2 }, deltaY);
  }

  function fitScene(): void {
    host?.fitScene();
  }

  async function toggleFullscreen(): Promise<void> {
    if (!canvasWrap) return;
    try {
      if (document.fullscreenElement) await document.exitFullscreen();
      else await canvasWrap.requestFullscreen();
    } catch (error) {
      status = error instanceof Error ? error.message : "Fullscreen failed";
    }
  }

  async function withBusy(label: string, action: () => Promise<void>): Promise<void> {
    busy = true;
    status = label;
    try {
      await action();
      if (status === label) status = "Ready";
    } catch (error) {
      status = error instanceof Error ? error.message : "Unknown error";
    } finally {
      busy = false;
    }
  }

  function activeGroupIdForSelection(currentScene: Scene | null, currentSelection: SceneSelection): string | undefined {
    if (!currentScene) return undefined;
    if (currentSelection.kind === "group") return currentSelection.id;
    if (currentSelection.kind === "node") return currentScene.nodes.find((node) => node.id === currentSelection.id)?.groupId;
    if (currentSelection.kind === "edge") return currentScene.edges.find((edge) => edge.id === currentSelection.id)?.groupId;
    return undefined;
  }

  function validSelection(currentScene: Scene, currentSelection: SceneSelection): SceneSelection {
    if (currentSelection.kind === "canvas") return currentSelection;
    if (currentSelection.kind === "group" && currentScene.groups.some((group) => group.id === currentSelection.id)) return currentSelection;
    if (currentSelection.kind === "node" && currentScene.nodes.some((node) => node.id === currentSelection.id)) return currentSelection;
    if (currentSelection.kind === "edge" && currentScene.edges.some((edge) => edge.id === currentSelection.id)) return currentSelection;
    return { kind: "canvas" };
  }

  function isDiagnosticsOnlyRendererStatus(message: string): boolean {
    return message.startsWith("WebGPU renderer unavailable:") || message.startsWith("WebGPU render failed");
  }
</script>

<div class="app-shell">
  <main class="studio-stage">
    <section class={`canvas-panel ${selection.kind === "node" ? "has-card-focus" : ""}`}>
      <div class="flow-wrap renderer-scene-surface" bind:this={canvasWrap}>
        <div class="canvas-watermark" aria-hidden="true">
          <BrainCircuit size={28} />
          <span>shape.ai</span>
        </div>
        <div class="scene-controls" aria-label="Canvas controls">
          <button
            class="icon-button {templatePickerOpen ? 'is-active' : ''}"
            type="button"
            onclick={() => (templatePickerOpen = !templatePickerOpen)}
            aria-label="Insert template"
            aria-expanded={templatePickerOpen}
            title="Insert template"
          >
            <LayoutTemplate size={15} />
          </button>
          <button
            class="icon-button {diagnosticsOpen ? 'is-active' : ''}"
            type="button"
            onclick={() => (diagnosticsOpen = !diagnosticsOpen)}
            aria-label={diagnosticsOpen ? "Close diagnostics" : "Open diagnostics"}
            aria-expanded={diagnosticsOpen}
            title={diagnosticsOpen ? "Close diagnostics" : "Open diagnostics"}
          >
            <Activity size={15} />
          </button>
          <button class="icon-button" onclick={() => zoomAtCenter(160)} aria-label="Zoom out" title="Zoom out">
            <Minus size={15} />
          </button>
          <button class="icon-button" onclick={() => zoomAtCenter(-160)} aria-label="Zoom in" title="Zoom in">
            <Plus size={15} />
          </button>
          <button class="icon-button" onclick={fitScene} aria-label="Fit scene" title="Fit scene">
            <Layers size={15} />
          </button>
          <button class="icon-button" onclick={() => void toggleFullscreen()} aria-label="Fullscreen" title="Fullscreen">
            <Maximize2 size={15} />
          </button>
        </div>
        {#if templatePickerOpen}
          <TemplatePicker
            templates={templateCatalog.map(({ id, title, description }) => ({ id, title, description }))}
            {busy}
            onApply={(templateId) => void applyTemplateById(templateId)}
            onClose={() => (templatePickerOpen = false)}
          />
        {/if}
        {#if diagnosticsOpen && rendererStats}
          <div class="renderer-diagnostics" id="renderer-diagnostics" aria-label="Renderer diagnostics">
            <strong>Renderer</strong>
            <span>backend: {rendererStats.backend}</span>
            <span>draw: {rendererStats.drawBackend}</span>
            <span>groups: {rendererStats.visibleGroups}/{rendererStats.totalGroups}</span>
            <span>cards: {rendererStats.visibleCards}/{rendererStats.totalCards}</span>
            <span>edges: {rendererStats.visibleEdges}/{rendererStats.totalEdges}</span>
            <span>frame: {rendererStats.frameMs.toFixed(2)}ms</span>
            <span>status: {rendererStatus}</span>
          </div>
        {/if}

        <CanvasHost
          initialCamera={camera}
          callbacks={hostCallbacks}
          {readyState}
          {rendererDetail}
          {hasRenderableScene}
          onHost={handleHost}
          onContextMenuRequest={handleContextMenuRequest}
        />
      </div>

      <div class="floating-groups {groupPanelOpen ? 'is-open' : 'is-closed'}">
        <button
          class="group-panel-toggle icon-button {groupPanelOpen ? 'is-active' : ''}"
          onclick={() => (groupPanelOpen = !groupPanelOpen)}
          aria-label={groupPanelOpen ? "Close groups" : "Open groups"}
        >
          {#if groupPanelOpen}
            <X size={16} />
          {:else}
            <PanelLeft size={16} />
          {/if}
        </button>
        <div class="floating-groups-body">
          <Sidebar
            groups={scene?.groups ?? []}
            tags={scene?.tags ?? []}
            {activeGroupId}
            {activeGroup}
            {prompt}
            {tagName}
            {busy}
            {activeTagIds}
            onPromptChange={(value) => (prompt = value)}
            onTagNameChange={(value) => (tagName = value)}
            onCreateGroup={() => void runCreateGroup()}
            onCreateTag={() => void runCreateTag()}
            onSelectGroup={onSelectGroupFromSidebar}
            onToggleGroupTag={(tag) => void toggleGroupTag(tag)}
            onToggleTagFilter={toggleTagFilter}
            onRefresh={() => void refreshScene().catch((error) => (status = error instanceof Error ? error.message : "Scene refresh failed"))}
          />
        </div>
      </div>

      {#if busy || status !== "Ready"}
        <div class="canvas-status" role="status">
          {#if busy}
            <Loader2 class="spin" size={15} />
          {/if}
          {status}
        </div>
      {/if}
    </section>
  </main>
</div>
