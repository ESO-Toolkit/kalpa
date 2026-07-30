import * as React from "react";
import { Popover as PopoverPrimitive } from "@base-ui/react/popover";

import { cn } from "@/lib/utils";

import {
  Popover,
  PopoverTrigger,
  PopoverPortal,
  PopoverPositioner,
  PopoverPopup,
  PopoverClose,
  type PopoverPopupProps,
} from "@/components/animate-ui/primitives/base/popover";

function PopoverContent({
  className,
  side = "bottom",
  sideOffset = 8,
  align = "center",
  children,
  ...props
}: PopoverPopupProps &
  Pick<
    React.ComponentProps<typeof PopoverPrimitive.Positioner>,
    "align" | "side" | "sideOffset"
  > & {
    children?: React.ReactNode;
  }) {
  return (
    <PopoverPortal>
      {/* The z-index MUST live here, on the positioner. The positioner carries a
          transform, which creates a stacking context — so any z-index on the
          popup inside it is scoped to that context and cannot compete with
          anything outside. Left on the popup alone, the popover lost to the
          header's `relative z-20` and rendered behind the nav. */}
      <PopoverPositioner side={side} sideOffset={sideOffset} align={align} className="z-50">
        <PopoverPopup
          className={cn(
            "relative w-64 origin-(--transform-origin) rounded-xl border border-structure-08 bg-surface-overlay p-3 shadow-lg backdrop-blur-xl",
            className
          )}
          transition={{ type: "spring", stiffness: 500, damping: 30 }}
          {...props}
        >
          {children}
        </PopoverPopup>
      </PopoverPositioner>
    </PopoverPortal>
  );
}

function PopoverTitle({ className, ...props }: React.ComponentProps<"h3">) {
  return (
    <h3
      data-slot="popover-title"
      className={cn("text-xs font-heading font-semibold text-foreground", className)}
      {...props}
    />
  );
}

function PopoverDescription({ className, ...props }: React.ComponentProps<"p">) {
  return (
    <p
      data-slot="popover-description"
      className={cn("mt-1 text-xs text-muted-foreground", className)}
      {...props}
    />
  );
}

export { Popover, PopoverTrigger, PopoverContent, PopoverClose, PopoverTitle, PopoverDescription };
