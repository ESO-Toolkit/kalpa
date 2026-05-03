import { cn } from "@/lib/utils";

function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      className={cn("relative overflow-hidden rounded-md bg-white/[0.06]", className)}
      {...props}
    >
      <div className="absolute inset-0 animate-shimmer bg-gradient-to-r from-transparent via-white/[0.08] to-transparent [animation-delay:var(--shimmer-delay,0ms)]" />
    </div>
  );
}

export { Skeleton };
