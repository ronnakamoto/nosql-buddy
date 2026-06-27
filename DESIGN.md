# Design System

## Color Strategy

**Restrained.** Tinted cool-gray neutrals + a single blue accent (oklch 256 hue). Accent covers <=10% of surface: primary actions, current selection, active tab underline, semantic indicators. Everything else is neutral.

## Color Tokens

All colors in OKLCH via `theme.css`.

| Token | Light | Dark | Role |
|---|---|---|---|
| `--bg` | 0.99 L, 0.002 C, 240 H | 0.165 L, 0.005 C, 248 H | App background |
| `--surface` | 0.97 L, 0.003 C, 240 H | 0.195 L, 0.006 C, 248 H | Cards, sidebar, toolbar |
| `--surface-2` | 0.95 L, 0.004 C, 240 H | 0.225 L, 0.007 C, 248 H | Hover, raised surface |
| `--surface-3` | 0.93 L, 0.005 C, 240 H | 0.255 L, 0.008 C, 248 H | Pressed, recessed |
| `--border` | 0.90 L, 0.005 C, 240 H | 0.275 L, 0.008 C, 248 H | Default hairline borders |
| `--border-strong` | 0.82 L, 0.006 C, 240 H | 0.36 L, 0.012 C, 248 H | Focus, emphasis |
| `--ink` | 0.22 L, 0.012 C, 240 H | 0.95 L, 0.004 C, 248 H | Body text |
| `--ink-muted` | 0.46 L, 0.012 C, 240 H | 0.72 L, 0.01 C, 248 H | Secondary text |
| `--ink-faint` | 0.60 L, 0.012 C, 240 H | 0.56 L, 0.012 C, 248 H | Tertiary, labels |
| `--accent-500` | 0.62 L, 0.175 C, 256 H | Primary actions, selection |
| `--accent-600` | 0.56 L, 0.178 C, 256 H | Hover on primary |
| `--accent-700` | 0.48 L, 0.16 C, 256 H | Active on primary |
| `--accent-100` | 0.93 L, 0.04 C, 256 H | 0.32 L, 0.07 C, 256 H | Tinted background |
| `--accent-fg` | 0.99 L, 0.005 C, 256 H | Filled accent foreground |
| `--highlight` | 1 L, 0 C, alpha 0.10 | 1 L, 0 C, alpha 0.06 | Subtle top-edge sheen on filled controls |
| `--success-500` | 0.66 L, 0.16 C, 152 H | Success states |
| `--warning-500` | 0.78 L, 0.15 C, 70 H | Warning states |
| `--danger-500` | 0.62 L, 0.21 C, 27 H | Destructive actions |
| `--info-500` | 0.70 L, 0.13 C, 230 H | Information |

## Typography

**Two families.** IBM Plex Sans Variable for UI (Zed Plex lineage), IBM Plex Mono for code/data. Fonts are bundled through `@fontsource`, not loaded from the network.

| Token | Value |
|---|---|
| `--font-sans` | IBM Plex Sans Variable, IBM Plex Sans, -apple-system, BlinkMacSystemFont, Segoe UI, Helvetica Neue, system-ui, sans-serif |
| `--font-mono` | IBM Plex Mono, ui-monospace, SF Mono, Menlo, Cascadia Mono, Consolas, monospace |

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

**Letter spacing:** -0.006em body (IBM Plex optical correction), 0 monospace.

## Spacing

4px base scale: 0, 4, 8, 12, 16, 20, 24, 32px (`--space-0` through `--space-8`).

## Radii

- `--radius-sm`: 4px (inputs, small elements)
- `--radius-md`: 6px (buttons, dropdowns)
- `--radius-lg`: 9px (cards, modals)

## Shadows

- `--shadow-xs`: Cards at rest
- `--shadow-sm`: Hover lift
- `--shadow-md`: Dropdowns, popovers

## Z-Index Scale

`--z-base` (0), `--z-elevated` (10), `--z-sticky` (20), `--z-dropdown` (30), `--z-modal-backdrop` (40), `--z-modal` (50), `--z-toast` (60), `--z-tooltip` (70).

## Component Vocabulary

- **Buttons**: `.btn--primary`, `.btn--ghost`, `.btn--danger`, `.btn--sm`. Primary buttons use the 256 hue accent, `--accent-fg`, and a restrained `--highlight` top sheen. Audit components use `<Button>` from `AuditUi.tsx`.
- **Cards**: `.pane` for main workspace panels. Audit components use `<Card>` from `AuditUi.tsx`.
- **Modals**: `.modal` with backdrop blur. Close on Escape or backdrop click.
- **Forms**: `.field__label`, `.field__input`, `.field__textarea` with accent border and ring on focus.
- **Tables**: `.results-grid` sticky header, monospace cells, hover row highlight.
- **Toasts**: Fixed bottom-right, auto-dismiss, semantic colors.
- **Badges**: `.kind-badge` for BSON types, `.tab__kind` for tab labels.
- **Keycaps**: `.kbd` for compact keyboard hints and version pills, styled as flat Zed-like neutral pills.

## Motion

150ms ease-out for hover/state transitions. 200ms for progress bars. No orchestrated load sequences. Respects `prefers-reduced-motion`.

## Icons

Text symbols (unicode geometric shapes): not ideal. TODO: replace with proper SVG icon set for consistency.

## Known Issues

1. Audit components (`AuditUi.tsx`) use extensive inline styles instead of CSS classes, diverging from the main app's CSS-first approach.
2. Symbol-based icons (``, ``, ``) lack visual consistency compared to SVG icons.

## Recently Resolved

1. IBM Plex Sans/Mono are bundled via `@fontsource`, so the desktop app no longer depends on Google Fonts or any network font request.
2. `--font-mono` is consolidated in `theme.css`; `styles.css` now consumes the token instead of redefining it.
3. Accent surfaces are unified app-wide: `--accent-400` is a real token in `theme.css` and `--accent-bg` maps to `--selection` in `styles.css`, so the Shell autocomplete and usage bars no longer fall back to a stray indigo. Audit primary buttons (`.audit-btn--primary`, `.audit-proof-card__primary-link`) now use the shared Zed primary signature (`--accent-500` fill, `--accent-fg` text, top-edge `--highlight` sheen), and every `color: white`/`#fff` on a filled accent surface is replaced with `--accent-fg` for correct light/dark behavior.
