import {
  Checkbox as CheckboxPrimitive,
  CheckboxIndicator,
  type CheckboxProps,
} from "@/components/animate-ui/primitives/base/checkbox";

import { cn } from "@/lib/utils";

function Checkbox({ className, ...props }: CheckboxProps) {
  return (
    <CheckboxPrimitive
      className={cn(
        "peer relative flex size-4 shrink-0 items-center justify-center rounded-[5px] border border-white/[0.12] bg-white/[0.04] shadow-[inset_0_1px_2px_rgba(0,0,0,0.2)] transition-all duration-150 outline-none group-has-disabled/field:opacity-50 after:absolute after:-inset-x-3 after:-inset-y-2 hover:border-white/[0.2] hover:bg-white/[0.06] focus-visible:border-accent-sky/50 focus-visible:shadow-[inset_0_1px_2px_rgba(0,0,0,0.2),0_0_0_2px_color-mix(in_oklab,var(--accent-sky)_12%,transparent)] disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 data-checked:border-primary/60 data-checked:bg-gradient-to-b data-checked:from-primary-hover data-checked:to-primary data-checked:text-[#0b1220] data-checked:shadow-[0_0_8px_color-mix(in_oklab,var(--primary)_20%,transparent),inset_0_1px_0_rgba(255,255,255,0.2)]",
        className
      )}
      {...props}
    >
      <CheckboxIndicator className="size-3.5 [&>svg]:stroke-[2.5]" />
    </CheckboxPrimitive>
  );
}

export { Checkbox };
