import { Download, FileImage, FileText, Network } from "lucide-react";
import { exportTypeLabels } from "../../shared/graph";
import type { DesignArtifact, ExportType } from "../../shared/schema";

type ExportDrawerProps = {
  artifacts: DesignArtifact[];
  busy: boolean;
  onExport: (type: ExportType) => void;
  designId?: string;
};

const exportTypes: ExportType[] = ["madr", "yadr", "mermaid", "image_prompt"];

const icons: Partial<Record<ExportType, typeof FileText>> = {
  madr: FileText,
  yadr: FileText,
  mermaid: Network,
  image_prompt: FileImage
};

export function ExportDrawer({ artifacts, busy, onExport, designId }: ExportDrawerProps) {
  return (
    <section className="export-drawer">
      <div className="export-actions">
        {exportTypes.map((type) => {
          const Icon = icons[type] ?? FileText;
          return (
            <button key={type} onClick={() => onExport(type)} disabled={busy || !designId}>
              <Icon size={15} />
              {exportTypeLabels[type]}
            </button>
          );
        })}
      </div>
      <div className="artifact-strip">
        {artifacts.slice(0, 4).map((artifact) => (
          <a key={artifact.id} href={`/api/designs/${designId}/artifacts/${artifact.id}`}>
            <Download size={14} />
            {artifact.title}
          </a>
        ))}
        {artifacts.length === 0 ? <span>No exports yet.</span> : null}
      </div>
    </section>
  );
}
