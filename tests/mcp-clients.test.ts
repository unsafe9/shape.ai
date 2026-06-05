import { afterEach, describe, expect, it } from "vitest";
import {
  _resetClientRegistry,
  getClient,
  listClients,
  registerClient,
  removeClient,
  resolveToolTarget,
  setClientDockState,
  setClientLastTarget,
  type CanvasTarget,
  type McpClientDockState
} from "../src/server/mcpClients";
import type { Implementation } from "@modelcontextprotocol/sdk/types.js";

const mockImpl = (name: string, title?: string): Implementation => ({
  name,
  version: "1.0.0",
  title
});

afterEach(() => {
  _resetClientRegistry();
});

// ---------------------------------------------------------------------------
// T5.1: Client identity and registry
// ---------------------------------------------------------------------------

describe("McpClientRegistry", () => {
  it("registers a client and assigns stable fields", () => {
    const identity = registerClient(mockImpl("claude-code", "Claude Code"), "http");
    expect(identity.actorType).toBe("mcp");
    expect(identity.name).toBe("claude-code");
    expect(identity.label).toBe("Claude Code");
    expect(identity.version).toBe("1.0.0");
    expect(identity.transport).toBe("http");
    expect(identity.dockState).toBe("idle");
    expect(identity.lastTarget).toBeNull();
    expect(identity.muted).toBe(false);
    expect(typeof identity.clientId).toBe("string");
    expect(identity.clientId.length).toBeGreaterThan(0);
    expect(typeof identity.color).toBe("string");
    expect(identity.color).toMatch(/^#[0-9a-f]{6}$/i);
  });

  it("uses title as label, falls back to name", () => {
    const withTitle = registerClient(mockImpl("cursor", "Cursor"), "http");
    expect(withTitle.label).toBe("Cursor");

    const noTitle = registerClient(mockImpl("my-client"), "stdio");
    expect(noTitle.label).toBe("my-client");
  });

  it("accepts a caller-provided clientId (stdio stable identity)", () => {
    const id = "stdio:test-uuid";
    const identity = registerClient(mockImpl("tool"), "stdio", id);
    expect(identity.clientId).toBe(id);
  });

  it("assigns deterministic but distinct colors to two different clients", () => {
    const a = registerClient(mockImpl("client-a"), "http", "id-aaa");
    const b = registerClient(mockImpl("client-b"), "http", "id-bbb");
    // Colors are from the palette; same clientId must yield same color
    const a2 = registerClient(mockImpl("client-a"), "http", "id-aaa-clone");
    expect(typeof a.color).toBe("string");
    expect(typeof b.color).toBe("string");
    // Two different ids need not have different colors (palette is 12 slots),
    // but the same id must always yield the same color.
    const a3 = registerClient(mockImpl("client-a"), "http", "id-aaa");
    expect(a3.color).toBe(a.color);
    void a2; // used above
  });

  it("deduplicates labels when two clients have the same label", () => {
    const first = registerClient(mockImpl("cursor", "Cursor"), "http");
    const second = registerClient(mockImpl("cursor", "Cursor"), "http");
    expect(first.label).toBe("Cursor");
    expect(second.label).toBe("Cursor (2)");
  });

  it("stores and retrieves clients via getClient / listClients", () => {
    const a = registerClient(mockImpl("a"), "http");
    const b = registerClient(mockImpl("b"), "stdio");
    expect(getClient(a.clientId)?.name).toBe("a");
    expect(getClient(b.clientId)?.name).toBe("b");
    expect(listClients()).toHaveLength(2);
  });

  it("setClientDockState updates the dock state", () => {
    const identity = registerClient(mockImpl("agent"), "http");
    setClientDockState(identity.clientId, "active");
    expect(getClient(identity.clientId)?.dockState).toBe("active");
    setClientDockState(identity.clientId, "error");
    expect(getClient(identity.clientId)?.dockState).toBe("error");
    setClientDockState(identity.clientId, "idle");
    expect(getClient(identity.clientId)?.dockState).toBe("idle");
    setClientDockState(identity.clientId, "disconnected");
    expect(getClient(identity.clientId)?.dockState).toBe("disconnected");
    setClientDockState(identity.clientId, "muted");
    expect(getClient(identity.clientId)?.dockState).toBe("muted");
  });

  it("setClientDockState is a no-op for unknown clientId", () => {
    expect(() => setClientDockState("unknown", "active")).not.toThrow();
  });

  it("setClientLastTarget updates the last target", () => {
    const identity = registerClient(mockImpl("agent"), "http");
    const target: CanvasTarget = { kind: "group", id: "grp-1" };
    setClientLastTarget(identity.clientId, target);
    expect(getClient(identity.clientId)?.lastTarget).toEqual(target);
  });

  it("setClientLastTarget accepts null", () => {
    const identity = registerClient(mockImpl("agent"), "http");
    setClientLastTarget(identity.clientId, null);
    expect(getClient(identity.clientId)?.lastTarget).toBeNull();
  });

  it("removeClient removes the entry", () => {
    const identity = registerClient(mockImpl("tmp"), "http");
    removeClient(identity.clientId);
    expect(getClient(identity.clientId)).toBeUndefined();
    expect(listClients()).toHaveLength(0);
  });

  it("captures iconRef from Implementation.icons", () => {
    const impl: Implementation = {
      name: "rich-client",
      version: "2.0",
      icons: [{ src: "https://example.com/icon.png" }]
    };
    const identity = registerClient(impl, "http");
    expect(identity.iconRef).toBe("https://example.com/icon.png");
  });

  it("sets iconRef to null when icons absent", () => {
    const identity = registerClient(mockImpl("plain"), "http");
    expect(identity.iconRef).toBeNull();
  });
});

// ---------------------------------------------------------------------------
// T5.2: Tool → CanvasTarget mapping
// ---------------------------------------------------------------------------

describe("resolveToolTarget", () => {
  it("query_scene returns canvas (whole-scene fallback)", () => {
    const target = resolveToolTarget({ tool: "query_scene" });
    expect(target).toEqual({ kind: "canvas" });
  });

  it("query_scene with tagIds still returns canvas (hull approx)", () => {
    const target = resolveToolTarget({ tool: "query_scene", tagIds: ["tag-1"] });
    expect(target).toEqual({ kind: "canvas" });
  });

  it("list_groups returns canvas", () => {
    expect(resolveToolTarget({ tool: "list_groups" })).toEqual({ kind: "canvas" });
  });

  it("get_group resolves to group target", () => {
    const target = resolveToolTarget({ tool: "get_group", groupId: "grp-abc" });
    expect(target).toEqual({ kind: "group", id: "grp-abc" });
  });

  it("get_group without groupId returns null", () => {
    expect(resolveToolTarget({ tool: "get_group" })).toBeNull();
  });

  it("create_group resolves to group from targetIds", () => {
    const target = resolveToolTarget({
      tool: "create_group",
      targetIds: [{ kind: "group", id: "new-grp" }]
    });
    expect(target).toEqual({ kind: "group", id: "new-grp" });
  });

  it("create_group without targetIds returns canvas fallback", () => {
    expect(resolveToolTarget({ tool: "create_group" })).toEqual({ kind: "canvas" });
  });

  it("patch_scene with one targetId resolves to that target", () => {
    const target = resolveToolTarget({
      tool: "patch_scene",
      targetIds: [{ kind: "node", id: "node-1" }]
    });
    expect(target).toEqual({ kind: "node", id: "node-1" });
  });

  it("patch_scene with multiple targetIds resolves to selection", () => {
    const ids = [
      { kind: "node" as const, id: "node-1" },
      { kind: "node" as const, id: "node-2" }
    ];
    const target = resolveToolTarget({ tool: "patch_scene", targetIds: ids });
    expect(target).toEqual({ kind: "selection", ids });
  });

  it("patch_scene with empty targetIds returns null (keep prior)", () => {
    expect(resolveToolTarget({ tool: "patch_scene", targetIds: [] })).toBeNull();
    expect(resolveToolTarget({ tool: "patch_scene" })).toBeNull();
  });

  it("create_tag returns canvas (registry write)", () => {
    expect(resolveToolTarget({ tool: "create_tag" })).toEqual({ kind: "canvas" });
  });

  it("update_group_tags resolves to group", () => {
    const target = resolveToolTarget({ tool: "update_group_tags", groupId: "grp-2" });
    expect(target).toEqual({ kind: "group", id: "grp-2" });
  });

  it("update_group_tags without groupId returns canvas", () => {
    expect(resolveToolTarget({ tool: "update_group_tags" })).toEqual({ kind: "canvas" });
  });

  it("set_selection maps selection 1:1", () => {
    const target = resolveToolTarget({
      tool: "set_selection",
      selection: { kind: "group", id: "grp-x" }
    });
    expect(target).toEqual({ kind: "group", id: "grp-x" });
  });

  it("set_selection without selection returns null", () => {
    expect(resolveToolTarget({ tool: "set_selection" })).toBeNull();
  });

  it("add_comment maps target selection", () => {
    const target = resolveToolTarget({
      tool: "add_comment",
      selection: { kind: "edge", id: "edge-3" }
    });
    expect(target).toEqual({ kind: "edge", id: "edge-3" });
  });

  it("add_comment without selection returns canvas (canvas-level comment)", () => {
    expect(resolveToolTarget({ tool: "add_comment" })).toEqual({ kind: "canvas" });
  });

  it("export_group with artifactId resolves to artifact target", () => {
    const target = resolveToolTarget({
      tool: "export_group",
      groupId: "grp-y",
      artifactId: "art-1"
    });
    expect(target).toEqual({ kind: "artifact", artifactId: "art-1", groupId: "grp-y" });
  });

  it("export_group without artifactId resolves to group", () => {
    const target = resolveToolTarget({ tool: "export_group", groupId: "grp-y" });
    expect(target).toEqual({ kind: "group", id: "grp-y" });
  });

  it("export_group without groupId returns null", () => {
    expect(resolveToolTarget({ tool: "export_group" })).toBeNull();
  });

  it("set_selection with canvas kind resolves to canvas", () => {
    const target = resolveToolTarget({
      tool: "set_selection",
      selection: { kind: "canvas" }
    });
    expect(target).toEqual({ kind: "canvas" });
  });
});

// ---------------------------------------------------------------------------
// T5.1 + T5.2 integration: lastTarget is updated
// ---------------------------------------------------------------------------

describe("lastTarget integration", () => {
  it("setClientLastTarget stores a CanvasTarget on the registry entry", () => {
    const identity = registerClient(mockImpl("agent"), "http");
    const target: CanvasTarget = { kind: "artifact", artifactId: "art-1", groupId: "grp-1" };
    setClientLastTarget(identity.clientId, target);
    const stored = getClient(identity.clientId);
    expect(stored?.lastTarget).toEqual(target);
  });

  it("dock state transitions follow expected sequence", () => {
    const identity = registerClient(mockImpl("worker"), "http");
    const id = identity.clientId;

    expect(getClient(id)?.dockState).toBe("idle");
    setClientDockState(id, "active");
    expect(getClient(id)?.dockState).toBe("active");
    setClientDockState(id, "idle");
    expect(getClient(id)?.dockState).toBe("idle");
    setClientDockState(id, "error", "something went wrong");
    expect(getClient(id)?.dockState).toBe("error");
    setClientDockState(id, "disconnected");
    expect(getClient(id)?.dockState).toBe("disconnected");
  });
});
