import { describe, expect, it } from "vitest";
import {
  decideFollowCommand,
  initialFollowState,
  jumpToCurrentTarget,
  onUserGrab,
  pauseFollow,
  reconcileFollowee,
  resolveFollowee,
  resumeFollow,
  stopFollow,
  targetKey,
  toggleFollow,
  type FollowState
} from "../src/client/lib/followController";
import type { McpClientInfo } from "../src/client/lib/api";

function makeClient(overrides: Partial<McpClientInfo> = {}): McpClientInfo {
  return {
    clientId: "c1",
    actorType: "mcp",
    label: "Claude",
    name: "claude-code",
    version: "1.0.0",
    color: "#3d82e0",
    iconRef: null,
    transport: "http",
    dockState: "active",
    lastTarget: { kind: "group", id: "g1" },
    connectedAt: 0,
    lastActivityAt: 0,
    muted: false,
    ...overrides
  };
}

const pinnedOn = (client: McpClientInfo, lastFramedKey: string | null = null): FollowState => ({
  mode: "pinned",
  followeeClientId: client.clientId,
  lastFramedKey
});

describe("targetKey", () => {
  it("derives stable keys for spatial targets and null for non-spatial", () => {
    expect(targetKey({ kind: "group", id: "g1" })).toBe("group:g1");
    expect(targetKey({ kind: "node", id: "n2" })).toBe("node:n2");
    expect(targetKey({ kind: "artifact", artifactId: "a1", groupId: "g1" })).toBe("artifact:g1:a1");
    expect(targetKey({ kind: "selection", ids: [{ kind: "node", id: "n1" }, { kind: "group", id: "g2" }] }))
      .toBe("selection:node:n1,group:g2");
    expect(targetKey({ kind: "canvas" })).toBeNull();
    expect(targetKey(null)).toBeNull();
    expect(targetKey({ kind: "group" })).toBeNull();
  });
});

describe("toggleFollow (§2 click-to-follow, §7 handoff)", () => {
  it("pins follow to a followable client", () => {
    const next = toggleFollow(initialFollowState, makeClient());
    expect(next.mode).toBe("pinned");
    expect(next.followeeClientId).toBe("c1");
  });

  it("does not follow a muted or disconnected client", () => {
    expect(toggleFollow(initialFollowState, makeClient({ dockState: "muted" }))).toBe(initialFollowState);
    expect(toggleFollow(initialFollowState, makeClient({ dockState: "disconnected" }))).toBe(initialFollowState);
  });

  it("clicking the followed client again hard-handoffs back to off", () => {
    const client = makeClient();
    const pinned = toggleFollow(initialFollowState, client);
    const off = toggleFollow(pinned, client);
    expect(off.mode).toBe("off");
    expect(off.followeeClientId).toBeNull();
  });
});

describe("pause / resume (§4)", () => {
  it("pause freezes the camera but retains the followee", () => {
    const client = makeClient();
    const paused = pauseFollow(pinnedOn(client));
    expect(paused.mode).toBe("paused");
    expect(paused.followeeClientId).toBe("c1");
  });

  it("resume re-pins and clears lastFramedKey so the live target re-frames", () => {
    const client = makeClient();
    const resumed = resumeFollow({ mode: "paused", followeeClientId: "c1", lastFramedKey: "group:old" });
    expect(resumed.mode).toBe("pinned");
    expect(resumed.lastFramedKey).toBeNull();
  });

  it("resume is a no-op when there is no followee", () => {
    expect(resumeFollow(initialFollowState)).toBe(initialFollowState);
  });
});

describe("onUserGrab — §8 never steal control", () => {
  it("a user grab during pinned follow demotes to paused (soft handoff, followee kept)", () => {
    const grabbed = onUserGrab(pinnedOn(makeClient()));
    expect(grabbed.mode).toBe("paused");
    expect(grabbed.followeeClientId).toBe("c1");
  });

  it("a user grab while paused or off is a no-op", () => {
    expect(onUserGrab({ mode: "paused", followeeClientId: "c1", lastFramedKey: null }).mode).toBe("paused");
    expect(onUserGrab(initialFollowState)).toBe(initialFollowState);
  });
});

describe("decideFollowCommand (§3 pinned re-frame, §8 guard)", () => {
  it("re-frames when the live target changes to a new spatial target", () => {
    const client = makeClient({ lastTarget: { kind: "node", id: "n9" } });
    const cmd = decideFollowCommand(pinnedOn(client, "group:g1"), client, false);
    expect(cmd.target).toEqual({ kind: "node", id: "n9" });
    expect(cmd.state.lastFramedKey).toBe("node:n9");
  });

  it("does not re-frame the same target twice", () => {
    const client = makeClient({ lastTarget: { kind: "group", id: "g1" } });
    const cmd = decideFollowCommand(pinnedOn(client, "group:g1"), client, false);
    expect(cmd.target).toBeNull();
  });

  it("issues no camera command while a user gesture is active (§8 rule 1)", () => {
    const client = makeClient({ lastTarget: { kind: "node", id: "n9" } });
    const cmd = decideFollowCommand(pinnedOn(client, "group:g1"), client, true);
    expect(cmd.target).toBeNull();
    expect(cmd.state.lastFramedKey).toBe("group:g1");
  });

  it("issues no camera command when paused (§8 rule 4)", () => {
    const client = makeClient({ lastTarget: { kind: "node", id: "n9" } });
    const cmd = decideFollowCommand({ mode: "paused", followeeClientId: "c1", lastFramedKey: null }, client, false);
    expect(cmd.target).toBeNull();
  });

  it("ignores non-spatial targets (canvas-wide reads do not move the camera)", () => {
    const client = makeClient({ lastTarget: { kind: "canvas" } });
    const cmd = decideFollowCommand(pinnedOn(client, null), client, false);
    expect(cmd.target).toBeNull();
  });
});

describe("jumpToCurrentTarget (§5)", () => {
  it("returns the live target without changing mode", () => {
    const client = makeClient({ lastTarget: { kind: "edge", id: "e1" } });
    expect(jumpToCurrentTarget(client)).toEqual({ kind: "edge", id: "e1" });
  });

  it("returns null for non-spatial / absent followee", () => {
    expect(jumpToCurrentTarget(makeClient({ lastTarget: { kind: "canvas" } }))).toBeNull();
    expect(jumpToCurrentTarget(null)).toBeNull();
  });
});

describe("stopFollow / reconcileFollowee (§1/§7/§8)", () => {
  it("stopFollow returns to a clean off state", () => {
    const off = stopFollow(pinnedOn(makeClient()));
    expect(off).toEqual(initialFollowState);
  });

  it("reconcile drops follow when the followee disconnects or is muted", () => {
    const state = pinnedOn(makeClient());
    expect(reconcileFollowee(state, [makeClient({ dockState: "disconnected" })]).mode).toBe("off");
    expect(reconcileFollowee(state, [makeClient({ muted: true })]).mode).toBe("off");
    expect(reconcileFollowee(state, []).mode).toBe("off");
  });

  it("reconcile keeps follow while the followee is present and live", () => {
    const state = pinnedOn(makeClient());
    expect(reconcileFollowee(state, [makeClient()])).toBe(state);
  });
});

describe("resolveFollowee", () => {
  it("resolves the live record for the followee id", () => {
    const client = makeClient();
    expect(resolveFollowee(pinnedOn(client), [client])).toBe(client);
    expect(resolveFollowee(initialFollowState, [client])).toBeNull();
  });
});
