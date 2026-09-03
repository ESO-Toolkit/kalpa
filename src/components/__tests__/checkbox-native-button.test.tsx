import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it } from "vitest";

import { Checkbox } from "@/components/ui/checkbox";

/**
 * The checkbox primitive hard-codes `render={<motion.button>}`, so it must tell
 * Base UI `nativeButton` is true. When it forwarded the prop instead, Base UI
 * received `undefined`, defaulted to `false`, and layered non-native button
 * semantics onto a real `<button>`.
 *
 * These assert the rendered result rather than the source shape, so they stay
 * meaningful if the primitive is rewritten. `native-button-prop.test.ts` guards
 * the source shape.
 */

function ControlledCheckbox({ disabled = false }: { disabled?: boolean }) {
  const [checked, setChecked] = useState(false);
  return (
    <label>
      <Checkbox
        checked={checked}
        disabled={disabled}
        onCheckedChange={(value) => setChecked(value === true)}
      />
      <span>Share diagnostics</span>
    </label>
  );
}

describe("Checkbox native button semantics", () => {
  it("renders a native button and keeps the checkbox role", () => {
    render(<ControlledCheckbox />);
    const checkbox = screen.getByRole("checkbox", { name: "Share diagnostics" });

    expect(checkbox.tagName).toBe("BUTTON");
    // Native mode contributes type="button"; without it the element is an
    // implicit submit button.
    expect(checkbox).toHaveAttribute("type", "button");
    expect(checkbox).toHaveAttribute("aria-checked", "false");
  });

  it("expresses disabled natively rather than with aria-disabled", () => {
    render(<ControlledCheckbox disabled />);
    const checkbox = screen.getByRole("checkbox", { name: "Share diagnostics" });

    // Non-native mode sets aria-disabled + tabIndex=-1 and leaves the element
    // enabled, which is what defeated the `disabled:` styling in ui/checkbox.tsx.
    expect(checkbox).toBeDisabled();
    expect(checkbox).not.toHaveAttribute("aria-disabled");
  });

  it("toggles on click and on Space", async () => {
    const user = userEvent.setup();
    render(<ControlledCheckbox />);
    const checkbox = screen.getByRole("checkbox", { name: "Share diagnostics" });

    await user.click(checkbox);
    expect(checkbox).toHaveAttribute("aria-checked", "true");

    checkbox.focus();
    await user.keyboard(" ");
    expect(checkbox).toHaveAttribute("aria-checked", "false");
  });
});
