<script lang="ts">
  // Peer cursor overlay (MG6.2). A lightweight, absolutely-positioned layer over
  // the canvas surface: each live peer's WORLD-space cursor is projected to this
  // client's screen via the shared screenToWorld inverse, so peers with different
  // cameras still point at the same canvas location. Pure presentation — it owns
  // no presence state; the SceneClient's PeerRegistry feeds it the live set.
  import { MousePointer2 } from "lucide-svelte";
  import type { CameraState } from "../../shared/geometry";
  import { worldToScreen } from "../renderer/scene";
  import type { PeerPresence } from "../lib/peers";

  type Props = {
    peers: PeerPresence[];
    camera: CameraState;
  };

  let { peers, camera }: Props = $props();

  // Only peers with a known cursor are drawn; each is projected to screen px.
  const placed = $derived(
    peers
      .filter((peer) => peer.cursor !== null)
      .map((peer) => ({
        userId: peer.userId,
        color: peer.color,
        label: peerLabel(peer.userId),
        screen: worldToScreen(peer.cursor!, camera)
      }))
  );

  // A short, human-ish label for the cursor flag from the userId.
  function peerLabel(userId: string): string {
    return userId.length > 12 ? `${userId.slice(0, 12)}…` : userId;
  }
</script>

{#if placed.length > 0}
  <div class="peer-cursor-layer" aria-hidden="true">
    {#each placed as peer (peer.userId)}
      <div class="peer-cursor" style:transform={`translate(${peer.screen.x}px, ${peer.screen.y}px)`}>
        <MousePointer2 size={18} style={`color: ${peer.color}; fill: ${peer.color};`} />
        <span class="peer-cursor-flag" style:background={peer.color}>{peer.label}</span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .peer-cursor-layer {
    position: absolute;
    inset: 0;
    pointer-events: none;
    overflow: hidden;
    z-index: 6;
  }

  .peer-cursor {
    position: absolute;
    top: 0;
    left: 0;
    display: flex;
    align-items: flex-start;
    gap: 2px;
    will-change: transform;
  }

  .peer-cursor-flag {
    margin-top: 10px;
    padding: 1px 6px;
    border-radius: 6px;
    font-size: 11px;
    line-height: 1.4;
    color: #fff;
    white-space: nowrap;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.25);
  }
</style>
