# Design System

## Color Strategy

**Restrained.** Tinted cool-gray neutrals + a single blue accent (oklch 252 hue). Accent covers <=10% of surface: primary actions, current selection, active tab underline, semantic indicators. Everything else is neutral.

## Color Tokens

All colors in OKLCH via `theme.css`.

| Token | Light | Dark | Role |
|---|---|---|---|
| `--bg` | 0.99 L, 0.002 C | 0.18 L | App background |
| `--surface` | 0.97 L | 0.21 L | Cards, sidebar, toolbar |
| `--surface-2` | 0.95 L | 0.24 L | Hover, raised surface |
| `--surface-3` | 0.93 L | 0.27 L | Pressed, recessed |
| `--border` | 0.90 L | 0.30 L | Default borders |
| `--border-strong` | 0.82 L | 0.40 L | Focus, emphasis |
| `--ink` | 0.22 L | 0.96 L | Body text |
| `--ink-muted` | 0.46 L | 0.74 L | Secondary text |
| `--ink-faint` | 0.60 L | 0.58 L | Tertiary, labels |
| `--accent-500` | 0.60 L, 0.18 C, 252 H | Primary actions, selection |
| `--accent-600` | 0.54 L | Hover on primary |
| `--accent-700` | 0.46 L | Active on primary |
| `--accent-100` | 0.93 L, 0.04 C | Tinted background |
| `--success-500` | 0.66 L, 0.16 C, 152 H | Success states |
| `--warning-500` | 0.78 L, 0.15 C, 70 H | Warning states |
| `--danger-500` | 0.62 L, 0.21 C, 27 H | Destructive actions |
| `--info-500` | 0.70 L, 0.13 C, 230 H | Information |

## Typography

**Two families.** Inter for UI (sans), JetBrains Mono for code/data (monospace).

| Token | Value |
|---|---|
| `--font-sans` | Inter, -apple-system, BlinkMacSystemFont, system-ui, sans-serif |
| `--font-mono` | JetBrains Mono, ui-monospace, SF Mono, Menlo, monospace |

**Scale (fixed rem, not fluid):**

| Token | Size |
|---|---|
| `--font-size-xs` | 0.75rem (12px) |
| `--font-size-sm` | 0.8125rem (13px) |
| `--font-size-md` | 0.875rem (14px) |
| `--font-size-lg` | 0.9375rem (15px) |
| `--font-size-xl` | 1rem (16px) |
| `--font-size-2xl` | 1.125rem (18px) |
| `--font-size-3xl` | 1.375rem (22px) |

**Line heights:** tight 1.3, normal 1.5, loose 1.65.

**Letter spacing:** -0.011em body (Inter optical correction), -0.003em monospace.

## Spacing

4px base scale: 0, 4, 8, 12, 16, 20, 24, 32px (`--space-0` through `--space-8`).

## Radii

- `--radius-sm`: 4px (inputs, small elements)
- `--radius-md`: 6px (buttons, dropdowns)
- `--radius-lg`: 10px (cards, modals)

## Shadows

- `--shadow-xs`: Cards at rest
- `--shadow-sm`: Hover lift
- `--shadow-md`: Dropdowns, popovers

## Z-Index Scale

`--z-base` (0), `--z-elevated` (10), `--z-sticky` (20), `--z-dropdown` (30), `--z-modal-backdrop` (40), `--z-modal` (50), `--z-toast` (60), `--z-tooltip` (70).

## Component Vocabulary

- **Buttons**: `.btn--primary`, `.btn--ghost`, `.btn--danger`, `.btn--sm`. Audit components use `<Button>` from `AuditUi.tsx`.
- **Cards**: `.pane` for main workspace panels. Audit components use `<Card>` from `AuditUi.tsx`.
- **Modals**: `.modal` with backdrop blur. Close on Escape or backdrop click.
- **Forms**: `.field__label`, `.field__input`, `.field__textarea` with focus border-color change.
- **Tables**: `.results-grid` sticky header, monospace cells, hover row highlight.
- **Toasts**: Fixed bottom-right, auto-dismiss, semantic colors.
- **Badges**: `.kind-badge` for BSON types, `.tab__kind` for tab labels.

## Motion

150ms ease-out for hover/state transitions. 200ms for progress bars. No orchestrated load sequences. Respects `prefers-reduced-motion`.

## Icons

Text symbols (unicode geometric shapes): not ideal. TODO: replace with proper SVG icon set for consistency.

## Known Issues

1. `--font-mono` is redefined in `styles.css` `:root` block, overriding `theme.css`. Must consolidate.
2. Audit components (`AuditUi.tsx`) use extensive inline styles instead of CSS classes, diverging from the main app's CSS-first approach.
3. Symbol-based icons (``, ``, ``) lack visual consistency compared to SVG icons.
4. Google Fonts `@import` in CSS requires network; should bundle via `@fontsource` for offline desktop use.
