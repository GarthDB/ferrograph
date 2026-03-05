#!/usr/bin/env bash
# Render SVG assets to PNG. Requires Node (npx). Run from repo root: ./assets/render-pngs.sh
set -e
cd "$(dirname "$0")"

echo "Rendering logo.svg -> icon.png (512×512)..."
npx --yes @resvg/resvg-js-cli logo.svg icon.png --fit-width 512

echo "Rendering social-preview.svg -> social-preview.png (1280×640)..."
npx --yes @resvg/resvg-js-cli social-preview.svg social-preview.png --fit-width 1280

echo "Done. icon.png and social-preview.png updated."
