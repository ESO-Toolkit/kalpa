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
      <PopoverPositioner side={side} sideOffset={sideOffset} align={align}>
        <PopoverPopup
          className={cn(
            "z-50 w-64 origin-(--transform-origin) rounded-xl border border-white/[0.08] bg-[rgba(15,23,42,0.92)] p-3 shadow-lg backdrop-blur-xl",
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
      className={cn("mt-1 text-[11px] text-muted-foreground", className)}
      {...props}
    />
  );
}

export { Popover, PopoverTrigger, PopoverContent, PopoverClose, PopoverTitle, PopoverDescription };
