<script lang="ts">
  import { Crosshair, Pause, Play } from "lucide-svelte";
  import type { McpClientInfo } from "../lib/mcpDock";
  import type { FollowMode } from "../lib/followController";

  type FollowControls = {
    followeeClientId: string | null;
    mode: FollowMode;
    onToggleFollow: (client: McpClientInfo) => void;
    onPauseResume: () => void;
    onJumpToCurrent: () => void;
  };

  type Props = {
    clients: McpClientInfo[];
    onFocusTarget?: (target: unknown) => void;
    follow?: FollowControls;
  };

  let { clients, onFocusTarget, follow }: Props = $props();

  const STATE_COLORS: Record<McpClientInfo["dockState"], string> = {
    idle: "var(--ok)",
    active: "var(--accent)",
    error: "var(--bad)",
    disconnected: "var(--muted)",
    muted: "var(--muted)"
  };

  function isFollowable(client: McpClientInfo): boolean {
    return client.dockState === "idle" || client.dockState === "active" || client.dockState === "error";
  }

  function chipTitle(client: McpClientInfo): string {
    const state = client.dockState;
    const target = client.lastTarget as { kind?: string; id?: string } | null;
    const targetDesc = target?.kind && target.kind !== "canvas" && target.id ? `${target.kind} ${target.id}` : target?.kind ?? null;
    return [
      `${client.label} v${client.version}`,
      `transport: ${client.transport}`,
      `state: ${state}`,
      targetDesc ? `last target: ${targetDesc}` : null
    ]
      .filter(Boolean)
      .join("\n");
  }

  const sorted = $derived(
    [...clients].sort((a, b) => {
      const order: Record<McpClientInfo["dockState"], number> = {
        active: 0,
        error: 1,
        idle: 2,
        muted: 3,
        disconnected: 4
      };
      const diff = order[a.dockState] - order[b.dockState];
      if (diff !== 0) return diff;
      return b.lastActivityAt - a.lastActivityAt;
    })
  );
</script>

{#if clients.length === 0}
  <div class="companion-dock companion-dock-empty" aria-label="MCP companions">
    <span class="companion-dock-empty-label">No MCP clients</span>
  </div>
{:else}
  <div class="companion-dock" aria-label="MCP companions" role="list">
    {#each sorted as client (client.clientId)}
      {@const dotColor = STATE_COLORS[client.dockState]}
      {@const initial = (client.label[0] ?? "?").toUpperCase()}
      {@const isActive = client.dockState === "active"}
      {@const isDisconnected = client.dockState === "disconnected"}
      {@const target = client.lastTarget as { kind?: string } | null}
      {@const hasSpatialTarget = target !== null && target?.kind !== undefined && target.kind !== "canvas"}
      {@const clickable = hasSpatialTarget && onFocusTarget !== undefined}
      {@const isFollowed = follow !== undefined && follow.followeeClientId === client.clientId && follow.mode !== "off"}
      {@const canFollow = follow !== undefined && isFollowable(client)}
      <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
      <div
        class="companion-chip {isActive ? 'companion-chip-active' : ''} {isDisconnected ? 'companion-chip-disconnected' : ''} {clickable ? 'companion-chip-focusable' : ''} {isFollowed ? 'companion-chip-followed' : ''}"
        title={chipTitle(client)}
        role={clickable ? "button" : "listitem"}
        aria-label={`${client.label}: ${client.dockState}`}
        tabindex={clickable ? 0 : undefined}
        onclick={clickable ? () => onFocusTarget?.(client.lastTarget) : undefined}
        onkeydown={clickable
          ? (e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                onFocusTarget?.(client.lastTarget);
              }
            }
          : undefined}
      >
        <span class="companion-chip-icon" style:background={client.iconRef ? undefined : client.color} style:border-color={client.color}>
          {#if client.iconRef}
            <img src={client.iconRef} alt={client.label} width={14} height={14} />
          {:else}
            {initial}
          {/if}
        </span>
        <span class="companion-chip-dot" style:background={dotColor} aria-hidden="true"></span>
        {#if canFollow && follow}
          <span class="companion-chip-follow" onclick={(e) => e.stopPropagation()} role="presentation">
            <button
              type="button"
              class="companion-follow-button {isFollowed ? 'is-following' : ''}"
              aria-pressed={isFollowed}
              aria-label={isFollowed ? `Stop following ${client.label}` : `Follow ${client.label}`}
              title={isFollowed ? "Stop following" : "Follow agent"}
              onclick={() => follow.onToggleFollow(client)}
            >
              <Crosshair size={12} />
            </button>
            {#if isFollowed}
              <button
                type="button"
                class="companion-follow-button"
                aria-label={follow.mode === "paused" ? "Resume following" : "Pause following"}
                title={follow.mode === "paused" ? "Resume following" : "Pause following"}
                onclick={() => follow.onPauseResume()}
              >
                {#if follow.mode === "paused"}
                  <Play size={12} />
                {:else}
                  <Pause size={12} />
                {/if}
              </button>
              <button
                type="button"
                class="companion-follow-button"
                aria-label="Jump to current target"
                title="Jump to current target"
                onclick={() => follow.onJumpToCurrent()}
              >
                <Crosshair size={12} strokeWidth={2.5} />
              </button>
            {/if}
          </span>
        {/if}
      </div>
    {/each}
  </div>
{/if}
