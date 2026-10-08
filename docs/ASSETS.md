# UI assets

The interface bundles its font and selected icons locally. No font or icon CDN
requests are needed, and no frontend framework or package install is required.

- Instrument Sans variable font: Google Fonts / Instrument, SIL Open Font
  License 1.1. See `public/instrument-sans-OFL.txt`.
  Source: https://github.com/google/fonts/tree/main/ofl/instrumentsans
- Selected stroke icons: `@hugeicons/core-free-icons` 4.3.5, MIT license.
  See `public/hugeicons-LICENSE.md`.
  Source: https://github.com/hugeicons/hugeicons

The font is embedded in the Rust executable and served at `/instrument-sans.ttf`.
Icons are inline SVG in `public/index.html`. Preserve the third-party license
files when sharing or redistributing this project.
