<script lang="ts">
  import { X } from "lucide-svelte";
  import { fetchMcpTrace, type McpClientInfo, type McpTraceEvent, type McpTraceKind } from "../lib/api";

  type Props = {
    open: boolean;
    clients: McpClientInfo[];
    onClose: () => void;
    onFocusTarget?: (target: unknown) => void;
  };

  let { open, clients, onClose, onFocusTarget }: Props = $props();

  const POLL_INTERVAL_MS = 4_000;
  const DEFAULT_LIMIT = 50;

  const KIND_LABELS: Record<McpTraceKind, string> = {
    read: "read",
    write: "write",
    comment: "comment",
    export: "export",
    "proposal-created": "proposed",
    "proposal-accepted": "accepted",
    "proposal-rejected": "rejected",
    error: "error"
  };

  function relativeTime(at: number): string {
    const deltaS = Math.floor((Date.now() - at) / 1000);
    if (deltaS < 5) return "just now";
    if (deltaS < 60) return `${deltaS}s ago`;
    const deltaM = Math.floor(deltaS / 60);
    if (deltaM < 60) return `${deltaM}m ago`;
    const deltaH = Math.floor(deltaM / 60);
    return `${deltaH}h ago`;
  }

  function isSpatialTarget(target: unknown): boolean {
    if (!target || typeof target !== "object") return false;
    const t = target as { kind?: unknown };
    return typeof t.kind === "string" && t.kind !== "canvas";
  }

  let activeClientId = $state<string | null>(null);
  let events = $state<McpTraceEvent[]>([]);
  let fetchError = $state<string | null>(null);

  const targetClientId = $derived(activeClientId ?? clients[0]?.clientId ?? null);
  const activeClient = $derived(clients.find((c) => c.clientId === targetClientId) ?? clients[0] ?? null);

  // Monotonic generation: every effect re-run (client switch or open toggle)
  // bumps it, so a stale in-flight response can't overwrite the current view.
  let traceGen = 0;

  async function fetchTrace(clientId: string, gen: number): Promise<void> {
    try {
      const result = await fetchMcpTrace(clientId, DEFAULT_LIMIT);
      if (gen !== traceGen) return;
      events = result.trace;
      fetchError = null;
    } catch (err) {
      if (gen !== traceGen) return;
      fetchError = err instanceof Error ? err.message : "Trace fetch failed";
    }
  }

  // Poll the active client's trace while open, mirroring CompanionTrace.tsx.
  $effect(() => {
    const clientId = targetClientId;
    const gen = ++traceGen;
    if (!open || !clientId) {
      events = [];
      return;
    }
    void fetchTrace(clientId, gen);
    const id = window.setInterval(() => {
      void fetchTrace(clientId, gen);
    }, POLL_INTERVAL_MS);
    return () => window.clearInterval(id);
  });
</script>

{#if open}
  <aside
    id="companion-trace"
    class="companion-trace-drawer"
    aria-label="Agent activity trail"
    onpointerdown={(event) => event.stopPropagation()}
    oncontextmenu={(event) => event.preventDefault()}
  >
    <div class="companion-trace-head">
      <div>
        <span>Agent Trace</span>
        <strong>{activeClient?.label ?? "No client"}</strong>
      </div>
      <button class="icon-button" type="button" onclick={onClose} aria-label="Close agent trace" title="Close agent trace">
        <X size={15} />
      </button>
    </div>

    {#if clients.length > 1}
      <div class="companion-trace-filter" role="list" aria-label="Filter by client">
        {#each clients as client (client.clientId)}
          <!-- svelte-ignore a11y_no_interactive_element_to_noninteractive_role -->
          <!-- svelte-ignore a11y_role_supports_aria_props -->
          <button
            type="button"
            role="listitem"
            class="companion-trace-chip {client.clientId === targetClientId ? 'is-active' : ''}"
            style:--chip-color={client.color}
            onclick={() => (activeClientId = client.clientId)}
            title={`${client.label} v${client.version}`}
            aria-pressed={client.clientId === targetClientId}
          >
            <span class="companion-trace-chip-icon" style:background={client.color}>
              {(client.label[0] ?? "?").toUpperCase()}
            </span>
            <span class="companion-trace-chip-label">{client.label}</span>
          </button>
        {/each}
      </div>
    {/if}

    {#if fetchError}
      <div class="companion-trace-error" role="alert">{fetchError}</div>
    {:else if events.length === 0}
      <div class="companion-trace-empty">No activity yet</div>
    {:else}
      <ol class="companion-trace-list" aria-label="Recent operations">
        {#each events as event, index (event.operationId ?? `read-${index}-${event.at}`)}
          {@const kindLabel = KIND_LABELS[event.kind] ?? event.kind}
          {@const isError = event.kind === "error"}
          {@const isPending = event.kind === "proposal-created"}
          {@const clickable = onFocusTarget !== undefined && isSpatialTarget(event.target)}
          {@const clientColor = activeClient?.color ?? "#888"}
          <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
          <li
            class="companion-trace-row {isError ? 'is-error' : ''} {isPending ? 'is-pending' : ''} {clickable ? 'companion-trace-row-focusable' : ''}"
            role={clickable ? "button" : "listitem"}
            tabindex={clickable ? 0 : undefined}
            title={clickable ? "Focus on canvas" : undefined}
            onclick={clickable ? () => onFocusTarget?.(event.target) : undefined}
            onkeydown={clickable
              ? (e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    onFocusTarget?.(event.target);
                  }
                }
              : undefined}
          >
            <span class="companion-trace-kind-dot" style:background={isError ? "var(--bad)" : clientColor} aria-hidden="true"></span>
            <div class="companion-trace-content">
              <span class="companion-trace-kind">{kindLabel}</span>
              <span class="companion-trace-summary">{event.summary}</span>
              {#if event.errorMessage}
                <span class="companion-trace-error-msg">{event.errorMessage}</span>
              {/if}
            </div>
            <time
              class="companion-trace-time"
              datetime={new Date(event.at).toISOString()}
              title={new Date(event.at).toLocaleString()}
            >
              {relativeTime(event.at)}
            </time>
          </li>
        {/each}
      </ol>
    {/if}
  </aside>
{/if}
