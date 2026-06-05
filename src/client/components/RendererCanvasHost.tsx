import { forwardRef, useEffect, useImperativeHandle, useRef, useState, type MouseEvent } from "react";
import type { Scene, SceneSelection } from "../../shared/schema";
import type { CameraState } from "../../shared/renderScene";
import { shapeSceneToFilteredRenderSnapshot, type RenderScenePatch } from "../../shared/renderPatch";
import { ShapeCanvasEngine, type EngineEvent, type FocusBoundsOptions } from "../renderer/engine";
import type { FrameStats, HitResult, SceneSnapshot, WorldRect } from "../renderer/scene";
import { loadRustCore, type RustCoreStatus, type RustWebGpuRenderer } from "../renderer/wasmLoader";

export type RendererStats = FrameStats;

export type RendererHealth = {
  state: "ready" | "webgpu-unavailable" | "wasm-unavailable";
  detail: string;
  rustAvailable: boolean;
  rustBackend: string;
  webGpuRendererAvailable: boolean;
};

export type RendererCanvasHostHandle = {
  fitScene: () => void;
  focusBounds: (bounds: WorldRect, options?: FocusBoundsOptions) => void;
  wheelAtScreen: (screen: { x: number; y: number }, deltaY: number) => void;
  setCamera: (camera: CameraState) => void;
  applyPatch: (patch: RenderScenePatch) => string[];
  getSnapshot: () => SceneSnapshot | null;
};

type RendererCanvasHostProps = {
  scene: Scene | null;
  activeTagIds: string[];
  camera: CameraState;
  selection: SceneSelection;
  onCameraChange: (camera: CameraState) => void;
  onSelectionChange: (selection: SceneSelection) => void;
  onPatch: (patch: RenderScenePatch) => void;
  onGestureChange: (active: boolean) => void;
  onStats: (stats: RendererStats) => void;
  onStatus: (message: string) => void;
  onHealthChange: (health: RendererHealth) => void;
  onContextMenuRequest: (point: { x: number; y: number }) => void;
};

const initialRustStatus: RustCoreStatus = {
  available: false,
  backend: "detecting",
  detail: "Checking generated Rust/WASM package.",
  probeWebGpu: null,
  createWebGpuRenderer: null
};

export const RendererCanvasHost = forwardRef<RendererCanvasHostHandle, RendererCanvasHostProps>(function RendererCanvasHost(
  {
    scene,
    activeTagIds,
    camera,
    selection,
    onCameraChange,
    onSelectionChange,
    onPatch,
    onGestureChange,
    onStats,
    onStatus,
    onHealthChange,
    onContextMenuRequest
  },
  ref
) {
  const inputCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const webGpuCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const overlayRef = useRef<HTMLDivElement | null>(null);
  const engineRef = useRef<ShapeCanvasEngine | null>(null);
  const cameraRef = useRef<CameraState>(camera);
  const selectionRef = useRef<SceneSelection>(selection);
  const callbacksRef = useRef({ onCameraChange, onSelectionChange, onPatch, onGestureChange, onStats, onStatus });
  const [rustStatus, setRustStatus] = useState<RustCoreStatus>(initialRustStatus);
  const [webGpuRenderer, setWebGpuRenderer] = useState<RustWebGpuRenderer | null>(null);
  const [engineWebGpuAvailable, setEngineWebGpuAvailable] = useState<boolean | null>(null);
  const [engineWebGpuDetail, setEngineWebGpuDetail] = useState<string | null>(null);
  const [webGpuDetail, setWebGpuDetail] = useState("Visible Rust/wgpu renderer has not been created.");
  const [engineReadyKey, setEngineReadyKey] = useState(0);

  callbacksRef.current = { onCameraChange, onSelectionChange, onPatch, onGestureChange, onStats, onStatus };
  selectionRef.current = selection;

  useImperativeHandle(
    ref,
    () => ({
      fitScene: () => engineRef.current?.fitScene(),
      focusBounds: (bounds, options) => engineRef.current?.focusBounds(bounds, options),
      wheelAtScreen: (screen, deltaY) => engineRef.current?.wheelAtScreen(screen, deltaY),
      setCamera: (nextCamera) => engineRef.current?.setCamera(nextCamera),
      applyPatch: (patch) => engineRef.current?.applyPatch(patch) ?? ["Renderer engine is not ready"],
      getSnapshot: () => engineRef.current?.getSnapshot() ?? null
    }),
    []
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
    const webGpuCanvas = webGpuCanvasRef.current;
    const sizeSource = inputCanvasRef.current;
    if (!webGpuCanvas || !sizeSource || !rustStatus.createWebGpuRenderer) {
      setWebGpuRenderer(null);
      setEngineWebGpuAvailable(false);
      setEngineWebGpuDetail(null);
      setWebGpuDetail(rustStatus.detail);
      return;
    }

    let disposed = false;
    setWebGpuRenderer(null);
    setEngineWebGpuAvailable(null);
    setEngineWebGpuDetail(null);
    setWebGpuDetail("Creating visible Rust/wgpu renderer.");
    const rect = sizeSource.getBoundingClientRect();
    void rustStatus
      .createWebGpuRenderer(webGpuCanvas, rect.width || 1, rect.height || 1, window.devicePixelRatio || 1)
      .then((renderer) => {
        if (disposed) return;
        setWebGpuRenderer(renderer);
        setEngineWebGpuAvailable(null);
        setEngineWebGpuDetail(null);
        setWebGpuDetail("Visible Rust/wgpu renderer created.");
      })
      .catch((error) => {
        if (disposed) return;
        setWebGpuRenderer(null);
        setEngineWebGpuAvailable(false);
        setEngineWebGpuDetail(null);
        setWebGpuDetail(errorMessage(error, "Visible Rust/wgpu renderer failed to initialize."));
      });
    return () => {
      disposed = true;
    };
  }, [rustStatus.createWebGpuRenderer, rustStatus.detail]);

  useEffect(() => {
    const canvas = inputCanvasRef.current;
    const overlayRoot = overlayRef.current;
    if (!canvas || !overlayRoot) return;

    const engine = new ShapeCanvasEngine({
      canvas,
      overlayRoot,
      backend: rustStatus.available ? rustStatus.backend : "webgpu-wasm-unavailable",
      webGpuRenderer,
      onEvent: handleEngineEvent
    });
    engineRef.current = engine;
    setEngineReadyKey((key) => key + 1);

    const resize = () => {
      const rect = canvas.getBoundingClientRect();
      engine.resize(rect.width, rect.height, window.devicePixelRatio || 1);
    };
    const observer = new ResizeObserver(resize);
    observer.observe(canvas);
    resize();
    engine.start();

    return () => {
      observer.disconnect();
      engine.stop();
      if (engineRef.current === engine) engineRef.current = null;
    };
  }, [rustStatus.available, rustStatus.backend, webGpuRenderer]);

  useEffect(() => {
    if (cameraAlmostEqual(cameraRef.current, camera)) return;
    cameraRef.current = camera;
    engineRef.current?.setCamera(camera);
  }, [camera]);

  useEffect(() => {
    if (!scene || !engineRef.current) return;
    const sceneForRenderer: Scene = { ...scene, selection: selectionRef.current };
    engineRef.current.loadScene(
      shapeSceneToFilteredRenderSnapshot(sceneForRenderer, activeTagIds, {
        camera: cameraRef.current,
        sceneId: `shape-scene-v${scene.sceneVersion}-production-renderer`
      })
    );
  }, [scene, activeTagIds, engineReadyKey]);

  useEffect(() => {
    selectionRef.current = selection;
    const errors = engineRef.current?.syncSelection(selection) ?? [];
    if (errors.length > 0) callbacksRef.current.onStatus(errors.join("; "));
  }, [selection, engineReadyKey]);

  function handleEngineEvent(event: EngineEvent) {
    if (event.type === "stats") {
      setEngineWebGpuAvailable(event.stats.webGpuRendererAvailable);
      callbacksRef.current.onStats(event.stats);
      const nextCamera = cameraFromStats(event.stats);
      if (nextCamera && !cameraAlmostEqual(cameraRef.current, nextCamera)) {
        cameraRef.current = nextCamera;
        callbacksRef.current.onCameraChange(nextCamera);
      }
      return;
    }

    if (event.type === "selection") {
      callbacksRef.current.onSelectionChange(hitToSceneSelection(event.hit));
      return;
    }

    if (event.type === "patch") {
      if (event.errors.length > 0) {
        callbacksRef.current.onStatus(event.errors.join("; "));
        return;
      }
      callbacksRef.current.onPatch(event.patch);
      return;
    }

    if (event.type === "gesture") {
      callbacksRef.current.onGestureChange(event.active);
      return;
    }

    if (event.type === "status") {
      if (isWebGpuRuntimeStatus(event.message)) {
        setEngineWebGpuAvailable(false);
        setEngineWebGpuDetail(event.message);
      }
      callbacksRef.current.onStatus(event.message);
    }
  }

  function handleContextMenu(event: MouseEvent<HTMLCanvasElement>) {
    event.preventDefault();
    onContextMenuRequest({ x: event.clientX, y: event.clientY });
  }

  const hasRenderableScene = Boolean(scene?.groups.length);
  const runtimeWebGpuAvailable = engineWebGpuAvailable ?? Boolean(webGpuRenderer);
  const readyState = runtimeWebGpuAvailable ? "ready" : rustStatus.available ? "webgpu-unavailable" : "wasm-unavailable";
  const rendererDetail = engineWebGpuDetail ?? webGpuDetail;

  useEffect(() => {
    onHealthChange({
      state: readyState,
      detail: rendererDetail,
      rustAvailable: rustStatus.available,
      rustBackend: rustStatus.backend,
      webGpuRendererAvailable: runtimeWebGpuAvailable
    });
  }, [onHealthChange, readyState, rendererDetail, rustStatus.available, rustStatus.backend, runtimeWebGpuAvailable]);

  return (
    <div className="renderer-host" data-renderer-state={readyState}>
      <canvas ref={webGpuCanvasRef} className="renderer-canvas renderer-webgpu-canvas" aria-label="Rust WebGPU scene canvas" />
      <canvas ref={inputCanvasRef} className="renderer-canvas renderer-input-canvas" aria-label="Renderer input surface" onContextMenu={handleContextMenu} />
      <div ref={overlayRef} className="renderer-overlay-root" />
      {!hasRenderableScene ? (
        <div className="empty-canvas">
          <h2>Create a group</h2>
          <p>Start with a proposition, architecture concern, or implementation plan. It will become a group on the infinite canvas.</p>
        </div>
      ) : null}
      {readyState !== "ready" && hasRenderableScene ? <div className="renderer-status-strip">{rendererDetail}</div> : null}
    </div>
  );
});

function hitToSceneSelection(hit: HitResult | null): SceneSelection {
  if (!hit) return { kind: "canvas" };
  if (hit.kind === "group") return { kind: "group", id: hit.id };
  if (hit.kind === "edge") return { kind: "edge", id: hit.id };
  return { kind: "node", id: hit.id };
}

function cameraFromStats(stats: RendererStats): CameraState | null {
  if (stats.rustCameraX === null || stats.rustCameraY === null || stats.rustCameraZoom === null) return null;
  return { x: stats.rustCameraX, y: stats.rustCameraY, zoom: stats.rustCameraZoom };
}

function cameraAlmostEqual(left: CameraState, right: CameraState): boolean {
  return Math.abs(left.x - right.x) < 0.5 && Math.abs(left.y - right.y) < 0.5 && Math.abs(left.zoom - right.zoom) < 0.0005;
}

function errorMessage(error: unknown, fallback: string): string {
  if (error instanceof Error) return error.message || fallback;
  if (typeof error === "string") return error || fallback;
  const message = String(error);
  return message === "[object Object]" ? fallback : message;
}

function isWebGpuRuntimeStatus(message: string): boolean {
  return message.startsWith("WebGPU renderer unavailable:") || message.startsWith("WebGPU render failed");
}
