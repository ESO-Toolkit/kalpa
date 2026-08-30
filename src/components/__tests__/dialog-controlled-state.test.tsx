import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { Dialog, DialogClose, useDialog } from "../animate-ui/primitives/base/dialog";

function DialogStateProbe() {
  const { isOpen } = useDialog();

  return (
    <>
      <output>{isOpen ? "open" : "closed"}</output>
      <DialogClose>Request close</DialogClose>
    </>
  );
}

describe("Dialog controlled state", () => {
  it("stays open when a controlled parent vetoes a close request", async () => {
    const onOpenChange = vi.fn();
    const user = userEvent.setup();

    render(
      <Dialog open onOpenChange={onOpenChange}>
        <DialogStateProbe />
      </Dialog>
    );

    await user.click(screen.getByRole("button", { name: "Request close" }));

    expect(onOpenChange).toHaveBeenCalled();
    expect(onOpenChange.mock.calls.every(([open]) => open === false)).toBe(true);
    expect(screen.getByText("open")).toBeInTheDocument();
  });

  it("closes normally when uncontrolled", async () => {
    const user = userEvent.setup();

    render(
      <Dialog defaultOpen>
        <DialogStateProbe />
      </Dialog>
    );

    await user.click(screen.getByRole("button", { name: "Request close" }));

    expect(screen.getByText("closed")).toBeInTheDocument();
  });
});
