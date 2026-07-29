import { Button as ButtonPrimitive } from "@base-ui/react/button";
import { cva, type VariantProps } from "class-variance-authority";

import { cn } from "@/lib/utils";
import { Shine } from "@/components/animate-ui/primitives/effects/shine";

const buttonVariants = cva(
  "group/button inline-flex shrink-0 items-center justify-center rounded-lg border border-transparent bg-clip-padding text-sm font-medium whitespace-nowrap transition-all duration-150 ease-[cubic-bezier(0.4,0,0.2,1)] outline-none select-none active:not-aria-[haspopup]:translate-y-px active:not-aria-[haspopup]:scale-[0.98] disabled:pointer-events-none disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-3 aria-invalid:ring-destructive/20 [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
  {
    variants: {
      variant: {
        default:
          "bg-gradient-to-b from-primary-hover to-primary text-primary-foreground font-semibold border-primary/50 shadow-[inset_0_1px_0_var(--structure-20),0_1px_3px_rgba(0,0,0,0.3)] hover:brightness-110 hover:shadow-[inset_0_1px_0_var(--structure-25),0_0_12px_var(--primary-glow),0_2px_6px_rgba(0,0,0,0.3)] focus-visible:ring-3 focus-visible:ring-primary/40",
        outline:
          "bg-structure-04 border-structure-10 text-foreground shadow-[inset_0_1px_0_var(--structure-04),0_1px_2px_rgba(0,0,0,0.15)] hover:bg-structure-08 hover:border-structure-15 hover:shadow-[inset_0_1px_0_var(--structure-06),0_2px_4px_rgba(0,0,0,0.2)] aria-expanded:bg-structure-08 aria-expanded:border-structure-15 focus-visible:border-accent-sky/40 focus-visible:ring-3 focus-visible:ring-accent-sky/20",
        secondary:
          "bg-structure-06 border-structure-08 text-foreground hover:bg-structure-10 hover:border-structure-12 focus-visible:border-ring focus-visible:ring-3 focus-visible:ring-ring/50",
        ghost:
          "text-muted-foreground hover:text-foreground hover:bg-structure-06 aria-expanded:bg-structure-06 aria-expanded:text-foreground focus-visible:border-accent-sky/40 focus-visible:ring-3 focus-visible:ring-accent-sky/20",
        destructive:
          "bg-red-500/[0.08] text-red-400 border-red-500/20 shadow-[inset_0_1px_0_color-mix(in_oklab,var(--status-error-strong)_6%,transparent)] hover:bg-red-500/[0.15] hover:border-red-500/30 hover:shadow-[inset_0_1px_0_color-mix(in_oklab,var(--status-error-strong)_8%,transparent),0_0_12px_color-mix(in_oklab,var(--status-error-strong)_15%,transparent)] focus-visible:border-red-500/40 focus-visible:ring-3 focus-visible:ring-red-500/20",
        link: "text-primary underline-offset-4 hover:underline focus-visible:ring-3 focus-visible:ring-ring/50",
      },
      size: {
        default:
          "h-8 gap-1.5 px-2.5 has-data-[icon=inline-end]:pr-2 has-data-[icon=inline-start]:pl-2",
        xs: "h-6 gap-1 rounded-[min(var(--radius-md),10px)] px-2 text-xs in-data-[slot=button-group]:rounded-lg has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 [&_svg:not([class*='size-'])]:size-3",
        sm: "h-7 gap-1 rounded-[min(var(--radius-md),12px)] px-2.5 text-[0.8rem] in-data-[slot=button-group]:rounded-lg has-data-[icon=inline-end]:pr-1.5 has-data-[icon=inline-start]:pl-1.5 [&_svg:not([class*='size-'])]:size-3.5",
        lg: "h-9 gap-1.5 px-2.5 has-data-[icon=inline-end]:pr-3 has-data-[icon=inline-start]:pl-3",
        icon: "size-8",
        "icon-xs":
          "size-6 rounded-[min(var(--radius-md),10px)] in-data-[slot=button-group]:rounded-lg [&_svg:not([class*='size-'])]:size-3",
        "icon-sm":
          "size-7 rounded-[min(var(--radius-md),12px)] in-data-[slot=button-group]:rounded-lg",
        "icon-lg": "size-9",
      },
    },
    defaultVariants: {
      variant: "default",
      size: "default",
    },
  }
);

function Button({
  className,
  variant = "default",
  size = "default",
  ...props
}: ButtonPrimitive.Props & VariantProps<typeof buttonVariants>) {
  const isGold = variant === "default" || variant === undefined;

  const btn = (
    <ButtonPrimitive
      data-slot="button"
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  );

  if (isGold) {
    return (
      <Shine asChild enableOnHover color="var(--structure-40)" duration={600} opacity={0.35}>
        {btn}
      </Shine>
    );
  }

  return btn;
}

export { Button, buttonVariants };
