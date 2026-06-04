import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => {
  delete process.env.SHAPE_AI_DATA_DIR;
  vi.resetModules();
});

describe("SQLite storage proposals", () => {
  async function createTempStorage() {
    const dataDir = await mkdtemp(join(tmpdir(), "shape-ai-storage-"));
    process.env.SHAPE_AI_DATA_DIR = dataDir;
    vi.resetModules();
    const [{ seedDesignGraph }, storage] = await Promise.all([
      import("../src/server/local"),
      import("../src/server/storage")
    ]);
    return { dataDir, seedDesignGraph, storage };
  }

  it("keeps proposal patches out of the canonical graph until approval", async () => {
    const { dataDir, seedDesignGraph, storage } = await createTempStorage();

    const seed = seedDesignGraph("SQLite proposal test");
    const design = await storage.createDesign({
      title: seed.title,
      prompt: "SQLite proposal test",
      graph: seed.graph
    });
    const proposal = await storage.createProposal({
      designId: design.id,
      title: "Add evidence node"
    });

    await storage.appendProposalPatch({
      proposalId: proposal.id,
      patch: {
        addNodes: [
          {
            id: "n-extra",
            type: "evidence",
            title: "Extra evidence",
            summary: "Added by proposal",
            detail: "Added by proposal",
            status: "draft",
            confidence: 0.5,
            evidenceRefs: [],
            childDecisionIds: []
          }
        ],
        updateNodes: [],
        removeNodeIds: [],
        addEdges: [],
        updateEdges: [],
        removeEdgeIds: []
      }
    });

    expect((await storage.readDesign(design.id))?.graph.nodes.some((node) => node.id === "n-extra")).toBe(false);

    const validation = await storage.validateProposal(proposal.id);
    expect(validation.validation.status).toBe("clean");

    const approved = await storage.approveProposal(proposal.id);
    expect(approved.design.graphVersion).toBe(1);
    expect((await storage.readDesign(design.id))?.graph.nodes.some((node) => node.id === "n-extra")).toBe(true);
    await expect(readFile(join(dataDir, "shape.sqlite"))).resolves.toBeInstanceOf(Buffer);
  });

  it("rolls back proposal validation updates when approval fails", async () => {
    const { seedDesignGraph, storage } = await createTempStorage();

    const seed = seedDesignGraph("Rollback proposal test");
    const design = await storage.createDesign({
      title: seed.title,
      prompt: "Rollback proposal test",
      graph: seed.graph
    });
    const proposal = await storage.createProposal({
      designId: design.id,
      title: "Stale proposal"
    });
    await storage.updateDesignGraph(design, {
      ...design.graph,
      nodes: design.graph.nodes.map((node) =>
        node.id === "n-proposition" ? { ...node, title: "Changed canonical graph" } : node
      )
    });

    await expect(storage.approveProposal(proposal.id)).rejects.toThrow("needs_rebase");

    const [storedProposal] = await storage.listOpenProposals(design.id);
    expect(storedProposal.validationStatus).toBe("clean");
  });
});
