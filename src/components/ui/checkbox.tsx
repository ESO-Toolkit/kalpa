import { Checkbox as CheckboxPrimitive } from "@base-ui/react/checkbox";

import { cn } from "@/lib/utils";
import { CheckIcon } from "lucide-react";

function Checkbox({ className, ...props }: CheckboxPrimitive.Root.Props) {
  return (
    <CheckboxPrimitive.Root
      data-slot="checkbox"
      className={cn(
        "peer relative flex size-4 shrink-0 items-center justify-center rounded-[5px] border border-white/[0.12] bg-white/[0.04] shadow-[inset_0_1px_2px_rgba(0,0,0,0.2)] transition-all duration-150 outline-none group-has-disabled/field:opacity-50 after:absolute after:-inset-x-3 after:-inset-y-2 hover:border-white/[0.2] hover:bg-white/[0.06] focus-visible:border-sky-400/50 focus-visible:shadow-[inset_0_1px_2px_rgba(0,0,0,0.2),0_0_0_2px_rgba(56,189,248,0.12)] disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 data-checked:border-[#c4a44a]/60 data-checked:bg-gradient-to-b data-checked:from-[#d4b45a] data-checked:to-[#c4a44a] data-checked:text-[#0b1220] data-checked:shadow-[0_0_8px_rgba(196,164,74,0.2),inset_0_1px_0_rgba(255,255,255,0.2)]",
        className
      )}
      {...props}
    >
      <CheckboxPrimitive.Indicator
        data-slot="checkbox-indicator"
        className="grid place-content-center text-current transition-none [&>svg]:size-3.5 [&>svg]:stroke-[2.5]"
      >
        <CheckIcon />
      </CheckboxPrimitive.Indicator>
    </CheckboxPrimitive.Root>
  );
}

export { Checkbox };
