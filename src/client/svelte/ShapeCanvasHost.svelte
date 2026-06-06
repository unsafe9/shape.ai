<script lang="ts">
  import { onMount } from "svelte";
  import type { CameraState } from "../../shared/renderScene";
  import { ShapeCanvasHost, type ShapeCanvasHostCallbacks } from "../lib/canvasHost";

  type Props = {
    initialCamera: CameraState;
    callbacks: ShapeCanvasHostCallbacks;
    readyState: "ready" | "webgpu-unavailable" | "wasm-unavailable";
    rendererDetail: string;
    hasRenderableScene: boolean;
    onHost: (host: ShapeCanvasHost) => void;
    onContextMenuRequest: (point: { x: number; y: number }) => void;
  };

  let {
    initialCamera,
    callbacks,
    readyState,
    rendererDetail,
    hasRenderableScene,
    onHost,
    onContextMenuRequest
  }: Props = $props();

  let inputCanvas: HTMLCanvasElement;
  let webGpuCanvas: HTMLCanvasElement;
  let overlayRoot: HTMLDivElement;

  onMount(() => {
    // T6.2 §4: the Svelte node only owns the three DOM nodes; the
    // framework-neutral ShapeCanvasHost owns the engine lifecycle.
    const host = new ShapeCanvasHost(callbacks);
    void host.mount(inputCanvas, webGpuCanvas, overlayRoot, initialCamera).then(() => onHost(host));
    return () => host.destroy();
  });

  function handleContextMenu(event: MouseEvent) {
    event.preventDefault();
    onContextMenuRequest({ x: event.clientX, y: event.clientY });
  }
</script>

<div class="renderer-host" data-renderer-state={readyState}>
  <canvas bind:this={webGpuCanvas} class="renderer-canvas renderer-webgpu-canvas" aria-label="Rust WebGPU scene canvas"></canvas>
  <canvas
    bind:this={inputCanvas}
    class="renderer-canvas renderer-input-canvas"
    aria-label="Renderer input surface"
    oncontextmenu={handleContextMenu}
  ></canvas>
  <div bind:this={overlayRoot} class="renderer-overlay-root"></div>
  {#if !hasRenderableScene}
    <div class="empty-canvas">
      <h2>Create a group</h2>
      <p>Start with a proposition, architecture concern, or implementation plan. It will become a group on the infinite canvas.</p>
    </div>
  {/if}
  {#if readyState !== "ready" && hasRenderableScene}
    <div class="renderer-status-strip">{rendererDetail}</div>
  {/if}
</div>
