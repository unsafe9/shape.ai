import type { CreateCommentRequest, ExportOutput, ExportRequest, ExportType, Scene, SceneArtifact, SceneComment, SceneGroup, ScenePatch } from "../../../../src/shared/schema";

const apiBase = import.meta.env.VITE_SHAPE_AI_API_BASE ?? "";

export async function fetchAppScene(): Promise<Scene> {
  const data = await request<{ scene: Scene }>("/api/scene?zoom=1");
  return data.scene;
}

export async function saveAppScenePatch(patch: ScenePatch): Promise<Scene> {
  const data = await request<{ scene: Scene }>("/api/scene", {
    method: "PATCH",
    body: JSON.stringify(patch)
  });
  return data.scene;
}

export async function updateAppGroupTags(groupId: string, tagIds: string[]): Promise<{ group: SceneGroup; scene: Scene }> {
  return request(`/api/groups/${groupId}/tags`, {
    method: "PATCH",
    body: JSON.stringify({ tagIds })
  });
}

export async function createAppComment(body: CreateCommentRequest): Promise<{ comment: SceneComment; scene: Scene }> {
  return request("/api/comments", {
    method: "POST",
    body: JSON.stringify(body)
  });
}

export async function exportAppGroup(
  groupId: string,
  type: ExportType,
  scope?: ExportRequest["scope"]
): Promise<{ scene: Scene; group: SceneGroup; artifact: SceneArtifact; preview: ExportOutput & { type: ExportType; contentType: string } }> {
  return request(`/api/groups/${groupId}/export`, {
    method: "POST",
    body: JSON.stringify({ type, scope })
  });
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`${apiBase}${path}`, {
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
