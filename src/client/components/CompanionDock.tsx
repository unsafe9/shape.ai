import type { McpClientInfo } from "../lib/api";

type Props = {
  clients: McpClientInfo[];
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

export function CompanionDock({ clients }: Props) {
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
        return (
          <div
            key={client.clientId}
            className={`companion-chip ${isActive ? "companion-chip-active" : ""} ${isDisconnected ? "companion-chip-disconnected" : ""}`}
            title={chipTitle(client)}
            role="listitem"
            aria-label={`${client.label}: ${client.dockState}`}
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
