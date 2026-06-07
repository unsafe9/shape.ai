<script lang="ts">
  import { Keyboard, X } from "lucide-svelte";
  import { commandCatalog, detectMac, formatShortcut, type CommandCategory } from "../lib/commandCatalog";

  // CC5.1 / O7 — settings overlay (Cmd+,). Shows the READ-ONLY shortcut list from
  // the command catalog. CC5.2 binding/command separation: each row pairs a
  // command (id/label) with its binding (defaultShortcut); editing is out of
  // scope, but the data model already keeps the two distinct so a future binding
  // editor can override the binding per command without touching the command set.
  type Props = {
    onClose: () => void;
  };

  let { onClose }: Props = $props();

  const isMac = detectMac();

  const categoryLabels: Record<CommandCategory, string> = {
    tool: "Tools",
    shape: "Shapes",
    view: "View",
    edit: "Edit",
    selection: "Selection",
    template: "Templates",
    canvas: "Canvas"
  };

  const categoryOrder: CommandCategory[] = ["tool", "shape", "view", "edit", "selection", "template", "canvas"];

  const grouped = categoryOrder
    .map((category) => ({ category, commands: commandCatalog.filter((command) => command.category === category) }))
    .filter((group) => group.commands.length > 0);
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
          <h3>{categoryLabels[group.category]}</h3>
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
    </div>
  </div>
</div>
