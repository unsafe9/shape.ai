import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => {
  delete process.env.SHAPE_AI_DATA_DIR;
  delete process.env.LOG_LEVEL;
  vi.resetModules();
});

describe("group API exports", () => {
  it("returns generated preview content with export metadata", async () => {
    process.env.SHAPE_AI_DATA_DIR = await mkdtemp(join(tmpdir(), "shape-ai-api-"));
    process.env.LOG_LEVEL = "silent";
    vi.resetModules();
    const [{ buildServer }, { ensureStorage }] = await Promise.all([import("../src/server/index"), import("../src/server/storage")]);
    await ensureStorage();
    const app = await buildServer();

    try {
      const created = await app.inject({
        method: "POST",
        url: "/api/groups",
        payload: { prompt: "Preview export content" }
      });
      expect(created.statusCode).toBe(201);
      const groupId = created.json().group.id;

      const exported = await app.inject({
        method: "POST",
        url: `/api/groups/${groupId}/export`,
        payload: { type: "madr", scope: { kind: "group", id: groupId } }
      });

      expect(exported.statusCode).toBe(200);
      const body = exported.json();
      expect(body.artifact.type).toBe("madr");
      expect(body.preview.type).toBe("madr");
      expect(body.preview.contentType).toContain("markdown");
      expect(body.preview.content).toContain("## Context and Problem Statement");
    } finally {
      await app.close();
    }
  });

  it("keeps narrowed export artifacts downloadable from their group", async () => {
    process.env.SHAPE_AI_DATA_DIR = await mkdtemp(join(tmpdir(), "shape-ai-api-"));
    process.env.LOG_LEVEL = "silent";
    vi.resetModules();
    const [{ buildServer }, { ensureStorage }] = await Promise.all([import("../src/server/index"), import("../src/server/storage")]);
    await ensureStorage();
    const app = await buildServer();

    try {
      const created = await app.inject({
        method: "POST",
        url: "/api/groups",
        payload: { prompt: "Node scoped export content" }
      });
      expect(created.statusCode).toBe(201);
      const body = created.json();
      const groupId = body.group.id;
      const nodeId = body.scene.nodes.find((node: { groupId: string }) => node.groupId === groupId).id;

      const exported = await app.inject({
        method: "POST",
        url: `/api/groups/${groupId}/export`,
        payload: { type: "madr", scope: { kind: "node", id: nodeId } }
      });

      expect(exported.statusCode).toBe(200);
      const exportBody = exported.json();
      expect(exportBody.artifact.target).toEqual({ kind: "group", id: groupId });

      const downloaded = await app.inject({
        method: "GET",
        url: `/api/groups/${groupId}/artifacts/${exportBody.artifact.id}`
      });

      expect(downloaded.statusCode).toBe(200);
      expect(downloaded.body).toContain("## Context and Problem Statement");
    } finally {
      await app.close();
    }
  });
});
