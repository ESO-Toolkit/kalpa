import * as React from "react";
import { Input as InputPrimitive } from "@base-ui/react/input";

import { cn } from "@/lib/utils";

function Input({ className, type, ...props }: React.ComponentProps<"input">) {
  return (
    <InputPrimitive
      type={type}
      data-slot="input"
      className={cn(
        "h-8 w-full min-w-0 rounded-[10px] border border-white/[0.08] bg-white/[0.04] px-2.5 py-1 text-base shadow-[inset_0_1px_2px_rgba(0,0,0,0.2),0_1px_0_rgba(255,255,255,0.02)] transition-all duration-200 ease-[cubic-bezier(0.4,0,0.2,1)] outline-none file:inline-flex file:h-6 file:border-0 file:bg-transparent file:text-sm file:font-medium file:text-foreground placeholder:text-muted-foreground/30 hover:border-white/[0.15] hover:bg-white/[0.05] focus-visible:border-sky-400/50 focus-visible:bg-white/[0.06] focus-visible:shadow-[inset_0_1px_2px_rgba(0,0,0,0.2),0_0_0_2px_rgba(56,189,248,0.12),0_0_12px_rgba(56,189,248,0.06)] disabled:pointer-events-none disabled:cursor-not-allowed disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 md:text-sm",
        className
      )}
      {...props}
    />
  );
}

export { Input };
