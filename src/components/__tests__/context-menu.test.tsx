import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ContextMenu } from "@/components/ui/context-menu";
import { isAddonContextMenuShortcut } from "@/components/addon-list";

describe("addon context-menu accessibility", () => {
  it("recognizes both standard keyboard context-menu shortcuts", () => {
    expect(isAddonContextMenuShortcut("ContextMenu", false)).toBe(true);
    expect(isAddonContextMenuShortcut("F10", true)).toBe(true);
    expect(isAddonContextMenuShortcut("F10", false)).toBe(false);
  });

  it("focuses the menu, exposes the active item, and activates it by keyboard", () => {
    const review = vi.fn();
    render(
      <ContextMenu
        items={[
          { label: "Review Update", onClick: review },
          { label: "Open Folder", onClick: vi.fn() },
        ]}
        position={{ x: 20, y: 20 }}
        onClose={vi.fn()}
      />
    );

    const menu = screen.getByRole("menu", { name: "Addon actions" });
    expect(menu).toHaveFocus();
    expect(menu).toHaveAttribute("aria-activedescendant", "addon-action-0");

    fireEvent.keyDown(menu, { key: "Enter" });
    expect(review).toHaveBeenCalledOnce();
  });
});
