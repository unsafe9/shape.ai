import { useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { createRoot } from "react-dom/client";
import {
  Activity,
  BringToFront,
  ClipboardPaste,
  Copy,
  Crosshair,
  Database,
  FileJson,
  Gauge,
  Layers,
  MessageSquare,
  MousePointer2,
  Play,
  Plus,
  RefreshCcw,
  ScanLine,
  SendToBack,
  Tags,
  Trash2,
  Type,
  Workflow
} from "lucide-react";
import { exportTypeLabels, sceneGraphForGroup } from "../../../../src/shared/graph";
import type { ExportOutput, ExportType, Scene, SceneGroup, SceneSelection } from "../../../../src/shared/schema";
import { generateLocalExport } from "../../../../src/server/local";
import {
  addShapeSceneComment,
  applyRenderPatchToShapeScene,
  createShapeSceneFixture,
  shapeSceneToFilteredRenderSnapshot,
  updateShapeSceneGroupTags,
  type AppliedRenderPatch
} from "./adapter";
import { createAppComment, exportAppGroup, fetchAppScene, saveAppScenePatch, updateAppGroupTags } from "./appSceneApi";
import { runScriptedPanZoom, type BenchmarkResult } from "./benchmark";
import { ShapeCanvasEngine, type EngineEvent } from "./engine";
import { createBenchmarkFixture, createSmallFixture } from "./fixtures";
import { loadRustCore, type RustCoreStatus, type RustWebGpuProbeReport, type RustWebGpuRenderer } from "./wasmLoader";
import type { FrameStats, HitResult, RenderCard, RenderGroup, SceneSnapshot } from "./scene";
import "./styles.css";

type AppSceneMode = "fixture" | "api" | null;
type PocExportPreview = ExportOutput & { type: ExportType; contentType?: string };

function InfiniteCanvasPoc() {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const webGpuCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const overlayRef = useRef<HTMLDivElement | null>(null);
  const engineRef = useRef<ShapeCanvasEngine | null>(null);
  const appSceneRef = useRef<Scene | null>(null);
  const appSceneModeRef = useRef<AppSceneMode>(null);
  const [snapshot, setSnapshot] = useState<SceneSnapshot>(() => createBenchmarkFixture());
  const [appScene, setAppScene] = useState<Scene | null>(null);
  const [appSceneMode, setAppSceneMode] = useState<AppSceneMode>(null);
  const [activeTagIds, setActiveTagIds] = useState<string[]>([]);
  const [stats, setStats] = useState<FrameStats | null>(null);
  const [selection, setSelection] = useState<HitResult | null>(null);
  const [status, setStatus] = useState("Ready");
  const [copiedCard, setCopiedCard] = useState<RenderCard | null>(null);
  const [commentDraft, setCommentDraft] = useState("");
  const [exportPreview, setExportPreview] = useState<PocExportPreview | null>(null);
  const [benchmark, setBenchmark] = useState<BenchmarkResult | null>(null);
  const [webGpuRenderer, setWebGpuRenderer] = useState<RustWebGpuRenderer | null>(null);
  const [webGpuRendererDetail, setWebGpuRendererDetail] = useState("Visible WebGPU renderer has not been created.");
  const [webGpuProbe, setWebGpuProbe] = useState<RustWebGpuProbeReport | null>(null);
  const [webGpuProbeError, setWebGpuProbeError] = useState<string | null>(null);
  const [rustStatus, setRustStatus] = useState<RustCoreStatus>({
    available: false,
    backend: "detecting",
    detail: "Checking generated Rust/WASM package.",
    probeWebGpu: null,
    createWebGpuRenderer: null
  });

  const cardsPerEdge = useMemo(() => {
    if (snapshot.edges.length === 0) return "n/a";
    return (snapshot.cards.length / snapshot.edges.length).toFixed(2);
  }, [snapshot.cards.length, snapshot.edges.length]);

  const selectedCard = useMemo(() => cardForSelection(snapshot, selection), [snapshot, selection]);
  const selectedGroup = useMemo(() => groupForSelection(appScene, snapshot, selection), [appScene, snapshot, selection]);
  const selectedTarget = useMemo(() => selectionForShell(snapshot, selection), [snapshot, selection]);
  const selectedComments = useMemo(
    () =>
      appScene && selectedTarget.kind !== "canvas"
        ? appScene.comments.filter((comment) => selectionsEqual(comment.target, selectedTarget))
        : [],
    [appScene, selectedTarget]
  );

  useEffect(() => {
    let disposed = false;
    void loadRustCore().then((next) => {
      if (!disposed) setRustStatus(next);
    });
    return () => {
      disposed = true;
    };
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !rustStatus.probeWebGpu) {
      setWebGpuProbe(null);
      setWebGpuProbeError(null);
      return;
    }
    let disposed = false;
    setWebGpuProbe(null);
    setWebGpuProbeError(null);
    const rect = canvas.getBoundingClientRect();
    const probeCanvas = document.createElement("canvas");
    probeCanvas.width = Math.max(1, Math.round((rect.width || 1) * (window.devicePixelRatio || 1)));
    probeCanvas.height = Math.max(1, Math.round((rect.height || 1) * (window.devicePixelRatio || 1)));
    void rustStatus
      .probeWebGpu(probeCanvas, rect.width || 1, rect.height || 1, window.devicePixelRatio || 1)
      .then((report) => {
        if (!disposed) setWebGpuProbe(report);
      })
      .catch((error) => {
        if (!disposed) setWebGpuProbeError(error instanceof Error ? error.message : "WebGPU probe failed");
      });
    return () => {
      disposed = true;
    };
  }, [rustStatus.probeWebGpu]);

  useEffect(() => {
    const webGpuCanvas = webGpuCanvasRef.current;
    const sizeSourceCanvas = canvasRef.current;
    if (!webGpuCanvas || !sizeSourceCanvas || !rustStatus.createWebGpuRenderer) {
      setWebGpuRenderer(null);
      setWebGpuRendererDetail("Visible WebGPU renderer has not been created.");
      return;
    }
    let disposed = false;
    setWebGpuRenderer(null);
    setWebGpuRendererDetail("Creating visible Rust/wgpu renderer.");
    const rect = sizeSourceCanvas.getBoundingClientRect();
    void rustStatus
      .createWebGpuRenderer(webGpuCanvas, rect.width || 1, rect.height || 1, window.devicePixelRatio || 1)
      .then((renderer) => {
        if (disposed) return;
        setWebGpuRenderer(renderer);
        setWebGpuRendererDetail("Visible Rust/wgpu renderer created for group/card/edge primitives.");
      })
      .catch((error) => {
        if (disposed) return;
        setWebGpuRenderer(null);
        setWebGpuRendererDetail(error instanceof Error ? error.message : "Visible Rust/wgpu renderer failed to initialize.");
      });
    return () => {
      disposed = true;
    };
  }, [rustStatus.createWebGpuRenderer]);

  useEffect(() => {
    appSceneRef.current = appScene;
  }, [appScene]);

  useEffect(() => {
    appSceneModeRef.current = appSceneMode;
  }, [appSceneMode]);

  useEffect(() => {
    const canvas = canvasRef.current;
    const overlayRoot = overlayRef.current;
    if (!canvas || !overlayRoot) return;
    const engine = new ShapeCanvasEngine({
      canvas,
      overlayRoot,
      backend: rustStatus.available ? rustStatus.backend : "webgpu-wasm-unavailable",
      webGpuRenderer,
      onEvent: (event: EngineEvent) => {
        if (event.type === "stats") setStats(event.stats);
        if (event.type === "selection") {
          setSelection(event.hit);
          const currentAppScene = appSceneRef.current;
          if (currentAppScene) {
            const applied = applyRenderPatchToShapeScene(currentAppScene, { kind: "select", selection: hitToSceneSelection(event.hit) });
            if (applied.errors.length === 0) {
              commitAppScenePatch(applied, "select", { updateSnapshot: false, persist: false });
            }
          }
        }
        if (event.type === "status") setStatus(event.message);
        if (event.type === "patch") {
          if (event.errors.length > 0) {
            setStatus(event.errors.join("; "));
            return;
          }
          const currentAppScene = appSceneRef.current;
          if (currentAppScene) {
            const applied = applyRenderPatchToShapeScene(currentAppScene, event.patch);
            if (applied.errors.length > 0) {
              setStatus(applied.errors.join("; "));
              return;
            }
            commitAppScenePatch(applied, event.patch.kind, { updateSnapshot: true });
            return;
          }
          setStatus(`Patch applied: ${event.patch.kind}`);
          const nextSnapshot = engine.getSnapshot();
          if (nextSnapshot) setSnapshot(nextSnapshot);
        }
      }
    });
    engineRef.current = engine;

    const resize = () => {
      const rect = canvas.getBoundingClientRect();
      engine.resize(rect.width, rect.height, window.devicePixelRatio || 1);
    };
    const observer = new ResizeObserver(resize);
    observer.observe(canvas);
    resize();
    engine.loadScene(snapshot);
    engine.start();
    return () => {
      observer.disconnect();
      engine.stop();
      engineRef.current = null;
    };
  }, [rustStatus.available, rustStatus.backend, webGpuRenderer]);

  useEffect(() => {
    engineRef.current?.loadScene(snapshot);
  }, [snapshot.sceneId]);

  function loadSmallFixture() {
    const next = createSmallFixture();
    appSceneRef.current = null;
    appSceneModeRef.current = null;
    setAppScene(null);
    setAppSceneMode(null);
    setActiveTagIds([]);
    setExportPreview(null);
    setSnapshot(next);
    setBenchmark(null);
    setStatus("Loaded small interaction fixture");
  }

  function loadBenchmarkFixture() {
    const next = createBenchmarkFixture({ seed: 42, groups: 6, cards: 1_240, edges: 1_320 });
    appSceneRef.current = null;
    appSceneModeRef.current = null;
    setAppScene(null);
    setAppSceneMode(null);
    setActiveTagIds([]);
    setExportPreview(null);
    setSnapshot(next);
    setBenchmark(null);
    setStatus("Loaded 1,240 card benchmark fixture");
  }

  function loadShapeSceneFixture() {
    const scene = createShapeSceneFixture();
    appSceneRef.current = scene;
    appSceneModeRef.current = "fixture";
    setAppScene(scene);
    setAppSceneMode("fixture");
    setActiveTagIds([]);
    setExportPreview(null);
    setSnapshot(shapeSceneToFilteredRenderSnapshot(scene, []));
    setBenchmark(null);
    setStatus(`Loaded Shape app scene fixture v${scene.sceneVersion}`);
  }

  async function loadRealAppScene() {
    setStatus("Loading real app scene from /api/scene");
    try {
      const scene = await fetchAppScene();
      appSceneRef.current = scene;
      appSceneModeRef.current = "api";
      setAppScene(scene);
      setAppSceneMode("api");
      setActiveTagIds([]);
      setExportPreview(null);
      setSnapshot(shapeSceneToFilteredRenderSnapshot(scene, []));
      setBenchmark(null);
      setStatus(`Loaded real app scene v${scene.sceneVersion}`);
    } catch (error) {
      setStatus(error instanceof Error ? `Real scene load failed: ${error.message}` : "Real scene load failed");
    }
  }

  function commitAppScenePatch(applied: AppliedRenderPatch, kind: string, options: { updateSnapshot: boolean; persist?: boolean }) {
    appSceneRef.current = applied.scene;
    setAppScene(applied.scene);
    if (options.updateSnapshot) setSnapshot(shapeSceneToFilteredRenderSnapshot(applied.scene, activeTagIds));
    if (options.persist === false) return;
    if (appSceneModeRef.current !== "api") {
      setStatus(`App patch persisted in memory: ${kind} -> v${applied.scene.sceneVersion}`);
      return;
    }
    setStatus(`Saving app patch: ${kind}`);
    void saveAppScenePatch(applied.appPatch)
      .then((scene) => {
        appSceneRef.current = scene;
        setAppScene(scene);
        if (options.updateSnapshot) setSnapshot(shapeSceneToFilteredRenderSnapshot(scene, activeTagIds));
        setStatus(`App patch saved: ${kind} -> v${scene.sceneVersion}`);
      })
      .catch((error) => {
        setStatus(error instanceof Error ? `App patch save failed: ${error.message}` : "App patch save failed");
      });
  }

  async function runBenchmark() {
    if (!engineRef.current) return;
    setStatus("Running scripted pan/zoom benchmark");
    const result = await runScriptedPanZoom(engineRef.current, 150);
    setBenchmark(result);
    setStatus(`Benchmark complete: avg ${result.averageFrameMs.toFixed(2)}ms, p95 ${result.p95FrameMs.toFixed(2)}ms`);
  }

  function fitScene() {
    engineRef.current?.fitScene();
    setStatus("Fit scene");
  }

  function exportSnapshot() {
    const blob = new Blob([JSON.stringify(snapshot, null, 2)], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `${snapshot.sceneId}.render-scene.json`;
    link.click();
    URL.revokeObjectURL(url);
    setStatus("Render scene snapshot exported");
  }

  function createNodeFromViewport() {
    const engine = engineRef.current;
    if (!engine) return;
    const current = engine.getSnapshot() ?? snapshot;
    const groupId = groupIdForNewCard(current, selection);
    const group = current.groups.find((candidate) => candidate.id === groupId) ?? current.groups[0];
    if (!group) {
      setStatus("Create node failed: no group in scene");
      return;
    }
    const camera = engine.getCamera();
    const rect = canvasRef.current?.getBoundingClientRect();
    const centerX = rect ? (rect.width * 0.5 - camera.x) / camera.zoom : group.bounds.x + 180;
    const centerY = rect ? (rect.height * 0.5 - camera.y) / camera.zoom : group.bounds.y + 180;
    const card: RenderCard = {
      id: `poc-node-${Date.now().toString(36)}`,
      groupId: group.id,
      title: "New canvas node",
      summary: "Created through the renderer patch boundary.",
      detail: "This node is translated back into app ScenePatch nodes when an app scene is loaded.",
      status: "draft",
      type: "task",
      bounds: {
        x: Math.round(centerX - 160),
        y: Math.round(centerY - 86),
        width: 320,
        height: 172
      },
      zIndex: nextCardZ(current, group.id),
      styleKey: "decision",
      accessibilityLabel: "task New canvas node. Created through the renderer patch boundary."
    };
    setBenchmark(null);
    engine.applyPatch({ kind: "create-card", card });
  }

  function createGroupFromViewport() {
    const engine = engineRef.current;
    if (!engine) return;
    const current = engine.getSnapshot() ?? snapshot;
    const camera = engine.getCamera();
    const rect = canvasRef.current?.getBoundingClientRect();
    const centerX = rect ? (rect.width * 0.5 - camera.x) / camera.zoom : 360;
    const centerY = rect ? (rect.height * 0.5 - camera.y) / camera.zoom : 260;
    const group: RenderGroup = {
      id: `poc-group-${Date.now().toString(36)}`,
      title: "New canvas group",
      summary: "Created through the renderer patch boundary.",
      bounds: {
        x: Math.round(centerX - 520),
        y: Math.round(centerY - 340),
        width: 1040,
        height: 680
      },
      tagIds: [],
      zIndex: nextGroupZ(current),
      styleKey: "default"
    };
    setBenchmark(null);
    engine.applyPatch({ kind: "create-group", group });
  }

  function toggleTagFilter(tagId: string) {
    setActiveTagIds((current) => {
      const next = current.includes(tagId) ? current.filter((id) => id !== tagId) : [...current, tagId];
      const currentAppScene = appSceneRef.current;
      if (currentAppScene) setSnapshot(shapeSceneToFilteredRenderSnapshot(currentAppScene, next));
      return next;
    });
  }

  function toggleSelectedGroupTag(tagId: string) {
    const currentAppScene = appSceneRef.current;
    if (!currentAppScene || !selectedGroup) {
      setStatus("Tag attach skipped: select a group or node");
      return;
    }
    const nextTagIds = selectedGroup.tagIds.includes(tagId) ? selectedGroup.tagIds.filter((id) => id !== tagId) : [...selectedGroup.tagIds, tagId];
    const applied = updateShapeSceneGroupTags(currentAppScene, selectedGroup.id, nextTagIds, new Date().toISOString());
    if (applied.errors.length > 0) {
      setStatus(applied.errors.join("; "));
      return;
    }
    appSceneRef.current = applied.scene;
    setAppScene(applied.scene);
    setSnapshot(shapeSceneToFilteredRenderSnapshot(applied.scene, activeTagIds));
    if (appSceneModeRef.current !== "api") {
      setStatus("Group tags updated in memory");
      return;
    }
    setStatus("Saving group tags");
    void updateAppGroupTags(selectedGroup.id, nextTagIds)
      .then(({ scene }) => {
        appSceneRef.current = scene;
        setAppScene(scene);
        setSnapshot(shapeSceneToFilteredRenderSnapshot(scene, activeTagIds));
        setStatus(`Group tags saved -> v${scene.sceneVersion}`);
      })
      .catch((error) => {
        setStatus(error instanceof Error ? `Group tag save failed: ${error.message}` : "Group tag save failed");
      });
  }

  async function addSelectedComment() {
    const currentAppScene = appSceneRef.current;
    if (!currentAppScene || selectedTarget.kind === "canvas") {
      setStatus("Comment skipped: select a group, node, or edge");
      return;
    }
    const body = commentDraft.trim();
    if (!body) {
      setStatus("Comment skipped: empty body");
      return;
    }
    if (appSceneModeRef.current === "api") {
      setStatus("Saving comment");
      try {
        const { scene } = await createAppComment({ target: selectedTarget, body });
        appSceneRef.current = scene;
        setAppScene(scene);
        setSnapshot(shapeSceneToFilteredRenderSnapshot(scene, activeTagIds));
        setCommentDraft("");
        setStatus(`Comment saved -> v${scene.sceneVersion}`);
      } catch (error) {
        setStatus(error instanceof Error ? `Comment save failed: ${error.message}` : "Comment save failed");
      }
      return;
    }
    const applied = addShapeSceneComment(currentAppScene, selectedTarget, body, new Date().toISOString());
    if (applied.errors.length > 0) {
      setStatus(applied.errors.join("; "));
      return;
    }
    appSceneRef.current = applied.scene;
    setAppScene(applied.scene);
    setSnapshot(shapeSceneToFilteredRenderSnapshot(applied.scene, activeTagIds));
    setCommentDraft("");
    setStatus("Comment added in memory");
  }

  async function exportSelectedGroup(type: ExportType) {
    const currentAppScene = appSceneRef.current;
    if (!currentAppScene || !selectedGroup) {
      setStatus("Export skipped: select a group or node");
      return;
    }
    const scope = { kind: "group" as const, id: selectedGroup.id };
    if (appSceneModeRef.current === "api") {
      setStatus(`Exporting ${exportTypeLabels[type]}`);
      try {
        const result = await exportAppGroup(selectedGroup.id, type, scope);
        appSceneRef.current = result.scene;
        setAppScene(result.scene);
        setSnapshot(shapeSceneToFilteredRenderSnapshot(result.scene, activeTagIds));
        setExportPreview(result.preview);
        setStatus(`Export saved: ${result.artifact.title}`);
      } catch (error) {
        setStatus(error instanceof Error ? `Export failed: ${error.message}` : "Export failed");
      }
      return;
    }
    const graph = sceneGraphForGroup(currentAppScene, selectedGroup.id);
    const generated = generateLocalExport(graph, { type, scope }, selectedGroup.title);
    setExportPreview({ ...generated, type, contentType: type === "mermaid" ? "text/plain; charset=utf-8" : "text/markdown; charset=utf-8" });
    setStatus(`Export preview generated: ${exportTypeLabels[type]}`);
  }

  async function copySelectedNode() {
    if (!selectedCard) {
      setStatus("Copy skipped: select a node");
      return;
    }
    setCopiedCard(selectedCard);
    const copiedToClipboard = await writeClipboardText(renderCardMarkdown(selectedCard));
    setStatus(copiedToClipboard ? "Copied node as Markdown" : "Copied node locally");
  }

  function duplicateSelectedNode() {
    if (!selectedCard) {
      setStatus("Duplicate skipped: select a node");
      return;
    }
    pasteCard(selectedCard, selectedCard);
  }

  function pasteCopiedNode() {
    if (!copiedCard) {
      setStatus("Paste skipped: no copied node");
      return;
    }
    pasteCard(copiedCard, selectedCard ?? copiedCard);
  }

  function pasteCard(source: RenderCard, anchor: RenderCard) {
    const engine = engineRef.current;
    const current = engine?.getSnapshot() ?? snapshot;
    if (!engine) return;
    const card = cloneCardForPaste(source, anchor, current);
    setBenchmark(null);
    engine.applyPatch({ kind: "create-card", card });
  }

  function moveSelectedNodeLayer(direction: "front" | "back") {
    const engine = engineRef.current;
    const current = engine?.getSnapshot() ?? snapshot;
    if (!engine || !selectedCard) {
      setStatus("Layer move skipped: select a node");
      return;
    }
    const groupZ = current.cards.filter((card) => card.groupId === selectedCard.groupId).map((card) => card.zIndex);
    const zIndex = direction === "front" ? Math.max(0, ...groupZ) + 1 : Math.min(0, ...groupZ) - 1;
    setBenchmark(null);
    engine.applyPatch({ kind: "set-card-z-index", id: selectedCard.id, zIndex });
  }

  function deleteSelectedObject() {
    const engine = engineRef.current;
    if (!engine) return;
    const selected = selection ?? snapshotSelectionToHit(snapshot);
    if (!selected) {
      setStatus("Delete skipped: canvas is selected");
      return;
    }
    setBenchmark(null);
    if (selected.kind === "edge") {
      engine.applyPatch({ kind: "delete-edge", id: selected.id });
      return;
    }
    if (selected.kind === "card" || selected.kind === "text" || selected.kind === "port") {
      engine.applyPatch({ kind: "delete-card", id: selected.id });
      return;
    }
    if (selected.kind === "group") {
      engine.applyPatch({ kind: "delete-group", id: selected.id });
      return;
    }
    setStatus("Delete skipped: unknown selection");
  }

  return (
    <main className="poc-shell">
      <section className="poc-canvas-region" aria-label="Infinite canvas engine prototype">
        <canvas
          ref={webGpuCanvasRef}
          className="poc-canvas poc-webgpu-canvas is-active"
          aria-label="Rust WebGPU retained scene canvas"
        />
        <canvas
          ref={canvasRef}
          className="poc-canvas poc-input-canvas is-input-only"
          aria-label="WebGPU input surface"
        />
        <div ref={overlayRef} className="poc-overlay-root" />
      </section>

      <aside className="poc-sidebar" aria-label="Prototype controls and evidence">
        <header className="poc-header">
          <div>
            <p className="poc-kicker">POC</p>
            <h1>Infinite Canvas Engine</h1>
          </div>
          <span className={webGpuRenderer ? "poc-status is-ok" : "poc-status"}>{webGpuRenderer ? "WebGPU" : "WASM required"}</span>
        </header>

        <div className="poc-toolbar" aria-label="Fixture controls">
          <button type="button" onClick={loadSmallFixture}>
            <Layers size={16} />
            Small
          </button>
          <button type="button" onClick={loadBenchmarkFixture}>
            <Gauge size={16} />
            1k fixture
          </button>
          <button type="button" onClick={loadShapeSceneFixture}>
            <Workflow size={16} />
            Shape scene
          </button>
          <button type="button" onClick={() => void loadRealAppScene()}>
            <Database size={16} />
            Real scene
          </button>
          <button type="button" onClick={fitScene}>
            <Crosshair size={16} />
            Fit
          </button>
          <button type="button" onClick={createNodeFromViewport}>
            <Plus size={16} />
            Node
          </button>
          <button type="button" onClick={createGroupFromViewport}>
            <Layers size={16} />
            Group
          </button>
          <button type="button" onClick={() => void copySelectedNode()} disabled={!selectedCard}>
            <Copy size={16} />
            Copy
          </button>
          <button type="button" onClick={pasteCopiedNode} disabled={!copiedCard}>
            <ClipboardPaste size={16} />
            Paste
          </button>
          <button type="button" onClick={duplicateSelectedNode} disabled={!selectedCard}>
            <Copy size={16} />
            Duplicate
          </button>
          <button type="button" onClick={() => moveSelectedNodeLayer("front")} disabled={!selectedCard}>
            <BringToFront size={16} />
            Front
          </button>
          <button type="button" onClick={() => moveSelectedNodeLayer("back")} disabled={!selectedCard}>
            <SendToBack size={16} />
            Back
          </button>
          <button type="button" onClick={deleteSelectedObject}>
            <Trash2 size={16} />
            Delete
          </button>
          <button type="button" onClick={() => void runBenchmark()}>
            <Play size={16} />
            Benchmark
          </button>
          <button type="button" onClick={exportSnapshot}>
            <FileJson size={16} />
            Snapshot
          </button>
        </div>

        <section className="poc-panel">
          <h2>
            <Activity size={15} />
            Frame Stats
          </h2>
          <dl className="poc-stats">
            <Metric label="Frame" value={stats ? `${stats.frameMs.toFixed(2)} ms` : "-"} />
            <Metric label="Visible" value={stats ? `${stats.visibleGroups}g / ${stats.visibleCards}c / ${stats.visibleEdges}e` : "-"} />
            <Metric label="Total" value={`${snapshot.groups.length}g / ${snapshot.cards.length}c / ${snapshot.edges.length}e`} />
            <Metric label="Cache" value={stats ? `${stats.cacheHits} hit / ${stats.cacheMisses} miss` : "-"} />
            <Metric label="Boundary" value={stats ? `${stats.boundaryCalls} calls` : "-"} />
            <Metric label="Draw" value={stats?.drawBackend ?? "rust-wgpu-visible"} />
            <Metric label="Rust boundary" value={stats ? `${stats.rustBoundaryCalls} calls` : "-"} />
            <Metric
              label="Rust frame"
              value={stats?.rustFrameCards === null || stats?.rustFrameCards === undefined ? "-" : `${stats.rustFrameCards}c / ${stats.rustFrameEdges ?? 0}e`}
            />
            <Metric label="GPU vertices" value={stats?.rustGpuVertices === null || stats?.rustGpuVertices === undefined ? "-" : String(stats.rustGpuVertices)} />
            <Metric label="Drawn vertices" value={stats?.rustDrawnVertices === null || stats?.rustDrawnVertices === undefined ? "-" : String(stats.rustDrawnVertices)} />
            <Metric label="Draw ranges" value={stats?.rustDrawRanges === null || stats?.rustDrawRanges === undefined ? "-" : String(stats.rustDrawRanges)} />
            <Metric label="GPU glyphs" value={stats?.rustTextGlyphs === null || stats?.rustTextGlyphs === undefined ? "-" : String(stats.rustTextGlyphs)} />
            <Metric label="Fallback glyphs" value={stats?.rustFallbackGlyphs === null || stats?.rustFallbackGlyphs === undefined ? "-" : String(stats.rustFallbackGlyphs)} />
            <Metric label="CJK glyphs" value={stats?.rustCjkGlyphs === null || stats?.rustCjkGlyphs === undefined ? "-" : String(stats.rustCjkGlyphs)} />
            <Metric
              label="Text cache"
              value={
                stats?.rustTextLayoutCacheHits === null || stats?.rustTextLayoutCacheHits === undefined
                  ? "-"
                  : `${stats.rustTextLayoutCacheHits} hit / ${stats.rustTextLayoutCacheMisses ?? 0} miss`
              }
            />
            <Metric label="Style tokens" value={stats?.rustStyleTokens === null || stats?.rustStyleTokens === undefined ? "-" : String(stats.rustStyleTokens)} />
            <Metric label="Camera flushes" value={stats?.rustCameraFlushes === null || stats?.rustCameraFlushes === undefined ? "-" : String(stats.rustCameraFlushes)} />
            <Metric label="GPU patches" value={stats?.rustPatchUpdates === null || stats?.rustPatchUpdates === undefined ? "-" : String(stats.rustPatchUpdates)} />
            <Metric label="GPU dirty" value={stats?.rustDirtyWrites === null || stats?.rustDirtyWrites === undefined ? "-" : String(stats.rustDirtyWrites)} />
            <Metric label="GPU rebuilds" value={stats?.rustFullRebuilds === null || stats?.rustFullRebuilds === undefined ? "-" : String(stats.rustFullRebuilds)} />
            <Metric label="Edge grows" value={stats?.rustEdgeCapacityGrows === null || stats?.rustEdgeCapacityGrows === undefined ? "-" : String(stats.rustEdgeCapacityGrows)} />
            <Metric label="Edge compacts" value={stats?.rustEdgeCompactions === null || stats?.rustEdgeCompactions === undefined ? "-" : String(stats.rustEdgeCompactions)} />
            <Metric label="Card grows" value={stats?.rustCardCapacityGrows === null || stats?.rustCardCapacityGrows === undefined ? "-" : String(stats.rustCardCapacityGrows)} />
            <Metric label="Card compacts" value={stats?.rustCardCompactions === null || stats?.rustCardCompactions === undefined ? "-" : String(stats.rustCardCompactions)} />
            <Metric label="Group grows" value={stats?.rustGroupCapacityGrows === null || stats?.rustGroupCapacityGrows === undefined ? "-" : String(stats.rustGroupCapacityGrows)} />
            <Metric label="Group compacts" value={stats?.rustGroupCompactions === null || stats?.rustGroupCompactions === undefined ? "-" : String(stats.rustGroupCompactions)} />
            <Metric
              label="Edge slots"
              value={
                stats?.rustEdgeSlots === null || stats?.rustEdgeSlots === undefined
                  ? "-"
                  : `${stats.rustEdgeSlots - (stats.rustFreeEdgeSlots ?? 0)} used / ${stats.rustFreeEdgeSlots ?? 0} free`
              }
            />
            <Metric
              label="Card slots"
              value={
                stats?.rustCardSlots === null || stats?.rustCardSlots === undefined
                  ? "-"
                  : `${stats.rustCardSlots - (stats.rustFreeCardSlots ?? 0)} used / ${stats.rustFreeCardSlots ?? 0} free`
              }
            />
            <Metric
              label="Group slots"
              value={
                stats?.rustGroupSlots === null || stats?.rustGroupSlots === undefined
                  ? "-"
                  : `${stats.rustGroupSlots - (stats.rustFreeGroupSlots ?? 0)} used / ${stats.rustFreeGroupSlots ?? 0} free`
              }
            />
            <Metric label="Memory" value={stats?.memoryBytes ? `${Math.round(stats.memoryBytes / 1024 / 1024)} MB` : "not exposed"} />
          </dl>
        </section>

        <section className="poc-panel">
          <h2>
            <MousePointer2 size={15} />
            Selection
          </h2>
          <p className="poc-readout">{selection ? `${selection.kind}: ${selection.id}${selection.field ? `:${selection.field}` : ""}` : "Canvas"}</p>
          <p className="poc-help">Double-click card text to open the DOM edit overlay. Drag source port to target port to create an edge.</p>
        </section>

        {appScene?.tags.length ? (
          <section className="poc-panel">
            <h2>
              <Tags size={15} />
              Tags
            </h2>
            <div className="poc-tag-label">Filter</div>
            <div className="poc-tag-list">
              {appScene.tags.map((tag) => (
                <button
                  key={tag.id}
                  type="button"
                  className={`poc-tag-chip${activeTagIds.includes(tag.id) ? " is-active" : ""}`}
                  style={{ "--tag-color": tag.color } as CSSProperties}
                  onClick={() => toggleTagFilter(tag.id)}
                >
                  {tag.name}
                </button>
              ))}
            </div>
            {selectedGroup ? (
              <>
                <div className="poc-tag-label">Selected group</div>
                <div className="poc-tag-list">
                  {appScene.tags.map((tag) => (
                    <button
                      key={tag.id}
                      type="button"
                      className={`poc-tag-chip${selectedGroup.tagIds.includes(tag.id) ? " is-attached" : ""}`}
                      style={{ "--tag-color": tag.color } as CSSProperties}
                      onClick={() => toggleSelectedGroupTag(tag.id)}
                    >
                      {tag.name}
                    </button>
                  ))}
                </div>
              </>
            ) : null}
          </section>
        ) : null}

        {appScene ? (
          <section className="poc-panel">
            <h2>
              <MessageSquare size={15} />
              Comments
            </h2>
            <textarea
              className="poc-comment-input"
              value={commentDraft}
              onChange={(event) => setCommentDraft(event.target.value)}
              disabled={selectedTarget.kind === "canvas"}
              placeholder={selectedTarget.kind === "canvas" ? "Select an object" : "Add comment"}
            />
            <div className="poc-inline-actions">
              <button type="button" onClick={() => void addSelectedComment()} disabled={selectedTarget.kind === "canvas" || !commentDraft.trim()}>
                <MessageSquare size={16} />
                Comment
              </button>
            </div>
            <div className="poc-comment-list">
              {selectedComments.map((comment) => (
                <p key={comment.id}>{comment.body}</p>
              ))}
              {selectedComments.length === 0 ? <p className="poc-help">No comments for selection.</p> : null}
            </div>
          </section>
        ) : null}

        {appScene ? (
          <section className="poc-panel">
            <h2>
              <FileJson size={15} />
              Product Export
            </h2>
            <div className="poc-inline-actions">
              <button type="button" onClick={() => void exportSelectedGroup("mermaid")} disabled={!selectedGroup}>
                <FileJson size={16} />
                Mermaid
              </button>
              <button type="button" onClick={() => void exportSelectedGroup("madr")} disabled={!selectedGroup}>
                <FileJson size={16} />
                MADR
              </button>
            </div>
            {exportPreview ? (
              <pre className="poc-export-preview">{exportPreview.content}</pre>
            ) : (
              <p className="poc-help">Export preview uses app Scene graph data, not the render snapshot.</p>
            )}
          </section>
        ) : null}

        <section className="poc-panel">
          <h2>
            <ScanLine size={15} />
            Benchmark
          </h2>
          {benchmark ? (
            <dl className="poc-stats">
              <Metric label="Frames" value={String(benchmark.frames)} />
              <Metric label="Average" value={`${benchmark.averageFrameMs.toFixed(2)} ms`} />
              <Metric label="P95" value={`${benchmark.p95FrameMs.toFixed(2)} ms`} />
              <Metric label="Max" value={`${benchmark.maxFrameMs.toFixed(2)} ms`} />
              <Metric label="Visible cards" value={benchmark.averageVisibleCards.toFixed(1)} />
              <Metric label="Draw" value={benchmark.drawBackend} />
              <Metric label="Rust boundary" value={`${benchmark.rustBoundaryCalls} calls`} />
              <Metric label="GPU vertices" value={benchmark.rustGpuVertices === null ? "-" : String(benchmark.rustGpuVertices)} />
              <Metric label="Drawn vertices" value={benchmark.rustDrawnVertices === null ? "-" : String(benchmark.rustDrawnVertices)} />
              <Metric label="Draw ranges" value={benchmark.rustDrawRanges === null ? "-" : String(benchmark.rustDrawRanges)} />
              <Metric label="GPU glyphs" value={benchmark.rustTextGlyphs === null ? "-" : String(benchmark.rustTextGlyphs)} />
              <Metric label="Fallback glyphs" value={benchmark.rustFallbackGlyphs === null ? "-" : String(benchmark.rustFallbackGlyphs)} />
              <Metric label="CJK glyphs" value={benchmark.rustCjkGlyphs === null ? "-" : String(benchmark.rustCjkGlyphs)} />
              <Metric
                label="Text cache"
                value={
                  benchmark.rustTextLayoutCacheHits === null
                    ? "-"
                    : `${benchmark.rustTextLayoutCacheHits} hit / ${benchmark.rustTextLayoutCacheMisses ?? 0} miss`
                }
              />
              <Metric label="Style tokens" value={benchmark.rustStyleTokens === null ? "-" : String(benchmark.rustStyleTokens)} />
              <Metric label="Camera flushes" value={benchmark.rustCameraFlushes === null ? "-" : String(benchmark.rustCameraFlushes)} />
              <Metric label="GPU patches" value={benchmark.rustPatchUpdates === null ? "-" : String(benchmark.rustPatchUpdates)} />
              <Metric label="GPU dirty" value={benchmark.rustDirtyWrites === null ? "-" : String(benchmark.rustDirtyWrites)} />
              <Metric label="GPU rebuilds" value={benchmark.rustFullRebuilds === null ? "-" : String(benchmark.rustFullRebuilds)} />
              <Metric label="Edge grows" value={benchmark.rustEdgeCapacityGrows === null ? "-" : String(benchmark.rustEdgeCapacityGrows)} />
              <Metric label="Edge compacts" value={benchmark.rustEdgeCompactions === null ? "-" : String(benchmark.rustEdgeCompactions)} />
              <Metric label="Card grows" value={benchmark.rustCardCapacityGrows === null ? "-" : String(benchmark.rustCardCapacityGrows)} />
              <Metric label="Card compacts" value={benchmark.rustCardCompactions === null ? "-" : String(benchmark.rustCardCompactions)} />
              <Metric label="Group grows" value={benchmark.rustGroupCapacityGrows === null ? "-" : String(benchmark.rustGroupCapacityGrows)} />
              <Metric label="Group compacts" value={benchmark.rustGroupCompactions === null ? "-" : String(benchmark.rustGroupCompactions)} />
              <Metric
                label="Edge slots"
                value={
                  benchmark.rustEdgeSlots === null
                    ? "-"
                    : `${benchmark.rustEdgeSlots - (benchmark.rustFreeEdgeSlots ?? 0)} used / ${benchmark.rustFreeEdgeSlots ?? 0} free`
                }
              />
              <Metric
                label="Card slots"
                value={
                  benchmark.rustCardSlots === null
                    ? "-"
                    : `${benchmark.rustCardSlots - (benchmark.rustFreeCardSlots ?? 0)} used / ${benchmark.rustFreeCardSlots ?? 0} free`
                }
              />
              <Metric
                label="Group slots"
                value={
                  benchmark.rustGroupSlots === null
                    ? "-"
                    : `${benchmark.rustGroupSlots - (benchmark.rustFreeGroupSlots ?? 0)} used / ${benchmark.rustFreeGroupSlots ?? 0} free`
                }
              />
            </dl>
          ) : (
            <p className="poc-help">Run Benchmark to collect scripted pan/zoom frame timings on the loaded fixture.</p>
          )}
        </section>

        <section className="poc-panel">
          <h2>
            <Type size={15} />
            Contract
          </h2>
          <dl className="poc-stats">
            <Metric label="Scene" value={snapshot.sceneId} />
            <Metric label="Source" value={snapshot.metadata.source} />
            <Metric label="App version" value={appScene ? `v${appScene.sceneVersion}` : "-"} />
            <Metric label="App source" value={appSceneMode ?? "-"} />
            <Metric label="Cards / edge" value={cardsPerEdge} />
            <Metric label="Backend" value={stats?.backend ?? rustStatus.backend} />
            <Metric label="WebGPU renderer" value={stats?.webGpuRendererAvailable || webGpuRenderer ? "available" : "missing"} />
            <Metric label="WebGPU" value={webGpuProbeLabel(webGpuProbe, webGpuProbeError, rustStatus.available)} />
            <Metric label="Visible GPU" value={webGpuRenderer ? "available" : "missing"} />
            <Metric label="Surface" value={webGpuProbeSurface(webGpuProbe)} />
            <Metric label="Render pass" value={webGpuProbePass(webGpuProbe)} />
          </dl>
          <p className="poc-help">{webGpuProbeDetail(webGpuProbe, webGpuProbeError, webGpuRendererDetail || rustStatus.detail)}</p>
        </section>

        <footer className="poc-footer">
          <RefreshCcw size={14} />
          {status}
        </footer>
      </aside>
    </main>
  );
}

function hitToSceneSelection(hit: HitResult | null): SceneSelection {
  if (!hit) return { kind: "canvas" };
  if (hit.kind === "group") return { kind: "group", id: hit.id };
  if (hit.kind === "edge") return { kind: "edge", id: hit.id };
  return { kind: "node", id: hit.id };
}

function groupIdForNewCard(snapshot: SceneSnapshot, selection: HitResult | null): string {
  if (selection?.kind === "group") return selection.id;
  if (selection?.groupId) return selection.groupId;
  const selected = snapshot.selection;
  if (selected.kind === "group") return selected.id;
  if (selected.kind === "node") {
    return snapshot.cards.find((card) => card.id === selected.id)?.groupId ?? snapshot.groups[0]?.id ?? "poc-group";
  }
  if (selected.kind === "edge") {
    return snapshot.edges.find((edge) => edge.id === selected.id)?.groupId ?? snapshot.groups[0]?.id ?? "poc-group";
  }
  return snapshot.groups[0]?.id ?? "poc-group";
}

function nextCardZ(snapshot: SceneSnapshot, groupId: string): number {
  return Math.max(0, ...snapshot.cards.filter((card) => card.groupId === groupId).map((card) => card.zIndex)) + 1;
}

function nextGroupZ(snapshot: SceneSnapshot): number {
  return Math.max(0, ...snapshot.groups.map((group) => group.zIndex)) + 1;
}

function cardForSelection(snapshot: SceneSnapshot, hit: HitResult | null): RenderCard | null {
  const selected = snapshot.selection;
  const id = hit && (hit.kind === "card" || hit.kind === "text" || hit.kind === "port") ? hit.id : selected.kind === "node" ? selected.id : null;
  if (!id) return null;
  return snapshot.cards.find((card) => card.id === id) ?? null;
}

function groupForSelection(scene: Scene | null, snapshot: SceneSnapshot, hit: HitResult | null): SceneGroup | null {
  if (!scene) return null;
  let groupId: string | null = null;
  const selected = snapshot.selection;
  if (hit?.kind === "group") groupId = hit.id;
  else if (hit?.groupId) groupId = hit.groupId;
  else if (selected.kind === "group") groupId = selected.id;
  else if (selected.kind === "node") groupId = snapshot.cards.find((card) => card.id === selected.id)?.groupId ?? null;
  else if (selected.kind === "edge") groupId = snapshot.edges.find((edge) => edge.id === selected.id)?.groupId ?? null;
  return scene.groups.find((group) => group.id === groupId) ?? null;
}

function selectionForShell(snapshot: SceneSnapshot, hit: HitResult | null): SceneSelection {
  if (hit) {
    if (hit.kind === "group") return { kind: "group", id: hit.id };
    if (hit.kind === "edge") return { kind: "edge", id: hit.id };
    if (hit.kind === "card" || hit.kind === "text" || hit.kind === "port") return { kind: "node", id: hit.id };
  }
  return snapshot.selection;
}

function selectionsEqual(left: SceneSelection, right: SceneSelection): boolean {
  if (left.kind !== right.kind) return false;
  if (left.kind === "canvas" && right.kind === "canvas") return true;
  if (left.kind === "group" && right.kind === "group") return left.id === right.id;
  if (left.kind === "node" && right.kind === "node") return left.id === right.id;
  if (left.kind === "edge" && right.kind === "edge") return left.id === right.id;
  return false;
}

function cloneCardForPaste(source: RenderCard, anchor: RenderCard, snapshot: SceneSnapshot): RenderCard {
  const id = `poc-node-${Date.now().toString(36)}-${crypto.randomUUID().slice(0, 6)}`;
  const group = snapshot.groups.find((candidate) => candidate.id === source.groupId) ?? snapshot.groups[0];
  const groupId = group?.id ?? source.groupId;
  return {
    ...source,
    id,
    groupId,
    title: `${source.title} copy`,
    bounds: {
      ...source.bounds,
      x: anchor.bounds.x + 450,
      y: anchor.bounds.y + 430
    },
    zIndex: nextCardZ(snapshot, groupId),
    accessibilityLabel: `${source.type} ${source.title} copy. ${source.summary}`
  };
}

async function writeClipboardText(value: string): Promise<boolean> {
  try {
    await navigator.clipboard?.writeText(value);
    return true;
  } catch {
    return false;
  }
}

function renderCardMarkdown(card: RenderCard): string {
  return [`# ${card.title}`, "", `- **Type:** ${card.type}`, `- **Status:** ${card.status}`, "", "## Summary", card.summary || "_No summary recorded._", "", "## Detail", card.detail || "_No detail recorded._", ""].join("\n");
}

function snapshotSelectionToHit(snapshot: SceneSnapshot): HitResult | null {
  const selected = snapshot.selection;
  if (selected.kind === "canvas") return null;
  if (selected.kind === "group") {
    return { kind: "group", id: selected.id, groupId: selected.id, world: { x: 0, y: 0 }, screen: { x: 0, y: 0 } };
  }
  if (selected.kind === "edge") {
    const edge = snapshot.edges.find((candidate) => candidate.id === selected.id);
    return edge ? { kind: "edge", id: edge.id, groupId: edge.groupId, world: { x: 0, y: 0 }, screen: { x: 0, y: 0 } } : null;
  }
  const card = snapshot.cards.find((candidate) => candidate.id === selected.id);
  return card ? { kind: "card", id: card.id, groupId: card.groupId, world: { x: 0, y: 0 }, screen: { x: 0, y: 0 } } : null;
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <>
      <dt>{label}</dt>
      <dd>{value}</dd>
    </>
  );
}

function webGpuProbeLabel(report: RustWebGpuProbeReport | null, error: string | null, rustAvailable: boolean): string {
  if (error) return "error";
  if (report) return report.supported && report.presented ? "wgpu ready" : "unavailable";
  return rustAvailable ? "probing" : "not probed";
}

function webGpuProbeSurface(report: RustWebGpuProbeReport | null): string {
  if (!report?.surfaceConfigured) return "-";
  return `${report.format ?? "unknown"} / ${report.width}x${report.height}`;
}

function webGpuProbePass(report: RustWebGpuProbeReport | null): string {
  if (!report) return "-";
  return report.renderPassSubmitted && report.presented ? "submitted / presented" : "not submitted";
}

function webGpuProbeDetail(report: RustWebGpuProbeReport | null, error: string | null, fallback: string): string {
  if (error) return error;
  return report?.detail ?? fallback;
}

createRoot(document.getElementById("root")!).render(<InfiniteCanvasPoc />);
