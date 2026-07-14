import { Combobox as ComboboxPrimitive } from "@base-ui/react/combobox";
import { motion } from "motion/react";

import { cn } from "@/lib/utils";
import { ChevronDownIcon, CheckIcon } from "lucide-react";

const Combobox = ComboboxPrimitive.Root;

function ComboboxValue({
  className,
  ...props
}: ComboboxPrimitive.Value.Props & { className?: string }) {
  // The Value primitive renders no element of its own, so wrap it in a styled
  // span to mirror SelectValue's layout inside the trigger.
  return (
    <span data-slot="combobox-value" className={cn("flex flex-1 truncate text-left", className)}>
      <ComboboxPrimitive.Value {...props} />
    </span>
  );
}

function ComboboxTrigger({
  className,
  size = "default",
  children,
  ...props
}: ComboboxPrimitive.Trigger.Props & {
  size?: "sm" | "default";
}) {
  return (
    <ComboboxPrimitive.Trigger
      data-slot="combobox-trigger"
      data-size={size}
      className={cn(
        "flex w-fit items-center justify-between gap-1.5 rounded-[10px] border border-white/[0.08] bg-white/[0.04] py-2 pr-2 pl-2.5 text-sm whitespace-nowrap shadow-[inset_0_1px_2px_rgba(0,0,0,0.2),0_1px_0_rgba(255,255,255,0.02)] transition-all duration-200 ease-[cubic-bezier(0.4,0,0.2,1)] outline-none select-none hover:border-white/[0.15] hover:bg-white/[0.06] focus-visible:border-accent-sky/50 focus-visible:shadow-[inset_0_1px_2px_rgba(0,0,0,0.2),0_0_0_2px_color-mix(in_oklab,var(--accent-sky)_12%,transparent),0_0_12px_color-mix(in_oklab,var(--accent-sky)_6%,transparent)] disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 data-placeholder:text-muted-foreground data-[size=default]:h-8 data-[size=sm]:h-7 data-[size=sm]:rounded-[min(var(--radius-md),10px)] *:data-[slot=combobox-value]:line-clamp-1 *:data-[slot=combobox-value]:flex *:data-[slot=combobox-value]:items-center *:data-[slot=combobox-value]:gap-1.5 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
        className
      )}
      {...props}
    >
      {children}
      <ComboboxPrimitive.Icon
        render={
          <ChevronDownIcon className="pointer-events-none size-4 text-muted-foreground/60 transition-transform duration-200 group-aria-expanded:rotate-180" />
        }
      />
    </ComboboxPrimitive.Trigger>
  );
}

function ComboboxContent({
  className,
  children,
  side = "bottom",
  sideOffset = 4,
  align = "center",
  alignOffset = 0,
  ...props
}: ComboboxPrimitive.Popup.Props &
  Pick<ComboboxPrimitive.Positioner.Props, "align" | "alignOffset" | "side" | "sideOffset">) {
  return (
    <ComboboxPrimitive.Portal>
      <ComboboxPrimitive.Positioner
        side={side}
        sideOffset={sideOffset}
        align={align}
        alignOffset={alignOffset}
        className="isolate z-50"
      >
        <ComboboxPrimitive.Popup
          data-slot="combobox-content"
          render={
            <motion.div
              initial={{ opacity: 0, scale: 0.95 }}
              animate={{ opacity: 1, scale: 1 }}
              exit={{ opacity: 0, scale: 0.95 }}
              transition={{ type: "spring", stiffness: 500, damping: 30 }}
            />
          }
          className={cn(
            "relative isolate z-50 flex max-h-(--available-height) w-(--anchor-width) min-w-36 origin-(--transform-origin) flex-col overflow-hidden rounded-xl border border-white/[0.08] bg-surface-overlay text-popover-foreground shadow-[0_16px_48px_rgba(0,0,0,0.5),0_0_0_1px_rgba(255,255,255,0.03),inset_0_1px_0_rgba(255,255,255,0.06)] backdrop-blur-2xl",
            className
          )}
          {...props}
        >
          {children}
        </ComboboxPrimitive.Popup>
      </ComboboxPrimitive.Positioner>
    </ComboboxPrimitive.Portal>
  );
}

function ComboboxInput({ className, ...props }: ComboboxPrimitive.Input.Props) {
  return (
    <ComboboxPrimitive.Input
      data-slot="combobox-input"
      className={cn(
        "h-8 w-full flex-1 bg-transparent text-xs outline-none placeholder:text-muted-foreground/50",
        className
      )}
      {...props}
    />
  );
}

function ComboboxList({ className, ...props }: ComboboxPrimitive.List.Props) {
  return (
    <ComboboxPrimitive.List
      data-slot="combobox-list"
      className={cn("max-h-64 overflow-y-auto p-1 empty:p-0", className)}
      {...props}
    />
  );
}

function ComboboxItem({ className, children, ...props }: ComboboxPrimitive.Item.Props) {
  return (
    <ComboboxPrimitive.Item
      data-slot="combobox-item"
      className={cn(
        "relative flex w-full cursor-default items-center gap-1.5 rounded-lg py-1.5 pr-8 pl-2 text-sm outline-hidden select-none transition-colors duration-100 focus:bg-white/[0.06] focus:text-foreground data-highlighted:bg-white/[0.06] data-highlighted:text-foreground data-disabled:pointer-events-none data-disabled:opacity-50 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4 *:[span]:last:flex *:[span]:last:items-center *:[span]:last:gap-2",
        className
      )}
      {...props}
    >
      <span className="flex flex-1 shrink-0 gap-2 whitespace-nowrap">{children}</span>
      <ComboboxPrimitive.ItemIndicator
        render={
          <span className="pointer-events-none absolute right-2 flex size-4 items-center justify-center text-primary" />
        }
      >
        <CheckIcon className="pointer-events-none size-3.5" />
      </ComboboxPrimitive.ItemIndicator>
    </ComboboxPrimitive.Item>
  );
}

function ComboboxEmpty({ className, ...props }: ComboboxPrimitive.Empty.Props) {
  return (
    <ComboboxPrimitive.Empty
      data-slot="combobox-empty"
      className={cn(
        "px-2 py-3 text-center text-xs text-muted-foreground empty:m-0 empty:p-0",
        className
      )}
      {...props}
    />
  );
}

function ComboboxStatus({ className, ...props }: ComboboxPrimitive.Status.Props) {
  return (
    <ComboboxPrimitive.Status
      data-slot="combobox-status"
      className={cn(
        "border-b border-white/[0.06] px-2 py-1.5 text-[11px] text-muted-foreground/70 empty:hidden",
        className
      )}
      {...props}
    />
  );
}

export {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
  ComboboxStatus,
  ComboboxTrigger,
  ComboboxValue,
};
