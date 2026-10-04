import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AddonFileEditor } from "../addon-file-editor";

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));

vi.mock("@/lib/tauri", () => ({ invokeOrThrow: invoke, getTauriErrorMessage: String }));
vi.mock("@/lib/use-theme", () => ({ useTheme: () => ({ activeTheme: { colors: {} } }) }));
vi.mock("@/lib/kalpa-codemirror-theme", () => ({ kalpaThemeForColors: () => [] }));
vi.mock("@uiw/react-codemirror", () => ({
  default: ({
    value,
    readOnly,
    onChange,
  }: {
    value: string;
    readOnly: boolean;
    onChange?: (value: string) => void;
  }) => (
    <textarea
      aria-label="File content"
      value={value}
      readOnly={readOnly}
      onChange={(event) => onChange?.(event.target.value)}
    />
  ),
}));

function deferredRead() {
  let resolve!: (value: string) => void;
  const promise = new Promise<string>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

const fileA = {
  addonsPath: "/addons",
  folderName: "AddonA",
  relativePath: "A.lua",
  isModified: false,
};

beforeEach(() => invoke.mockReset());

describe("AddonFileEditor file identity", () => {
  it.each([{ addonsPath: "/other-addons" }, { folderName: "AddonB" }, { relativePath: "B.lua" }])(
    "clears old content and editing permission while reading %j",
    async (identity) => {
      const readA = deferredRead();
      const readB = deferredRead();
      invoke.mockReturnValueOnce(readA.promise).mockReturnValueOnce(readB.promise);
      const callbacks = { onClose: vi.fn(), onSaved: vi.fn() };
      const view = render(<AddonFileEditor {...fileA} {...callbacks} />);
      await act(async () => readA.resolve("A original"));
      fireEvent.click(screen.getByRole("button", { name: "Enable Editing" }));
      fireEvent.change(screen.getByRole("textbox"), { target: { value: "A edited" } });
      expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();

      const fileB = { ...fileA, ...identity };
      view.rerender(<AddonFileEditor {...fileB} {...callbacks} />);
      // No asynchronous B completion is needed to remove A's save action/content.
      expect(screen.getByText("Loading file...")).toBeInTheDocument();
      expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
      expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
      expect(invoke).toHaveBeenNthCalledWith(2, "read_addon_file", {
        addonsPath: fileB.addonsPath,
        folderName: fileB.folderName,
        relativePath: fileB.relativePath,
      });

      await act(async () => readB.resolve("B original"));
      expect(screen.getByRole("textbox")).toHaveValue("B original");
      expect(screen.getByRole("textbox")).toHaveAttribute("readonly");
      expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
      expect(screen.getByRole("button", { name: "Enable Editing" })).toBeInTheDocument();
      expect(invoke).toHaveBeenCalledTimes(2);

      fireEvent.click(screen.getByRole("button", { name: "Enable Editing" }));
      fireEvent.change(screen.getByRole("textbox"), { target: { value: "B edited" } });
      invoke.mockResolvedValueOnce(undefined);
      await act(async () => fireEvent.click(screen.getByRole("button", { name: "Save" })));
      expect(invoke).toHaveBeenLastCalledWith("write_addon_file", {
        addonsPath: fileB.addonsPath,
        folderName: fileB.folderName,
        relativePath: fileB.relativePath,
        content: "B edited",
      });
      expect(callbacks.onSaved).toHaveBeenCalledOnce();
    }
  );

  it("ignores a stale editable-file read while its replacement is still loading", async () => {
    const readA = deferredRead();
    const readB = deferredRead();
    invoke.mockReturnValueOnce(readA.promise).mockReturnValueOnce(readB.promise);
    const callbacks = { onClose: vi.fn(), onSaved: vi.fn() };
    const view = render(<AddonFileEditor {...fileA} isModified {...callbacks} />);
    view.rerender(<AddonFileEditor {...fileA} relativePath="B.lua" {...callbacks} />);

    await act(async () => readA.resolve("stale A content"));
    expect(screen.getByText("Loading file...")).toBeInTheDocument();
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();

    await act(async () => readB.resolve("B content"));
    expect(screen.getByRole("textbox")).toHaveValue("B content");
    expect(screen.getByRole("textbox")).toHaveAttribute("readonly");
    expect(screen.getByRole("button", { name: "Enable Editing" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Save" })).not.toBeInTheDocument();
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(callbacks.onSaved).not.toHaveBeenCalled();
  });
});
