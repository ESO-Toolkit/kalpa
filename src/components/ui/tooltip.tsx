import { Tooltip as TooltipPrimitive } from "@base-ui/react/tooltip";

import { cn } from "@/lib/utils";

function TooltipProvider({ delay = 0, ...props }: TooltipPrimitive.Provider.Props) {
  return <TooltipPrimitive.Provider data-slot="tooltip-provider" delay={delay} {...props} />;
}

function Tooltip({ ...props }: TooltipPrimitive.Root.Props) {
  return <TooltipPrimitive.Root data-slot="tooltip" {...props} />;
}

function TooltipTrigger({ ...props }: TooltipPrimitive.Trigger.Props) {
  return <TooltipPrimitive.Trigger data-slot="tooltip-trigger" {...props} />;
}

function TooltipContent({
  className,
  side = "top",
  sideOffset = 8,
  align = "center",
  alignOffset = 0,
  children,
  ...props
}: TooltipPrimitive.Popup.Props &
  Pick<TooltipPrimitive.Positioner.Props, "align" | "alignOffset" | "side" | "sideOffset">) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Positioner
        align={align}
        alignOffset={alignOffset}
        side={side}
        sideOffset={sideOffset}
        className="isolate z-50"
      >
        <TooltipPrimitive.Popup
          data-slot="tooltip-content"
          className={cn(
            // Layout
            "z-50 inline-flex w-fit max-w-xs origin-(--transform-origin) items-center gap-1.5",
            // Glass morphism surface
            "rounded-lg border border-white/[0.08] bg-[rgba(15,23,42,0.88)] px-3 py-1.5 shadow-[0_8px_32px_rgba(0,0,0,0.4),0_1px_0_rgba(255,255,255,0.06)_inset] backdrop-blur-xl backdrop-saturate-[1.3]",
            // Typography
            "text-xs font-medium text-[#e5e7eb]",
            // Keyboard shortcut styling
            "has-data-[slot=kbd]:pr-1.5 **:data-[slot=kbd]:relative **:data-[slot=kbd]:isolate **:data-[slot=kbd]:z-50 **:data-[slot=kbd]:rounded-[4px] **:data-[slot=kbd]:border **:data-[slot=kbd]:border-white/[0.1] **:data-[slot=kbd]:bg-white/[0.06] **:data-[slot=kbd]:px-1.5 **:data-[slot=kbd]:py-0.5 **:data-[slot=kbd]:text-[10px] **:data-[slot=kbd]:font-mono **:data-[slot=kbd]:text-muted-foreground",
            // Slide-in animations per side
            "data-[side=bottom]:slide-in-from-top-1 data-[side=inline-end]:slide-in-from-left-1 data-[side=inline-start]:slide-in-from-right-1 data-[side=left]:slide-in-from-right-1 data-[side=right]:slide-in-from-left-1 data-[side=top]:slide-in-from-bottom-1",
            // Enter/exit animations
            "data-[state=delayed-open]:animate-in data-[state=delayed-open]:fade-in-0 data-[state=delayed-open]:zoom-in-[0.97]",
            "data-open:animate-in data-open:fade-in-0 data-open:zoom-in-[0.97]",
            "data-closed:animate-out data-closed:fade-out-0 data-closed:zoom-out-[0.97]",
            className
          )}
          {...props}
        >
          {children}
          <TooltipPrimitive.Arrow className="z-50 size-2 translate-y-[calc(-50%-1.5px)] rotate-45 rounded-[1.5px] border border-white/[0.08] bg-[rgba(15,23,42,0.88)] shadow-[0_1px_0_rgba(255,255,255,0.06)_inset] backdrop-blur-xl data-[side=bottom]:top-0.5 data-[side=bottom]:border-t-0 data-[side=bottom]:border-l-0 data-[side=left]:top-1/2! data-[side=left]:-right-1 data-[side=left]:-translate-y-1/2 data-[side=left]:border-t-0 data-[side=left]:border-r-0 data-[side=right]:top-1/2! data-[side=right]:-left-1 data-[side=right]:-translate-y-1/2 data-[side=right]:border-b-0 data-[side=right]:border-l-0 data-[side=top]:-bottom-2 data-[side=top]:border-b-0 data-[side=top]:border-r-0" />
        </TooltipPrimitive.Popup>
      </TooltipPrimitive.Positioner>
    </TooltipPrimitive.Portal>
  );
}

// ── Convenience wrapper ──────────────────────────────────────────────────
// Replaces verbose Tooltip+Trigger+Content nesting for the common case of
// wrapping a single element with a simple text tooltip.

interface SimpleTooltipProps {
  /** The tooltip label text */
  content: string;
  /** Which side to place the tooltip */
  side?: "top" | "bottom" | "left" | "right";
  /** The element to wrap */
  children: React.ReactElement;
  /** Extra className on the popup */
  className?: string;
}

function SimpleTooltip({ content, side = "top", children, className }: SimpleTooltipProps) {
  if (!content) return children;
  return (
    <Tooltip>
      <TooltipTrigger render={children} />
      <TooltipContent side={side} className={className}>
        {content}
      </TooltipContent>
    </Tooltip>
  );
}

export { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider, SimpleTooltip };
