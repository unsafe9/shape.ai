// MCP companion dock read API — ephemeral HTTP, NOT the scene data path.
//
// The dock surfaces live MCP companions and their recent operation trace. These
// are read-only ephemeral views served by the Rust server's `/api/mcp/*` routes
// (crates/server/src/app.rs), distinct from the scene document path (which flows
// through the WS transport client). The scene HTTP client (`api.ts`) was retired
// in the MG-7 cutover; this companion-dock read API stays on HTTP because it
// reflects transient process state, not persisted canvas documents.

export type McpClientDockState = "idle" | "active" | "error" | "disconnected" | "muted";

export type McpClientInfo = {
  clientId: string;
  actorType: "mcp";
  label: string;
  name: string;
  version: string;
  color: string;
  iconRef: string | null;
  transport: "stdio" | "http";
  dockState: McpClientDockState;
  lastTarget: unknown;
  connectedAt: number;
  lastActivityAt: number;
  muted: boolean;
};

export type McpTraceKind =
  | "read" | "write" | "comment" | "export"
  | "proposal-created" | "proposal-accepted" | "proposal-rejected" | "error";

export type McpTraceEvent = {
  clientId: string;
  kind: McpTraceKind;
  target: unknown;
  verb: string;
  operationId: string | null;
  callId: string | null;
  at: number;
  summary: string;
  errorMessage: string | null;
};

export async function fetchMcpClients(): Promise<{ clients: McpClientInfo[] }> {
  return request("/api/mcp/clients");
}

export async function fetchMcpTrace(
  clientId: string,
  limit?: number
): Promise<{ clientId: string; trace: McpTraceEvent[]; total: number }> {
  const params = new URLSearchParams({ clientId });
  if (limit !== undefined) params.set("limit", String(limit));
  return request(`/api/mcp/trace?${params}`);
}

async function request<T>(url: string, init?: RequestInit): Promise<T> {
  const response = await fetch(url, {
    headers: {
      "content-type": "application/json"
    },
    ...init
  });
  if (!response.ok) {
    const error = await response.json().catch(() => ({ message: response.statusText }));
    throw new Error(error.message || response.statusText);
  }
  return (await response.json()) as T;
}
