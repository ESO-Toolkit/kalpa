import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { AddonFileTree } from "@/types";

import { AddonFileBrowser } from "../addon-file-browser";
import { AddonFileEditor } from "../addon-file-editor";

const mocks = vi.hoisted(() => ({
  invokeOrThrow: vi.fn(),
}));

vi.mock("@/lib/tauri", () => ({
  getTauriErrorMessage: (error: unknown) =>
    error instanceof Error ? error.message : String(error),
  invokeOrThrow: mocks.invokeOrThrow,
}));

vi.mock("sonner", () => ({
  toast: {
    error: vi.fn(),
    info: vi.fn(),
    success: vi.fn(),
  },
}));

vi.mock("@uiw/react-codemirror", () => ({
  default: ({
    value,
    readOnly,
    onChange,
  }: {
    value: string;
    readOnly?: boolean;
    onChange?: (value: string) => void;
  }) => (
    <textarea
      aria-label="addon file contents"
      data-readonly={String(readOnly)}
      onChange={(event) => onChange?.(event.currentTarget.value)}
      readOnly={readOnly}
      value={value}
    />
  ),
}));

vi.mock("@codemirror/language", () => ({
  StreamLanguage: {
    define: vi.fn(() => ({ extension: "lua" })),
  },
}));

vi.mock("@codemirror/legacy-modes/mode/lua", () => ({
  lua: {},
}));

vi.mock("@codemirror/lang-xml", () => ({
  xml: vi.fn(() => ({ extension: "xml" })),
}));

vi.mock("@/lib/kalpa-codemirror-theme", () => ({
  kalpaThemeForColors: vi.fn(() => []),
}));

vi.mock("@/lib/use-theme", () => ({
  useTheme: () => ({
    activeTheme: {
      colors: {},
    },
  }),
}));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((promiseResolve, promiseReject) => {
    resolve = promiseResolve;
    reject = promiseReject;
  });
  return { promise, reject, resolve };
}

describe("Addon file editor stale acknowledgment behavior", () => {
  beforeEach(() => {
    mocks.invokeOrThrow.mockReset();
  });

  it("remounts the editor when the browser switches files so editing is locked again", async () => {
    const user = userEvent.setup();
    const fileTree: AddonFileTree = {
      files: [
        {
          extension: "lua",
          isDirectory: false,
          relativePath: "Modified.lua",
          sizeBytes: 11,
          status: "modified",
        },
        {
          extension: "lua",
          isDirectory: false,
          relativePath: "Clean.lua",
          sizeBytes: 9,
          status: "stock",
        },
      ],
      folderName: "DemoAddon",
      modifiedCount: 1,
    };

    mocks.invokeOrThrow.mockImplementation((command: string, args?: { relativePath?: string }) => {
      if (command === "list_addon_files") {
        return Promise.resolve(fileTree);
      }
      if (command === "read_addon_file") {
        return Promise.resolve(`content:${args?.relativePath}`);
      }
      throw new Error(`Unexpected command: ${command}`);
    });

    render(<AddonFileBrowser addonsPath="C:/ESO/AddOns" folderName="DemoAddon" />);

    await user.click(await screen.findByRole("button", { name: /Modified\.lua/ }));
    expect(await screen.findByLabelText("addon file contents")).toHaveValue("content:Modified.lua");
    expect(screen.getByLabelText("addon file contents")).not.toHaveAttribute("readonly");

    await user.click(screen.getByRole("button", { name: /Clean\.lua/ }));

    await waitFor(() =>
      expect(screen.getByLabelText("addon file contents")).toHaveValue("content:Clean.lua")
    );
    expect(screen.getByRole("button", { name: "Enable Editing" })).toBeInTheDocument();
    expect(screen.getByLabelText("addon file contents")).toHaveAttribute("readonly");
  });

  it("returns to loading state when an unkeyed editor receives a new path", async () => {
    const firstRead = deferred<string>();
    const secondRead = deferred<string>();

    mocks.invokeOrThrow.mockImplementation((command: string, args?: { relativePath?: string }) => {
      if (command !== "read_addon_file") {
        throw new Error(`Unexpected command: ${command}`);
      }
      if (args?.relativePath === "First.lua") {
        return firstRead.promise;
      }
      if (args?.relativePath === "Second.lua") {
        return secondRead.promise;
      }
      throw new Error(`Unexpected path: ${args?.relativePath}`);
    });

    const { rerender } = render(
      <AddonFileEditor
        addonsPath="C:/ESO/AddOns"
        folderName="DemoAddon"
        isModified={true}
        onClose={vi.fn()}
        onSaved={vi.fn()}
        relativePath="First.lua"
      />
    );

    await act(async () => {
      firstRead.resolve("first file text");
      await firstRead.promise;
    });

    expect(await screen.findByLabelText("addon file contents")).toHaveValue("first file text");

    rerender(
      <AddonFileEditor
        addonsPath="C:/ESO/AddOns"
        folderName="DemoAddon"
        isModified={false}
        onClose={vi.fn()}
        onSaved={vi.fn()}
        relativePath="Second.lua"
      />
    );

    expect(screen.getByText("Loading file...")).toBeInTheDocument();
    expect(screen.queryByLabelText("addon file contents")).not.toBeInTheDocument();

    await act(async () => {
      secondRead.resolve("second file text");
      await secondRead.promise;
    });

    expect(await screen.findByLabelText("addon file contents")).toHaveValue("second file text");
  });

  it("re-locks editing when the open file turns stock, unless there are unsaved edits", async () => {
    const user = userEvent.setup();
    mocks.invokeOrThrow.mockImplementation((command: string) => {
      if (command !== "read_addon_file") {
        throw new Error(`Unexpected command: ${command}`);
      }
      return Promise.resolve("file text");
    });

    const props = {
      addonsPath: "C:/ESO/AddOns",
      folderName: "DemoAddon",
      onClose: vi.fn(),
      onSaved: vi.fn(),
      relativePath: "Same.lua",
    };
    const { rerender } = render(<AddonFileEditor {...props} isModified={true} />);

    expect(await screen.findByLabelText("addon file contents")).toHaveValue("file text");
    expect(screen.queryByRole("button", { name: "Enable Editing" })).not.toBeInTheDocument();

    // Clean buffer + the file turning stock re-arms the acknowledgment.
    rerender(<AddonFileEditor {...props} isModified={false} />);
    expect(screen.getByRole("button", { name: "Enable Editing" })).toBeInTheDocument();
    expect(screen.getByLabelText("addon file contents")).toHaveAttribute("readonly");

    // Unlock again and type: unsaved edits must survive a status flip.
    await user.click(screen.getByRole("button", { name: "Enable Editing" }));
    await user.type(screen.getByLabelText("addon file contents"), "!");
    rerender(<AddonFileEditor {...props} isModified={true} />);
    rerender(<AddonFileEditor {...props} isModified={false} />);
    expect(screen.queryByRole("button", { name: "Enable Editing" })).not.toBeInTheDocument();
    expect(screen.getByLabelText("addon file contents")).toHaveValue("file text!");
  });
});
