<script lang="ts">
  // Canvas switch UI (MG9.2). A thin shell-chrome control that lists the
  // canvases from the durable index, switches the active canvas (which makes the
  // data layer reconnect + re-subscribe the window for the new id), and
  // creates/deletes canvases. It owns no transport state of its own: every
  // action is delegated to the parent, which drives the SceneClient. This keeps
  // the component a pure platform-layer widget (per the core/adapter/shell
  // boundary) — it renders identity and intent, the data layer does the work.
  import { Check, Loader2, Plus, Trash2, Wifi, WifiOff } from "lucide-svelte";
  import type { CanvasSummary } from "../lib/sceneClient";
  import type { ConnectionStatus } from "../lib/wsTransport";

  type Props = {
    canvases: CanvasSummary[];
    activeCanvasId: string | null;
    /** Connectivity badge: the offline state is what reconnect (MG8.4) surfaces. */
    status: ConnectionStatus;
    busy: boolean;
    onSelect: (canvasId: string) => void;
    onCreate: (title: string) => void;
    onDelete: (canvasId: string) => void;
  };

  let { canvases, activeCanvasId, status, busy, onSelect, onCreate, onDelete }: Props = $props();

  let open = $state(false);
  let newTitle = $state("");

  const activeCanvas = $derived(canvases.find((c) => c.id === activeCanvasId) ?? null);

  function submitCreate(): void {
    const title = newTitle.trim();
    if (!title) return;
    onCreate(title);
    newTitle = "";
  }
</script>

<div class="canvas-switcher {open ? 'is-open' : ''}">
  <button
    class="canvas-switcher-trigger"
    type="button"
    onclick={() => (open = !open)}
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-label="Switch canvas"
  >
    <span class="canvas-switcher-name">{activeCanvas?.title ?? activeCanvasId ?? "No canvas"}</span>
    {#if status === "online"}
      <Wifi class="canvas-switcher-status is-online" size={14} aria-label="Online" />
    {:else}
      <WifiOff class="canvas-switcher-status is-offline" size={14} aria-label="Offline" />
    {/if}
  </button>

  {#if open}
    <div class="canvas-switcher-panel" role="listbox" aria-label="Canvases">
      <div class="canvas-switcher-list">
        {#each canvases as canvas (canvas.id)}
          <div class="canvas-switcher-row {canvas.id === activeCanvasId ? 'is-active' : ''}">
            <button
              class="canvas-switcher-select"
              type="button"
              role="option"
              aria-selected={canvas.id === activeCanvasId}
              disabled={busy}
              onclick={() => {
                onSelect(canvas.id);
                open = false;
              }}
            >
              {#if canvas.id === activeCanvasId}
                <Check size={14} />
              {:else}
                <span class="canvas-switcher-check-spacer" aria-hidden="true"></span>
              {/if}
              <span class="canvas-switcher-row-title">{canvas.title}</span>
            </button>
            <button
              class="canvas-switcher-delete icon-button"
              type="button"
              disabled={busy || canvas.id === activeCanvasId}
              aria-label={`Delete ${canvas.title}`}
              title={canvas.id === activeCanvasId ? "Switch away before deleting" : `Delete ${canvas.title}`}
              onclick={() => onDelete(canvas.id)}
            >
              <Trash2 size={14} />
            </button>
          </div>
        {/each}
        {#if canvases.length === 0}
          <p class="muted">No canvases yet.</p>
        {/if}
      </div>

      <form
        class="canvas-switcher-create"
        onsubmit={(event) => {
          event.preventDefault();
          submitCreate();
        }}
      >
        <input
          value={newTitle}
          oninput={(event) => (newTitle = event.currentTarget.value)}
          placeholder="New canvas"
          aria-label="New canvas title"
        />
        <button class="icon-button" type="submit" disabled={busy || !newTitle.trim()} aria-label="Create canvas">
          {#if busy}
            <Loader2 class="spin" size={15} />
          {:else}
            <Plus size={15} />
          {/if}
        </button>
      </form>
    </div>
  {/if}
</div>

<style>
  .canvas-switcher {
    position: relative;
    display: inline-flex;
  }
  .canvas-switcher-trigger {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    border-radius: 8px;
    border: 1px solid var(--border, #2a2f3a);
    background: var(--surface, #14181f);
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
  .canvas-switcher-name {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  :global(.canvas-switcher-status.is-online) {
    color: #3fb950;
  }
  :global(.canvas-switcher-status.is-offline) {
    color: #d29922;
  }
  .canvas-switcher-panel {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    z-index: 40;
    min-width: 240px;
    padding: 6px;
    border-radius: 10px;
    border: 1px solid var(--border, #2a2f3a);
    background: var(--surface, #14181f);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.4);
  }
  .canvas-switcher-list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 280px;
    overflow-y: auto;
  }
  .canvas-switcher-row {
    display: flex;
    align-items: center;
    gap: 4px;
    border-radius: 6px;
  }
  .canvas-switcher-row.is-active {
    background: rgba(110, 141, 242, 0.12);
  }
  .canvas-switcher-select {
    flex: 1;
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 7px 8px;
    border: none;
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
    border-radius: 6px;
  }
  .canvas-switcher-select:disabled {
    opacity: 0.6;
    cursor: default;
  }
  .canvas-switcher-check-spacer {
    width: 14px;
    height: 14px;
  }
  .canvas-switcher-row-title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .canvas-switcher-create {
    display: flex;
    gap: 6px;
    margin-top: 6px;
    padding-top: 6px;
    border-top: 1px solid var(--border, #2a2f3a);
  }
  .canvas-switcher-create input {
    flex: 1;
    min-width: 0;
    padding: 6px 8px;
    border-radius: 6px;
    border: 1px solid var(--border, #2a2f3a);
    background: var(--surface-2, #0f1218);
    color: inherit;
    font: inherit;
  }
  .muted {
    margin: 6px 8px;
    opacity: 0.6;
    font-size: 0.85em;
  }
</style>
