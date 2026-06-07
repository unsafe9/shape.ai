// Server-side scene operations that are NOT expressible as scene-core ops.
//
// The MG-7 cutover routes every operation that IS a scene-core op
// (`RenderScenePatch`) through the WS transport client (`SceneClient`). The
// operations here are the residual that the WS op path cannot express and that
// genuinely run on the server:
//
//   - createGroup : a SERVER-SIDE seed of a ~10-node / 9-edge decision graph
//                   (crates/server/src/group_seed.rs), not a plain op.
//   - exportGroup : SERVER-SIDE artifact generation + file write
//                   (crates/server/src/local_export.rs).
//   - createComment / updateComment : `add-comment` is an ExtendedOpPatch with no
//                   scene-core apply path, and comment update has no op at all.
//
// They keep using the Rust server's `/api/*` routes (crates/server/src/scene_api.rs)
// until a WS request/response RPC seam exists for non-op, server-computed
// operations. Everything else moved to the WS client; this is the documented
// minimal HTTP residual.

import type {
  CreateCommentRequest,
  ExportOutput,
  ExportRequest,
  ExportType,
  Scene,
  SceneArtifact,
  SceneGroup
} from "../../shared/schema";

export async function createGroup(
  prompt: string,
  title?: string,
  tagIds?: string[]
): Promise<{ group: SceneGroup; scene: Scene; message: string }> {
  return request("/api/groups", {
    method: "POST",
    body: JSON.stringify({ prompt, title, tagIds })
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
