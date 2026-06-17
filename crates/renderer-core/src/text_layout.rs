//! Text run layout against a derived region. Pure CPU: takes a `measure` closure
//! returning per-char advance width and emits absolute glyph placements, decoupled
//! from the raster path (`text.rs`) so it is unit-testable with a stub and usable
//! against the MSDF atlas described by [`MsdfAtlasPlan`].

/// Horizontal alignment of laid-out lines within the region width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextAlign {
    Start,
    Center,
    End,
    /// Stretch inter-word gaps so each non-final line fills the region width.
    Justify,
}

/// Vertical alignment of the laid-out block within the region height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextVAlign {
    Top,
    Middle,
    Bottom,
}

/// A single styled run (local mirror of the OB-3 `Text.runs[]` entry).
#[derive(Clone, Debug)]
pub struct TextRunInput {
    pub text: String,
    pub color: [f32; 4],
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub font: String,
}

/// One placed glyph in region-local px. `x`/`y` is the glyph pen origin (top-left
/// of the line cell; the shader applies its own per-glyph bearing). `run_index`
/// indexes back into `runs` for paint/style.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphPlacement {
    pub ch: char,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub color: [f32; 4],
    pub run_index: usize,
}

/// An atomic flow token carrying the originating run so style follows the glyph.
struct Token {
    ch: char,
    advance: f32,
    size: f32,
    color: [f32; 4],
    run_index: usize,
    whitespace: bool,
}

struct Line {
    tokens: Vec<Token>,
    /// Width up to and including the last non-whitespace token.
    visible_width: f32,
    /// Sum of visible (non-whitespace) advances. Justify distributes
    /// `region_width - glyph_width` across the inter-word gaps.
    glyph_width: f32,
    /// Inter-word whitespace tokens eligible for justify stretch (strictly between
    /// two visible tokens).
    justify_gaps: usize,
    /// Line height: the max run size on the line (else block default).
    height: f32,
}

/// Flow `runs` into lines wrapped to the region width, then align horizontally and
/// vertically. Line height is the tallest run on each line. `measure` returns the
/// advance width of `ch` at a given pixel size. `line_box` is the font's vertical
/// extent per logical px — `(ascent, descent)` at size 1 (descent ≤ 0, fontdue's
/// sign) — so vertical centering aligns the true ascent..descent box rather than the
/// bare line height; the per-glyph shader bearing pins that box's top at `pen_y`.
pub fn layout_runs(
    runs: &[TextRunInput],
    region_min: (f32, f32),
    region_max: (f32, f32),
    align: TextAlign,
    valign: TextVAlign,
    line_box: (f32, f32),
    measure: &dyn Fn(char, f32) -> f32,
) -> Vec<GlyphPlacement> {
    let region_width = (region_max.0 - region_min.0).max(0.0);
    let region_height = (region_max.1 - region_min.1).max(0.0);

    let lines = flow_lines(runs, region_width, measure);
    if lines.is_empty() {
        return Vec::new();
    }

    // Each line advances the pen by `line.height` (line top -> next line top), but the
    // glyph shader places ink so the metric box top sits at `pen_y` and its bottom at
    // `pen_y + box_height(line)`. The block's optical extent therefore runs from the
    // first line top through the last line's box bottom, not a bare line-height sum.
    let (ascent_px, descent_px) = line_box;
    let box_ratio = (ascent_px - descent_px).max(0.0);
    let box_height = |line: &Line| line.height * box_ratio;
    let advance_height: f32 = lines.iter().map(|line| line.height).sum();
    let last_box = lines.last().map(box_height).unwrap_or(0.0);
    let last_advance = lines.last().map(|line| line.height).unwrap_or(0.0);
    let block_height = advance_height - last_advance + last_box;
    let mut pen_y = match valign {
        TextVAlign::Top => region_min.1,
        TextVAlign::Middle => region_min.1 + (region_height - block_height) * 0.5,
        TextVAlign::Bottom => region_max.1 - block_height,
    };

    let mut placements = Vec::new();
    let last_index = lines.len() - 1;
    for (line_index, line) in lines.iter().enumerate() {
        let slack = (region_width - line.visible_width).max(0.0);
        let (mut pen_x, justify_extra) = match align {
            TextAlign::Start => (region_min.0, 0.0),
            TextAlign::Center => (region_min.0 + slack * 0.5, 0.0),
            TextAlign::End => (region_min.0 + slack, 0.0),
            TextAlign::Justify => {
                // The final line (and any line with no inter-word gaps) stays
                // start-aligned. Slack is measured against the visible glyph width,
                // so a gap stretches to natural_advance + justify_extra.
                let justify_slack = (region_width - line.glyph_width).max(0.0);
                let stretch = if line_index != last_index && line.justify_gaps > 0 {
                    justify_slack / line.justify_gaps as f32
                } else {
                    0.0
                };
                (region_min.0, stretch)
            }
        };

        let visible_count = line
            .tokens
            .iter()
            .filter(|token| !token.whitespace)
            .count();
        let mut emitted_visible = 0;
        for token in &line.tokens {
            if token.whitespace {
                pen_x += token.advance;
                // Only stretch gaps that sit between two visible tokens.
                if emitted_visible > 0 && emitted_visible < visible_count {
                    pen_x += justify_extra;
                }
                continue;
            }
            placements.push(GlyphPlacement {
                ch: token.ch,
                x: pen_x,
                y: pen_y,
                size: token.size,
                color: token.color,
                run_index: token.run_index,
            });
            pen_x += token.advance;
            emitted_visible += 1;
        }
        pen_y += line.height;
    }
    placements
}

/// Default line height when a run carries a non-positive size.
const DEFAULT_LINE_HEIGHT: f32 = 16.0;

fn flow_lines(
    runs: &[TextRunInput],
    region_width: f32,
    measure: &dyn Fn(char, f32) -> f32,
) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut current: Vec<Token> = Vec::new();
    // Pending word: tokens since the last break opportunity, flushed together so
    // words wrap atomically.
    let mut word: Vec<Token> = Vec::new();
    let mut line_width = 0.0_f32; // width of `current` including trailing ws
    let mut word_width = 0.0_f32;

    let flush_line =
        |lines: &mut Vec<Line>, tokens: Vec<Token>, runs: &[TextRunInput]| {
            lines.push(finalize_line(tokens, runs));
        };

    for (run_index, run) in runs.iter().enumerate() {
        let size = if run.size > 0.0 {
            run.size
        } else {
            DEFAULT_LINE_HEIGHT
        };
        for ch in run.text.chars() {
            if ch == '\n' {
                // Commit any pending word, then hard-break.
                current.append(&mut word);
                word_width = 0.0;
                flush_line(&mut lines, std::mem::take(&mut current), runs);
                line_width = 0.0;
                continue;
            }

            let advance = measure(ch, size);
            let whitespace = ch.is_whitespace();
            let token = Token {
                ch,
                advance,
                size,
                color: run.color,
                run_index,
                whitespace,
            };

            if whitespace {
                // A break opportunity: commit the pending word to the line.
                line_width += word_width;
                current.append(&mut word);
                word_width = 0.0;

                // Trailing whitespace stays on the line; it is excluded from
                // visible width during finalize and may push later words to
                // wrap, but a lone space never forces its own wrap.
                current.push(token);
                line_width += advance;
                continue;
            }

            // Would appending this char to the current word overflow the line?
            if !current.is_empty()
                && line_width + word_width + advance > region_width
                && region_width > 0.0
            {
                // The word does not fit after existing content: wrap before it.
                flush_line(&mut lines, std::mem::take(&mut current), runs);
                line_width = 0.0;
                // The pending word moves to the fresh line untouched.
            }

            // The word itself overflows an empty line: hard char-wrap so a
            // single long token never exceeds the region.
            if current.is_empty()
                && word_width + advance > region_width
                && region_width > 0.0
                && !word.is_empty()
            {
                flush_line(&mut lines, std::mem::take(&mut word), runs);
                word_width = 0.0;
            }

            word.push(token);
            word_width += advance;
        }
    }

    // Drain the trailing word and line.
    current.append(&mut word);
    if !current.is_empty() {
        flush_line(&mut lines, current, runs);
    }
    lines
}

fn finalize_line(tokens: Vec<Token>, runs: &[TextRunInput]) -> Line {
    let mut visible_width = 0.0_f32;
    let mut glyph_width = 0.0_f32;
    let mut running = 0.0_f32;
    let mut justify_gaps = 0;
    let mut seen_visible = false;
    let mut pending_ws = 0;
    let mut height = 0.0_f32;

    for token in &tokens {
        height = height.max(line_height_for(token, runs));
        running += token.advance;
        if token.whitespace {
            if seen_visible {
                pending_ws += 1;
            }
        } else {
            // This visible token closes any whitespace run that followed a
            // prior visible token: those gaps are interior and justifiable.
            if seen_visible {
                justify_gaps += pending_ws;
            }
            pending_ws = 0;
            seen_visible = true;
            visible_width = running;
            glyph_width += token.advance;
        }
    }
    if height <= 0.0 {
        height = DEFAULT_LINE_HEIGHT;
    }
    Line {
        tokens,
        visible_width,
        glyph_width,
        justify_gaps,
        height,
    }
}

fn line_height_for(token: &Token, runs: &[TextRunInput]) -> f32 {
    let run_size = runs.get(token.run_index).map(|run| run.size).unwrap_or(0.0);
    if run_size > 0.0 {
        run_size
    } else {
        token.size
    }
}

// SDF atlas generator. World/canvas text bakes a TRUE multi-channel MSDF from the
// glyph's vector outline (via `fdsm`), so corners stay sharp at any zoom — the three
// colored channels disagree near a corner and the shader's `median3` reconstructs it
// crisply where a single isotropic SDF rounds it off. When no outline is threaded
// (the host couldn't extract one for this glyph) the generator degrades per-glyph to
// the legacy single-channel field built from the fontdue coverage raster (the 8-point
// Signed Sequential Euclidean Distance Transform replicated into R/G/B), so a font
// table the outliner can't reach still renders. Both paths are pure and deterministic
// — fdsm's edge coloring takes a fixed seed — so the same input yields byte-identical
// atlas pixels.

/// The fixed edge-coloring seed: the pure core forbids randomness, so a hardcoded
/// constant keeps the MSDF byte-deterministic across runs (same outline -> same
/// atlas pixels). Any value works; this is fdsm's own README seed.
const MSDF_EDGE_SEED: u64 = 69441337420;

/// `sin` of the corner angle past which fdsm's edge coloring switches channels.
/// fdsm's documented default; smaller keeps near-straight joins on one channel.
const MSDF_CORNER_SIN: f64 = 0.03;

/// Identifies a rasterized MSDF glyph in the atlas. `px` is the quantized SDF cell
/// size (resolution-independent, but a fixed cell keeps the atlas predictable).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MsdfGlyphKey {
    pub font_index: usize,
    pub glyph_id: u16,
    pub px: u16,
    /// A coverage slot and an SDF slot for the same glyph store different texels, so
    /// the mode is part of the key — the two never collide in one shared atlas.
    pub coverage: bool,
}

/// One glyph's atlas slot: four corner UVs (CCW from top-left) plus the bearing/size
/// metrics the shader applies to position the quad relative to the pen origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MsdfGlyphEntry {
    /// Corner UVs: [top-left, top-right, bottom-right, bottom-left].
    pub uv: [[f32; 2]; 4],
    /// Bearing from pen origin to glyph left (px at `px` cell size).
    pub bearing_x: f32,
    /// Bearing from line top to glyph top (px at `px` cell size).
    pub bearing_y: f32,
    pub width: f32,
    pub height: f32,
    /// True when the slot holds a raw coverage raster (sampled directly as alpha)
    /// rather than an SDF; the quad threads this to the per-vertex shader mode flag.
    pub coverage: bool,
}

/// A glyph's fontdue coverage raster + placement metrics, the generator input.
/// `coverage` is row-major 8-bit alpha, `width`×`height` texels. When the host
/// oversamples (rasterizes at `font_size * oversample` px so the SDF source grid is
/// finer than the on-screen glyph — sharp retina text), `coverage`/`width`/`height`
/// and the bearings are in those high-res texels; `oversample` rescales the emitted
/// quad geometry back to logical px so layout stays resolution-independent. `1.0`
/// means no oversample (texels == logical px).
#[derive(Clone, Debug)]
pub struct GlyphCoverage<'a> {
    pub key: MsdfGlyphKey,
    pub coverage: &'a [u8],
    pub width: usize,
    pub height: usize,
    /// Bearing from pen origin to glyph left (fontdue `xmin`).
    pub bearing_x: f32,
    pub bearing_y: f32,
    /// Raster-texels-per-logical-px (≥ 1.0); divides the emitted entry geometry.
    pub oversample: f32,
    /// The glyph's vector outline, when the host extracted it: the SDF path bakes a
    /// TRUE multi-channel MSDF from it instead of the coverage-derived single-channel
    /// field. `None` degrades that one glyph to the legacy `coverage_to_sdf` path. The
    /// coverage (UI) path ignores this entirely.
    pub outline: Option<MsdfOutline>,
}

/// A glyph's vector outline plus the font-unit → atlas-cell registration the MSDF
/// baker needs. The host (text.rs) loads the contours from the same ttf-parser face
/// rustybuzz shaped with; the pure core scales them into the SAME padded ink cell the
/// coverage raster occupies so the field registers against the pen-origin bearings.
#[derive(Clone, Debug)]
pub struct MsdfOutline {
    /// Glyph contours in font units, y-up (the font coordinate convention).
    pub shape: fdsm::shape::Shape<fdsm::shape::Contour>,
    /// Font-unit ink bbox, the registration origin/extent the cell maps onto.
    pub x_min: f32,
    pub y_max: f32,
    /// Font units per atlas texel (== `units_per_em / px`); the MSDF transform divides
    /// outline coordinates by this so the ink box fills the same `width`×`height` cell.
    pub shrinkage: f32,
}

/// The shader-facing SDF atlas: dimensions, distance range, glyph->slot mapping, and
/// the generated RGBA distance-field pixels.
#[derive(Clone, Debug)]
pub struct MsdfAtlasPlan {
    pub atlas_width: u32,
    pub atlas_height: u32,
    /// Distance field spread in atlas texels; the shader scales sampled distances by
    /// this for AA. Doubles as the per-cell padding so the field has room to ramp.
    pub distance_range: f32,
    entries: std::collections::HashMap<MsdfGlyphKey, MsdfGlyphEntry>,
    /// RGBA8 texels. The signed distance is replicated into R/G/B; A is the same.
    pixels: Vec<u8>,
    /// Shelf allocator cursor.
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
}

impl MsdfAtlasPlan {
    pub fn new(atlas_width: u32, atlas_height: u32, distance_range: f32) -> Self {
        let width = atlas_width.max(4);
        let height = atlas_height.max(4);
        MsdfAtlasPlan {
            atlas_width: width,
            atlas_height: height,
            distance_range: distance_range.max(1.0),
            entries: std::collections::HashMap::new(),
            // 0 = fully outside, so untouched texels never read as "inside".
            pixels: vec![0_u8; (width * height * 4) as usize],
            cursor_x: 1,
            cursor_y: 1,
            row_height: 0,
        }
    }

    /// Register a pre-built glyph slot directly. Generation uses [`Self::generate_glyph`].
    pub fn insert(&mut self, key: MsdfGlyphKey, entry: MsdfGlyphEntry) {
        self.entries.insert(key, entry);
    }

    /// Generate one glyph's SDF cell, pack it, and register its entry. Idempotent
    /// per key. `None` means the atlas is full (caller falls back to the legacy
    /// raster atlas); a blank glyph maps to a zero-size entry (no quad).
    pub fn generate_glyph(&mut self, glyph: &GlyphCoverage<'_>) -> Option<MsdfGlyphEntry> {
        if let Some(entry) = self.entries.get(&glyph.key) {
            return Some(*entry);
        }
        if glyph.width == 0 || glyph.height == 0 || glyph.coverage.is_empty() {
            let entry = MsdfGlyphEntry {
                uv: [[0.0, 0.0]; 4],
                bearing_x: glyph.bearing_x,
                bearing_y: glyph.bearing_y,
                width: 0.0,
                height: 0.0,
                coverage: glyph.key.coverage,
            };
            self.entries.insert(glyph.key, entry);
            return Some(entry);
        }

        // Pad each cell by `distance_range` texels so the field has room to ramp.
        let pad = crate::cast::round_u32(self.distance_range.ceil().max(1.0));
        let glyph_w = u32::try_from(glyph.width).unwrap_or(u32::MAX);
        let glyph_h = u32::try_from(glyph.height).unwrap_or(u32::MAX);
        let cell_w = glyph_w + pad * 2;
        let cell_h = glyph_h + pad * 2;

        let (origin_x, origin_y) = self.allocate_cell(cell_w, cell_h)?;

        let atlas_w = self.atlas_width;
        if let Some(outline) = glyph.outline.as_ref() {
            // TRUE multi-channel MSDF: bake the three colored distance channels from
            // the vector outline into R/G/B, A = median3 so the slot stays a valid
            // signed field for any reader. Corners survive where the single-channel
            // field blurs them.
            let msdf = outline_to_msdf(outline, cell_w, cell_h, pad, self.distance_range);
            for row in 0..cell_h {
                for col in 0..cell_w {
                    let texel = (row * cell_w + col) as usize * 3;
                    let r = msdf[texel];
                    let g = msdf[texel + 1];
                    let b = msdf[texel + 2];
                    let index = (((origin_y + row) * atlas_w + (origin_x + col)) * 4) as usize;
                    self.pixels[index] = r;
                    self.pixels[index + 1] = g;
                    self.pixels[index + 2] = b;
                    self.pixels[index + 3] = median3_u8(r, g, b);
                }
            }
        } else {
            // No outline for this glyph: degrade to the legacy single-channel field
            // (R=G=B=A), which the shader's median3 returns unchanged.
            let sdf = coverage_to_sdf(
                glyph.coverage,
                glyph.width,
                glyph.height,
                pad as usize,
                self.distance_range,
            );
            for row in 0..cell_h {
                for col in 0..cell_w {
                    let value = sdf[(row * cell_w + col) as usize];
                    #[allow(
                        clippy::cast_possible_truncation,
                        reason = "clamped to [0.0, 255.0] then rounded; the value is an exact integer in u8 range"
                    )]
                    let texel = (value * 255.0).round().clamp(0.0, 255.0) as u8;
                    let index = (((origin_y + row) * atlas_w + (origin_x + col)) * 4) as usize;
                    self.pixels[index] = texel;
                    self.pixels[index + 1] = texel;
                    self.pixels[index + 2] = texel;
                    self.pixels[index + 3] = texel;
                }
            }
        }

        // The quad covers the full padded cell (AA ramp visible); bearings shift
        // left/up by `pad` to keep the glyph's ink registered against the pen origin.
        // The cell/bearings are in high-res texels; dividing by `oversample` emits the
        // quad in logical px so an oversampled (sharper) atlas slot still lays out at
        // the same size — the layout is resolution-independent, only the SDF is finer.
        let aw = self.atlas_width as f32;
        let ah = self.atlas_height as f32;
        let left = origin_x as f32 / aw;
        let right = (origin_x + cell_w) as f32 / aw;
        let top = origin_y as f32 / ah;
        let bottom = (origin_y + cell_h) as f32 / ah;
        let inv_oversample = 1.0 / glyph.oversample.max(1.0);
        let entry = MsdfGlyphEntry {
            uv: [[left, top], [right, top], [right, bottom], [left, bottom]],
            bearing_x: (glyph.bearing_x - pad as f32) * inv_oversample,
            bearing_y: (glyph.bearing_y - pad as f32) * inv_oversample,
            width: cell_w as f32 * inv_oversample,
            height: cell_h as f32 * inv_oversample,
            coverage: false,
        };
        self.entries.insert(glyph.key, entry);
        Some(entry)
    }

    /// Pack one glyph's RAW fontdue coverage (NOT an SDF) into the atlas and register
    /// its entry, for screen-space UI text. The coverage bitmap maps ~1:1 to device
    /// pixels (the host rasterizes at `font_size * dpr`), so the shader samples it
    /// directly as alpha and the glyph stays crisp at its fixed size — the browser-blit
    /// behavior. No distance-range pad, no `coverage_to_sdf`: a 1px guard band only, so
    /// linear sampling never bleeds a neighbor. Idempotent per (coverage-keyed) slot.
    pub fn generate_coverage_glyph(&mut self, glyph: &GlyphCoverage<'_>) -> Option<MsdfGlyphEntry> {
        if let Some(entry) = self.entries.get(&glyph.key) {
            return Some(*entry);
        }
        if glyph.width == 0 || glyph.height == 0 || glyph.coverage.is_empty() {
            let entry = MsdfGlyphEntry {
                uv: [[0.0, 0.0]; 4],
                bearing_x: glyph.bearing_x,
                bearing_y: glyph.bearing_y,
                width: 0.0,
                height: 0.0,
                coverage: true,
            };
            self.entries.insert(glyph.key, entry);
            return Some(entry);
        }

        let glyph_w = u32::try_from(glyph.width).unwrap_or(u32::MAX);
        let glyph_h = u32::try_from(glyph.height).unwrap_or(u32::MAX);
        let (origin_x, origin_y) = self.allocate_cell(glyph_w, glyph_h)?;

        let atlas_w = self.atlas_width;
        for row in 0..glyph_h {
            for col in 0..glyph_w {
                let alpha = glyph.coverage[(row * glyph_w + col) as usize];
                let index = (((origin_y + row) * atlas_w + (origin_x + col)) * 4) as usize;
                self.pixels[index] = alpha;
                self.pixels[index + 1] = alpha;
                self.pixels[index + 2] = alpha;
                self.pixels[index + 3] = alpha;
            }
        }

        // The quad spans exactly the ink cell (no AA pad); bearings register the ink
        // against the pen origin. Cell/bearings are in device texels; dividing by
        // `oversample` (the dpr the host rasterized at) emits the quad in logical px so
        // layout stays resolution-independent while the texels stay device-resolution.
        let aw = self.atlas_width as f32;
        let ah = self.atlas_height as f32;
        let left = origin_x as f32 / aw;
        let right = (origin_x + glyph_w) as f32 / aw;
        let top = origin_y as f32 / ah;
        let bottom = (origin_y + glyph_h) as f32 / ah;
        let inv_oversample = 1.0 / glyph.oversample.max(1.0);
        let entry = MsdfGlyphEntry {
            uv: [[left, top], [right, top], [right, bottom], [left, bottom]],
            bearing_x: glyph.bearing_x * inv_oversample,
            bearing_y: glyph.bearing_y * inv_oversample,
            width: glyph_w as f32 * inv_oversample,
            height: glyph_h as f32 * inv_oversample,
            coverage: true,
        };
        self.entries.insert(glyph.key, entry);
        Some(entry)
    }

    /// Shelf-allocate a `w`×`h` cell; `None` when no row has vertical room left.
    fn allocate_cell(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
        if self.cursor_x + w >= self.atlas_width {
            self.cursor_x = 1;
            self.cursor_y += self.row_height + 1;
            self.row_height = 0;
        }
        if self.cursor_y + h >= self.atlas_height {
            return None;
        }
        let origin = (self.cursor_x, self.cursor_y);
        self.cursor_x += w + 1;
        self.row_height = self.row_height.max(h);
        Some(origin)
    }

    /// Resolve a glyph's atlas slot; `None` means not yet generated.
    pub fn lookup(&self, key: &MsdfGlyphKey) -> Option<&MsdfGlyphEntry> {
        self.entries.get(key)
    }

    pub fn glyph_count(&self) -> usize {
        self.entries.len()
    }

    /// The generated RGBA8 distance-field texels, ready to upload.
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}

/// Bake a TRUE multi-channel MSDF for one padded cell from the glyph's vector
/// outline. Returns `cell_w * cell_h * 3` bytes (R, G, B per texel). The outline is
/// scaled by `1/shrinkage` (font units per texel) and translated so its ink bbox lands
/// at `(pad, pad)` with the Y axis flipped (font space is y-up, the atlas is y-down),
/// filling the same ink region the coverage raster occupies. fdsm's `range` argument
/// is `2*distance_range`, which maps `[-distance_range, +distance_range]` texels onto
/// `[0,1]` with 0.5 on the outline — byte-identical calibration to `coverage_to_sdf`,
/// so the shader's `screenPxRange` (fed `distance_range`) stays untouched. Pure: the
/// edge coloring uses a fixed seed, so the same outline yields identical pixels.
fn outline_to_msdf(
    outline: &MsdfOutline,
    cell_w: u32,
    cell_h: u32,
    pad: u32,
    distance_range: f32,
) -> Vec<u8> {
    use fdsm::generate::generate_msdf;
    use fdsm::render::correct_sign_msdf;
    use fdsm::shape::Shape;
    use fdsm::transform::Transform;
    use image::RgbImage;
    use nalgebra::{Affine2, Matrix3};

    let inv = 1.0 / f64::from(outline.shrinkage);
    let pad = f64::from(pad);
    // Map font-unit (fx, fy) -> cell px: x = pad + (fx - x_min)*inv,
    //                                    y = pad + (y_max - fy)*inv  (Y flip).
    let tx = pad - f64::from(outline.x_min) * inv;
    let ty = pad + f64::from(outline.y_max) * inv;
    let transform = Affine2::from_matrix_unchecked(Matrix3::new(
        inv, 0.0, tx, //
        0.0, -inv, ty, //
        0.0, 0.0, 1.0,
    ));

    let mut shape = outline.shape.clone();
    shape.transform(&transform);
    let colored = Shape::edge_coloring_simple(shape, MSDF_CORNER_SIN, MSDF_EDGE_SEED);
    let prepared = colored.prepare();

    let mut img = RgbImage::new(cell_w, cell_h);
    // fdsm's range maps [-range/2, +range/2] -> [0,1]; 2*distance_range reproduces the
    // ±distance_range -> [0,1] calibration coverage_to_sdf emits.
    let range = 2.0 * f64::from(distance_range);
    generate_msdf(&prepared, range, &mut img);
    correct_sign_msdf(&mut img, &prepared, fdsm::bezier::scanline::FillRule::Nonzero);

    img.into_raw()
}

/// Median of three `u8` channels — the value the shader's `median3(sample.rgb)`
/// reconstructs, stored in A so the slot stays a valid signed field for any reader.
fn median3_u8(r: u8, g: u8, b: u8) -> u8 {
    r.max(g).min(b.max(r.min(g)))
}

/// Build a single-channel signed distance field from a glyph coverage raster.
/// The continuous coverage ramp (not a 1-bit threshold) places the outline at the
/// 0.5 crossing with sub-texel precision: each boundary texel contributes a
/// fractional offset `(0.5 - coverage)/|∇coverage|` along the coverage gradient,
/// which the 8SSEDT dead-reckoning transform carries with the nearest-feature
/// offset so the zero crossing lands between texels instead of snapping to one.
/// Normalized so 0.5 sits on the outline and ±`distance_range` texels map to
/// [0, 1]. Pure.
fn coverage_to_sdf(
    coverage: &[u8],
    width: usize,
    height: usize,
    pad: usize,
    distance_range: f32,
) -> Vec<f32> {
    let cell_w = width + pad * 2;
    let cell_h = height + pad * 2;
    let n = cell_w * cell_h;

    // Continuous coverage in the padded cell, plus the inside/outside mask the
    // sign comes from. The padding stays at 0 (fully outside).
    let mut cov = vec![0.0_f32; n];
    let mut inside = vec![false; n];
    for y in 0..height {
        for x in 0..width {
            let c = f32::from(coverage[y * width + x]) / 255.0;
            let idx = (y + pad) * cell_w + (x + pad);
            cov[idx] = c;
            inside[idx] = c >= 0.5;
        }
    }

    // Per-boundary-texel sub-texel edge offset, measured from the texel center
    // along the coverage gradient: how far (in texels, unsigned) the true 0.5
    // crossing sits from this feature point. Non-boundary texels get 0.
    let frac = boundary_subtexel_offset(&cov, &inside, cell_w, cell_h);

    // One distance transform per side, each carrying the nearest feature's
    // fractional offset so the integer hop is corrected to the real crossing.
    let dist_inside = euclidean_distance_to_other(&inside, &frac, cell_w, cell_h, true);
    let dist_outside = euclidean_distance_to_other(&inside, &frac, cell_w, cell_h, false);

    let mut out = vec![0.0_f32; n];
    for i in 0..n {
        // Positive inside, negative outside; the carried fractional offset (not a
        // constant half-texel) places the zero crossing at the coverage 0.5 edge.
        let signed = if inside[i] {
            dist_inside[i]
        } else {
            -dist_outside[i]
        };
        // Map [-range, +range] texels -> [0, 1], 0.5 = outline.
        out[i] = (signed / (2.0 * distance_range) + 0.5).clamp(0.0, 1.0);
    }
    out
}

/// For every texel adjacent to the 0.5 coverage crossing, the unsigned sub-texel
/// distance from its center to the crossing, reconstructed from the anti-aliased
/// coverage ramp: `|0.5 - coverage| / |∇coverage|` (central differences). This
/// recovers the true outline position discarded by a 1-bit threshold. Texels not
/// straddling the crossing — and any with a flat local gradient — get 0, falling
/// back to the integer feature distance. Result is in [0, 0.5].
fn boundary_subtexel_offset(cov: &[f32], inside: &[bool], w: usize, h: usize) -> Vec<f32> {
    let mut frac = vec![0.0_f32; w * h];
    let at = |x: usize, y: usize| cov[y * w + x];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            // A feature point is a texel with at least one opposite-side 4-neighbour.
            let here = inside[i];
            let boundary = (x > 0 && inside[i - 1] != here)
                || (x + 1 < w && inside[i + 1] != here)
                || (y > 0 && inside[i - w] != here)
                || (y + 1 < h && inside[i + w] != here);
            if !boundary {
                continue;
            }
            // Central-difference gradient (clamped one-sided at the border).
            let xr = (x + 1).min(w - 1);
            let xl = x.saturating_sub(1);
            let yd = (y + 1).min(h - 1);
            let yu = y.saturating_sub(1);
            let dx = (at(xr, y) - at(xl, y)) * 0.5;
            let dy = (at(x, yd) - at(x, yu)) * 0.5;
            let grad = (dx * dx + dy * dy).sqrt();
            // Flat ramp -> no reliable sub-texel info; leave 0 (integer fallback).
            if grad > 1e-4 {
                // Distance to the 0.5 crossing along the gradient, in texels.
                frac[i] = ((0.5 - cov[i]).abs() / grad).min(0.5);
            }
        }
    }
    frac
}

/// Euclidean distance from every `target`-side cell to the nearest opposite-side
/// outline crossing (others get 0). Dead-reckoning 8SSEDT: store the offset to the
/// nearest boundary feature point and relax it in a forward then backward sweep.
/// Each feature carries its sub-texel `frac` (distance from its center to the 0.5
/// crossing along the coverage gradient); the returned distance subtracts the
/// nearest feature's `frac`, so the zero crossing lands on the real outline rather
/// than snapping to the feature texel's center.
fn euclidean_distance_to_other(
    inside: &[bool],
    frac: &[f32],
    w: usize,
    h: usize,
    target: bool,
) -> Vec<f32> {
    const INF: f32 = 1.0e9;
    let n = w * h;
    // Per-cell nearest-feature offset (dx, dy) and that feature's sub-texel frac.
    let mut dx = vec![0_i32; n];
    let mut dy = vec![0_i32; n];
    let mut frac_at = vec![0.0_f32; n];
    let mut dist = vec![INF; n];

    // Seed: a cell of the OPPOSITE side is a boundary feature point at distance 0,
    // carrying its own coverage-derived sub-texel offset to the true crossing.
    for i in 0..n {
        if inside[i] != target {
            dist[i] = 0.0;
            dx[i] = 0;
            dy[i] = 0;
            frac_at[i] = frac[i];
        }
    }

    let hypot = |x: i32, y: i32| -> f32 { ((x * x + y * y) as f32).sqrt() };

    // Relax cell `i` against neighbour `j` offset by (ox, oy).
    let relax = |i: usize,
                 j: usize,
                 ox: i32,
                 oy: i32,
                 dx: &mut [i32],
                 dy: &mut [i32],
                 frac_at: &mut [f32],
                 dist: &mut [f32]| {
        let cand_x = dx[j] + ox;
        let cand_y = dy[j] + oy;
        let cand = hypot(cand_x, cand_y);
        if cand < dist[i] {
            dist[i] = cand;
            dx[i] = cand_x;
            dy[i] = cand_y;
            frac_at[i] = frac_at[j];
        }
    };

    // Forward pass: top-to-bottom, left-to-right (W, NW, N, NE neighbours).
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if x > 0 {
                relax(i, i - 1, 1, 0, &mut dx, &mut dy, &mut frac_at, &mut dist);
            }
            if y > 0 {
                relax(i, i - w, 0, 1, &mut dx, &mut dy, &mut frac_at, &mut dist);
                if x > 0 {
                    relax(i, i - w - 1, 1, 1, &mut dx, &mut dy, &mut frac_at, &mut dist);
                }
                if x + 1 < w {
                    relax(i, i - w + 1, 1, 1, &mut dx, &mut dy, &mut frac_at, &mut dist);
                }
            }
        }
    }
    // Backward pass: bottom-to-top, right-to-left (E, SE, S, SW neighbours).
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            if x + 1 < w {
                relax(i, i + 1, 1, 0, &mut dx, &mut dy, &mut frac_at, &mut dist);
            }
            if y + 1 < h {
                relax(i, i + w, 0, 1, &mut dx, &mut dy, &mut frac_at, &mut dist);
                if x + 1 < w {
                    relax(i, i + w + 1, 1, 1, &mut dx, &mut dy, &mut frac_at, &mut dist);
                }
                if x > 0 {
                    relax(i, i + w - 1, 1, 1, &mut dx, &mut dy, &mut frac_at, &mut dist);
                }
            }
        }
    }

    // The true outline sits `frac_at` texels short of the feature center along the
    // gradient, so subtract it (the old fixed half-texel, now reconstructed).
    for i in 0..n {
        if dist[i] >= INF {
            dist[i] = 0.0;
        } else {
            dist[i] = (dist[i] - frac_at[i]).max(0.0);
        }
    }
    dist
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

    /// Stub line box: `ascent - descent == 1` per px, so the centered visual box equals
    /// the line height — the geometry these unit tests assert against.
    const STUB_BOX: (f32, f32) = (1.0, 0.0);

    /// Stub measure: every char is `size` wide, to exercise wrapping without fontdue.
    fn unit_measure(_ch: char, size: f32) -> f32 {
        size
    }

    fn run(text: &str, size: f32) -> TextRunInput {
        TextRunInput {
            text: text.to_string(),
            color: WHITE,
            size,
            bold: false,
            italic: false,
            font: "default".to_string(),
        }
    }

    fn lines_of(placements: &[GlyphPlacement]) -> Vec<f32> {
        let mut ys: Vec<f32> = placements.iter().map(|p| p.y).collect();
        ys.dedup();
        ys
    }

    #[test]
    fn single_run_wraps_to_n_lines_within_narrow_region() {
        // Each glyph is 10px wide; region is 30px so 3 glyphs fit per line.
        // "aaa bbb ccc" -> three words of width 30 each -> three lines.
        let runs = [run("aaa bbb ccc", 10.0)];
        let placements = layout_runs(
            &runs,
            (0.0, 0.0),
            (30.0, 100.0),
            TextAlign::Start,
            TextVAlign::Top,
            STUB_BOX,
            &unit_measure,
        );

        let line_ys = lines_of(&placements);
        assert_eq!(line_ys.len(), 3, "expected three wrapped lines");
        assert_eq!(placements.len(), 9);
        assert_eq!(line_ys, vec![0.0, 10.0, 20.0]);
        for line_first in [0usize, 3, 6] {
            assert_eq!(placements[line_first].x, 0.0);
        }
    }

    #[test]
    fn long_word_hard_wraps_by_char() {
        // A single 5-char word at 10px = 50px in a 30px region must char-wrap.
        let runs = [run("abcde", 10.0)];
        let placements = layout_runs(
            &runs,
            (0.0, 0.0),
            (30.0, 100.0),
            TextAlign::Start,
            TextVAlign::Top,
            STUB_BOX,
            &unit_measure,
        );
        let line_ys = lines_of(&placements);
        assert_eq!(line_ys.len(), 2, "5 chars / 3-per-line -> two lines");
        assert_eq!(placements.len(), 5);
    }

    #[test]
    fn center_align_centers_line_within_region() {
        // One glyph (10px) in a 100px region: slack 90 -> centered at x=45.
        let runs = [run("a", 10.0)];
        let placements = layout_runs(
            &runs,
            (0.0, 0.0),
            (100.0, 100.0),
            TextAlign::Center,
            TextVAlign::Top,
            STUB_BOX,
            &unit_measure,
        );
        assert_eq!(placements.len(), 1);
        assert!((placements[0].x - 45.0).abs() < 1e-4, "x={}", placements[0].x);
    }

    #[test]
    fn end_align_right_justifies_line() {
        let runs = [run("a", 10.0)];
        let placements = layout_runs(
            &runs,
            (0.0, 0.0),
            (100.0, 100.0),
            TextAlign::End,
            TextVAlign::Top,
            STUB_BOX,
            &unit_measure,
        );
        // slack 90 -> glyph origin at x=90, right edge lands at region_max.x.
        assert!((placements[0].x - 90.0).abs() < 1e-4, "x={}", placements[0].x);
    }

    #[test]
    fn valign_middle_offsets_block_downward() {
        // Single line height 10 in a 100px-tall region: (100-10)/2 = 45.
        let runs = [run("a", 10.0)];
        let placements = layout_runs(
            &runs,
            (0.0, 0.0),
            (100.0, 100.0),
            TextAlign::Start,
            TextVAlign::Middle,
            STUB_BOX,
            &unit_measure,
        );
        assert!((placements[0].y - 45.0).abs() < 1e-4, "y={}", placements[0].y);
    }

    #[test]
    fn valign_bottom_pins_block_to_region_bottom() {
        // Two lines, total height 20, in a 100px region starting at y=0:
        // first line y = 100 - 20 = 80.
        let runs = [run("aaa bbb", 10.0)];
        let placements = layout_runs(
            &runs,
            (0.0, 0.0),
            (30.0, 100.0),
            TextAlign::Start,
            TextVAlign::Bottom,
            STUB_BOX,
            &unit_measure,
        );
        let line_ys = lines_of(&placements);
        assert_eq!(line_ys.len(), 2);
        assert!((line_ys[0] - 80.0).abs() < 1e-4, "first line y={}", line_ys[0]);
        assert!((line_ys[1] - 90.0).abs() < 1e-4, "second line y={}", line_ys[1]);
    }

    #[test]
    fn justify_stretches_interior_gaps_but_not_last_line() {
        // Region 50px, glyph 10px: "aa bb" fills the first line, "cc" the second;
        // justify stretches the first line's single gap to fill 50.
        let runs = [run("aa bb cc", 10.0)];
        let placements = layout_runs(
            &runs,
            (0.0, 0.0),
            (50.0, 100.0),
            TextAlign::Justify,
            TextVAlign::Top,
            STUB_BOX,
            &unit_measure,
        );
        let line_ys = lines_of(&placements);
        assert_eq!(line_ys.len(), 2, "expected two lines");

        // One gap, slack 10: gap becomes space(10) + extra(10) = 20, so 'b' starts at 40.
        let first_line: Vec<&GlyphPlacement> =
            placements.iter().filter(|p| p.y == 0.0).collect();
        assert_eq!(first_line.len(), 4);
        assert_eq!(first_line[0].x, 0.0);
        assert_eq!(first_line[1].x, 10.0);
        assert!(
            (first_line[2].x - 40.0).abs() < 1e-4,
            "justified second word x={}",
            first_line[2].x
        );

        // Last line ("cc") is NOT justified -> starts at region_min.x.
        let last_line: Vec<&GlyphPlacement> =
            placements.iter().filter(|p| p.y > 0.0).collect();
        assert_eq!(last_line.len(), 2);
        assert_eq!(last_line[0].x, 0.0);
    }

    #[test]
    fn explicit_newline_forces_line_break() {
        let runs = [run("a\nb", 10.0)];
        let placements = layout_runs(
            &runs,
            (0.0, 0.0),
            (100.0, 100.0),
            TextAlign::Start,
            TextVAlign::Top,
            STUB_BOX,
            &unit_measure,
        );
        let line_ys = lines_of(&placements);
        assert_eq!(line_ys.len(), 2);
        assert_eq!(placements.len(), 2);
        assert_eq!(placements[0].ch, 'a');
        assert_eq!(placements[1].ch, 'b');
    }

    #[test]
    fn multiple_runs_preserve_run_index_and_per_run_size() {
        // Two runs on one line: run 0 ("Ab", size 10), run 1 ("Cd", size 20).
        // Line height is the max run size (20). Wide region keeps one line.
        let runs = [run("Ab", 10.0), run("Cd", 20.0)];
        let placements = layout_runs(
            &runs,
            (0.0, 0.0),
            (1000.0, 100.0),
            TextAlign::Start,
            TextVAlign::Top,
            STUB_BOX,
            &unit_measure,
        );
        assert_eq!(placements.len(), 4);
        assert_eq!(placements[0].run_index, 0);
        assert_eq!(placements[0].size, 10.0);
        assert_eq!(placements[3].run_index, 1);
        assert_eq!(placements[3].size, 20.0);

        // Advances: A@0, b@10 (size 10), C@20, d@40 (size 20 advances).
        assert_eq!(placements[1].x, 10.0);
        assert_eq!(placements[2].x, 20.0);
        assert_eq!(placements[3].x, 40.0);
    }

    #[test]
    fn empty_runs_produce_no_placements() {
        let placements = layout_runs(
            &[],
            (0.0, 0.0),
            (100.0, 100.0),
            TextAlign::Start,
            TextVAlign::Top,
            STUB_BOX,
            &unit_measure,
        );
        assert!(placements.is_empty());
    }

    #[test]
    fn msdf_atlas_plan_round_trips_glyph_entries() {
        let mut plan = MsdfAtlasPlan::new(2048, 2048, 4.0);
        let key = MsdfGlyphKey {
            font_index: 0,
            glyph_id: 42,
            px: 64,
            coverage: false,
        };
        let entry = MsdfGlyphEntry {
            uv: [[0.0, 0.0], [0.1, 0.0], [0.1, 0.1], [0.0, 0.1]],
            bearing_x: 1.0,
            bearing_y: 2.0,
            width: 40.0,
            height: 48.0,
            coverage: false,
        };
        assert_eq!(plan.lookup(&key), None);
        plan.insert(key, entry);
        assert_eq!(plan.glyph_count(), 1);
        assert_eq!(plan.lookup(&key), Some(&entry));
        assert_eq!(plan.atlas_width, 2048);
        assert_eq!(plan.distance_range, 4.0);
    }

    /// A fully-covered square block as a synthetic glyph coverage raster.
    fn solid_block(side: usize) -> Vec<u8> {
        vec![255_u8; side * side]
    }

    fn glyph_key(glyph_id: u16) -> MsdfGlyphKey {
        MsdfGlyphKey { font_index: 0, glyph_id, px: 32, coverage: false }
    }

    /// FALSIFIABLE: the coverage atlas path stores the RAW fontdue coverage — flat
    /// opaque interior, flat-zero exterior, anti-aliased intermediate edges — NOT a
    /// signed distance field. A distance field has a smooth ramp centered at 0.5
    /// everywhere with no flat-255 interior; coverage has both saturated extremes.
    /// This fails if `generate_coverage_glyph` ever routes through `coverage_to_sdf`:
    /// the stored texels would no longer equal the source bitmap and the interior
    /// would never read fully opaque.
    #[test]
    fn coverage_glyph_stores_raw_coverage_not_a_distance_field() {
        let engine = crate::text::TextEngine::new().expect("bundled fonts load");
        // A large 'B' so the interior is solidly covered and the edges anti-aliased.
        let cov = engine
            .glyph_coverage('B', 48.0, 1.0)
            .expect("'B' rasterizes");
        let mut plan = MsdfAtlasPlan::new(256, 256, 4.0);
        let entry = plan
            .generate_coverage_glyph(&GlyphCoverage {
                key: MsdfGlyphKey {
                    font_index: cov.font_index,
                    glyph_id: cov.glyph_id,
                    px: cov.px,
                    coverage: true,
                },
                coverage: &cov.coverage,
                width: cov.width,
                height: cov.height,
                bearing_x: cov.bearing_x,
                bearing_y: cov.bearing_y,
                oversample: cov.oversample,
                outline: None,
            })
            .expect("atlas has room");
        assert!(entry.coverage, "the entry is flagged coverage-mode");

        // The slot has NO distance-range pad: the quad spans exactly the ink cell.
        assert_eq!(entry.width, cov.width as f32);
        assert_eq!(entry.height, cov.height as f32);

        // Read the packed alpha channel back from the atlas at the slot origin and
        // assert it is BYTE-IDENTICAL to the source fontdue coverage. An SDF transform
        // would rewrite every texel, so this equality is the falsifier.
        let origin_x = crate::cast::round_u32(entry.uv[0][0] * plan.atlas_width as f32);
        let origin_y = crate::cast::round_u32(entry.uv[0][1] * plan.atlas_height as f32);
        let aw = plan.atlas_width;
        let mut max_alpha = 0_u8;
        let mut min_alpha = 255_u8;
        let mut has_edge = false;
        for row in 0..crate::cast::len_u32(cov.height) {
            for col in 0..crate::cast::len_u32(cov.width) {
                let src = cov.coverage[(row * crate::cast::len_u32(cov.width) + col) as usize];
                let i = (((origin_y + row) * aw + (origin_x + col)) * 4) as usize;
                assert_eq!(
                    plan.pixels()[i + 3],
                    src,
                    "stored alpha equals raw coverage (no SDF transform)"
                );
                max_alpha = max_alpha.max(src);
                min_alpha = min_alpha.min(src);
                if (1..=254).contains(&src) {
                    has_edge = true;
                }
            }
        }
        assert_eq!(max_alpha, 255, "a fully-covered interior texel (flat opaque)");
        assert_eq!(min_alpha, 0, "a fully-uncovered exterior texel (flat zero)");
        assert!(has_edge, "an anti-aliased intermediate edge texel exists");
    }

    /// A coverage slot and an SDF slot for the SAME glyph never collide: the keys
    /// differ by their `coverage` flag, so both register in one shared atlas.
    #[test]
    fn coverage_and_sdf_slots_for_one_glyph_coexist() {
        let engine = crate::text::TextEngine::new().expect("bundled fonts load");
        let cov = engine.glyph_coverage('A', 24.0, 1.0).expect("'A' rasterizes");
        let mut plan = MsdfAtlasPlan::new(256, 256, 4.0);
        let mk = |coverage: bool| GlyphCoverage {
            key: MsdfGlyphKey {
                font_index: cov.font_index,
                glyph_id: cov.glyph_id,
                px: cov.px,
                coverage,
            },
            coverage: &cov.coverage,
            width: cov.width,
            height: cov.height,
            bearing_x: cov.bearing_x,
            bearing_y: cov.bearing_y,
            oversample: cov.oversample,
            // The coverage branch ignores this; the SDF branch bakes the true MSDF.
            outline: if coverage { None } else { cov.outline.clone() },
        };
        let sdf = plan.generate_glyph(&mk(false)).expect("sdf slot");
        let coverage = plan.generate_coverage_glyph(&mk(true)).expect("coverage slot");
        assert_eq!(plan.glyph_count(), 2, "two slots, no collision");
        assert!(!sdf.coverage && coverage.coverage);
        assert_ne!(sdf.uv, coverage.uv, "distinct atlas regions");
    }

    /// Sample the median-of-3 SDF value the shader reads at atlas texel (x, y).
    fn sampled_distance(plan: &MsdfAtlasPlan, x: u32, y: u32) -> f32 {
        let i = ((y * plan.atlas_width + x) * 4) as usize;
        let px = plan.pixels();
        let r = px[i] as f32 / 255.0;
        let g = px[i + 1] as f32 / 255.0;
        let b = px[i + 2] as f32 / 255.0;
        // median3 — what msdf_text.wgsl computes.
        (r.min(g)).max(b.min(r.max(g)))
    }

    #[test]
    fn generate_glyph_packs_a_real_distance_field() {
        let mut plan = MsdfAtlasPlan::new(256, 256, 4.0);
        let side = 16;
        let coverage = solid_block(side);
        let entry = plan
            .generate_glyph(&GlyphCoverage {
                key: glyph_key(7),
                coverage: &coverage,
                width: side,
                height: side,
                bearing_x: 2.0,
                bearing_y: 3.0,
                oversample: 1.0,
                outline: None,
            })
            .expect("atlas has room");

        // The glyph is registered, with a padded cell larger than the raw raster.
        assert_eq!(plan.glyph_count(), 1);
        assert!(entry.width > side as f32 && entry.height > side as f32);
        // Bearings are shifted left/up by the pad so the ink stays registered.
        assert!(entry.bearing_x < 2.0 && entry.bearing_y < 3.0);
        // Non-degenerate UVs that fit inside the atlas.
        for [u, v] in entry.uv {
            assert!((0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v));
        }
        assert!(entry.uv[1][0] > entry.uv[0][0], "right u > left u");
        assert!(entry.uv[2][1] > entry.uv[0][1], "bottom v > top v");

        // The field reads "inside" (>0.5) at the cell center and "outside" (<0.5)
        // at a far corner of the padded cell — a real signed distance ramp.
        let origin_x = crate::cast::round_u32(entry.uv[0][0] * plan.atlas_width as f32);
        let origin_y = crate::cast::round_u32(entry.uv[0][1] * plan.atlas_height as f32);
        let cell_w = crate::cast::round_u32(entry.width);
        let cell_h = crate::cast::round_u32(entry.height);
        let center = sampled_distance(&plan, origin_x + cell_w / 2, origin_y + cell_h / 2);
        let corner = sampled_distance(&plan, origin_x, origin_y);
        assert!(center > 0.5, "glyph interior is inside the outline: {center}");
        assert!(corner < 0.5, "padded corner is outside the outline: {corner}");
        assert!(center > corner, "distance ramps from corner to center");
    }

    /// Oversampling raises the SDF source resolution (the host rasterizes the
    /// coverage at `size * oversample` px for sharp retina text) WITHOUT changing
    /// layout: `generate_glyph` divides the emitted quad geometry by `oversample`, so
    /// the same coverage fills the same atlas slot but lays out at the logical size.
    /// Fails if the division is dropped — text would render `oversample`× too big.
    #[test]
    fn oversample_keeps_layout_logical_while_slot_stays_high_res() {
        let side = 16;
        let coverage = solid_block(side);
        let make = |oversample: f32| {
            let mut plan = MsdfAtlasPlan::new(256, 256, 4.0);
            let entry = plan
                .generate_glyph(&GlyphCoverage {
                    key: glyph_key(1),
                    coverage: &coverage,
                    width: side,
                    height: side,
                    bearing_x: 10.0,
                    bearing_y: 10.0,
                    oversample,
                    outline: None,
                })
                .expect("atlas has room");
            entry
        };
        let e1 = make(1.0);
        let e2 = make(2.0);

        // Same source coverage => identical atlas-slot footprint: the SDF stays
        // high-res, the slot is NOT shrunk by oversampling.
        let span = |e: &MsdfGlyphEntry| (e.uv[1][0] - e.uv[0][0], e.uv[2][1] - e.uv[0][1]);
        assert!((span(&e1).0 - span(&e2).0).abs() < 1e-6);
        assert!((span(&e1).1 - span(&e2).1).abs() < 1e-6);

        // ...but the emitted quad geometry is HALVED at oversample=2 (logical layout
        // is resolution-independent). A dropped division leaves e2 == e1 and fails.
        assert!((e2.width - e1.width / 2.0).abs() < 1e-3, "width halves: {} vs {}", e2.width, e1.width);
        assert!((e2.height - e1.height / 2.0).abs() < 1e-3, "height halves");
        assert!((e2.bearing_x - e1.bearing_x / 2.0).abs() < 1e-3, "bearing halves");
    }

    #[test]
    fn generate_glyph_is_idempotent_per_key() {
        let mut plan = MsdfAtlasPlan::new(256, 256, 4.0);
        let coverage = solid_block(12);
        let g = GlyphCoverage {
            key: glyph_key(9),
            coverage: &coverage,
            width: 12,
            height: 12,
            bearing_x: 0.0,
            bearing_y: 0.0,
            oversample: 1.0,
            outline: None,
        };
        let first = plan.generate_glyph(&g).unwrap();
        let pixels_after_first = plan.pixels().to_vec();
        let second = plan.generate_glyph(&g).unwrap();
        // Same entry, no re-packing (pixel buffer unchanged, count stays 1).
        assert_eq!(first, second);
        assert_eq!(plan.glyph_count(), 1);
        assert_eq!(plan.pixels(), pixels_after_first.as_slice());
    }

    #[test]
    fn blank_glyph_maps_to_a_zero_size_entry() {
        let mut plan = MsdfAtlasPlan::new(64, 64, 4.0);
        let entry = plan
            .generate_glyph(&GlyphCoverage {
                key: glyph_key(0),
                coverage: &[],
                width: 0,
                height: 0,
                bearing_x: 5.0,
                bearing_y: 6.0,
                oversample: 1.0,
                outline: None,
            })
            .expect("blank always fits");
        // No quad (zero size), bearings preserved (e.g. a space advance).
        assert_eq!(entry.width, 0.0);
        assert_eq!(entry.height, 0.0);
        assert_eq!(entry.bearing_x, 5.0);
        assert_eq!(entry.bearing_y, 6.0);
    }

    #[test]
    fn generate_glyph_returns_none_when_atlas_is_full() {
        // A tiny atlas with no room for even one padded cell.
        let mut plan = MsdfAtlasPlan::new(8, 8, 4.0);
        let coverage = solid_block(16);
        let result = plan.generate_glyph(&GlyphCoverage {
            key: glyph_key(1),
            coverage: &coverage,
            width: 16,
            height: 16,
            bearing_x: 0.0,
            bearing_y: 0.0,
            oversample: 1.0,
            outline: None,
        });
        assert!(result.is_none(), "no room: caller falls back to fontdue raster");
        assert_eq!(plan.glyph_count(), 0);
    }

    /// DEFECT 3 (i): the populated atlas has REAL non-zero coverage for a committed
    /// run's glyphs, sourced from the bundled fonts via the host coverage seam. The
    /// blank `MsdfAtlasPlan::new` upload the live path shipped is all-zero, so the
    /// shader's `median3` was 0 and every glyph painted fully transparent — this
    /// FAILS against that all-zero atlas and passes once real coverage is packed.
    #[test]
    fn generate_glyph_from_real_font_coverage_has_nonzero_texels() {
        let engine = crate::text::TextEngine::new().expect("bundled fonts load");
        let mut plan = MsdfAtlasPlan::new(2048, 2048, 4.0);

        // A committed run "AB" at the size:None default (16px after de-quant).
        let mut packed_any = false;
        for ch in "AB".chars() {
            let cov = engine
                .glyph_coverage(ch, 16.0, 1.0)
                .unwrap_or_else(|| panic!("glyph '{ch}' rasterizes"));
            let entry = plan
                .generate_glyph(&GlyphCoverage {
                    key: MsdfGlyphKey {
                        font_index: cov.font_index,
                        glyph_id: cov.glyph_id,
                        px: cov.px,
                        coverage: false,
                    },
                    coverage: &cov.coverage,
                    width: cov.width,
                    height: cov.height,
                    bearing_x: cov.bearing_x,
                    bearing_y: cov.bearing_y,
                    oversample: cov.oversample,
                    outline: cov.outline,
                })
                .expect("atlas has room");

            // The glyph's slot must contain at least one texel above 0 (real ink),
            // not the all-zero blank atlas.
            let origin_x = crate::cast::round_u32(entry.uv[0][0] * plan.atlas_width as f32);
            let origin_y = crate::cast::round_u32(entry.uv[0][1] * plan.atlas_height as f32);
            let cell_w = crate::cast::round_u32(entry.width);
            let cell_h = crate::cast::round_u32(entry.height);
            let mut has_ink = false;
            for row in 0..cell_h {
                for col in 0..cell_w {
                    if sampled_distance(&plan, origin_x + col, origin_y + row) > 0.0 {
                        has_ink = true;
                    }
                }
            }
            assert!(has_ink, "glyph '{ch}' slot has a non-zero coverage texel");
            packed_any = true;
        }
        assert!(packed_any && plan.glyph_count() > 0, "atlas packed real glyphs");
    }

    #[test]
    fn coverage_to_sdf_is_deterministic_and_centered() {
        let side = 10;
        let coverage = solid_block(side);
        let a = coverage_to_sdf(&coverage, side, side, 4, 4.0);
        let b = coverage_to_sdf(&coverage, side, side, 4, 4.0);
        assert_eq!(a, b, "pure: same input -> identical field");
        let cell = side + 8; // width + pad*2
        // Center is inside (>0.5), an outer-padding corner is outside (<0.5).
        let center = a[(cell / 2) * cell + cell / 2];
        let corner = a[0];
        assert!(center > 0.5, "interior inside: {center}");
        assert!(corner < 0.5, "padding outside: {corner}");
    }

    /// A vertical edge raster: `cols` columns are solid inside (255), one transition
    /// column carries `edge_cov`, the rest are fully outside (0). The transition
    /// column's anti-aliased coverage pins the true 0.5 crossing at a sub-texel
    /// position. `width`/`height` chosen so the gradient at the edge is well-defined.
    fn vertical_edge(edge_cov: u8) -> (Vec<u8>, usize, usize) {
        let width = 6;
        let height = 4;
        let inside_cols = 2; // cols 0,1 solid inside; col 2 is the AA transition.
        let mut cov = vec![0_u8; width * height];
        for y in 0..height {
            for x in 0..width {
                cov[y * width + x] = if x < inside_cols {
                    255
                } else if x == inside_cols {
                    edge_cov
                } else {
                    0
                };
            }
        }
        (cov, width, height)
    }

    /// FALSIFIABLE: the SDF is reconstructed from the anti-aliased coverage ramp,
    /// NOT a 1-bit threshold. Two rasters whose transition column both threshold to
    /// "inside" (>=128) but carry DIFFERENT anti-aliased coverage place the 0.5
    /// crossing at different sub-texel positions, so they MUST yield different
    /// fields. The old `coverage >= 128` path collapses both to the same inside mask
    /// and produces byte-identical fields — this test fails on that path.
    #[test]
    fn coverage_to_sdf_reconstructs_subtexel_edge_from_coverage_ramp() {
        let pad = 4;
        // Both transition coverages are >= 128 (same 1-bit threshold), but one sits
        // barely inside the crossing and the other deep inside it.
        let (near, w, h) = vertical_edge(135); // ~0.53: edge just past the texel center
        let (far, _, _) = vertical_edge(250); // ~0.98: edge a near-full texel away
        let field_near = coverage_to_sdf(&near, w, h, pad, 4.0);
        let field_far = coverage_to_sdf(&far, w, h, pad, 4.0);

        assert_ne!(
            field_near, field_far,
            "sub-texel edge must move with the AA ramp; the 1-bit path makes these identical"
        );

        // The deeper-inside transition pushes the outline farther right; sampling an
        // OUTSIDE cell just past the edge, whose nearest feature is that transition
        // column, reads LESS far outside (larger SDF) for `far`.
        let cell_w = w + pad * 2;
        let row = pad + h / 2;
        let sample = row * cell_w + (pad + 3); // first fully-outside column's cell.
        assert!(
            field_far[sample] > field_near[sample],
            "stronger coverage -> edge farther out -> less-outside SDF: far={} near={}",
            field_far[sample],
            field_near[sample]
        );
    }

    /// FALSIFIABLE: moving the sub-texel edge by sweeping the transition coverage
    /// across the 0.5 crossing moves the reconstructed zero crossing monotonically.
    /// A 1-bit threshold would step exactly once (at 128) and stay flat either side,
    /// so the strictly-monotone progression below cannot hold on that path.
    #[test]
    fn coverage_to_sdf_zero_crossing_moves_monotonically_with_subtexel_coverage() {
        let pad = 4;
        let cell_w = 6 + pad * 2;
        let row = pad + 2;
        let sample = row * cell_w + (pad + 3); // first fully-outside column's cell.
        let mut last = f32::NEG_INFINITY;
        // Sweep the transition coverage across the unsaturated sub-texel band (the
        // crossing stays within ±0.5 texel of the feature center). All >= 128, so
        // the 1-bit threshold is constant and the field would be flat on that path.
        for edge_cov in [130_u8, 150, 170, 188] {
            let (cov, w, h) = vertical_edge(edge_cov);
            let field = coverage_to_sdf(&cov, w, h, pad, 4.0);
            let v = field[sample];
            assert!(
                v > last,
                "edge SDF must rise as coverage rises (sub-texel edge moves out): \
                 edge_cov={edge_cov} v={v} prev={last}"
            );
            last = v;
        }
    }

    /// Build the world/SDF atlas slot for one real glyph through the FULL path
    /// (raster -> outline extraction -> `generate_glyph`), returning the plan + entry.
    /// When `with_outline` is false the outline is dropped, forcing the legacy
    /// coverage-derived single-channel field — the discriminator the MSDF tests use.
    fn build_sdf_slot(ch: char, size: f32, with_outline: bool) -> (MsdfAtlasPlan, MsdfGlyphEntry) {
        let engine = crate::text::TextEngine::new().expect("bundled fonts load");
        let cov = engine
            .glyph_coverage(ch, size, 1.0)
            .unwrap_or_else(|| panic!("{ch:?} rasterizes at {size}px"));
        if with_outline {
            assert!(cov.outline.is_some(), "{ch:?} must yield a vector outline");
        }
        let mut plan = MsdfAtlasPlan::new(512, 512, 4.0);
        let entry = plan
            .generate_glyph(&GlyphCoverage {
                key: MsdfGlyphKey {
                    font_index: cov.font_index,
                    glyph_id: cov.glyph_id,
                    px: cov.px,
                    coverage: false,
                },
                coverage: &cov.coverage,
                width: cov.width,
                height: cov.height,
                bearing_x: cov.bearing_x,
                bearing_y: cov.bearing_y,
                oversample: cov.oversample,
                outline: if with_outline { cov.outline } else { None },
            })
            .expect("atlas has room");
        (plan, entry)
    }

    /// Read the raw R/G/B atlas texel at the slot-local `(col, row)`.
    fn slot_rgb(plan: &MsdfAtlasPlan, entry: &MsdfGlyphEntry, col: u32, row: u32) -> (u8, u8, u8) {
        let origin_x = crate::cast::round_u32(entry.uv[0][0] * plan.atlas_width as f32);
        let origin_y = crate::cast::round_u32(entry.uv[0][1] * plan.atlas_height as f32);
        let i = (((origin_y + row) * plan.atlas_width + (origin_x + col)) * 4) as usize;
        let px = plan.pixels();
        (px[i], px[i + 1], px[i + 2])
    }

    /// FALSIFIABLE (the load-bearing true-MSDF discriminator): a sharp-cornered glyph
    /// baked through the world/SDF path stores THREE DISTINCT distance channels near
    /// its corners — `max(|R-G|,|G-B|,|R-B|)` exceeds a real spread somewhere. The old
    /// coverage-derived path replicated ONE distance into R=G=B everywhere, so the
    /// channel spread was 0 at every texel: this test FAILS on that path and passes
    /// only for a true multi-channel MSDF. 'L' has a single hard right-angle corner.
    #[test]
    fn world_sdf_glyph_has_distinct_rgb_channels_for_true_msdf() {
        let (plan, entry) = build_sdf_slot('L', 48.0, true);
        let cell_w = crate::cast::round_u32(entry.width);
        let cell_h = crate::cast::round_u32(entry.height);

        let mut max_spread = 0_u8;
        for row in 0..cell_h {
            for col in 0..cell_w {
                let (r, g, b) = slot_rgb(&plan, &entry, col, row);
                let spread = r.abs_diff(g).max(g.abs_diff(b)).max(r.abs_diff(b));
                max_spread = max_spread.max(spread);
            }
        }
        // The single-channel field has spread == 0 at every texel; a true MSDF puts the
        // colored channels meaningfully apart where edges of different orientation meet.
        assert!(
            max_spread > 8,
            "world SDF slot is single-channel (R==G==B), not a true MSDF: max channel spread {max_spread}"
        );
    }

    /// FALSIFIABLE: the true MSDF preserves a sharp corner the coverage-derived SDF
    /// rounds. Both fields are built for the same corner glyph; just OUTSIDE the corner
    /// tip the median3 the shader reads is LARGER (closer to the 0.5 outline) for the
    /// MSDF than for the single-channel SDF, because the single-channel field rounds the
    /// corner inward (reads more-outside) while median3 of the colored channels follows
    /// the true corner. Fails if `generate_glyph` still emits a replicated channel (the
    /// two fields would be identical and the medians equal).
    #[test]
    fn world_msdf_median_preserves_corner_that_single_channel_blurs() {
        let (msdf_plan, msdf) = build_sdf_slot('L', 64.0, true);
        let (sdf_plan, sdf) = build_sdf_slot('L', 64.0, false);
        assert_eq!(msdf.uv, sdf.uv, "same slot geometry; only the texels differ");

        let median = |plan: &MsdfAtlasPlan, e: &MsdfGlyphEntry, col: u32, row: u32| -> f32 {
            let (r, g, b) = slot_rgb(plan, e, col, row);
            let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
            (r.min(g)).max(b.min(r.max(g)))
        };
        let cell_w = crate::cast::round_u32(msdf.width);
        let cell_h = crate::cast::round_u32(msdf.height);

        // Scan the band just outside the ink for the texel where the two fields disagree
        // most. A corner is exactly where the single-channel (isotropic) field is most
        // pessimistic vs the corner-preserving median — the MSDF reads less-outside.
        let mut best_gain = 0.0_f32;
        for row in 0..cell_h {
            for col in 0..cell_w {
                let m = median(&msdf_plan, &msdf, col, row);
                let s = median(&sdf_plan, &sdf, col, row);
                // Only an outside texel (below the 0.5 outline) where MSDF is sharper.
                if s < 0.5 && m > s {
                    best_gain = best_gain.max(m - s);
                }
            }
        }
        assert!(
            best_gain > 0.02,
            "true MSDF must reconstruct a sharper corner than the single-channel SDF; \
             max median gain just outside the ink was only {best_gain}"
        );
    }

    /// The MSDF bake is deterministic: a fixed edge-coloring seed and pure generation
    /// mean two runs of the same glyph produce byte-identical atlas pixels (the pure
    /// core forbids randomness, and the old code documented this guarantee).
    #[test]
    fn world_msdf_is_byte_deterministic() {
        let (plan_a, entry_a) = build_sdf_slot('R', 40.0, true);
        let (plan_b, entry_b) = build_sdf_slot('R', 40.0, true);
        assert_eq!(entry_a, entry_b);
        assert_eq!(
            plan_a.pixels(),
            plan_b.pixels(),
            "fixed seed -> identical MSDF pixels across runs"
        );
    }

    /// Graceful degrade: a glyph with no threaded outline falls back to the legacy
    /// single-channel field (R==G==B at every texel), still a valid non-empty signed
    /// field. This guards the per-glyph fallback the host uses for an unsupported glyph.
    #[test]
    fn world_sdf_without_outline_degrades_to_single_channel() {
        let (plan, entry) = build_sdf_slot('L', 48.0, false);
        let cell_w = crate::cast::round_u32(entry.width);
        let cell_h = crate::cast::round_u32(entry.height);
        let mut any_ink = false;
        for row in 0..cell_h {
            for col in 0..cell_w {
                let (r, g, b) = slot_rgb(&plan, &entry, col, row);
                assert!(r == g && g == b, "fallback must replicate one channel (R==G==B)");
                if r > 0 {
                    any_ink = true;
                }
            }
        }
        assert!(any_ink, "fallback still packs a real non-empty field");
    }

    /// The true MSDF stays a VALID signed field: median3 reads inside (>0.5) at the
    /// glyph center and outside (<0.5) at a padded corner — the same contract the
    /// single-channel field upheld, so screenPxRange AA stays calibrated.
    #[test]
    fn world_msdf_median_is_a_valid_signed_field() {
        let (plan, entry) = build_sdf_slot('o', 64.0, true);
        let cell_w = crate::cast::round_u32(entry.width);
        let cell_h = crate::cast::round_u32(entry.height);
        let median = |col: u32, row: u32| -> f32 {
            let (r, g, b) = slot_rgb(&plan, &entry, col, row);
            let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
            (r.min(g)).max(b.min(r.max(g)))
        };
        // 'o' is a ring: its geometric center is a hole (outside), so probe a point on
        // the stroke instead — a quarter in from the left edge at mid-height is ink.
        let stroke = median(cell_w / 6, cell_h / 2);
        let corner = median(0, 0);
        assert!(stroke > 0.5, "glyph stroke reads inside the outline: {stroke}");
        assert!(corner < 0.5, "padded corner reads outside the outline: {corner}");
    }
}
