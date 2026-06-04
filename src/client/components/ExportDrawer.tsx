import { Check, Copy, Download, FileImage, FileText, Network, X } from "lucide-react";
import { exportTypeLabels } from "../../shared/graph";
import type { ExportOutput, ExportType, SceneArtifact } from "../../shared/schema";

export type ExportPreview = ExportOutput & {
  type: ExportType;
  contentType: string;
};

type ExportDrawerProps = {
  artifacts: SceneArtifact[];
  busy: boolean;
  onExport: (type: ExportType) => void;
  onCopyPreview: () => void;
  onClosePreview: () => void;
  preview: ExportPreview | null;
  previewCopied: boolean;
  groupId?: string;
};

const exportTypes: ExportType[] = ["madr", "yadr", "mermaid", "image_prompt"];

const icons: Partial<Record<ExportType, typeof FileText>> = {
  madr: FileText,
  yadr: FileText,
  mermaid: Network,
  image_prompt: FileImage
};

export function ExportDrawer({
  artifacts,
  busy,
  onExport,
  onCopyPreview,
  onClosePreview,
  preview,
  previewCopied,
  groupId
}: ExportDrawerProps) {
  return (
    <section className={`export-drawer ${preview ? "has-preview" : ""}`}>
      <div className="export-drawer-main">
        <div className="export-actions">
          {exportTypes.map((type) => {
            const Icon = icons[type] ?? FileText;
            return (
              <button key={type} onClick={() => onExport(type)} disabled={busy || !groupId}>
                <Icon size={15} />
                {exportTypeLabels[type]}
              </button>
            );
          })}
        </div>
        <div className="artifact-strip">
          {artifacts.slice(0, 4).map((artifact) => (
            <a key={artifact.id} href={`/api/groups/${groupId}/artifacts/${artifact.id}`}>
              <Download size={14} />
              {artifact.title}
            </a>
          ))}
          {artifacts.length === 0 ? <span>No exports yet.</span> : null}
        </div>
      </div>

      {preview ? (
        <div className="export-preview">
          <div className="export-preview-head">
            <div>
              <span>{exportTypeLabels[preview.type]}</span>
              <strong>{preview.title}</strong>
            </div>
            <div className="export-preview-actions">
              <button type="button" onClick={onCopyPreview}>
                {previewCopied ? <Check size={14} /> : <Copy size={14} />}
                {previewCopied ? "Copied" : "Copy"}
              </button>
              <button type="button" className="icon-button" onClick={onClosePreview} aria-label="Close export preview">
                <X size={14} />
              </button>
            </div>
          </div>
          <pre className="export-preview-body">{preview.content}</pre>
        </div>
      ) : null}
    </section>
  );
}
