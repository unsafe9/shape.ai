<script lang="ts">
  import { Circle, LayoutTemplate, MoveRight, Shapes, Square, StickyNote, SquareDashed } from "lucide-svelte";

  /** The basic primitives the palette can drop onto the canvas. */
  export type PrimitiveKindId = "rectangle" | "ellipse" | "connector" | "sticky" | "frame";

  type TemplateEntry = {
    id: string;
    title: string;
    description: string;
  };

  type Props = {
    templates: TemplateEntry[];
    busy: boolean;
    onInsertPrimitive: (kind: PrimitiveKindId) => void;
    onApplyTemplate: (templateId: string) => void;
  };

  let { templates, busy, onInsertPrimitive, onApplyTemplate }: Props = $props();

  const primitives: { id: PrimitiveKindId; label: string; icon: typeof Square }[] = [
    { id: "rectangle", label: "Rectangle", icon: Square },
    { id: "ellipse", label: "Ellipse", icon: Circle },
    { id: "connector", label: "Line / Arrow", icon: MoveRight },
    { id: "sticky", label: "Sticky / Text", icon: StickyNote },
    { id: "frame", label: "Frame", icon: SquareDashed }
  ];
</script>

<aside class="shape-palette" aria-label="Insert palette">
  <section class="palette-section">
    <div class="palette-section-title">
      <Shapes size={13} />
      <span>Basic</span>
    </div>
    <div class="palette-primitive-grid">
      {#each primitives as primitive (primitive.id)}
        <button
          class="palette-primitive"
          type="button"
          disabled={busy}
          title={primitive.label}
          aria-label={`Insert ${primitive.label}`}
          onclick={() => onInsertPrimitive(primitive.id)}
        >
          <primitive.icon size={18} />
          <span>{primitive.label}</span>
        </button>
      {/each}
    </div>
  </section>

  <section class="palette-section">
    <div class="palette-section-title">
      <LayoutTemplate size={13} />
      <span>Templates</span>
    </div>
    <div class="palette-template-list">
      {#each templates as template (template.id)}
        <button
          class="palette-template-item"
          type="button"
          disabled={busy}
          onclick={() => onApplyTemplate(template.id)}
        >
          <span class="palette-template-title">{template.title}</span>
          <span class="palette-template-desc">{template.description}</span>
        </button>
      {/each}
    </div>
  </section>
</aside>
