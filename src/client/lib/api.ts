import type {
  CreateCommentRequest,
  DecisionGraph,
  Design,
  DesignArtifact,
  ExportRequest,
  ExportType,
  GraphLayout,
  GraphSelection,
  UpdateCommentRequest
} from "../../shared/schema";

export type RuntimeStatus = {
  ok: boolean;
  dataRoot: string;
  mcp: {
    command: string;
    transport: string;
  };
};

export async function listDesigns(): Promise<Design[]> {
  const data = await request<{ designs: Design[] }>("/api/designs");
  return data.designs;
}

export async function getRuntime(): Promise<RuntimeStatus> {
  return request<RuntimeStatus>("/api/health");
}

export async function createDesign(prompt: string, title?: string): Promise<{ design: Design; message: string }> {
  return request("/api/designs", {
    method: "POST",
    body: JSON.stringify({ prompt, title })
  });
}

export async function exportDesign(
  id: string,
  type: ExportType,
  body: ExportRequest["scope"]
): Promise<{ design: Design; artifact: DesignArtifact }> {
  return request(`/api/designs/${id}/export`, {
    method: "POST",
    body: JSON.stringify({ type, scope: body })
  });
}

export async function saveGraphEdit(
  id: string,
  body: { graph?: DecisionGraph; layout?: GraphLayout; selection?: GraphSelection }
): Promise<{ design: Design }> {
  return request(`/api/designs/${id}/graph`, {
    method: "PATCH",
    body: JSON.stringify(body)
  });
}

export async function createComment(
  id: string,
  body: CreateCommentRequest
): Promise<{ design: Design }> {
  return request(`/api/designs/${id}/comments`, {
    method: "POST",
    body: JSON.stringify(body)
  });
}

export async function updateComment(
  id: string,
  commentId: string,
  body: UpdateCommentRequest
): Promise<{ design: Design }> {
  return request(`/api/designs/${id}/comments/${commentId}`, {
    method: "PATCH",
    body: JSON.stringify(body)
  });
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(path, {
    headers: {
      "Content-Type": "application/json",
      ...(init.headers ?? {})
    },
    ...init
  });
  if (!response.ok) {
    const body = await response.json().catch(() => ({}));
    throw new Error(body.message || `Request failed: ${response.status}`);
  }
  return response.json() as Promise<T>;
}
