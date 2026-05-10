import { env } from "cloudflare:workers";
import { describe, it, expect } from "vitest";
import {
  getPackIndex,
  getPack,
  putPack,
  putPackIndex,
  getVote,
  putVote,
  deleteVote,
} from "../src/kv";
import type { Pack, PackIndex } from "../src/types";

function makePack(id: string): Pack {
  return {
    id,
    title: `Pack ${id}`,
    description: "Test",
    pack_type: "addon-pack",
    author_id: "1",
    author_name: "tester",
    is_anonymous: false,
    addons: [{ esouiId: 1, name: "Addon", required: true }],
    tags: [],
    vote_count: 0,
    install_count: 0,
    created_at: "2025-01-01T00:00:00.000Z",
    updated_at: "2025-01-01T00:00:00.000Z",
    status: "published",
  };
}

describe("pack KV operations", () => {
  it("returns null for missing pack index", async () => {
    const index = await getPackIndex(env as unknown as import("../src/types").Env);
    expect(index).toBeNull();
  });

  it("stores and retrieves a pack", async () => {
    const pack = makePack("test-pack");
    const e = env as unknown as import("../src/types").Env;
    await putPack(e, pack);
    const result = await getPack(e, "test-pack");
    expect(result).toEqual(pack);
  });

  it("returns null for missing pack", async () => {
    const result = await getPack(env as unknown as import("../src/types").Env, "nonexistent");
    expect(result).toBeNull();
  });

  it("stores and retrieves a pack index", async () => {
    const e = env as unknown as import("../src/types").Env;
    const index: PackIndex = { packs: [makePack("a"), makePack("b")] };
    await putPackIndex(e, index);
    const result = await getPackIndex(e);
    expect(result).toEqual(index);
    expect(result!.packs).toHaveLength(2);
  });
});

describe("vote KV operations", () => {
  it("returns null for no vote", async () => {
    const e = env as unknown as import("../src/types").Env;
    const vote = await getVote(e, "pack1", "user1");
    expect(vote).toBeNull();
  });

  it("stores and retrieves a vote", async () => {
    const e = env as unknown as import("../src/types").Env;
    await putVote(e, "pack1", "user1");
    const vote = await getVote(e, "pack1", "user1");
    expect(vote).not.toBeNull();
    expect(vote!.userId).toBe("user1");
    expect(vote!.packId).toBe("pack1");
    expect(vote!.votedAt).toBeTruthy();
  });

  it("deletes a vote", async () => {
    const e = env as unknown as import("../src/types").Env;
    await putVote(e, "pack2", "user2");
    await deleteVote(e, "pack2", "user2");
    const vote = await getVote(e, "pack2", "user2");
    expect(vote).toBeNull();
  });
});
