# Component Patterns — shadcn Implementation Recipes

Concrete patterns for building addon manager UI using shadcn (base-nova) + Tailwind v4, derived from the ESO Log Aggregator design language.

## Utility: cn() + CVA

All components use `cn()` from `@/lib/utils` for class merging and `cva` from `class-variance-authority` for variants. This is already set up.

## 1. Glass Panel Component

A reusable wrapper for the glass morphism effect. Create as `src/components/ui/glass-panel.tsx`:

```tsx
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "@/lib/utils";

const glassPanelVariants = cva(
  "rounded-xl backdrop-blur-lg [-webkit-backdrop-filter:blur(16px)] transition-shadow duration-300 ease-[cubic-bezier(0.4,0,0.2,1)]",
  {
    variants: {
      variant: {
        primary:
          "bg-[rgba(15,23,42,0.84)] border border-white/[0.09] shadow-[0_8px_32px_rgba(0,0,0,0.32),inset_0_1px_0_rgba(255,255,255,0.05)]",
        default:
          "bg-[rgba(15,23,42,0.66)] border border-white/[0.06] shadow-[0_4px_16px_rgba(0,0,0,0.2)]",
        subtle:
          "bg-white/[0.02] border border-white/[0.04] shadow-none",
      },
    },
    defaultVariants: { variant: "default" },
  }
);

function GlassPanel({
  className,
  variant,
  ...props
}: React.ComponentProps<"div"> & VariantProps<typeof glassPanelVariants>) {
  return (
    <div
      data-slot="glass-panel"
      className={cn(glassPanelVariants({ variant }), className)}
      {...props}
    />
  );
}

export { GlassPanel, glassPanelVariants };
```

## 2. Section Header (Uppercase Micro-Label)

Small, uppercase section titles used throughout the ESO-LOG-AG roster builder:

```tsx
function SectionHeader({
  className,
  ...props
}: React.ComponentProps<"h3">) {
  return (
    <h3
      data-slot="section-header"
      className={cn(
        "font-heading text-[11px] font-bold uppercase tracking-[0.05em] text-muted-foreground/60",
        className
      )}
      {...props}
    />
  );
}
```

Usage: `<SectionHeader>Addon Details</SectionHeader>`

## 3. Addon Card (Status-Colored Card)

Maps to the DPS/Healer/Tank slot cards in ESO-LOG-AG. The left border color indicates addon status:

```tsx
import { cva, type VariantProps } from "class-variance-authority";

const addonCardVariants = cva(
  "group relative rounded-xl border p-3 transition-all duration-200 ease-[cubic-bezier(0.4,0,0.2,1)] hover:-translate-y-px cursor-pointer",
  {
    variants: {
      status: {
        healthy:
          "bg-emerald-500/[0.04] border-emerald-500/[0.12] border-l-[3px] border-l-emerald-500 hover:border-emerald-500/[0.22] hover:shadow-[0_4px_12px_rgba(0,0,0,0.15)]",
        outdated:
          "bg-amber-500/[0.04] border-amber-500/[0.12] border-l-[3px] border-l-amber-500 hover:border-amber-500/[0.22] hover:shadow-[0_4px_12px_rgba(0,0,0,0.15)]",
        error:
          "bg-red-500/[0.04] border-red-500/[0.12] border-l-[3px] border-l-red-500 hover:border-red-500/[0.22] hover:shadow-[0_4px_12px_rgba(0,0,0,0.15)]",
        library:
          "bg-violet-400/[0.04] border-violet-400/[0.12] border-l-[3px] border-l-violet-400 hover:border-violet-400/[0.22] hover:shadow-[0_4px_12px_rgba(0,0,0,0.15)]",
        disabled:
          "bg-gray-500/[0.04] border-gray-500/[0.12] border-l-[3px] border-l-gray-500 hover:border-gray-500/[0.22] hover:shadow-[0_4px_12px_rgba(0,0,0,0.15)]",
      },
    },
    defaultVariants: { status: "healthy" },
  }
);
```

## 4. Status Badge / Info Pill

Small metadata badges used inline on cards (version, author, dependency count):

```tsx
const infoPillVariants = cva(
  "inline-flex items-center gap-1 rounded-lg border px-2 py-0.5 text-xs font-semibold transition-all duration-150",
  {
    variants: {
      color: {
        gold: "border-[#c4a44a]/20 bg-[#c4a44a]/[0.04] text-[#c4a44a]",
        sky: "border-sky-400/20 bg-sky-400/[0.04] text-sky-400",
        emerald: "border-emerald-400/20 bg-emerald-400/[0.04] text-emerald-400",
        amber: "border-amber-400/20 bg-amber-400/[0.04] text-amber-400",
        red: "border-red-400/20 bg-red-400/[0.04] text-red-400",
        violet: "border-violet-400/20 bg-violet-400/[0.04] text-violet-400",
        muted: "border-white/10 bg-white/[0.03] text-muted-foreground",
      },
    },
    defaultVariants: { color: "muted" },
  }
);
```

## 5. Glass Input Override

Override the default shadcn Input with glass styling. Apply via className or create a variant:

```
Tailwind classes for glass inputs:
"bg-white/[0.03] border-white/[0.08] rounded-[10px]
 hover:border-white/[0.15]
 focus:border-sky-400/40 focus:shadow-[0_0_0_2px_rgba(56,189,248,0.15)]
 placeholder:text-muted-foreground/40
 transition-all duration-250 ease-[cubic-bezier(0.4,0,0.2,1)]"
```

For search inputs, add a left icon and result count badge:
```
┌─ 🔍 │ Search addons...            │ 42 results │ ✕ ─┐
```

## 6. Dialog Enhancements

Extend the existing shadcn Dialog with glass morphism styling:

### DialogContent Override
```tsx
// In your DialogContent usage, pass these classes:
<DialogContent className="bg-gradient-to-br from-[rgba(15,23,42,0.97)] to-[rgba(30,41,59,0.97)] backdrop-blur-xl border-white/[0.08] shadow-[0_8px_30px_rgba(0,0,0,0.25)]">
```

### DialogTitle with Gradient Text
```tsx
<DialogTitle className="font-heading bg-gradient-to-r from-[#c4a44a] to-[#d4b45a] bg-clip-text text-transparent">
  Browse ESOUI
</DialogTitle>
```

### Picker Dialog Pattern (Search + Tabs + Body)
For the browse/install dialogs, follow this compound layout:

```tsx
<Dialog>
  <DialogContent className="sm:max-w-lg max-h-[90vh] flex flex-col gap-0 p-0">
    {/* Header */}
    <DialogHeader className="px-4 pt-4 pb-0">
      <DialogTitle className="font-heading">Browse Addons</DialogTitle>
    </DialogHeader>

    {/* Search */}
    <div className="px-4 py-3">
      <Input
        className="bg-white/[0.04] border-white/[0.08] rounded-[10px]"
        placeholder="Search addons..."
      />
    </div>

    {/* Filter Tabs — horizontal scroll */}
    <div className="flex gap-1 px-4 pb-3 overflow-x-auto [scrollbar-width:none] [-webkit-scrollbar]:hidden">
      {tabs.map(tab => (
        <Button key={tab} variant="ghost" size="xs" className="shrink-0">
          {tab}
        </Button>
      ))}
    </div>

    {/* Scrollable Body */}
    <div className="flex-1 overflow-y-auto px-4 pb-4 max-h-[calc(90vh-200px)]">
      {children}
    </div>
  </DialogContent>
</Dialog>
```

## 7. Accordion / Expandable Section

For addon details with collapsible sections (dependencies, changelog):

```
Tailwind classes:
"rounded-xl border border-border
 bg-gradient-to-br from-sky-400/[0.08] via-violet-400/[0.05] to-indigo-300/[0.03]
 shadow-[0_4px_12px_rgba(0,0,0,0.15)]
 [&>*:first-child]:rounded-t-xl"
```

Remove the default divider line between accordion items.

## 8. Toast / Notification Pattern

Already using `sonner`. Style toasts to match the glass aesthetic:

```tsx
// In your Toaster setup:
<Toaster
  toastOptions={{
    className: "!bg-[rgba(15,23,42,0.95)] !border-white/10 !text-foreground !backdrop-blur-lg",
  }}
/>
```

## 9. Empty State Pattern

When no addons are found or search returns nothing:

```tsx
<div className="flex flex-col items-center justify-center gap-3 py-12 text-center">
  <div className="rounded-xl bg-white/[0.03] p-4">
    <PackageOpen className="size-8 text-muted-foreground/40" />
  </div>
  <div>
    <p className="font-heading text-sm font-medium text-foreground/80">
      No addons found
    </p>
    <p className="text-xs text-muted-foreground/60">
      Set your ESO addons folder in Settings to get started.
    </p>
  </div>
</div>
```

## 10. Loading / Skeleton Pattern

Use pulse animation matching the glass panels:

```
Skeleton base classes:
"animate-pulse rounded-lg bg-white/[0.04]"
```

For card skeletons, use the status-colored left border but with muted tint:
```
"border-l-[3px] border-l-white/10 bg-white/[0.02] rounded-xl p-3"
```

## Component File Naming

Follow kebab-case: `glass-panel.tsx`, `addon-card.tsx`, `section-header.tsx`, `info-pill.tsx`.

Place reusable primitives in `src/components/ui/`.
Place feature-specific components in `src/components/`.
