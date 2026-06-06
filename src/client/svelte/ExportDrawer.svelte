<script lang="ts" module>
  import type { ExportOutput, ExportType } from "../../shared/schema";

  export type ExportPreview = ExportOutput & {
    type: ExportType;
    contentType: string;
  };
</script>

<script lang="ts">
  import { Check, Copy, Download, FileImage, FileText, Network, X } from "lucide-svelte";
  import { exportTypeLabels } from "../../shared/graph";
  import type { SceneArtifact } from "../../shared/schema";

  type Props = {
    artifacts: SceneArtifact[];
    busy: boolean;
    onExport: (type: ExportType) => void;
    onCopyPreview: () => void;
    onClosePreview: () => void;
    preview: ExportPreview | null;
    previewCopied: boolean;
    groupId?: string;
  };

  let { artifacts, busy, onExport, onCopyPreview, onClosePreview, preview, previewCopied, groupId }: Props = $props();

  const exportTypes: ExportType[] = ["madr", "yadr", "mermaid", "image_prompt"];

  const icons = {
    madr: FileText,
    yadr: FileText,
    mermaid: Network,
    image_prompt: FileImage
  } as const;
</script>

<section class="export-drawer {preview ? 'has-preview' : ''}">
  <div class="export-drawer-main">
    <div class="export-actions">
      {#each exportTypes as type (type)}
        {@const Icon = icons[type] ?? FileText}
        <button onclick={() => onExport(type)} disabled={busy || !groupId}>
          <Icon size={15} />
          {exportTypeLabels[type]}
        </button>
      {/each}
    </div>
    <div class="artifact-strip">
      {#each artifacts.slice(0, 4) as artifact (artifact.id)}
        <a href={`/api/groups/${groupId}/artifacts/${artifact.id}`}>
          <Download size={14} />
          {artifact.title}
        </a>
      {/each}
      {#if artifacts.length === 0}
        <span>No exports yet.</span>
      {/if}
    </div>
  </div>

  {#if preview}
    <div class="export-preview">
      <div class="export-preview-head">
        <div>
          <span>{exportTypeLabels[preview.type]}</span>
          <strong>{preview.title}</strong>
        </div>
        <div class="export-preview-actions">
          <button type="button" onclick={onCopyPreview}>
            {#if previewCopied}
              <Check size={14} />
            {:else}
              <Copy size={14} />
            {/if}
            {previewCopied ? "Copied" : "Copy"}
          </button>
          <button type="button" class="icon-button" onclick={onClosePreview} aria-label="Close export preview">
            <X size={14} />
          </button>
        </div>
      </div>
      <pre class="export-preview-body">{preview.content}</pre>
    </div>
  {/if}
</section>
