import * as React from "react";
import { Dialog as DialogPrimitive } from "@base-ui/react/dialog";

import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import { useCappedAnimationRate } from "@/hooks/use-capped-animation-rate";
import { XIcon } from "lucide-react";

import {
  Dialog,
  DialogTrigger,
  DialogPortal,
  DialogBackdrop,
  DialogPopup,
  DialogClose,
  DialogTitle as AnimatedDialogTitle,
  DialogDescription as AnimatedDialogDescription,
  type DialogBackdropProps,
  type DialogPopupProps,
} from "@/components/animate-ui/primitives/base/dialog";

function DialogOverlay({ className, ...props }: DialogBackdropProps) {
  return (
    <DialogBackdrop
      className={cn(
        "fixed inset-0 isolate z-50 bg-[radial-gradient(ellipse_at_center,rgba(0,0,0,0.45)_0%,rgba(0,0,0,0.7)_100%)] backdrop-blur-sm",
        className
      )}
      transition={{ duration: 0.15 }}
      {...props}
    />
  );
}

function DialogContent({
  className,
  children,
  showCloseButton = true,
  ...props
}: DialogPopupProps & {
  children?: React.ReactNode;
  showCloseButton?: boolean;
}) {
  return (
    <DialogPortal>
      <DialogOverlay />
      <DialogPopup
        className={cn(
          "fixed top-1/2 left-1/2 z-50 grid w-full max-w-[calc(100%-2rem)] -translate-x-1/2 -translate-y-1/2 gap-4 overflow-hidden rounded-2xl bg-surface-overlay backdrop-blur-2xl backdrop-saturate-[1.3] p-5 text-sm text-popover-foreground border border-white/[0.08] shadow-[0_24px_80px_rgba(0,0,0,0.6),0_0_0_1px_rgba(255,255,255,0.03),inset_0_1px_0_rgba(255,255,255,0.06)] outline-none sm:max-w-sm",
          className
        )}
        transition={{ type: "spring", stiffness: 400, damping: 30 }}
        {...props}
      >
        {children}
        {showCloseButton && (
          <DialogPrimitive.Close
            data-slot="dialog-close"
            render={
              <Button
                variant="ghost"
                className="absolute top-4 right-5 size-7 rounded-lg border border-white/[0.06] bg-white/[0.04] text-muted-foreground/60 shadow-[inset_0_1px_0_rgba(255,255,255,0.04)] hover:bg-white/[0.1] hover:border-white/[0.12] hover:text-foreground active:scale-95 transition-all duration-150"
                size="icon-sm"
              />
            }
          >
            <XIcon className="size-3.5" />
            <span className="sr-only">Close</span>
          </DialogPrimitive.Close>
        )}
      </DialogPopup>
    </DialogPortal>
  );
}

function DialogHeader({ className, children, ...props }: React.ComponentProps<"div">) {
  // Cap the sweep at ~60 updates/s: on high-refresh displays a free-running
  // compositor animation presents at every vsync and re-executes the popup's
  // backdrop-blur each frame (measured ~42% of a core at 240 Hz for this one
  // strip). 60 Hz is the cadence the design has always rendered at on
  // standard displays, so nothing is visually lost.
  const sweepRef = React.useRef<HTMLSpanElement>(null);
  useCappedAnimationRate(sweepRef, 16);
  return (
    <div
      data-slot="dialog-header"
      className={cn(
        "relative -mx-5 -mt-5 flex flex-col gap-1.5 border-b border-white/[0.06] bg-gradient-to-b from-white/[0.04] to-transparent pl-5 pr-12 pt-5 pb-4",
        className
      )}
      {...props}
    >
      {/* Accent shimmer strip. A real clipped child animated with transform
          (compositor-only) rather than a ::before animating background-position:
          a paint-property animation forces a main-thread repaint every frame for
          as long as any dialog is open. The 200%-wide sweep translating from
          +100% to -100% inside the 2px clip strip reproduces the exact
          background-size:200% / background-position -200%→200% trajectory. */}
      <span
        aria-hidden
        data-slot="dialog-header-accent"
        className="pointer-events-none absolute inset-x-0 top-0 h-[2px] overflow-hidden"
      >
        <span ref={sweepRef} className="dialog-accent-sweep block h-full w-[200%]" />
      </span>
      {children}
    </div>
  );
}

function DialogFooter({
  className,
  showCloseButton = false,
  children,
  ...props
}: React.ComponentProps<"div"> & {
  showCloseButton?: boolean;
}) {
  return (
    <div
      data-slot="dialog-footer"
      className={cn(
        "-mx-5 -mb-5 flex flex-col-reverse gap-2 rounded-b-2xl border-t border-white/[0.06] bg-gradient-to-b from-white/[0.02] to-transparent p-4 sm:flex-row sm:justify-end",
        className
      )}
      {...props}
    >
      {children}
      {showCloseButton && (
        <DialogPrimitive.Close render={<Button variant="outline" />}>Close</DialogPrimitive.Close>
      )}
    </div>
  );
}

function DialogTitle({ className, ...props }: React.ComponentProps<"h2">) {
  return (
    <AnimatedDialogTitle
      className={cn(
        "font-heading text-base leading-none font-semibold bg-gradient-to-r from-primary to-primary-hover bg-clip-text text-transparent",
        className
      )}
      {...props}
    />
  );
}

function DialogDescription({ className, ...props }: React.ComponentProps<"p">) {
  return (
    <AnimatedDialogDescription
      className={cn(
        "text-[13px] leading-relaxed text-muted-foreground/80 *:[a]:underline *:[a]:underline-offset-3 *:[a]:hover:text-foreground",
        className
      )}
      {...props}
    />
  );
}

export {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogOverlay,
  DialogPortal,
  DialogTitle,
  DialogTrigger,
};
