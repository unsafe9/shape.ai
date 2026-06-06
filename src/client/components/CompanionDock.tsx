import type { McpClientInfo } from "../lib/api";

type Props = {
  clients: McpClientInfo[];
  onFocusTarget?: (target: unknown) => void;
};

const STATE_COLORS: Record<McpClientInfo["dockState"], string> = {
  idle: "var(--ok)",
  active: "var(--accent)",
  error: "var(--bad)",
  disconnected: "var(--muted)",
  muted: "var(--muted)"
};

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

export function CompanionDock({ clients, onFocusTarget }: Props) {
  if (clients.length === 0) {
    return (
      <div className="companion-dock companion-dock-empty" aria-label="MCP companions">
        <span className="companion-dock-empty-label">No MCP clients</span>
      </div>
    );
  }

  const sorted = [...clients].sort((a, b) => {
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
  });

  return (
    <div className="companion-dock" aria-label="MCP companions" role="list">
      {sorted.map((client) => {
        const dotColor = STATE_COLORS[client.dockState];
        const initial = (client.label[0] ?? "?").toUpperCase();
        const isActive = client.dockState === "active";
        const isDisconnected = client.dockState === "disconnected";
        const target = client.lastTarget as { kind?: string } | null;
        const hasSpatialTarget = target !== null && target?.kind !== undefined && target.kind !== "canvas";
        const clickable = hasSpatialTarget && onFocusTarget !== undefined;
        return (
          <div
            key={client.clientId}
            className={`companion-chip ${isActive ? "companion-chip-active" : ""} ${isDisconnected ? "companion-chip-disconnected" : ""} ${clickable ? "companion-chip-focusable" : ""}`}
            title={chipTitle(client)}
            role={clickable ? "button" : "listitem"}
            aria-label={`${client.label}: ${client.dockState}`}
            tabIndex={clickable ? 0 : undefined}
            onClick={clickable ? () => onFocusTarget(client.lastTarget) : undefined}
            onKeyDown={clickable ? (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onFocusTarget(client.lastTarget); } } : undefined}
          >
            <span
              className="companion-chip-icon"
              style={{
                background: client.iconRef ? undefined : client.color,
                borderColor: client.color
              }}
            >
              {client.iconRef ? (
                <img src={client.iconRef} alt={client.label} width={14} height={14} />
              ) : (
                initial
              )}
            </span>
            <span
              className="companion-chip-dot"
              style={{ background: dotColor }}
              aria-hidden="true"
            />
          </div>
        );
      })}
    </div>
  );
}
