#!/usr/bin/env bash
# Render SVG assets to PNG. Requires ImageMagick 7+ (magick).
# Run from repo root: ./assets/render-pngs.sh
set -e
cd "$(dirname "$0")"

echo "Rendering logo.svg -> icon.png (512×512)..."
sed 's/var(--fg-body, currentColor)/#0A0A0A/g' logo.svg \
  | magick -background none -density 1200 svg:- -resize 512x512 icon.png

echo "Rendering social-preview.svg -> social-preview.png (1280×640)..."
magick -density 300 social-preview.svg -resize 1280x640 social-preview.png

echo "Done. icon.png and social-preview.png updated."
