#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::collections::HashMap;

use fontdue::{Font, FontSettings, Metrics};
use rustybuzz::{Face, UnicodeBuffer};
use unicode_segmentation::UnicodeSegmentation;

const LATIN_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/NotoSansKR-RendererLatin.ttf");
const KOREAN_FONT_BYTES: &[u8] = include_bytes!("../assets/fonts/NotoSansKR-RendererKorean.ttf");
const ELLIPSIS: &str = "...";

pub const TEXT_ATLAS_WIDTH: u32 = 2048;
pub const TEXT_ATLAS_HEIGHT: u32 = 2048;
pub const TEXT_ATLAS_SOLID_UV: [[f32; 2]; 4] = [
    [0.00024414062, 0.00024414062],
    [0.00024414062, 0.00024414062],
    [0.00024414062, 0.00024414062],
    [0.00024414062, 0.00024414062],
];

#[derive(Clone, Copy, Default)]
pub struct TextBuildStats {
    pub glyph_count: usize,
    pub fallback_glyph_count: usize,
    pub cjk_glyph_count: usize,
    pub fallback_run_count: usize,
    pub missing_glyph_count: usize,
    pub atlas_overflow_glyph_count: usize,
    pub missing_raster_glyph_count: usize,
}

impl TextBuildStats {
    pub fn add(&mut self, other: TextBuildStats) {
        self.glyph_count += other.glyph_count;
        self.fallback_glyph_count += other.fallback_glyph_count;
        self.cjk_glyph_count += other.cjk_glyph_count;
        self.fallback_run_count += other.fallback_run_count;
        self.missing_glyph_count += other.missing_glyph_count;
        self.atlas_overflow_glyph_count += other.atlas_overflow_glyph_count;
        self.missing_raster_glyph_count += other.missing_raster_glyph_count;
    }
}

#[derive(Clone)]
pub struct CachedTextGlyph {
    pub offset_x: f32,
    pub offset_y: f32,
    pub width: f32,
    pub height: f32,
    pub uv: [[f32; 2]; 4],
    advance_end: f32,
    font_index: usize,
    cjk: bool,
    missing: bool,
}

#[derive(Clone)]
pub struct CachedTextLine {
    pub glyphs: Vec<CachedTextGlyph>,
    pub stats: TextBuildStats,
    pub width: f32,
}

#[derive(Clone, Default)]
pub struct TextLayoutCache {
    lines: HashMap<String, CachedTextLine>,
    wrapped_lines: HashMap<String, Vec<String>>,
    pub hits: usize,
    pub misses: usize,
}

impl TextLayoutCache {
    pub fn text_line(
        &mut self,
        text_engine: &mut TextEngine,
        value: &str,
        max_width: f32,
        font_size: f32,
    ) -> CachedTextLine {
        let key = text_line_cache_key(value, max_width, font_size);
        if let Some(cached) = self.lines.get(&key) {
            self.hits += 1;
            return cached.clone();
        }
        self.misses += 1;
        let line = text_engine.layout_text_line(value, max_width, font_size);
        self.trim_if_needed();
        self.lines.insert(key, line.clone());
        line
    }

    pub fn wrap_lines(
        &mut self,
        text_engine: &TextEngine,
        value: &str,
        max_width: f32,
        font_size: f32,
        max_lines: usize,
    ) -> Vec<String> {
        let key = wrapped_lines_cache_key(value, max_width, font_size, max_lines);
        if let Some(cached) = self.wrapped_lines.get(&key) {
            self.hits += 1;
            return cached.clone();
        }
        self.misses += 1;
        let lines = wrap_text_lines(text_engine, value, max_width, font_size, max_lines);
        self.trim_if_needed();
        self.wrapped_lines.insert(key, lines.clone());
        lines
    }

    fn trim_if_needed(&mut self) {
        if self.lines.len() + self.wrapped_lines.len() <= TEXT_LAYOUT_CACHE_LIMIT {
            return;
        }
        self.lines.clear();
        self.wrapped_lines.clear();
    }
}

pub struct TextEngine {
    fonts: Vec<RendererFont>,
    atlas: TextAtlas,
    raster_cache_hits: usize,
    raster_cache_misses: usize,
}

impl TextEngine {
    pub fn new() -> Result<Self, String> {
        Ok(TextEngine {
            fonts: vec![
                RendererFont::from_bytes("Noto Sans KR Renderer Latin", LATIN_FONT_BYTES)?,
                RendererFont::from_bytes("Noto Sans KR Renderer Korean", KOREAN_FONT_BYTES)?,
            ],
            atlas: TextAtlas::new(),
            raster_cache_hits: 0,
            raster_cache_misses: 0,
        })
    }

    #[cfg(test)]
    fn with_atlas_size(width: u32, height: u32) -> Result<Self, String> {
        let mut engine = Self::new()?;
        engine.atlas = TextAtlas::with_size(width, height);
        Ok(engine)
    }

    pub fn atlas_pixels(&self) -> &[u8] {
        &self.atlas.pixels
    }

    pub fn atlas_glyph_count(&self) -> usize {
        self.atlas.glyphs.len()
    }

    pub fn raster_cache_hits(&self) -> usize {
        self.raster_cache_hits
    }

    pub fn raster_cache_misses(&self) -> usize {
        self.raster_cache_misses
    }

    pub fn take_atlas_dirty(&mut self) -> bool {
        let dirty = self.atlas.dirty;
        self.atlas.dirty = false;
        dirty
    }

    pub fn measure_text_width(&self, value: &str, font_size: f32) -> f32 {
        self.shape_width(value, font_size)
    }

    fn layout_text_line(&mut self, value: &str, max_width: f32, font_size: f32) -> CachedTextLine {
        let mut line = self.shape_visible_line(value, font_size);
        let right_limit = max_width.max(0.0);
        if line.width <= right_limit {
            return line;
        }

        let ellipsis = self.shape_visible_line(ELLIPSIS, font_size);
        let ellipsis_width = ellipsis.width;
        let mut glyphs = Vec::new();
        for glyph in line.glyphs {
            if glyph.advance_end + ellipsis_width <= right_limit {
                glyphs.push(glyph);
            } else {
                break;
            }
        }
        let base_x = glyphs
            .last()
            .map(|glyph| glyph.advance_end)
            .unwrap_or(0.0)
            .min((right_limit - ellipsis_width).max(0.0));
        if ellipsis_width <= right_limit {
            for mut glyph in ellipsis.glyphs {
                glyph.offset_x += base_x;
                glyph.advance_end += base_x;
                glyphs.push(glyph);
            }
        }
        let width = glyphs
            .last()
            .map(|glyph| glyph.advance_end)
            .unwrap_or(0.0)
            .min(right_limit);
        let mut stats = stats_for_glyphs(&glyphs);
        stats.atlas_overflow_glyph_count += line.stats.atlas_overflow_glyph_count;
        stats.atlas_overflow_glyph_count += ellipsis.stats.atlas_overflow_glyph_count;
        stats.missing_raster_glyph_count += line.stats.missing_raster_glyph_count;
        stats.missing_raster_glyph_count += ellipsis.stats.missing_raster_glyph_count;
        line.glyphs = glyphs;
        line.width = width;
        line.stats = stats;
        line
    }

    fn shape_visible_line(&mut self, value: &str, font_size: f32) -> CachedTextLine {
        let mut glyphs = Vec::new();
        let mut failure_stats = TextBuildStats::default();
        let mut cursor_x = 0.0;
        for run in self.text_runs(value) {
            let shaped = self.shape_run(&run.text, run.font_index, font_size);
            for glyph in shaped {
                let advance_end = cursor_x + glyph.x_advance;
                match self.atlas_glyph(run.font_index, glyph.glyph_id, font_size) {
                    Ok(AtlasGlyphLookup::Visible(atlas_glyph)) => {
                        glyphs.push(CachedTextGlyph {
                            offset_x: cursor_x + glyph.x_offset + atlas_glyph.xmin,
                            offset_y: glyph.y_offset + atlas_glyph.line_top,
                            width: atlas_glyph.width as f32,
                            height: atlas_glyph.height as f32,
                            uv: atlas_glyph.uv,
                            advance_end,
                            font_index: run.font_index,
                            cjk: run.cjk,
                            missing: glyph.glyph_id == 0,
                        });
                    }
                    Ok(AtlasGlyphLookup::Blank) => {}
                    Err(AtlasGlyphError::AtlasFull) => {
                        failure_stats.atlas_overflow_glyph_count += 1;
                    }
                    Err(AtlasGlyphError::MissingRaster) => {
                        failure_stats.missing_raster_glyph_count += 1;
                    }
                }
                cursor_x = advance_end;
            }
        }
        let mut stats = stats_for_glyphs(&glyphs);
        stats.add(failure_stats);
        CachedTextLine {
            glyphs,
            stats,
            width: cursor_x,
        }
    }

    fn shape_width(&self, value: &str, font_size: f32) -> f32 {
        let mut width = 0.0;
        for run in self.text_runs(value) {
            width += self
                .shape_run_positions(&run.text, run.font_index, font_size)
                .iter()
                .map(|glyph| glyph.x_advance)
                .sum::<f32>();
        }
        width
    }

    fn text_runs(&self, value: &str) -> Vec<TextRun> {
        let mut runs: Vec<TextRun> = Vec::new();
        for grapheme in value.graphemes(true) {
            if grapheme == "\n" {
                break;
            }
            let font_index = self.select_font_for_grapheme(grapheme).unwrap_or(0);
            let cjk = is_cjk_grapheme(grapheme);
            if let Some(last) = runs.last_mut() {
                if last.font_index == font_index {
                    last.text.push_str(grapheme);
                    last.cjk |= cjk;
                    continue;
                }
            }
            runs.push(TextRun {
                font_index,
                text: grapheme.to_string(),
                cjk,
            });
        }
        runs
    }

    fn select_font_for_grapheme(&self, grapheme: &str) -> Option<usize> {
        if grapheme.chars().all(|ch| ch.is_whitespace()) {
            return Some(0);
        }
        for (font_index, font) in self.fonts.iter().enumerate() {
            if grapheme.chars().all(|ch| {
                ch.is_control() || ch.is_whitespace() || font.raster.lookup_glyph_index(ch) != 0
            }) {
                return Some(font_index);
            }
        }
        None
    }

    fn shape_run(&mut self, value: &str, font_index: usize, font_size: f32) -> Vec<ShapedGlyph> {
        self.shape_run_positions(value, font_index, font_size)
    }

    fn shape_run_positions(
        &self,
        value: &str,
        font_index: usize,
        font_size: f32,
    ) -> Vec<ShapedGlyph> {
        let Some(font) = self.fonts.get(font_index) else {
            return Vec::new();
        };
        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(value);
        let shaped = rustybuzz::shape(&font.face, &[], buffer);
        let scale = font_size / font.face.units_per_em() as f32;
        shaped
            .glyph_infos()
            .iter()
            .zip(shaped.glyph_positions())
            .map(|(info, position)| ShapedGlyph {
                glyph_id: info.glyph_id as u16,
                x_advance: position.x_advance as f32 * scale,
                x_offset: position.x_offset as f32 * scale,
                y_offset: -(position.y_offset as f32) * scale,
            })
            .collect()
    }

    fn atlas_glyph(
        &mut self,
        font_index: usize,
        glyph_id: u16,
        font_size: f32,
    ) -> Result<AtlasGlyphLookup, AtlasGlyphError> {
        let px = quantize_font_size(font_size);
        let key = GlyphRasterKey {
            font_index,
            glyph_id,
            px,
        };
        if let Some(glyph) = self.atlas.glyphs.get(&key) {
            self.raster_cache_hits += 1;
            return Ok(AtlasGlyphLookup::Visible(*glyph));
        }
        self.raster_cache_misses += 1;
        let font = self
            .fonts
            .get(font_index)
            .ok_or(AtlasGlyphError::MissingRaster)?;
        let (metrics, bitmap) = font.raster.rasterize_indexed(glyph_id, px as f32);
        if metrics.width == 0 || metrics.height == 0 || bitmap.is_empty() {
            return Ok(AtlasGlyphLookup::Blank);
        }
        let baseline = font
            .raster
            .horizontal_line_metrics(px as f32)
            .map(|metrics| metrics.ascent)
            .unwrap_or(px as f32 * 0.86);
        self.atlas
            .insert(key, &metrics, &bitmap, baseline)
            .map(AtlasGlyphLookup::Visible)
            .ok_or(AtlasGlyphError::AtlasFull)
    }
}

struct RendererFont {
    #[allow(dead_code)]
    name: &'static str,
    face: Face<'static>,
    raster: Font,
}

impl RendererFont {
    fn from_bytes(name: &'static str, bytes: &'static [u8]) -> Result<Self, String> {
        let face = Face::from_slice(bytes, 0)
            .ok_or_else(|| format!("Failed to parse {name} with rustybuzz"))?;
        let raster = Font::from_bytes(bytes, FontSettings::default())
            .map_err(|error| format!("Failed to parse {name} with fontdue: {error}"))?;
        Ok(RendererFont { name, face, raster })
    }
}

struct TextRun {
    font_index: usize,
    text: String,
    cjk: bool,
}

struct ShapedGlyph {
    glyph_id: u16,
    x_advance: f32,
    x_offset: f32,
    y_offset: f32,
}

#[derive(Clone, Copy)]
struct AtlasGlyph {
    uv: [[f32; 2]; 4],
    xmin: f32,
    line_top: f32,
    width: usize,
    height: usize,
}

#[derive(Clone, Copy)]
enum AtlasGlyphLookup {
    Visible(AtlasGlyph),
    Blank,
}

#[derive(Clone, Copy)]
enum AtlasGlyphError {
    AtlasFull,
    MissingRaster,
}

#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct GlyphRasterKey {
    font_index: usize,
    glyph_id: u16,
    px: u16,
}

struct TextAtlas {
    pixels: Vec<u8>,
    glyphs: HashMap<GlyphRasterKey, AtlasGlyph>,
    width: u32,
    height: u32,
    cursor_x: u32,
    cursor_y: u32,
    row_height: u32,
    dirty: bool,
}

impl TextAtlas {
    fn new() -> Self {
        TextAtlas::with_size(TEXT_ATLAS_WIDTH, TEXT_ATLAS_HEIGHT)
    }

    fn with_size(width: u32, height: u32) -> Self {
        let width = width.max(4);
        let height = height.max(4);
        let mut pixels = vec![0_u8; (width * height * 4) as usize];
        write_pixel(&mut pixels, width, 0, 0, 255);
        TextAtlas {
            pixels,
            glyphs: HashMap::new(),
            width,
            height,
            cursor_x: 2,
            cursor_y: 2,
            row_height: 0,
            dirty: true,
        }
    }

    fn insert(
        &mut self,
        key: GlyphRasterKey,
        metrics: &Metrics,
        bitmap: &[u8],
        baseline: f32,
    ) -> Option<AtlasGlyph> {
        let padding = 1;
        let width = metrics.width as u32;
        let height = metrics.height as u32;
        let packed_width = width + padding * 2;
        let packed_height = height + padding * 2;
        if self.cursor_x + packed_width >= self.width {
            self.cursor_x = 2;
            self.cursor_y += self.row_height + padding;
            self.row_height = 0;
        }
        if self.cursor_y + packed_height >= self.height {
            return None;
        }

        let origin_x = self.cursor_x + padding;
        let origin_y = self.cursor_y + padding;
        for row in 0..height {
            for col in 0..width {
                let alpha = bitmap[(row * width + col) as usize];
                write_pixel(
                    &mut self.pixels,
                    self.width,
                    origin_x + col,
                    origin_y + row,
                    alpha,
                );
            }
        }
        self.cursor_x += packed_width + padding;
        self.row_height = self.row_height.max(packed_height);
        self.dirty = true;

        let left = origin_x as f32 / self.width as f32;
        let right = (origin_x + width) as f32 / self.width as f32;
        let top = origin_y as f32 / self.height as f32;
        let bottom = (origin_y + height) as f32 / self.height as f32;
        let glyph = AtlasGlyph {
            uv: [[left, top], [right, top], [right, bottom], [left, bottom]],
            xmin: metrics.xmin as f32,
            line_top: baseline - metrics.ymin as f32 - metrics.height as f32,
            width: metrics.width,
            height: metrics.height,
        };
        self.glyphs.insert(key, glyph);
        Some(glyph)
    }
}

fn write_pixel(pixels: &mut [u8], width: u32, x: u32, y: u32, alpha: u8) {
    let index = ((y * width + x) * 4) as usize;
    pixels[index] = 255;
    pixels[index + 1] = 255;
    pixels[index + 2] = 255;
    pixels[index + 3] = alpha;
}

fn text_line_cache_key(value: &str, max_width: f32, font_size: f32) -> String {
    format!(
        "line:{}:{}:{}",
        quantize_text_metric(max_width),
        quantize_text_metric(font_size),
        value
    )
}

fn wrapped_lines_cache_key(
    value: &str,
    max_width: f32,
    font_size: f32,
    max_lines: usize,
) -> String {
    format!(
        "wrap:{}:{}:{}:{}",
        quantize_text_metric(max_width),
        quantize_text_metric(font_size),
        max_lines,
        value
    )
}

fn quantize_text_metric(value: f32) -> i32 {
    (value * 100.0).round() as i32
}

fn quantize_font_size(value: f32) -> u16 {
    value.clamp(1.0, 256.0).round() as u16
}

fn wrap_text_lines(
    text_engine: &TextEngine,
    value: &str,
    max_width: f32,
    font_size: f32,
    max_lines: usize,
) -> Vec<String> {
    let max_lines = max_lines.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0.0;

    for segment in value.split_word_bounds() {
        append_segment_with_newlines(
            text_engine,
            &mut lines,
            &mut line,
            &mut line_width,
            segment,
            max_width,
            font_size,
            max_lines,
        );
        if lines.len() >= max_lines {
            ellipsize_last_line(text_engine, &mut lines, max_width, font_size, true);
            return lines;
        }
    }

    push_current_line(&mut lines, &mut line, &mut line_width, max_lines);
    if lines.len() >= max_lines {
        ellipsize_last_line(text_engine, &mut lines, max_width, font_size, false);
    }
    lines
}

fn append_segment_with_newlines(
    text_engine: &TextEngine,
    lines: &mut Vec<String>,
    line: &mut String,
    line_width: &mut f32,
    segment: &str,
    max_width: f32,
    font_size: f32,
    max_lines: usize,
) {
    let mut start = 0;
    for (index, ch) in segment.char_indices() {
        if ch == '\n' {
            append_wrapped_segment(
                text_engine,
                lines,
                line,
                line_width,
                &segment[start..index],
                max_width,
                font_size,
                max_lines,
            );
            push_current_line(lines, line, line_width, max_lines);
            if lines.len() >= max_lines {
                return;
            }
            start = index + ch.len_utf8();
        }
    }
    append_wrapped_segment(
        text_engine,
        lines,
        line,
        line_width,
        &segment[start..],
        max_width,
        font_size,
        max_lines,
    );
}

fn append_wrapped_segment(
    text_engine: &TextEngine,
    lines: &mut Vec<String>,
    line: &mut String,
    line_width: &mut f32,
    segment: &str,
    max_width: f32,
    font_size: f32,
    max_lines: usize,
) {
    if segment.is_empty() || lines.len() >= max_lines {
        return;
    }
    if segment.chars().all(|ch| ch.is_whitespace()) {
        append_space(
            text_engine,
            lines,
            line,
            line_width,
            max_width,
            font_size,
            max_lines,
        );
        return;
    }

    let segment_width = text_engine.measure_text_width(segment, font_size);
    if segment_width <= max_width {
        if !line.is_empty() && *line_width + segment_width > max_width {
            push_current_line(lines, line, line_width, max_lines);
        }
        if lines.len() < max_lines {
            line.push_str(segment);
            *line_width += segment_width;
        }
        return;
    }

    for grapheme in segment.graphemes(true) {
        append_grapheme(
            text_engine,
            lines,
            line,
            line_width,
            grapheme,
            max_width,
            font_size,
            max_lines,
        );
        if lines.len() >= max_lines {
            return;
        }
    }
}

fn append_grapheme(
    text_engine: &TextEngine,
    lines: &mut Vec<String>,
    line: &mut String,
    line_width: &mut f32,
    grapheme: &str,
    max_width: f32,
    font_size: f32,
    max_lines: usize,
) {
    if grapheme == "\n" {
        push_current_line(lines, line, line_width, max_lines);
        return;
    }
    if grapheme.chars().all(|ch| ch.is_whitespace()) {
        append_space(
            text_engine,
            lines,
            line,
            line_width,
            max_width,
            font_size,
            max_lines,
        );
        return;
    }

    let width = text_engine.measure_text_width(grapheme, font_size);
    if !line.is_empty() && *line_width + width > max_width {
        push_current_line(lines, line, line_width, max_lines);
    }
    if lines.len() < max_lines {
        line.push_str(grapheme);
        *line_width += width;
    }
}

fn append_space(
    text_engine: &TextEngine,
    lines: &mut Vec<String>,
    line: &mut String,
    line_width: &mut f32,
    max_width: f32,
    font_size: f32,
    max_lines: usize,
) {
    if line.is_empty() || lines.len() >= max_lines {
        return;
    }
    let width = text_engine.measure_text_width(" ", font_size);
    if *line_width + width > max_width {
        push_current_line(lines, line, line_width, max_lines);
        return;
    }
    line.push(' ');
    *line_width += width;
}

fn push_current_line(
    lines: &mut Vec<String>,
    line: &mut String,
    line_width: &mut f32,
    max_lines: usize,
) {
    if lines.len() < max_lines {
        let trimmed = line.trim_end();
        if !trimmed.is_empty() {
            lines.push(trimmed.to_string());
        }
    }
    line.clear();
    *line_width = 0.0;
}

fn ellipsize_last_line(
    text_engine: &TextEngine,
    lines: &mut [String],
    max_width: f32,
    font_size: f32,
    force: bool,
) {
    let Some(line) = lines.last_mut() else {
        return;
    };
    let ellipsis_width = text_engine.measure_text_width(ELLIPSIS, font_size);
    if !force && text_engine.measure_text_width(line, font_size) <= max_width {
        return;
    }
    while !line.is_empty()
        && text_engine.measure_text_width(line, font_size) + ellipsis_width > max_width
    {
        line.pop();
    }
    line.push_str(ELLIPSIS);
}

fn stats_for_glyphs(glyphs: &[CachedTextGlyph]) -> TextBuildStats {
    let mut stats = TextBuildStats::default();
    let mut in_fallback_run = false;
    for glyph in glyphs {
        stats.glyph_count += 1;
        if glyph.font_index != 0 {
            stats.fallback_glyph_count += 1;
            if !in_fallback_run {
                stats.fallback_run_count += 1;
            }
            in_fallback_run = true;
        } else {
            in_fallback_run = false;
        }
        if glyph.cjk {
            stats.cjk_glyph_count += 1;
        }
        if glyph.missing {
            stats.missing_glyph_count += 1;
        }
    }
    stats
}

fn is_cjk_grapheme(grapheme: &str) -> bool {
    grapheme.chars().any(is_cjk_char)
}

fn is_cjk_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{1100}'..='\u{11ff}'
            | '\u{2e80}'..='\u{2eff}'
            | '\u{3000}'..='\u{303f}'
            | '\u{3040}'..='\u{30ff}'
            | '\u{3130}'..='\u{318f}'
            | '\u{31f0}'..='\u{31ff}'
            | '\u{3400}'..='\u{4dbf}'
            | '\u{4e00}'..='\u{9fff}'
            | '\u{a960}'..='\u{a97f}'
            | '\u{ac00}'..='\u{d7af}'
            | '\u{d7b0}'..='\u{d7ff}'
            | '\u{f900}'..='\u{faff}'
            | '\u{ff00}'..='\u{ffef}'
    )
}

const TEXT_LAYOUT_CACHE_LIMIT: usize = 4096;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shaped_width_supports_korean_and_latin() {
        let engine = TextEngine::new().unwrap();

        assert!(engine.measure_text_width("한글", 18.0) > engine.measure_text_width("AB", 18.0));
        assert!(engine.measure_text_width("Rust 한글", 18.0) > 0.0);
    }

    #[test]
    fn wrapped_lines_break_no_space_korean_and_add_ellipsis() {
        let engine = TextEngine::new().unwrap();

        let lines = wrap_text_lines(&engine, "한글테스트문장공백없음입니다", 72.0, 18.0, 2);

        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with("..."));
        assert!(engine.measure_text_width(&lines[0], 18.0) <= 72.0);
    }

    #[test]
    fn shaped_line_tracks_font_fallback_cjk_and_atlas_cache() {
        let mut engine = TextEngine::new().unwrap();

        let line = engine.layout_text_line("A한B", 300.0, 18.0);
        let first_misses = engine.raster_cache_misses();
        let cached = engine.layout_text_line("A한B", 300.0, 18.0);

        assert_eq!(line.stats.glyph_count, 3);
        assert_eq!(line.stats.fallback_glyph_count, 1);
        assert_eq!(line.stats.fallback_run_count, 1);
        assert_eq!(line.stats.cjk_glyph_count, 1);
        assert_eq!(line.stats.missing_glyph_count, 0);
        assert!(engine.atlas_glyph_count() >= 3);
        assert!(engine.raster_cache_hits() >= 3);
        assert_eq!(engine.raster_cache_misses(), first_misses);
        assert_eq!(cached.stats.glyph_count, 3);
    }

    #[test]
    fn shaped_line_handles_latin_punctuation_without_fallback() {
        let mut engine = TextEngine::new().unwrap();

        let line = engine.layout_text_line("Plan: A, B? Yes!", 300.0, 18.0);

        assert!(line.stats.glyph_count >= 12);
        assert_eq!(line.stats.fallback_glyph_count, 0);
        assert_eq!(line.stats.cjk_glyph_count, 0);
        assert_eq!(line.stats.missing_glyph_count, 0);
        assert_eq!(line.stats.atlas_overflow_glyph_count, 0);
        assert_eq!(line.stats.missing_raster_glyph_count, 0);
    }

    #[test]
    fn tiny_text_atlas_reports_overflowed_glyphs() {
        let mut engine = TextEngine::with_atlas_size(8, 8).unwrap();

        let line = engine.layout_text_line("Overflow", 600.0, 18.0);

        assert_eq!(line.stats.glyph_count, 0);
        assert!(line.stats.atlas_overflow_glyph_count > 0);
        assert_eq!(line.stats.missing_raster_glyph_count, 0);
    }
}
