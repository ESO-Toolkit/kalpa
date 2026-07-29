import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const infoPillVariants = cva(
  "inline-flex items-center gap-1 rounded-lg border px-2 py-0.5 text-xs font-semibold transition-all duration-150",
  {
    variants: {
      color: {
        gold: "border-primary/20 bg-primary/[0.04] text-primary",
        sky: "border-accent-sky/20 bg-accent-sky/[0.04] text-accent-sky",
        emerald: "border-status-success/20 bg-status-success/[0.04] text-status-success",
        amber: "border-status-warning/20 bg-status-warning/[0.04] text-status-warning",
        red: "border-status-danger/20 bg-status-danger/[0.04] text-status-danger",
        violet: "border-status-library/20 bg-status-library/[0.04] text-status-library",
        muted: "border-structure-10 bg-structure-03 text-muted-foreground",
      },
    },
    defaultVariants: { color: "muted" },
  }
);

function InfoPill({
  className,
  color,
  ...props
}: React.ComponentProps<"span"> & VariantProps<typeof infoPillVariants>) {
  return (
    <span data-slot="info-pill" className={cn(infoPillVariants({ color }), className)} {...props} />
  );
}

export { InfoPill, infoPillVariants };
