import { LayoutTemplate } from "lucide-react";

export type TemplatePickerEntry = {
  id: string;
  title: string;
  description: string;
};

type TemplatePickerProps = {
  templates: TemplatePickerEntry[];
  busy: boolean;
  onApply: (templateId: string) => void;
  onClose: () => void;
};

export function TemplatePicker({ templates, busy, onApply, onClose }: TemplatePickerProps) {
  return (
    <div
      className="template-picker"
      role="menu"
      aria-label="Insert template"
      onPointerDown={(event) => event.stopPropagation()}
    >
      <div className="template-picker-header">
        <LayoutTemplate size={14} />
        <span>Insert template</span>
      </div>
      <div className="template-picker-list">
        {templates.map((template) => (
          <button
            key={template.id}
            className="template-picker-item"
            role="menuitem"
            type="button"
            disabled={busy}
            onClick={() => {
              onApply(template.id);
              onClose();
            }}
          >
            <span className="template-picker-title">{template.title}</span>
            <span className="template-picker-desc">{template.description}</span>
          </button>
        ))}
      </div>
    </div>
  );
}
