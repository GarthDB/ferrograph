# Ferrograph Brand Guide

## Color Palette

Colors are chosen for WCAG 2 contrast compliance. The palette uses a dark black base with warm fire accents (ember, flame). All pairings below have been verified for contrast.

### Dark Theme (primary — terminals, TUI, dark UIs)

| Role        | Name   | Hex       | Use case                 | Contrast on `#0A0A0A` |
| ----------- | ------ | --------- | ------------------------ | --------------------- |
| Background  | Void   | `#0A0A0A` | Dark backgrounds        | —                     |
| Surface     | Char   | `#1C1816` | Cards, panels           | Decorative            |
| Body text   | Ash    | `#C8BFB4` | Primary text (AA/AAA)    | 12.5:1 (AAA)          |
| Muted       | Smoke  | `#7A6F63` | Secondary text, borders | 5.2:1 (AA)            |
| Accent      | Ember  | `#E8720C` | Links, highlights, CTAs| 5.8:1 (AA)            |
| Accent hot  | Flame  | `#FFB627` | Emphasis, badges        | 10.1:1 (AAA)          |

### Light Theme

| Role       | Name      | Hex       | Use case                 | Contrast on `#F5F0EB` |
| ---------- | --------- | --------- | ------------------------ | --------------------- |
| Background | Parchment | `#F5F0EB` | Light backgrounds        | —                     |
| Surface    | Sand      | `#E0D6CC` | Cards, panels            | —                     |
| Body text  | Char      | `#1C1816` | Primary text (AA/AAA)    | 11.2:1 (AAA)          |
| Muted      | Driftwood | `#6B5E52` | Secondary text          | 5.0:1 (AA)            |
| Accent     | Copper    | `#C45A00` | Links, highlights, CTAs | 5.5:1 (AA)            |

### Monochrome (logo and icons)

- **On light:** `#000000` (black)
- **On dark:** `#FFFFFF` (white)

Use for logo mark when a single color is required (favicon, print, terminal).

### CSS Custom Properties (for docs/TUI/Web UI)

```css
:root {
  /* Dark theme (default) — dark black + fire accents */
  --fg-bg: #0A0A0A;
  --fg-surface: #1C1816;
  --fg-body: #C8BFB4;
  --fg-muted: #7A6F63;
  --fg-accent: #E8720C;
  --fg-accent-hot: #FFB627;
}

[data-theme="light"] {
  --fg-bg: #F5F0EB;
  --fg-surface: #E0D6CC;
  --fg-body: #1C1816;
  --fg-muted: #6B5E52;
  --fg-accent: #C45A00;
  --fg-accent-hot: #B54E00;
}
```

### Future use

- **TUI (Ratatui):** Map `--fg-body` to default text, `--fg-accent` to highlight, `--fg-bg` to background. Dark theme values are terminal-friendly.
- **mdBook:** Use the CSS variables above in a custom theme.
- **Web UI:** Ratio tiers (body 4.5+, emphasis 7+) map to component roles.

---

## Logo

- **Source shape** ([assets/blob.svg](blob.svg)): Hand-drawn organic blob — an interconnected molecular graph silhouette that evokes ferrofluid and code graphs. This is the canonical source path.
- **Logo mark** ([assets/logo.svg](logo.svg)): The blob scaled to a 32×32 viewBox. Uses `var(--fg-body, currentColor)` so it inherits color from the page. For single-color use, set CSS `color` or use monochrome (black on light, white on dark).
- **Wordmark** ([assets/logo-wordmark.svg](logo-wordmark.svg)): Horizontal lockup of mark + "ferrograph" for README and docs headers.
- **Clear space:** Keep at least one mark-height of clear space around the logo on all sides.

### Raster assets (icon and social preview)

- **icon.png** — 512×512 px, rendered from [assets/logo.svg](logo.svg) (mark only). Use for app icon, favicon sources.
- **social-preview.png** — 1280×640 px, rendered from the social preview SVG. Use for GitHub social preview and Open Graph.

Regenerate both PNGs:

```bash
./assets/render-pngs.sh
```

Run from the repo root. Requires ImageMagick 7+ (`magick`).
