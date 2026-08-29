import { Collapsible as CollapsiblePrimitive } from "@base-ui/react/collapsible";

import { cn } from "@/lib/utils";

/**
 * Disclosure primitive wrapping Base UI's Collapsible.
 *
 * Deliberately unopinionated: the consumer supplies the trigger content and the
 * panel body. This wrapper only provides the height animation, overflow
 * containment, and a focus ring consistent with the app's other interactive
 * elements.
 */
function Collapsible({ className, ...props }: CollapsiblePrimitive.Root.Props) {
  return <CollapsiblePrimitive.Root data-slot="collapsible" className={cn(className)} {...props} />;
}

function CollapsibleTrigger({ className, ...props }: CollapsiblePrimitive.Trigger.Props) {
  return (
    <CollapsiblePrimitive.Trigger
      data-slot="collapsible-trigger"
      className={cn(
        "rounded-lg outline-none transition-colors duration-150 motion-reduce:transition-none",
        "focus-visible:ring-[3px] focus-visible:ring-accent-sky/20 focus-visible:border-accent-sky/40",
        "disabled:pointer-events-none disabled:opacity-50",
        className
      )}
      {...props}
    />
  );
}

/**
 * The panel body.
 *
 * Base UI publishes the measured content height as `--collapsible-panel-height`
 * on this element and flags the transition edges with `data-starting-style` /
 * `data-ending-style`, so the open/close animation is a plain CSS height
 * transition between `0` and that variable — no JS measurement in this file.
 */
function CollapsiblePanel({ className, ...props }: CollapsiblePrimitive.Panel.Props) {
  return (
    <CollapsiblePrimitive.Panel
      data-slot="collapsible-panel"
      className={cn(
        "h-[var(--collapsible-panel-height)] overflow-hidden",
        "transition-[height] duration-[250ms] ease-[cubic-bezier(0,0,0.2,1)]",
        "data-starting-style:h-0 data-ending-style:h-0",
        "motion-reduce:transition-none",
        className
      )}
      {...props}
    />
  );
}

export { Collapsible, CollapsibleTrigger, CollapsiblePanel };
