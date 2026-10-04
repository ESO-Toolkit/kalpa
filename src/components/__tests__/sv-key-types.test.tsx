import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { EditorTab } from "../saved-variables";
import type { SvTreeNode } from "../../types";
import { treePathId, treePathSegment, updateTreeNode } from "../../lib/sv-helpers";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@/lib/tauri", () => ({ invokeOrThrow: invoke, getTauriErrorMessage: String }));
vi.mock("@/lib/store", () => ({ getSetting: vi.fn().mockResolvedValue({}), setSetting: vi.fn() }));
vi.mock("@/hooks/use-capped-animation-rate", () => ({ useCappedAnimationRate: vi.fn() }));

function table(key: string, children: SvTreeNode[], keyType?: "number" | "string"): SvTreeNode {
  return { key, keyType, valueType: "table", children };
}
function leaf(key: string, value: number, keyType?: "number" | "string"): SvTreeNode {
  return { key, keyType, valueType: "number", value };
}
let saved: SvTreeNode | undefined;
function mount(tree: SvTreeNode) {
  invoke.mockImplementation(async (command: string, args: { tree?: SvTreeNode }) => {
    if (command === "read_saved_variable") return { tree, stamp: { size: 10, modifiedMs: 1 } };
    if (command === "write_saved_variable") {
      saved = args.tree;
      return {};
    }
    if (command === "preview_sv_save") {
      return {
        changes: args.tree?.children?.[0]?.children?.map((node, index) => ({
          path: ["Addon", node.key],
          pathKeyTypes: [null, node.keyType],
          changeType: "modified",
          oldValue: String(index === 0 ? 10 : 20),
          newValue: String(node.value),
        })),
      };
    }
    return { hints: {} };
  });
  render(
    <EditorTab
      files={[]}
      addonsPath="/addons"
      initialFile="Test.lua"
      esoRunning={false}
      characters={[]}
      onDirtyChange={vi.fn()}
    />
  );
}
async function changeNumber(index: number, value: string) {
  const user = userEvent.setup();
  const input = screen.getAllByRole("spinbutton")[index];
  if (!input) throw new Error("Missing number input");
  await user.clear(input);
  await user.type(input, value);
  await user.tab();
}

beforeEach(() => {
  invoke.mockReset();
  saved = undefined;
});

describe("SavedVariables typed key editing", () => {
  it("previews both typed sibling edits with distinct labels and correct values", async () => {
    const errors = vi.spyOn(console, "error");
    mount(table("", [table("Addon", [leaf("1", 10, "number"), leaf("1", 20, "string")])]));
    await userEvent.click(await screen.findByRole("button", { name: /Addon/ }));
    await changeNumber(0, "11");
    await changeNumber(1, "21");
    await userEvent.click(screen.getByRole("button", { name: /Preview/ }));
    const dialog = await screen.findByRole("dialog");
    for (const [label, before, after] of [
      ["[1]", "10", "11"],
      ['["1"]', "20", "21"],
    ]) {
      const heading = within(dialog).getByText(label!);
      const row = heading.parentElement?.parentElement;
      if (!row) throw new Error("Missing change row");
      expect(within(row).getByText(before!)).toBeInTheDocument();
      expect(within(row).getByText(after!)).toBeInTheDocument();
    }
    expect(within(dialog).getByText("2 changes will be saved")).toBeInTheDocument();
    expect(
      errors.mock.calls.some((args) => args.some((arg) => String(arg).includes("same key")))
    ).toBe(false);
    errors.mockRestore();
  });

  it("edits same-spelling numeric and string leaves independently and retains metadata on save", async () => {
    mount(table("", [table("Addon", [leaf("1", 10, "number"), leaf("1", 20, "string")])]));
    await userEvent.click(await screen.findByRole("button", { name: /Addon/ }));
    await changeNumber(0, "11");
    expect(
      screen.getAllByRole("spinbutton").map((input) => (input as HTMLInputElement).value)
    ).toEqual(["11", "20"]);
    await changeNumber(1, "21");
    await userEvent.click(screen.getByRole("button", { name: "Save Changes" }));
    await waitFor(() =>
      expect(saved?.children?.[0]?.children).toEqual([
        leaf("1", 11, "number"),
        leaf("1", 21, "string"),
      ])
    );
  });

  it("navigates same-spelling numeric and string tables without selecting the other sibling", async () => {
    mount(
      table("", [
        table("Addon", [
          table("1", [leaf("amount", 10)], "number"),
          table("1", [leaf("amount", 20)], "string"),
        ]),
      ])
    );
    await userEvent.click(await screen.findByRole("button", { name: /Addon/ }));
    const siblings = screen.getAllByRole("button", { name: /^1/ });
    await userEvent.click(siblings[1]!);
    expect((screen.getByRole("spinbutton") as HTMLInputElement).value).toBe("20");
    await changeNumber(0, "22");
    await userEvent.click(screen.getAllByRole("button", { name: /^1/ })[0]!);
    expect((screen.getByRole("spinbutton") as HTMLInputElement).value).toBe("10");
    await userEvent.click(screen.getByRole("button", { name: "Save Changes" }));
    await waitFor(() =>
      expect(saved?.children?.[0]?.children?.map((node) => node.children?.[0]?.value)).toEqual([
        10, 22,
      ])
    );
  });

  it("search results edit the matching typed sibling", async () => {
    mount(table("", [table("Addon", [leaf("1", 10, "number"), leaf("1", 20, "string")])]));
    await screen.findByRole("button", { name: /Addon/ });
    await userEvent.type(screen.getByPlaceholderText("Search settings..."), "1");
    await screen.findAllByRole("spinbutton");
    await changeNumber(1, "23");
    await userEvent.click(screen.getByRole("button", { name: "Save Changes" }));
    await waitFor(() =>
      expect(saved?.children?.[0]?.children?.map((node) => node.value)).toEqual([10, 23])
    );
  });

  it("preserves old paths and cannot alias literal reserved-prefix keys", () => {
    const numeric = leaf("1", 10, "number");
    const literal = leaf("\0number:1", 20, "string");
    const legacy = leaf("legacy", 30);
    const tree = table("", [numeric, literal, legacy]);
    expect(treePathSegment(legacy)).toBe("legacy");
    expect(treePathId(["Addon", "legacy"])).toBe("Addon\0legacy");
    expect(treePathId([treePathSegment(numeric)])).not.toBe(
      treePathId(["", `path:${JSON.stringify([treePathSegment(numeric)])}`])
    );
    expect(treePathId([treePathSegment(numeric)])).not.toBe(treePathId(["\\0number:1"]));
    expect(updateTreeNode(tree, [treePathSegment(numeric)], 11).children).toEqual([
      leaf("1", 11, "number"),
      literal,
      legacy,
    ]);
    expect(updateTreeNode(tree, [treePathSegment(literal)], 21).children).toEqual([
      numeric,
      leaf("\0number:1", 21, "string"),
      legacy,
    ]);
    expect(updateTreeNode(tree, ["legacy"], 31).children?.[2]?.value).toBe(31);
  });
});
