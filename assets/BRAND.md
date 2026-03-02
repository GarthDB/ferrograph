# Ferrograph Brand Guide

## Color Palette

Colors are chosen for WCAG 2 contrast compliance. All pairings below have been verified with the Leonardo contrast checker.

### Dark Theme (primary — terminals, TUI, dark UIs)

| Role       | Name     | Hex       | Use case                    | Contrast on `#0F1419` |
| ---------- | -------- | --------- | --------------------------- | --------------------- |
| Background | Obsidian | `#0F1419` | Dark backgrounds           | —                     |
| Surface    | Graphite | `#2D3748` | Cards, panels (1.54:1)     | Decorative            |
| Body text  | Steel    | `#9CA8B8` | Primary text (AA/AAA)      | 7.68:1 (AAA)          |
| Muted      | Slate    | `#6B7B8D` | Secondary text, borders    | 4.27:1 (large text)   |
| Accent     | Cobalt   | `#5B8FC6` | Links, highlights, CTAs    | 5.46:1 (AA)           |

### Light Theme

| Role       | Name     | Hex       | Use case                    | Contrast on `#F0F2F5` |
| ---------- | -------- | --------- | --------------------------- | --------------------- |
| Background | Frost    | `#F0F2F5` | Light backgrounds          | —                     |
| Surface    | Silver   | `#C8CED6` | Cards, panels               | —                     |
| Body text  | Graphite | `#2D3748` | Primary text (AA/AAA)       | 10.69:1 (AAA)         |
| Muted      | Steel    | `#5C6B7D` | Secondary text              | 4.86:1 (AA)           |
| Accent     | Cobalt   | `#3D6B99` | Links, highlights, CTAs    | 4.98:1 (AA)           |

### Monochrome (logo and icons)

- **On light:** `#000000` (black)
- **On dark:** `#FFFFFF` (white)

Use for logo mark when a single color is required (favicon, print, terminal).

### CSS Custom Properties (for docs/TUI/Web UI)

```css
:root {
  /* Dark theme (default) */
  --fg-bg: #0F1419;
  --fg-surface: #2D3748;
  --fg-body: #9CA8B8;
  --fg-muted: #6B7B8D;
  --fg-accent: #5B8FC6;
}

[data-theme="light"] {
  --fg-bg: #F0F2F5;
  --fg-surface: #C8CED6;
  --fg-body: #2D3748;
  --fg-muted: #5C6B7D;
  --fg-accent: #3D6B99;
}
```

### Future use

- **TUI (Ratatui):** Map `--fg-body` to default text, `--fg-accent` to highlight, `--fg-bg` to background. Dark theme values are terminal-friendly.
- **mdBook:** Use the CSS variables above in a custom theme.
- **Web UI:** Ratio tiers (body 4.5+, emphasis 7+) map to component roles.

---

## Logo

- **Logo mark** ([assets/logo.svg](logo.svg)): Central hub with 8 radiating edges and leaf nodes (ferrofluid/graph motif). Use on light or dark backgrounds; ensure sufficient contrast. For single-color use, set CSS `color` or use monochrome (black on light, white on dark).
- **Wordmark** ([assets/logo-wordmark.svg](logo-wordmark.svg)): Horizontal lockup of mark + "ferrograph" for README and docs headers.
- **Clear space:** Keep at least one mark-height of clear space around the logo on all sides.
- **Variants:** Override `--fg-steel` and `--fg-accent` for the color variant; omit for monochrome (inherits `currentColor`).
