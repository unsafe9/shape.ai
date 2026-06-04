import type {
  Bounds,
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
  viewport?: Bounds;
  zoom?: number;
  tagIds?: string[];
  focusGroupId?: string;
};

export async function fetchScene(query: SceneQuery = {}): Promise<Scene> {
  const params = new URLSearchParams();
  if (query.viewport) {
    params.set("x", String(query.viewport.x));
    params.set("y", String(query.viewport.y));
    params.set("width", String(query.viewport.width));
    params.set("height", String(query.viewport.height));
  }
  if (query.zoom) params.set("zoom", String(query.zoom));
  if (query.tagIds?.length) params.set("tags", query.tagIds.join(","));
  if (query.focusGroupId) params.set("focusGroupId", query.focusGroupId);
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
