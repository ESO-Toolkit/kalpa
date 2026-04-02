import * as React from "react";
import { Popover as PopoverPrimitive } from "@base-ui/react/popover";

import { cn } from "@/lib/utils";

const Popover = PopoverPrimitive.Root;

const PopoverTrigger = PopoverPrimitive.Trigger;

const PopoverClose = PopoverPrimitive.Close;

function PopoverContent({
  className,
  side = "bottom",
  sideOffset = 8,
  align = "center",
  children,
  ...props
}: PopoverPrimitive.Popup.Props &
  Pick<PopoverPrimitive.Positioner.Props, "align" | "side" | "sideOffset">) {
  return (
    <PopoverPrimitive.Portal>
      <PopoverPrimitive.Positioner side={side} sideOffset={sideOffset} align={align}>
        <PopoverPrimitive.Popup
          data-slot="popover-content"
          className={cn(
            "z-50 w-64 origin-(--transform-origin) rounded-xl border border-white/[0.08] bg-[rgba(15,23,42,0.92)] p-3 shadow-lg backdrop-blur-xl",
            "data-open:animate-in data-open:fade-in-0 data-open:zoom-in-95",
            "data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-95",
            className
          )}
          {...props}
        >
          {children}
        </PopoverPrimitive.Popup>
      </PopoverPrimitive.Positioner>
    </PopoverPrimitive.Portal>
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
