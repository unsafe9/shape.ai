# Renderer Font Assets

These fonts make the Rust/WASM text path explicit and reproducible. The renderer
does not read operating-system fonts.

- Source family: Noto Sans KR Regular.
- Source CSS: `https://fonts.googleapis.com/css2?family=Noto+Sans+KR:wght@400&display=swap`
- Source TTF: `https://fonts.gstatic.com/s/notosanskr/v39/PbyxFmXiEBPT4ITbgNA5Cgms3VYcOA-vvnIzzuoyeLQ.ttf`
- License: SIL Open Font License 1.1, as published for Noto fonts by Google Fonts.
- Local notice: `OFL-1.1-NotoSansKR.txt` includes the OFL text and the copyright
  retained in the generated subset font name tables.

Generated subsets:

```sh
pyftsubset NotoSansKR-Regular.ttf \
  --output-file=NotoSansKR-RendererLatin.ttf \
  --unicodes=U+0020-007E,U+00A0-00FF,U+2000-206F,U+2190-21FF \
  --layout-features='*' --glyph-names --symbol-cmap --legacy-cmap \
  --notdef-glyph --notdef-outline --recommended-glyphs \
  --name-IDs='*' --name-legacy --name-languages='*'

pyftsubset NotoSansKR-Regular.ttf \
  --output-file=NotoSansKR-RendererKorean.ttf \
  --unicodes=U+1100-11FF,U+3130-318F,U+AC00-D7A3,U+3000-303F,U+FF00-FFEF,U+3040-30FF,U+4E00,U+4E2D,U+6587,U+65E5,U+672C,U+8A9E,U+6F22,U+5B57,U+2026 \
  --layout-features='*' --glyph-names --symbol-cmap --legacy-cmap \
  --notdef-glyph --notdef-outline --recommended-glyphs \
  --name-IDs='*' --name-legacy --name-languages='*'
```
