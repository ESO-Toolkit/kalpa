import { cn } from "@/lib/utils";

function ProgressBar({
  value,
  max = 100,
  label,
  className,
  ...props
}: React.ComponentProps<"div"> & {
  value: number;
  max?: number;
  label?: string;
}) {
  const pct = max > 0 ? Math.min(100, Math.max(0, (value / max) * 100)) : 0;

  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={max}
      aria-valuenow={Math.round(Math.min(max, Math.max(0, value)))}
      aria-label={label}
      className={cn(
        "relative h-2 overflow-hidden rounded-full bg-scrim-40 ring-1 ring-inset ring-structure-06",
        className
      )}
      {...props}
    >
      <div
        className="absolute inset-y-0 left-0 rounded-full bg-gradient-to-r from-primary to-primary-hover transition-[width] duration-250 ease-[cubic-bezier(0.4,0,0.2,1)]"
        style={{ width: `${pct}%` }}
      />
    </div>
  );
}

export { ProgressBar };
