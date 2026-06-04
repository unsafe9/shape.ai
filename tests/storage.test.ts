import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => {
  delete process.env.SHAPE_AI_DATA_DIR;
  vi.resetModules();
});

describe("scene storage", () => {
  async function createTempStorage() {
    const dataDir = await mkdtemp(join(tmpdir(), "shape-ai-storage-"));
    process.env.SHAPE_AI_DATA_DIR = dataDir;
    vi.resetModules();
    const storage = await import("../src/server/storage");
    return { dataDir, storage };
  }

  it("creates groups, registered tags, and viewport-filtered scene reads", async () => {
    const { dataDir, storage } = await createTempStorage();

    const tag = await storage.createTag({ name: "Architecture", color: "#2f7ee6" });
    const created = await storage.createGroup({
      prompt: "Scene storage test",
      tagIds: [tag.tag.id]
    });

    expect(created.group.tagIds).toEqual([tag.tag.id]);
    expect(created.scene.nodes.length).toBeGreaterThan(0);

    const visible = await storage.readScene({
      viewport: created.group.bounds,
      zoom: 0.8,
      tagIds: [tag.tag.id]
    });
    expect(visible.groups.map((group) => group.id)).toContain(created.group.id);
    expect(visible.nodes.length).toBeGreaterThan(0);

    const overview = await storage.readScene({
      viewport: created.group.bounds,
      zoom: 0.05
    });
    expect(overview.groups.length).toBeGreaterThan(0);
    expect(overview.nodes).toHaveLength(0);
    await expect(readFile(join(dataDir, "shape.sqlite"))).resolves.toBeInstanceOf(Buffer);
  });

  it("stores comments and artifacts against group targets", async () => {
    const { storage } = await createTempStorage();
    const created = await storage.createGroup({ prompt: "Group target test" });
    const comment = await storage.addComment({
      target: { kind: "group", id: created.group.id },
      body: "Needs review"
    });
    expect(comment.comment.target).toEqual({ kind: "group", id: created.group.id });

    const persisted = await storage.writeArtifactContent({
      groupId: created.group.id,
      type: "madr",
      title: "Group target test",
      content: "# Group target test",
      contentType: "text/markdown; charset=utf-8"
    });
    const artifact = await storage.addArtifact(created.group.id, {
      type: "madr",
      title: "Group target test",
      target: { kind: "group", id: created.group.id },
      path: persisted.path,
      contentType: persisted.contentType
    });
    expect(artifact.artifact.target).toEqual({ kind: "group", id: created.group.id });
    expect(storage.isExportPath(persisted.path)).toBe(true);
  });

  it("places generated top-level groups without stacking their bounds", async () => {
    const { storage } = await createTempStorage();
    for (let index = 0; index < 12; index += 1) {
      await storage.createGroup({ prompt: `Generated group ${index}` });
    }

    const scene = await storage.readFullScene();
    expect(scene.groups).toHaveLength(12);
    for (let index = 0; index < scene.groups.length; index += 1) {
      for (let next = index + 1; next < scene.groups.length; next += 1) {
        expect(overlapArea(scene.groups[index].bounds, scene.groups[next].bounds)).toBe(0);
      }
    }
  });

  it("shrinks group bounds after deleting edge nodes", async () => {
    const { storage } = await createTempStorage();
    const created = await storage.createGroup({ prompt: "Bounds recompute test" });
    const originalWidth = created.group.bounds.width;
    const edgeNodeIds = created.scene.nodes
      .filter((node) => node.groupId === created.group.id && node.position.x > created.group.bounds.x + originalWidth * 0.65)
      .map((node) => node.id);

    expect(edgeNodeIds.length).toBeGreaterThan(0);
    const scene = await storage.saveScenePatch({ removeNodeIds: edgeNodeIds });
    const updated = scene.groups.find((group) => group.id === created.group.id);

    expect(updated?.bounds.width).toBeLessThan(originalWidth);
  });

  it("limits detailed scene objects to the focused group", async () => {
    const { storage } = await createTempStorage();
    const first = await storage.createGroup({ prompt: "Focused scene group A" });
    const second = await storage.createGroup({ prompt: "Focused scene group B" });
    const viewport = unionBounds([first.group.bounds, second.group.bounds]);

    const scene = await storage.readScene({
      viewport,
      zoom: 0.58,
      focusGroupId: second.group.id
    });

    expect(scene.groups.length).toBeGreaterThan(1);
    expect(scene.nodes.length).toBeGreaterThan(0);
    expect(new Set(scene.nodes.map((node) => node.groupId))).toEqual(new Set([second.group.id]));
  });
});

function overlapArea(
  a: { x: number; y: number; width: number; height: number },
  b: { x: number; y: number; width: number; height: number }
): number {
  const x = Math.max(0, Math.min(a.x + a.width, b.x + b.width) - Math.max(a.x, b.x));
  const y = Math.max(0, Math.min(a.y + a.height, b.y + b.height) - Math.max(a.y, b.y));
  return x * y;
}

function unionBounds(boundsList: Array<{ x: number; y: number; width: number; height: number }>) {
  const minX = Math.min(...boundsList.map((bounds) => bounds.x));
  const minY = Math.min(...boundsList.map((bounds) => bounds.y));
  const maxX = Math.max(...boundsList.map((bounds) => bounds.x + bounds.width));
  const maxY = Math.max(...boundsList.map((bounds) => bounds.y + bounds.height));
  return { x: minX, y: minY, width: maxX - minX, height: maxY - minY };
}
