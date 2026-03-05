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

- **Logo mark** ([assets/logo.svg](logo.svg)): Metaball-style graph mark — circles (nodes) connected by smooth bezier “membranes” so they read as one organic blob. Asymmetric layout; evokes ferrofluid and a code graph. Use on light or dark backgrounds; ensure sufficient contrast. For single-color use, set CSS `color` or use monochrome (black on light, white on dark).
- **Wordmark** ([assets/logo-wordmark.svg](logo-wordmark.svg)): Horizontal lockup of mark + “ferrograph” for README and docs headers.
- **Clear space:** Keep at least one mark-height of clear space around the logo on all sides.
- **Variants:** Override `--fg-body` for the mark; omit for monochrome (inherits `currentColor`).

### Logo generator and animation

The mark is generated programmatically so the layout can be tuned and, in the future, animated.

- **Static logo (bezier bridges):** The Rust tool in [assets/gen-logo/](gen-logo/) outputs clean SVG paths. It places 9 nodes in a 3-cluster graph (3 anchors + 6 satellites) with 12 edges, then draws cubic bezier bridges between connected circles (Varun Vachhar / Paper.js–style algorithm). Use this for favicon, print, and README.

  Regenerate the logo:
  ```bash
  cd assets/gen-logo && cargo run > ../logo.svg
  ```
  Then refresh the embedded mark in `logo-wordmark.svg` (copy the `<g id="mark">` contents from `logo.svg` into the wordmark so it stays self-contained).

- **Animated demo (WebGL2 ferrofluid):** [assets/metaball-demo.html](metaball-demo.html) uses the same 3-cluster topology but renders with a WebGL2 3-pass pipeline: metaball field → blur heightmap → ferrofluid shading (directional light, specular, normal-derived from silhouette). Interactive sliders control lighting (Light X/Y/Z, diffuse, spec power/intensity, normal scale) and shape (separation, big/small blob min–max). Use this as a reference for the liquid mercury / ferrofluid aesthetic in web or docs.
