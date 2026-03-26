# Theme Tokens — CSS Variables & Tailwind v4 Configuration

Ready-to-apply theme updates for `src/index.css` that align the addon manager with the ESO Log Aggregator design language.

## Updated index.css

Below is the target state for `src/index.css`. Key changes from the current theme:

1. **Added Space Grotesk** as heading font
2. **Deeper, richer background colors** matching ESO-LOG-AG's navy-slate palette
3. **Added accent-sky and accent-cyan** for interactive elements (the sky-blue glow from ESO-LOG-AG)
4. **Added semantic addon status colors** as custom properties
5. **Added glass morphism tokens** as custom properties for reuse
6. **Improved scrollbar** with accent-colored thumb
7. **Added animation tokens** for consistent timing

### :root Variables (updated)

```css
:root {
    /* === Core Palette (ESO-LOG-AG aligned) === */
    --background: #0b1220;
    --foreground: #e5e7eb;
    --card: #0f172a;
    --card-foreground: #e5e7eb;
    --card-alt: #0d1430;
    --popover: #0f172a;
    --popover-foreground: #e5e7eb;

    /* === ESO Gold (unchanged) === */
    --primary: #c4a44a;
    --primary-foreground: #0b1220;

    /* === Interactive Accent (sky-blue from ESO-LOG-AG) === */
    --secondary: #0f3460;
    --secondary-foreground: #e5e7eb;
    --accent: #0f3460;
    --accent-foreground: #e5e7eb;
    --accent-sky: #38bdf8;
    --accent-cyan: #00e1ff;

    /* === Semantic === */
    --muted: #1a2a4a;
    --muted-foreground: #94a3b8;
    --destructive: #ef4444;
    --success: #22c55e;
    --warning: #ff9800;

    /* === Surfaces === */
    --border: #1f2937;
    --input: #1f2937;
    --ring: #c4a44a;

    /* === Addon Status Colors === */
    --addon-healthy: #22c55e;
    --addon-outdated: #ff9800;
    --addon-error: #ef4444;
    --addon-library: #a78bfa;
    --addon-disabled: #6b7280;

    /* === Glass Morphism Tokens === */
    --glass-bg: rgba(15, 23, 42, 0.84);
    --glass-bg-light: rgba(15, 23, 42, 0.66);
    --glass-border: rgba(255, 255, 255, 0.09);
    --glass-border-subtle: rgba(255, 255, 255, 0.04);
    --glass-blur: 16px;
    --glass-shadow: 0 8px 32px rgba(0, 0, 0, 0.32), 0 1px 0 rgba(255, 255, 255, 0.05) inset;
    --glass-shadow-light: 0 4px 16px rgba(0, 0, 0, 0.2);

    /* === Scrollbar === */
    --scrollbar-track: rgba(15, 23, 42, 0.5);
    --scrollbar-thumb: rgba(56, 189, 248, 0.3);
    --scrollbar-thumb-hover: rgba(56, 189, 248, 0.5);
    --scrollbar-thumb-active: rgba(56, 189, 248, 0.7);

    /* === Charts (unchanged) === */
    --chart-1: #c4a44a;
    --chart-2: #3498db;
    --chart-3: #2ecc71;
    --chart-4: #f39c12;
    --chart-5: #e74c3c;

    /* === Radius === */
    --radius: 0.5rem;

    /* === Sidebar === */
    --sidebar: #0f172a;
    --sidebar-foreground: #e5e7eb;
    --sidebar-primary: #c4a44a;
    --sidebar-primary-foreground: #0b1220;
    --sidebar-accent: #0f3460;
    --sidebar-accent-foreground: #e5e7eb;
    --sidebar-border: #1f2937;
    --sidebar-ring: #c4a44a;
}
```

### @theme inline Additions

Add these new token mappings to the existing `@theme inline` block:

```css
@theme inline {
    /* Existing mappings stay... */

    /* --- New: Heading font --- */
    --font-heading: 'Space Grotesk Variable', 'Geist Variable', system-ui, sans-serif;

    /* --- New: Accent colors --- */
    --color-accent-sky: var(--accent-sky);
    --color-accent-cyan: var(--accent-cyan);

    /* --- New: Semantic status --- */
    --color-success: var(--success);
    --color-warning: var(--warning);

    /* --- New: Addon status --- */
    --color-addon-healthy: var(--addon-healthy);
    --color-addon-outdated: var(--addon-outdated);
    --color-addon-error: var(--addon-error);
    --color-addon-library: var(--addon-library);
    --color-addon-disabled: var(--addon-disabled);

    /* --- New: Glass tokens --- */
    --color-glass-bg: var(--glass-bg);
    --color-glass-border: var(--glass-border);

    /* --- New: Card alt --- */
    --color-card-alt: var(--card-alt);

    /* --- Keep existing gold --- */
    --color-gold: #c4a44a;
    --color-gold-hover: #d4b45a;
}
```

### Updated Scrollbar (App.css)

Replace the current scrollbar styles:

```css
/* Custom scrollbar — sky-blue accent (ESO-LOG-AG style) */
::-webkit-scrollbar {
  width: 10px;
  height: 10px;
}

::-webkit-scrollbar-track {
  background: var(--scrollbar-track, rgba(15, 23, 42, 0.5));
  border-radius: 6px;
  margin: 2px;
}

::-webkit-scrollbar-thumb {
  background: var(--scrollbar-thumb, rgba(56, 189, 248, 0.3));
  border-radius: 6px;
  border: 2px solid var(--scrollbar-track, rgba(15, 23, 42, 0.5));
  background-clip: padding-box;
  transition: background-color 0.2s ease;
}

::-webkit-scrollbar-thumb:hover {
  background: var(--scrollbar-thumb-hover, rgba(56, 189, 248, 0.5));
}

::-webkit-scrollbar-thumb:active {
  background: var(--scrollbar-thumb-active, rgba(56, 189, 248, 0.7));
}

/* Firefox */
* {
  scrollbar-width: thin;
  scrollbar-color: var(--scrollbar-thumb) var(--scrollbar-track);
}
```

### Animation Utilities (add to index.css)

```css
@layer base {
  /* View transitions */
  ::view-transition-old(root) {
    animation: vt-fade-out 0.15s ease-out both;
  }
  ::view-transition-new(root) {
    animation: vt-fade-in 0.15s ease-in both;
  }
}

@keyframes vt-fade-out {
  to { opacity: 0; }
}

@keyframes vt-fade-in {
  from { opacity: 0; }
}
```

## Tailwind Utility Classes Quick Reference

Common class combinations you'll use repeatedly:

| Pattern | Classes |
|---------|---------|
| Glass panel (primary) | `bg-[--glass-bg] backdrop-blur-lg border border-[--glass-border] rounded-xl shadow-[--glass-shadow]` |
| Glass panel (subtle) | `bg-white/[0.02] border border-white/[0.04] rounded-[10px]` |
| Glass input | `bg-white/[0.03] border-white/[0.08] rounded-[10px] hover:border-white/15 focus:border-accent-sky/40` |
| Section header | `font-heading text-[11px] font-bold uppercase tracking-[0.05em] text-muted-foreground/60` |
| Card hover lift | `hover:-translate-y-px transition-all duration-200` |
| Panel hover lift | `hover:-translate-y-[3px] transition-all duration-300` |
| Gradient gold button | `bg-gradient-to-br from-gold to-gold-hover shadow-[0_4px_15px_rgba(196,164,74,0.25)]` |
| Gradient dialog bg | `bg-gradient-to-br from-[rgba(15,23,42,0.97)] to-[rgba(30,41,59,0.97)]` |
| Fade transition | `transition-opacity duration-250 ease-[cubic-bezier(0.4,0,0.2,1)]` |
| Hidden scrollbar | `overflow-x-auto [scrollbar-width:none] [&::-webkit-scrollbar]:hidden` |

## Dependencies to Add

```bash
npm install @fontsource-variable/space-grotesk
```

Then add to `src/index.css`:
```css
@import "@fontsource-variable/space-grotesk";
```

No other new dependencies are needed — the existing stack (shadcn base-nova, CVA, Tailwind v4, tw-animate-css, lucide-react) covers everything.
