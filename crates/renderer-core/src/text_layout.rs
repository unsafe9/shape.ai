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
/// advance width of `ch` at a given pixel size.
pub fn layout_runs(
    runs: &[TextRunInput],
    region_min: (f32, f32),
    region_max: (f32, f32),
    align: TextAlign,
    valign: TextVAlign,
    measure: &dyn Fn(char, f32) -> f32,
) -> Vec<GlyphPlacement> {
    let region_width = (region_max.0 - region_min.0).max(0.0);
    let region_height = (region_max.1 - region_min.1).max(0.0);

    let lines = flow_lines(runs, region_width, measure);
    if lines.is_empty() {
        return Vec::new();
    }

    let total_height: f32 = lines.iter().map(|line| line.height).sum();
    let mut pen_y = match valign {
        TextVAlign::Top => region_min.1,
        TextVAlign::Middle => region_min.1 + (region_height - total_height) * 0.5,
        TextVAlign::Bottom => region_max.1 - total_height,
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

// SDF atlas generator. Single-channel SDF, not MSDF-proper: a true MSDF needs the
// glyph's vector contours (which fontdue does not expose), so this builds a
// single-channel signed distance field from the fontdue coverage raster and
// replicates it into R/G/B (the shader's `median3` returns that one distance
// unchanged). The distance transform is the 8-points Signed Sequential Euclidean
// Distance Transform (dead-reckoning), pure and deterministic, so the same coverage
// always yields byte-identical atlas pixels.

/// Identifies a rasterized MSDF glyph in the atlas. `px` is the quantized SDF cell
/// size (resolution-independent, but a fixed cell keeps the atlas predictable).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MsdfGlyphKey {
    pub font_index: usize,
    pub glyph_id: u16,
    pub px: u16,
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
}

/// A glyph's fontdue coverage raster + placement metrics, the generator input.
/// `coverage` is row-major 8-bit alpha, `width`×`height` px.
#[derive(Clone, Debug)]
pub struct GlyphCoverage<'a> {
    pub key: MsdfGlyphKey,
    pub coverage: &'a [u8],
    pub width: usize,
    pub height: usize,
    /// Bearing from pen origin to glyph left (fontdue `xmin`).
    pub bearing_x: f32,
    pub bearing_y: f32,
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
            };
            self.entries.insert(glyph.key, entry);
            return Some(entry);
        }

        // Pad each cell by `distance_range` texels so the field has room to ramp.
        let pad = self.distance_range.ceil().max(1.0) as u32;
        let glyph_w = glyph.width as u32;
        let glyph_h = glyph.height as u32;
        let cell_w = glyph_w + pad * 2;
        let cell_h = glyph_h + pad * 2;

        let (origin_x, origin_y) = self.allocate_cell(cell_w, cell_h)?;

        let sdf = coverage_to_sdf(
            glyph.coverage,
            glyph.width,
            glyph.height,
            pad as usize,
            self.distance_range,
        );
        let atlas_w = self.atlas_width;
        for row in 0..cell_h {
            for col in 0..cell_w {
                let value = sdf[(row * cell_w + col) as usize];
                let texel = (value * 255.0).round().clamp(0.0, 255.0) as u8;
                let index = (((origin_y + row) * atlas_w + (origin_x + col)) * 4) as usize;
                self.pixels[index] = texel;
                self.pixels[index + 1] = texel;
                self.pixels[index + 2] = texel;
                self.pixels[index + 3] = texel;
            }
        }

        // The quad covers the full padded cell (AA ramp visible); bearings shift
        // left/up by `pad` to keep the glyph's ink registered against the pen origin.
        let aw = self.atlas_width as f32;
        let ah = self.atlas_height as f32;
        let left = origin_x as f32 / aw;
        let right = (origin_x + cell_w) as f32 / aw;
        let top = origin_y as f32 / ah;
        let bottom = (origin_y + cell_h) as f32 / ah;
        let entry = MsdfGlyphEntry {
            uv: [[left, top], [right, top], [right, bottom], [left, bottom]],
            bearing_x: glyph.bearing_x - pad as f32,
            bearing_y: glyph.bearing_y - pad as f32,
            width: cell_w as f32,
            height: cell_h as f32,
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

/// Build a single-channel signed distance field from a glyph coverage raster.
/// Coverage is thresholded at 0.5 into an inside/outside mask in a `pad`-padded
/// cell, converted via the 8SSEDT dead-reckoning transform, then normalized so 0.5
/// sits on the outline and ±`distance_range` texels map to the [0,1] ends. Pure.
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

    // Inside mask in the padded cell: coverage >= 128 is "inside the glyph".
    let mut inside = vec![false; n];
    for y in 0..height {
        for x in 0..width {
            if coverage[y * width + x] >= 128 {
                inside[(y + pad) * cell_w + (x + pad)] = true;
            }
        }
    }

    // One distance transform per side, combined into a signed distance.
    let dist_inside = euclidean_distance_to_other(&inside, cell_w, cell_h, true);
    let dist_outside = euclidean_distance_to_other(&inside, cell_w, cell_h, false);

    let mut out = vec![0.0_f32; n];
    for i in 0..n {
        // Positive inside, negative outside; the half-texel centers the zero crossing.
        let signed = if inside[i] {
            dist_inside[i] - 0.5
        } else {
            -(dist_outside[i] - 0.5)
        };
        // Map [-range, +range] texels -> [0, 1], 0.5 = outline.
        out[i] = (signed / (2.0 * distance_range) + 0.5).clamp(0.0, 1.0);
    }
    out
}

/// Euclidean distance from every `target`-side cell to the nearest opposite-side
/// cell (others get 0). Dead-reckoning 8SSEDT: store the offset to the nearest
/// boundary feature point and relax it in a forward then backward sweep.
fn euclidean_distance_to_other(
    inside: &[bool],
    w: usize,
    h: usize,
    target: bool,
) -> Vec<f32> {
    const INF: f32 = 1.0e9;
    let n = w * h;
    // Per-cell nearest-feature offset (dx, dy); distance is its hypot.
    let mut dx = vec![0_i32; n];
    let mut dy = vec![0_i32; n];
    let mut dist = vec![INF; n];

    // Seed: a cell of the OPPOSITE side is a boundary feature point at distance 0.
    for i in 0..n {
        if inside[i] != target {
            dist[i] = 0.0;
            dx[i] = 0;
            dy[i] = 0;
        }
    }

    let hypot = |x: i32, y: i32| -> f32 { ((x * x + y * y) as f32).sqrt() };

    // Relax cell `i` against neighbour `j` offset by (ox, oy).
    let relax = |i: usize, j: usize, ox: i32, oy: i32, dx: &mut [i32], dy: &mut [i32], dist: &mut [f32]| {
        let cand_x = dx[j] + ox;
        let cand_y = dy[j] + oy;
        let cand = hypot(cand_x, cand_y);
        if cand < dist[i] {
            dist[i] = cand;
            dx[i] = cand_x;
            dy[i] = cand_y;
        }
    };

    // Forward pass: top-to-bottom, left-to-right (W, NW, N, NE neighbours).
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if x > 0 {
                relax(i, i - 1, 1, 0, &mut dx, &mut dy, &mut dist);
            }
            if y > 0 {
                relax(i, i - w, 0, 1, &mut dx, &mut dy, &mut dist);
                if x > 0 {
                    relax(i, i - w - 1, 1, 1, &mut dx, &mut dy, &mut dist);
                }
                if x + 1 < w {
                    relax(i, i - w + 1, 1, 1, &mut dx, &mut dy, &mut dist);
                }
            }
        }
    }
    // Backward pass: bottom-to-top, right-to-left (E, SE, S, SW neighbours).
    for y in (0..h).rev() {
        for x in (0..w).rev() {
            let i = y * w + x;
            if x + 1 < w {
                relax(i, i + 1, 1, 0, &mut dx, &mut dy, &mut dist);
            }
            if y + 1 < h {
                relax(i, i + w, 0, 1, &mut dx, &mut dy, &mut dist);
                if x + 1 < w {
                    relax(i, i + w + 1, 1, 1, &mut dx, &mut dy, &mut dist);
                }
                if x > 0 {
                    relax(i, i + w - 1, 1, 1, &mut dx, &mut dy, &mut dist);
                }
            }
        }
    }

    for d in &mut dist {
        if *d >= INF {
            *d = 0.0;
        }
    }
    dist
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];

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
        };
        let entry = MsdfGlyphEntry {
            uv: [[0.0, 0.0], [0.1, 0.0], [0.1, 0.1], [0.0, 0.1]],
            bearing_x: 1.0,
            bearing_y: 2.0,
            width: 40.0,
            height: 48.0,
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
        MsdfGlyphKey { font_index: 0, glyph_id, px: 32 }
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
        let origin_x = (entry.uv[0][0] * plan.atlas_width as f32).round() as u32;
        let origin_y = (entry.uv[0][1] * plan.atlas_height as f32).round() as u32;
        let cell_w = entry.width as u32;
        let cell_h = entry.height as u32;
        let center = sampled_distance(&plan, origin_x + cell_w / 2, origin_y + cell_h / 2);
        let corner = sampled_distance(&plan, origin_x, origin_y);
        assert!(center > 0.5, "glyph interior is inside the outline: {center}");
        assert!(corner < 0.5, "padded corner is outside the outline: {corner}");
        assert!(center > corner, "distance ramps from corner to center");
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
        });
        assert!(result.is_none(), "no room: caller falls back to fontdue raster");
        assert_eq!(plan.glyph_count(), 0);
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
}
