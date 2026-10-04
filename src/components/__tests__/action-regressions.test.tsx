import { readFileSync } from "node:fs";
import ts from "typescript";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import { PackImportView } from "../pack-import";
import { AddonList } from "../addon-list";
import type { AddonManifest, SharedPack } from "../../types";
import type { ReactNode } from "react";

vi.mock("@/components/animate-ui/primitives/effects/fade", () => ({
  Fade: ({ children }: { children: ReactNode }) => children,
}));
vi.mock("@/components/discover-panel", () => ({ DiscoverPanel: () => null }));
vi.mock("@/lib/platform", () => ({ modKeyLabel: () => "Ctrl" }));
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: ({ count }: { count: number }) => ({
    getVirtualItems: () =>
      Array.from({ length: count }, (_, index) => ({ index, key: index, start: index * 52 })),
    getTotalSize: () => count * 52,
    measureElement: vi.fn(),
    scrollToIndex: vi.fn(),
  }),
}));

// Execute the production hook callbacks without mounting App's native services.
function callback(file: string, name: string, scope: Record<string, unknown>) {
  const source = ts.createSourceFile(
    file,
    readFileSync(file, "utf8"),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX
  );
  let expression: ts.Expression | undefined;
  function visit(node: ts.Node) {
    if (
      ts.isVariableDeclaration(node) &&
      node.name.getText(source) === name &&
      node.initializer &&
      (ts.isCallExpression(node.initializer) || ts.isArrowFunction(node.initializer))
    ) {
      expression = ts.isCallExpression(node.initializer)
        ? node.initializer.arguments[0]
        : node.initializer;
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
  if (!expression) throw new Error(`Missing production callback ${name}`);
  const js = ts.transpileModule(`const callback = ${expression.getText(source)};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(scope), `${js}\nreturn callback;`)(
    ...Object.values(scope)
  ) as (...args: unknown[]) => unknown;
}

describe("production action callbacks", () => {
  it("disables only enabled selections and is idempotent", async () => {
    const addons = [
      { folderName: "enabled", disabled: false },
      { folderName: "disabled", disabled: true },
    ];
    const invoke = vi.fn().mockResolvedValue({ enabled: [], disabled: ["enabled"], failed: [] });
    const scope = {
      addons,
      selectedFolders: new Set(["enabled", "disabled", "gone"]),
      addonsPath: "/addons",
      invokeOrThrow: invoke,
      setSelectedFolders: vi.fn(),
      setBatchDisabling: vi.fn(),
      setAddons: (update: (value: typeof addons) => typeof addons) =>
        addons.splice(0, addons.length, ...update(addons)),
      setSelectedAddon: vi.fn(),
      togglingAffectsDependencies: () => false,
      scanAddons: vi.fn(),
      toast: { success: vi.fn(), error: vi.fn() },
      getTauriErrorMessage: String,
    };
    await callback("src/App.tsx", "handleBatchDisable", scope)();
    expect(invoke).toHaveBeenCalledWith("batch_set_enabled", {
      addonsPath: "/addons",
      entries: [{ folderName: "enabled", enable: false }],
    });
    await callback("src/App.tsx", "handleBatchDisable", scope)();
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(addons.every((addon) => addon.disabled)).toBe(true);
  });

  it.each(["Enter", " ", "ArrowDown"])("leaves descendant %s keys to the checkbox", (key) => {
    const onSelect = vi.fn();
    const addon = { folderName: "one" };
    const handle = callback("src/components/addon-list.tsx", "handleListKeyDown", {
      addons: [addon],
      selectedAddon: addon,
      onSelect,
      rowVirtualizer: { scrollToIndex: vi.fn() },
    });
    const preventDefault = vi.fn();
    handle({
      key,
      target: document.createElement("button"),
      currentTarget: document.createElement("div"),
      preventDefault,
    });
    expect(onSelect).not.toHaveBeenCalled();
    expect(preventDefault).not.toHaveBeenCalled();
  });

  it("includes selected optional addons in the imported installation request", async () => {
    const optional = { esouiId: 2, name: "Optional", required: false };
    const required = { esouiId: 1, name: "Required", required: true };
    const candidates = callback("src/components/packs.tsx", "importedPackAddonsToInstall", {
      importedPack: { addons: [required, optional] },
      installedEsouiIds: new Set([1]),
      selectedImportedAddons: new Set([2]),
    });
    expect(candidates()).toEqual([optional]);
    const runBatchPackInstall = vi
      .fn()
      .mockResolvedValue({ installed: [], failed: [], installedFolders: [], pendingDeps: [] });
    const setInstallProgress = vi.fn();
    await callback("src/components/packs.tsx", "handleInstallImportedPack", {
      importedPack: { addons: [required, optional] },
      importedPackAddonsToInstall: candidates(),
      importedFileSettings: null,
      applyingSettings: false,
      installing: false,
      setInstalling: vi.fn(),
      ensureEsoNotBlocking: async () => true,
      setInstallProgress,
      runBatchPackInstall,
      addonsPath: "/addons",
      onRefresh: vi.fn(),
      resolvePendingDeps: vi.fn(),
      toast: { success: vi.fn(), error: vi.fn() },
    })();
    expect(runBatchPackInstall).toHaveBeenCalledWith(
      "/addons",
      [{ esouiId: 2, label: "Optional" }],
      setInstallProgress
    );
  });
});

describe("imported optional addon preview", () => {
  it("offers optional selection without claiming uninstalled addons are installed", async () => {
    const onToggleImportedAddon = vi.fn();
    const pack: SharedPack = {
      title: "Optional pack",
      description: "",
      packType: "custom",
      tags: [],
      addons: [{ esouiId: 2, name: "Optional", required: false }],
      sharedBy: "",
      sharedAt: "",
      expiresAt: "",
    };
    const props = {
      shareCodeInput: "",
      onShareCodeInputChange: vi.fn(),
      resolvingCode: false,
      importedPack: pack,
      importError: null,
      installing: false,
      installProgress: null,
      installedEsouiIds: new Set<number>(),
      importedPackAddonsToInstall: [],
      selectedImportedAddons: new Set<number>(),
      onToggleImportedAddon,
      onResolveCode: vi.fn(),
      onImportFile: vi.fn(),
      onInstall: vi.fn(),
      onClear: vi.fn(),
    };
    const { rerender } = render(<PackImportView {...props} />);
    expect(screen.queryByText("All addons already installed")).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("checkbox", { name: "Install Optional" }));
    expect(onToggleImportedAddon).toHaveBeenCalledWith(2);
    rerender(
      <PackImportView
        {...props}
        selectedImportedAddons={new Set([2])}
        importedPackAddonsToInstall={pack.addons}
      />
    );
    expect(screen.getByRole("button", { name: "Install 1 New Addon" })).toBeEnabled();
  });
});

describe("addon selection keyboard access", () => {
  it.each(["[Enter]", " "])(
    "can enter batch selection with %s without opening details",
    async (key) => {
      const addon: AddonManifest = {
        folderName: "one",
        title: "One",
        author: "",
        version: "1",
        addonVersion: null,
        apiVersion: [],
        description: "",
        isLibrary: false,
        dependsOn: [],
        optionalDependsOn: [],
        missingDependencies: [],
        outdatedDependencies: [],
        missingOptionalDependencies: [],
        esouiId: null,
        tags: [],
        esouiLastUpdate: 0,
        installedAt: "",
        disabled: false,
        modifiedFileCount: 0,
      };
      const onSelect = vi.fn();
      const onToggleSelect = vi.fn();
      render(
        <AddonList
          addons={[addon]}
          allAddons={[addon]}
          selectedAddon={addon}
          onSelect={onSelect}
          searchQuery=""
          onSearchChange={vi.fn()}
          loading={false}
          updateResults={[]}
          sortMode="name"
          onSortChange={vi.fn()}
          filterMode="all"
          onFilterChange={vi.fn()}
          activeTagFilter={null}
          onActiveTagFilterChange={vi.fn()}
          selectedFolders={new Set()}
          onToggleSelect={onToggleSelect}
          viewMode="installed"
          onViewModeChange={vi.fn()}
          discoverTab="search"
          onDiscoverTabChange={vi.fn()}
          addonsPath="/addons"
          onInstalled={vi.fn()}
          onSelectDiscoverResult={vi.fn()}
          selectedDiscoverResultId={null}
          installedEsouiIds={new Set()}
        />
      );
      const checkbox = screen.getByRole("checkbox", { name: "Select One" });
      expect(checkbox.tabIndex).toBe(0);
      checkbox.focus();
      await userEvent.keyboard(key);
      expect(onToggleSelect).toHaveBeenCalledWith("one");
      expect(onSelect).not.toHaveBeenCalled();
    }
  );
});
