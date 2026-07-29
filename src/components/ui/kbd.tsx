import { cn } from "@/lib/utils";

function Kbd({ className, ...props }: React.ComponentProps<"kbd">) {
  return (
    <kbd
      data-slot="kbd"
      className={cn(
        "inline-flex min-w-[1.75rem] items-center justify-center rounded-[4px] border border-structure-10 bg-structure-06 px-1.5 py-0.5 font-mono text-[11px] leading-none text-foreground",
        className
      )}
      {...props}
    />
  );
}

export { Kbd };
