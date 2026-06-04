import type {
  CreateCommentRequest,
  DecisionGraph,
  ExportRequest,
  ExportType,
  GraphLayout,
  GraphSelection,
  Shape,
  ShapeArtifact,
  UpdateCommentRequest
} from "../../shared/schema";

export type RuntimeStatus = {
  ok: boolean;
  dataRoot: string;
  mcp: {
    command: string;
    transport?: string;
    stdioTransport?: string;
    remoteTransport?: string;
    url?: string;
  };
};

export async function listShapes(): Promise<Shape[]> {
  const data = await request<{ shapes: Shape[] }>("/api/shapes");
  return data.shapes;
}

export async function getRuntime(): Promise<RuntimeStatus> {
  return request<RuntimeStatus>("/api/health");
}

export async function createShape(prompt: string, title?: string): Promise<{ shape: Shape; message: string }> {
  return request("/api/shapes", {
    method: "POST",
    body: JSON.stringify({ prompt, title })
  });
}

export async function exportShape(
  id: string,
  type: ExportType,
  body: ExportRequest["scope"]
): Promise<{ shape: Shape; artifact: ShapeArtifact }> {
  return request(`/api/shapes/${id}/export`, {
    method: "POST",
    body: JSON.stringify({ type, scope: body })
  });
}

export async function saveGraphEdit(
  id: string,
  body: { graph?: DecisionGraph; layout?: GraphLayout; selection?: GraphSelection }
): Promise<{ shape: Shape }> {
  return request(`/api/shapes/${id}/graph`, {
    method: "PATCH",
    body: JSON.stringify(body)
  });
}

export async function createComment(
  id: string,
  body: CreateCommentRequest
): Promise<{ shape: Shape }> {
  return request(`/api/shapes/${id}/comments`, {
    method: "POST",
    body: JSON.stringify(body)
  });
}

export async function updateComment(
  id: string,
  commentId: string,
  body: UpdateCommentRequest
): Promise<{ shape: Shape }> {
  return request(`/api/shapes/${id}/comments/${commentId}`, {
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
