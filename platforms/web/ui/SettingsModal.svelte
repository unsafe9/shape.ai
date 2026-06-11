<script lang="ts">
  import { Keyboard, X } from "lucide-svelte";
  import { detectMac, formatShortcut } from "../controller/shortcuts";
  import { formatGestureTrigger } from "../controller/gestures";
  import type { ObjectCommand, ObjectGesture } from "../bridge/sceneCoreWasm";

  // CC5.1 / U4 — settings overlay (Cmd+,). Shows the READ-ONLY shortcut list from
  // the object command catalog (the wasm core's `object_command_catalog()`, P1 —
  // no TS mirror). Each row pairs a command (id/label) with its binding
  // (defaultShortcut); editing is out of scope.
  //
  // SM1 (#2): a second section lists the hold-key gesture catalog (the wasm core's
  // `object_gesture_catalog()`, C2) — registering a gesture once self-documents it.
  type Props = {
    catalog: ObjectCommand[];
    gestures: ObjectGesture[];
    onClose: () => void;
  };

  let { catalog, gestures, onClose }: Props = $props();

  const isMac = detectMac();

  // Group by the catalog's own category strings, in first-seen order.
  const grouped = $derived.by(() => {
    const order: string[] = [];
    const byCategory = new Map<string, ObjectCommand[]>();
    for (const command of catalog) {
      if (!byCategory.has(command.category)) {
        byCategory.set(command.category, []);
        order.push(command.category);
      }
      byCategory.get(command.category)!.push(command);
    }
    return order.map((category) => ({ category, commands: byCategory.get(category)! }));
  });
</script>

<div
  class="settings-overlay"
  role="dialog"
  aria-modal="true"
  aria-label="Settings"
  tabindex="-1"
  onpointerdown={(event) => {
    if (event.target === event.currentTarget) onClose();
  }}
>
  <div class="settings-modal">
    <div class="settings-modal-header">
      <div class="settings-modal-title">
        <Keyboard size={16} />
        <span>Keyboard Shortcuts</span>
      </div>
      <button class="icon-button" type="button" aria-label="Close settings" title="Close" onclick={onClose}>
        <X size={15} />
      </button>
    </div>

    <p class="settings-modal-note">
      Shortcuts are read-only here. Each command keeps its binding separate from the action, so bindings can be remapped
      in a future release without changing the commands.
    </p>

    <div class="settings-modal-body">
      {#each grouped as group (group.category)}
        <section class="settings-section">
          <h3>{group.category}</h3>
          <ul>
            {#each group.commands as command (command.id)}
              <li>
                <span class="settings-command-label">{command.label}</span>
                {#if command.defaultShortcut}
                  <kbd class="settings-command-binding">{formatShortcut(command.defaultShortcut, isMac)}</kbd>
                {:else}
                  <span class="settings-command-unbound">—</span>
                {/if}
              </li>
            {/each}
          </ul>
        </section>
      {/each}

      {#if gestures.length > 0}
        <section class="settings-section settings-gestures">
          <h3>Gestures</h3>
          <ul>
            {#each gestures as gesture (gesture.id)}
              <li>
                <span class="settings-command-label">{gesture.label}</span>
                <kbd class="settings-command-binding">{formatGestureTrigger(gesture.trigger, isMac)}</kbd>
                <span class="settings-gesture-description">{gesture.description}</span>
              </li>
            {/each}
          </ul>
        </section>
      {/if}
    </div>
  </div>
</div>
