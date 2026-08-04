# Design System — ESO Log Aggregator Visual Language

> **Partly superseded:** the design DNA below still holds, but this document predates the light-theme token migration. Kalpa is **dark-first with light themes in scope** — 54 built-in themes including three light and two high-contrast accessibility themes — not "always-dark". Every literal `rgba(…)` / `rgb(255,255,255,…)` value quoted here now lives behind a theme-aware token (`structure-*`, `scrim-*`, `status-*`, `glass-*`), because a hardcoded white-alpha border is invisible on a light theme. Treat the hex/rgba tables here as the *dark* resolution of tokens, never as values to type into a component. `src/index.css`, `src/lib/theme-apply.ts`, and `context/42-theme-tokens.md` are the authority.

This document defines the design language ported from the ESO Log Aggregator (ESO-LOG-AG) project. All new UI work in this repo should follow these patterns, adapted for **shadcn (base-nova) + Tailwind CSS v4 + Base UI React**.

## Design DNA

The ESO-LOG-AG design system is built on five pillars:

1. **Glass Morphism** — Translucent panels with backdrop blur, subtle borders, and layered depth
2. **Accent-Colored Cards** — Cards with colored left-border stripes and tinted backgrounds
3. **Dual-Font Typography** — Space Grotesk for headings/labels, Inter (or Geist) for body text
4. **Token-Driven Theming** — All colors defined as CSS variables, dark-first with light mode support
5. **Smooth Micro-Interactions** — Subtle translateY, opacity, and box-shadow transitions on hover

## Fonts

| Role | Font | Weight Range | Usage |
|------|------|-------------|-------|
| Headings | Space Grotesk | 300–700 | h1–h6, section headers, dialog titles, badges |
| Body | Geist Variable (this project) / Inter (ESO-LOG-AG) | 100–900 | Body text, inputs, descriptions |

**Section Header Pattern** (uppercase micro-labels):
```
font-family: 'Space Grotesk', system-ui
font-size: 11px (0.6875rem)
font-weight: 700
letter-spacing: 0.8px (0.05em)
text-transform: uppercase
color: muted-foreground (reduced opacity)
```

Install Space Grotesk:
```bash
npm install @fontsource-variable/space-grotesk
```

Import in `index.css`:
```css
@import "@fontsource-variable/space-grotesk";
```

Add to `@theme inline`:
```css
--font-heading: 'Space Grotesk Variable', 'Geist Variable', system-ui, sans-serif;
```

## Color Palette

### Dark Mode (the default theme's resolution of the base tokens)

| Token | Value | Usage |
|-------|-------|-------|
| `--background` | `#0b1220` | App background |
| `--card` | `#0f172a` | Panel/card background |
| `--card-alt` | `#0d1430` | Secondary panel background |
| `--foreground` | `#e5e7eb` | Primary text |
| `--muted-foreground` | `#94a3b8` | Secondary/muted text |
| `--primary` | `#c4a44a` | ESO Gold accent (keep from current theme) |
| `--accent-sky` | `#38bdf8` | Interactive accent (sky blue) |
| `--accent-cyan` | `#00e1ff` | Secondary accent (cyan glow) |
| `--success` | `#22c55e` | Success states |
| `--warning` | `#ff9800` | Warning states |
| `--destructive` | `#ef4444` | Error/destructive states |
| `--border` | `#1f2937` | Default borders |

### ESO-Specific Semantic Colors

| Token | Value | Usage |
|-------|-------|-------|
| `--eso-gold` | `#c4a44a` | Brand gold (primary actions) |
| `--eso-gold-hover` | `#d4b45a` | Gold hover state |
| `--addon-enabled` | `#22c55e` | Enabled addon indicator |
| `--addon-disabled` | `#6b7280` | Disabled addon indicator |
| `--addon-outdated` | `#ff9800` | Outdated addon indicator |
| `--addon-error` | `#ef4444` | Error/missing dependency |
| `--library` | `#a78bfa` | Library addon type |

## Glass Morphism

Three tiers of glass panels, from most prominent to most subtle. Use the `GlassPanel` primitive (`src/components/ui/glass-panel.tsx`) rather than re-rolling the effect; the classes below are what it applies today.

### Primary Glass (feature panels, main content areas)
```
bg-glass-bg border border-structure-09
shadow-[0_8px_32px_var(--scrim-32),inset_0_1px_0_var(--structure-05)]
rounded-xl backdrop-blur-lg
```

### Default Glass (secondary panels, sidebars)
```
bg-glass-bg-light border border-structure-06
shadow-[0_4px_16px_var(--scrim-20)]
rounded-xl backdrop-blur-lg
```

### Subtle Glass (nested sections, input containers)
```
bg-structure-02 border border-structure-04
shadow-[inset_0_1px_0_var(--structure-03)]
hover:border-structure-07 transition-colors duration-200
```

### Glass Input Fields

Use the overridden `Input` (`src/components/ui/input.tsx`), which already carries:
```
rounded-[10px] border border-structure-08 bg-structure-04
shadow-[inset_0_1px_2px_var(--scrim-20),0_1px_0_var(--structure-02)]
hover:border-structure-15 hover:bg-structure-05
focus-visible:border-accent-sky/50 focus-visible:bg-structure-06
```

## Card Patterns

### Addon Card (equivalent to Roster Card)
Cards should have a colored left-border accent to indicate status/type:

```css
border-radius: 12px;
background: rgba(<status-color>, 0.04);    /* very subtle tint */
border: 1px solid rgba(<status-color>, 0.12);
border-left: 3px solid <status-color>;
padding: 12px;

/* hover */
transform: translateY(-1px);
border-color: rgba(<status-color>, 0.22);
box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
transition: all 0.2s ease;
```

Status color mapping:
- Enabled & up-to-date: `--eso-gold` or `--success`
- Outdated: `--warning`
- Missing dependencies: `--destructive`
- Library: `--library`
- Disabled: `--addon-disabled`

### Info Pill / Action Badge (equivalent to Slot Action Pill)
Small linked badges showing metadata with an action button:

```css
display: inline-flex;
border-radius: 8px;
border: 1px solid rgba(<accent>, 0.19);
background: rgba(<accent>, 0.04);
font-size: 0.75rem;
font-weight: 600;

/* divider between sections */
border-right: 1px solid rgba(<accent>, 0.15);

/* hover */
border-color: rgba(<accent>, 0.31);
box-shadow: 0 0 8px rgba(<accent>, 0.08);
```

## Dialog / Modal Patterns

### Standard Dialog
```css
/* Overlay */
background: rgba(0, 0, 0, 0.5);
backdrop-filter: blur(4px);

/* Content panel */
background: linear-gradient(135deg, rgba(15, 23, 42, 0.97), rgba(30, 41, 59, 0.97));
backdrop-filter: blur(20px);
border-radius: 0.75rem;
border: 1px solid rgba(255, 255, 255, 0.08);
max-height: 90vh;
box-shadow: 0 8px 30px rgba(0, 0, 0, 0.25);
```

### Picker / Search Dialog (compound component pattern)
Used for browsing/selecting items with search + tabs + scrollable body:

```
Structure:
┌─ Title bar (gradient text, close button) ────────────┐
│─ Search input (glass style, result count badge) ─────│
│─ Filter tabs (horizontal scroll, pill buttons) ──────│
│─ Body (scrollable, max-height: calc(90vh - 200px)) ──│
└──────────────────────────────────────────────────────┘
```

- Title uses `font-heading` with gradient text
- Search input is glass-styled with left search icon, right clear button + count badge
- Tabs are horizontal pills with `overflow-x: auto` and hidden scrollbar
- Body has `overflow-y: auto` with custom scrollbar

### Player Card Modal (detail view with navigation)
For addon detail views with prev/next navigation:

```
Structure:
┌─ [<] Title / Counter (3/12) [>] [X] ────────────────┐
│                                                       │
│  Content with fade transitions (250ms)               │
│                                                       │
└──────────────────────────────────────────────────────┘
```

## Animation & Transitions

### Duration Scale
| Name | Duration | Usage |
|------|----------|-------|
| `fast` | `150ms` | Hover states, toggles, micro-interactions |
| `normal` | `250ms` | Standard transitions, fade in/out |
| `slow` | `400ms` | Page transitions, dialog open/close |

### Easing Functions
| Name | Value | Usage |
|------|-------|-------|
| `standard` | `cubic-bezier(0.4, 0, 0.2, 1)` | Most transitions |
| `decelerate` | `cubic-bezier(0, 0, 0.2, 1)` | Elements entering view |
| `accelerate` | `cubic-bezier(0.4, 0, 1, 1)` | Elements leaving view |

### Common Hover Effects
```css
/* Card hover — subtle lift */
transform: translateY(-1px);
box-shadow: <enhanced-shadow>;
transition: all 0.2s cubic-bezier(0.4, 0, 0.2, 1);

/* Button hover — brightness boost */
filter: brightness(1.05);

/* Panel hover — enhanced lift */
transform: translateY(-3px);
box-shadow: <large-shadow>;
```

### Tab Content Transitions
```css
/* Fade out */
opacity: 0;
transform: translateY(6px) scale(0.99);
filter: blur(2px);
transition: 150ms ease;

/* Fade in */
opacity: 1;
transform: translateY(0) scale(1);
filter: blur(0);
transition: 200ms cubic-bezier(0, 0, 0.2, 1);
```

## Scrollbar Styling

```css
::-webkit-scrollbar { width: 10px; height: 10px; }
::-webkit-scrollbar-track {
  background: rgba(15, 23, 42, 0.5);
  border-radius: 6px;
  margin: 2px;
}
::-webkit-scrollbar-thumb {
  background: rgba(56, 189, 248, 0.3);
  border-radius: 6px;
  border: 2px solid rgba(15, 23, 42, 0.5);
  background-clip: padding-box;
}
::-webkit-scrollbar-thumb:hover { background: rgba(56, 189, 248, 0.5); }
::-webkit-scrollbar-thumb:active { background: rgba(56, 189, 248, 0.7); }

/* Firefox */
* { scrollbar-width: thin; scrollbar-color: rgba(56, 189, 248, 0.3) rgba(15, 23, 42, 0.5); }
```

## Spacing Conventions

Base unit: **4px** (Tailwind's default spacing scale)

| Context | Value | Tailwind Class |
|---------|-------|---------------|
| Card padding | 12px | `p-3` |
| Section gap | 12px | `gap-3` |
| Compact row gap | 8px | `gap-2` |
| Section container padding | 10px | `p-2.5` |
| Dialog padding | 16px horizontal, 12px bottom | `px-4 pb-3` |
| Between form fields | 12px | `space-y-3` |

### Responsive Flex Layout
```css
/* Two-column fields that stack on narrow viewports */
display: flex;
flex-wrap: wrap;
gap: 12px;

/* Each field */
flex: 1 1 45%;
min-width: 200px;
```

## Border Radius Scale

| Element | Radius | Tailwind |
|---------|--------|----------|
| Buttons | 8px | `rounded-lg` |
| Inputs | 10px | `rounded-[10px]` |
| Cards | 12px | `rounded-xl` |
| Panels/Dialogs | 14px | `rounded-[14px]` |
| Pills/Badges | 8px | `rounded-lg` |

## Gradient Patterns

### Primary Button Gradient
```css
background: linear-gradient(135deg, var(--eso-gold), var(--eso-gold-hover));
box-shadow: 0 4px 15px rgba(196, 164, 74, 0.25);
```

### Dialog Background Gradient
```css
background: linear-gradient(135deg, rgba(15, 23, 42, 0.97), rgba(30, 41, 59, 0.97));
```

### Paper/Card Background Gradient
```css
background: linear-gradient(180deg, rgba(15, 23, 42, 0.66) 0%, rgba(3, 7, 18, 0.66) 100%);
```

### Accordion/Expandable Section Gradient
```css
background: linear-gradient(135deg,
  rgb(110 170 240 / 25%) 0%,
  rgb(152 131 227 / 15%) 50%,
  rgb(173 192 255 / 8%) 100%);
```
