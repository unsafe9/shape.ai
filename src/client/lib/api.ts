import type {
  CreateCommentRequest,
  ExportOutput,
  ExportRequest,
  ExportType,
  Scene,
  SceneArtifact,
  SceneGroup,
  ScenePatch,
  Tag
} from "../../shared/schema";

type SceneQuery = {
  tagIds?: string[];
};

export async function fetchScene(query: SceneQuery = {}): Promise<Scene> {
  const params = new URLSearchParams();
  if (query.tagIds?.length) params.set("tags", query.tagIds.join(","));
  const suffix = params.toString() ? `?${params}` : "";
  const data = await request<{ scene: Scene }>(`/api/scene${suffix}`);
  return data.scene;
}

export async function createGroup(prompt: string, title?: string, tagIds?: string[]): Promise<{ group: SceneGroup; scene: Scene; message: string }> {
  return request("/api/groups", {
    method: "POST",
    body: JSON.stringify({ prompt, title, tagIds })
  });
}

export async function saveScenePatch(body: ScenePatch): Promise<{ scene: Scene }> {
  return request("/api/scene", {
    method: "PATCH",
    body: JSON.stringify(body)
  });
}

export async function createTag(name: string, color: string, description = ""): Promise<{ tag: Tag; scene: Scene }> {
  return request("/api/tags", {
    method: "POST",
    body: JSON.stringify({ name, color, description })
  });
}

export async function updateGroupTags(groupId: string, tagIds: string[]): Promise<{ group: SceneGroup; scene: Scene }> {
  return request(`/api/groups/${groupId}/tags`, {
    method: "PATCH",
    body: JSON.stringify({ tagIds })
  });
}

export async function exportGroup(
  groupId: string,
  type: ExportType,
  scope?: ExportRequest["scope"]
): Promise<{ scene: Scene; group: SceneGroup; artifact: SceneArtifact; preview: ExportOutput & { type: ExportType; contentType: string } }> {
  return request(`/api/groups/${groupId}/export`, {
    method: "POST",
    body: JSON.stringify({ type, scope })
  });
}

export async function createComment(body: CreateCommentRequest): Promise<{ scene: Scene }> {
  return request("/api/comments", {
    method: "POST",
    body: JSON.stringify(body)
  });
}

export async function updateComment(
  commentId: string,
  body: { body?: string; resolved?: boolean }
): Promise<{ scene: Scene }> {
  return request(`/api/comments/${commentId}`, {
    method: "PATCH",
    body: JSON.stringify(body)
  });
}

// ---------------------------------------------------------------------------
// MCP companion state — mirrors src/server/mcpClients.ts (ephemeral, HTTP only)
// ---------------------------------------------------------------------------

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
