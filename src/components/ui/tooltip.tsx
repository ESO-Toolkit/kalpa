import * as React from "react";
import { Tooltip as TooltipPrimitive } from "@base-ui/react/tooltip";

import { cn } from "@/lib/utils";

import {
  Tooltip,
  TooltipTrigger,
  TooltipPortal,
  TooltipPositioner,
  TooltipPopup,
  TooltipArrow,
  TooltipProvider,
  type TooltipPopupProps,
} from "@/components/animate-ui/primitives/base/tooltip";

function TooltipContent({
  className,
  side = "top",
  sideOffset = 8,
  align = "center",
  alignOffset = 0,
  children,
  ...props
}: TooltipPopupProps &
  Pick<
    React.ComponentProps<typeof TooltipPrimitive.Positioner>,
    "align" | "alignOffset" | "side" | "sideOffset"
  > & { children?: React.ReactNode }) {
  return (
    <TooltipPortal>
      <TooltipPositioner
        align={align}
        alignOffset={alignOffset}
        side={side}
        sideOffset={sideOffset}
        className="isolate z-50"
      >
        <TooltipPopup
          className={cn(
            "z-50 inline-flex w-fit max-w-xs origin-(--transform-origin) items-center gap-1.5",
            "rounded-lg border border-white/[0.08] bg-[rgba(15,23,42,0.88)] px-3 py-1.5 shadow-[0_8px_32px_rgba(0,0,0,0.4),0_1px_0_rgba(255,255,255,0.06)_inset] backdrop-blur-xl backdrop-saturate-[1.3]",
            "text-xs font-medium text-[#e5e7eb]",
            "has-data-[slot=kbd]:pr-1.5 **:data-[slot=kbd]:relative **:data-[slot=kbd]:isolate **:data-[slot=kbd]:z-50 **:data-[slot=kbd]:rounded-[4px] **:data-[slot=kbd]:border **:data-[slot=kbd]:border-white/[0.1] **:data-[slot=kbd]:bg-white/[0.06] **:data-[slot=kbd]:px-1.5 **:data-[slot=kbd]:py-0.5 **:data-[slot=kbd]:text-[10px] **:data-[slot=kbd]:font-mono **:data-[slot=kbd]:text-muted-foreground",
            className
          )}
          transition={{ type: "spring", stiffness: 500, damping: 30 }}
          {...props}
        >
          {children}
          <TooltipArrow className="z-50 size-2 translate-y-[calc(-50%-1.5px)] rotate-45 rounded-[1.5px] border border-white/[0.08] bg-[rgba(15,23,42,0.88)] shadow-[0_1px_0_rgba(255,255,255,0.06)_inset] backdrop-blur-xl data-[side=bottom]:top-0.5 data-[side=bottom]:border-t-0 data-[side=bottom]:border-l-0 data-[side=left]:top-1/2! data-[side=left]:-right-1 data-[side=left]:-translate-y-1/2 data-[side=left]:border-t-0 data-[side=left]:border-r-0 data-[side=right]:top-1/2! data-[side=right]:-left-1 data-[side=right]:-translate-y-1/2 data-[side=right]:border-b-0 data-[side=right]:border-l-0 data-[side=top]:-bottom-2 data-[side=top]:border-b-0 data-[side=top]:border-r-0" />
        </TooltipPopup>
      </TooltipPositioner>
    </TooltipPortal>
  );
}

interface SimpleTooltipProps {
  content: string;
  side?: "top" | "bottom" | "left" | "right";
  children: React.ReactElement;
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
