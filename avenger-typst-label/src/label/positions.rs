//! Positions in a label's text: the source position at a point, and the caret and the selection
//! at positions, across the label's lines. Points are in the label's box, as [`TextBounds`]
//! gives it and the outputs draw it, with its top left at the origin. An equation is one unit,
//! which selects whole and copies as its markup.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use super::bounds::{TextBounds, first_baseline};
use super::engine::{CompiledLabel, LabelEquation};
use super::frame::{TextItem, Transform};

/// Which side of an offset a position belongs to, where one offset lies in two places: the end
/// of one run of text and the start of the next, as at a change of direction or a wrap.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Affinity {
    /// With the text after the offset.
    #[default]
    Downstream,
    /// With the text before the offset.
    Upstream,
}

/// A position in a label's text: a byte offset in its source, on a grapheme boundary, and the
/// side of the offset it belongs to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TextPosition {
    pub offset: usize,
    pub affinity: Affinity,
}

impl TextPosition {
    pub fn new(offset: usize, affinity: Affinity) -> Self {
        Self { offset, affinity }
    }
}

/// A rectangle in a label's box.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TextRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A compiled label's text, laid out for positions: its lines, top to bottom, and each line's
/// clusters, left to right, with the source they come from.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayout {
    source: String,
    lines: Vec<Line>,
    /// An empty line's selection width.
    empty_width: f32,
}

/// A line of a label, in the label's box.
#[derive(Debug, Clone, PartialEq)]
struct Line {
    /// The line's box, its text's edges padded to at least the font size, as a label's box
    /// pads its first and last lines.
    top: f32,
    bottom: f32,
    /// Where the line starts, which places an empty line's caret.
    left: f32,
    /// The source of the line's text, with the break that ends it.
    source: Range<usize>,
    /// The line's clusters in visual order.
    clusters: Vec<Cluster>,
}

/// One cluster of a line's text: glyphs that come from one source, or an equation.
#[derive(Debug, Clone, PartialEq)]
struct Cluster {
    source: Range<usize>,
    /// The text that the glyphs show, or an equation's markup.
    text: String,
    left: f32,
    width: f32,
    rtl: bool,
    /// Whether the cluster shows its source text, so positions fall between its graphemes.
    /// Other clusters, such as a formatted number's or an equation, select whole.
    verbatim: bool,
}

impl Cluster {
    fn right(&self) -> f32 {
        self.left + self.width
    }

    /// The offsets that positions in the cluster can take, in logical order.
    fn boundaries(&self, source: &str) -> Vec<usize> {
        let range = self.source.clone();
        let mut boundaries = vec![range.start];
        if self.verbatim {
            let inner = source[range.clone()].grapheme_indices(true).skip(1);
            boundaries.extend(inner.map(|(offset, _)| range.start + offset));
        }
        boundaries.push(range.end);
        boundaries
    }

    /// Where `offset`, one of the cluster's boundaries, lies across the cluster.
    fn x_at(&self, source: &str, offset: usize) -> f32 {
        let boundaries = self.boundaries(source);
        let index = boundaries
            .iter()
            .position(|boundary| *boundary >= offset)
            .unwrap_or(boundaries.len() - 1);
        let fraction = index as f32 / (boundaries.len() - 1) as f32;
        if self.rtl {
            self.right() - self.width * fraction
        } else {
            self.left + self.width * fraction
        }
    }

    /// The boundary of the cluster nearest `x`, and the side of it that `x` lies on.
    fn position_at(&self, source: &str, x: f32) -> TextPosition {
        let boundaries = self.boundaries(source);
        let slots = boundaries.len() - 1;
        let fraction = ((x - self.left) / self.width.max(f32::EPSILON)).clamp(0.0, 1.0);
        let visual = (fraction * slots as f32).round() as usize;
        let index = if self.rtl { slots - visual } else { visual };
        let offset = boundaries[index];
        let affinity = if offset == self.source.end {
            Affinity::Upstream
        } else {
            Affinity::Downstream
        };
        TextPosition::new(offset, affinity)
    }

    /// The position at the cluster's left or right edge.
    fn edge(&self, right: bool) -> TextPosition {
        if right != self.rtl {
            TextPosition::new(self.source.end, Affinity::Upstream)
        } else {
            TextPosition::new(self.source.start, Affinity::Downstream)
        }
    }
}

impl TextLayout {
    /// The layout of a compiled label whose text is `font_size` points.
    pub fn new(label: &CompiledLabel, font_size: f32) -> Self {
        let bounds = TextBounds::new(&label.metrics, font_size);
        let down = bounds.ascent - first_baseline(&label.metrics);
        let size = font_size.max(1.0);
        let mut lines: Vec<Line> = label
            .metrics
            .lines
            .iter()
            .zip(&label.line_sources)
            .map(|(line, source)| {
                let pad = (size - (line.bottom - line.top)).max(0.0) / 2.0;
                Line {
                    top: down + line.top - pad,
                    bottom: down + line.bottom + pad,
                    left: line.left,
                    source: source.clone(),
                    clusters: Vec::new(),
                }
            })
            .collect();
        for (ts, item) in label.frame.text_items() {
            let baseline = down + ts.ty;
            let Some(line) = nearest_line(&mut lines, baseline) else {
                continue;
            };
            // An equation's glyphs take no positions of their own.
            let outside =
                |cluster: &Cluster| !in_equation(&label.equations, &cluster.source);
            line.clusters
                .extend(clusters(ts, item, &label.source).into_iter().filter(outside));
        }
        for equation in &label.equations {
            let Some(line) = lines.get_mut(equation.line) else {
                continue;
            };
            line.clusters.push(Cluster {
                source: equation.source.clone(),
                text: equation.markup.clone(),
                left: equation.left,
                width: equation.right - equation.left,
                rtl: false,
                verbatim: false,
            });
        }
        for line in &mut lines {
            line.clusters.sort_by(|a, b| a.left.total_cmp(&b.left));
            merge_shared_sources(&mut line.clusters);
        }
        Self {
            source: label.source.clone(),
            lines,
            empty_width: size / 4.0,
        }
    }

    /// The label's source, which positions are offsets in.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// The position at `point`: on the line at its height, or the nearest line above or below
    /// every line, at the boundary nearest its x. A point past a line's text takes the text's
    /// end on that side.
    pub fn position_at(&self, point: [f32; 2]) -> TextPosition {
        let Some(line) = self.line_at_y(point[1]) else {
            return TextPosition::default();
        };
        let (Some(first), Some(last)) = (line.clusters.first(), line.clusters.last())
        else {
            return TextPosition::new(line.source.start, Affinity::Downstream);
        };
        let x = point[0];
        if x <= first.left {
            return first.edge(false);
        }
        if x >= last.right() {
            return last.edge(true);
        }
        let cluster = line
            .clusters
            .iter()
            .find(|cluster| x <= cluster.right())
            .unwrap_or(last);
        if x < cluster.left {
            // In the gap before the cluster: the nearer edge of the two clusters.
            let index = line.clusters.iter().position(|c| c == cluster).unwrap_or(0);
            let before = &line.clusters[index.saturating_sub(1)];
            return if x - before.right() < cluster.left - x {
                before.edge(true)
            } else {
                cluster.edge(false)
            };
        }
        cluster.position_at(&self.source, x)
    }

    /// The caret at `position`: a line's height at the offset's place across the line. At an
    /// offset that ends one cluster and starts another, the affinity picks the cluster, as it
    /// picks the upper line's end or the lower line's start at a wrap.
    pub fn caret(&self, position: TextPosition) -> TextRect {
        let (line, x) = match self.place(position) {
            Some((line, cluster)) => (line, cluster.x_at(&self.source, position.offset)),
            // An offset between clusters, such as a break's: the end of its line's text, or an
            // empty line's start.
            None => match self.line_of(position.offset) {
                Some(line) => {
                    let end = line.clusters.iter().max_by_key(|c| c.source.end);
                    (line, end.map_or(line.left, |c| c.x_at(&self.source, c.source.end)))
                }
                None => return TextRect::default(),
            },
        };
        TextRect {
            x,
            y: line.top,
            width: 0.0,
            height: line.bottom - line.top,
        }
    }

    /// The rectangles that show `range` selected: on each line, one for each run of selected
    /// clusters that lie side by side, so mixed directions take several, and a small one on an
    /// empty line in the range.
    pub fn selection_rects(&self, range: Range<usize>) -> Vec<TextRect> {
        let mut rects = Vec::new();
        if range.is_empty() {
            return rects;
        }
        for line in &self.lines {
            let height = line.bottom - line.top;
            if line.clusters.is_empty() {
                if range.start <= line.source.start && line.source.start < range.end {
                    rects.push(TextRect {
                        x: line.left,
                        y: line.top,
                        width: self.empty_width,
                        height,
                    });
                }
                continue;
            }
            let mut spans: Vec<(f32, f32)> = Vec::new();
            for cluster in &line.clusters {
                let start = range.start.max(cluster.source.start);
                let end = range.end.min(cluster.source.end);
                if start >= end {
                    continue;
                }
                let a = cluster.x_at(&self.source, start);
                let b = cluster.x_at(&self.source, end);
                let (left, right) = (a.min(b), a.max(b));
                match spans.last_mut() {
                    Some(last) if left - last.1 < 0.01 => last.1 = last.1.max(right),
                    _ => spans.push((left, right)),
                }
            }
            rects.extend(spans.into_iter().map(|(left, right)| TextRect {
                x: left,
                y: line.top,
                width: right - left,
                height,
            }));
        }
        rects
    }

    /// The word at `position`, as a double press selects it: the source's word around the
    /// offset, or its grapheme between words, or the whole of a cluster that selects whole.
    pub fn word_at(&self, position: TextPosition) -> Range<usize> {
        if let Some((_, cluster)) = self.place(position)
            && !cluster.verbatim
        {
            return cluster.source.clone();
        }
        let source = self.source.as_str();
        let offset = position.offset.min(source.len());
        let word = source.unicode_word_indices().find(|(start, word)| {
            let end = start + word.len();
            *start <= offset && offset < end
                || position.affinity == Affinity::Upstream && offset == end
        });
        if let Some((start, word)) = word {
            return start..start + word.len();
        }
        let grapheme = source
            .grapheme_indices(true)
            .find(|(start, grapheme)| offset < start + grapheme.len());
        grapheme.map_or(offset..offset, |(start, grapheme)| start..start + grapheme.len())
    }

    /// The text on the line at `position`, as a triple press selects it: from its first
    /// cluster's source to its last's, without the break that ends it.
    pub fn line_at(&self, position: TextPosition) -> Range<usize> {
        let line = match self.place(position) {
            Some((line, _)) => Some(line),
            None => self.line_of(position.offset),
        };
        let Some(line) = line else {
            return position.offset..position.offset;
        };
        let start = line.clusters.iter().map(|c| c.source.start).min();
        let end = line.clusters.iter().map(|c| c.source.end).max();
        match (start, end) {
            (Some(start), Some(end)) => start..end,
            _ => line.source.start..line.source.start,
        }
    }

    /// The text that `range` selects, as the label shows it: each line's selected clusters in
    /// reading order, with part of a cluster that shows its source, and the whole of one that
    /// doesn't, once, so an equation that breaks across lines gives its markup once. A newline
    /// ends each line that a break in the source ends, and a line that wraps runs on into the
    /// next.
    pub fn text_in(&self, range: Range<usize>) -> String {
        let mut text = String::new();
        let mut ends_line = false;
        let mut taken_whole: Vec<&Range<usize>> = Vec::new();
        for line in &self.lines {
            let mut clusters: Vec<&Cluster> = line.clusters.iter().collect();
            clusters.sort_by_key(|cluster| cluster.source.start);
            let mut selected = String::new();
            for cluster in clusters {
                let start = range.start.max(cluster.source.start);
                let end = range.end.min(cluster.source.end);
                if start >= end {
                    continue;
                }
                if cluster.verbatim {
                    selected.push_str(&self.source[start..end]);
                } else if !taken_whole.contains(&&cluster.source) {
                    taken_whole.push(&cluster.source);
                    selected.push_str(&cluster.text);
                }
            }
            let empty_selected = line.clusters.is_empty()
                && range.start <= line.source.start
                && line.source.start < range.end;
            if selected.is_empty() && !empty_selected {
                continue;
            }
            if ends_line {
                text.push('\n');
            }
            text.push_str(&selected);
            ends_line = self.breaks(line);
        }
        text
    }

    /// The line at height `y`, or the nearest one.
    fn line_at_y(&self, y: f32) -> Option<&Line> {
        let distance = |line: &Line| (line.top - y).max(y - line.bottom).max(0.0);
        self.lines.iter().min_by(|a, b| distance(a).total_cmp(&distance(b)))
    }

    /// The line whose source holds `offset`, or the last line.
    fn line_of(&self, offset: usize) -> Option<&Line> {
        self.lines
            .iter()
            .find(|line| line.source.start <= offset && offset < line.source.end)
            .or_else(|| self.lines.last())
    }

    /// The line and cluster where `position` lies: a cluster whose boundaries hold the offset,
    /// preferring the one that its affinity faces.
    fn place(&self, position: TextPosition) -> Option<(&Line, &Cluster)> {
        let offset = position.offset;
        let clusters = || {
            self.lines
                .iter()
                .flat_map(|line| line.clusters.iter().map(move |cluster| (line, cluster)))
        };
        let inside = |(_, cluster): &(&Line, &Cluster)| {
            cluster.source.start < offset && offset < cluster.source.end
        };
        let starts = |(_, cluster): &(&Line, &Cluster)| cluster.source.start == offset;
        let ends = |(_, cluster): &(&Line, &Cluster)| cluster.source.end == offset;
        clusters().find(inside).or_else(|| match position.affinity {
            Affinity::Downstream => {
                clusters().find(starts).or_else(|| clusters().find(ends))
            }
            Affinity::Upstream => {
                clusters().find(ends).or_else(|| clusters().find(starts))
            }
        })
    }

    /// Whether a break in the source ends `line`, rather than a wrap: the source between its
    /// last cluster and its end holds a newline, or markup's `\`.
    fn breaks(&self, line: &Line) -> bool {
        let end = line.clusters.iter().map(|c| c.source.end).max();
        let rest =
            end.unwrap_or(line.source.start).max(line.source.start)..line.source.end;
        self.source
            .get(rest)
            .is_some_and(|rest| rest.contains('\n') || rest.contains('\\'))
    }
}

/// Whether `source` lies in an equation's source.
fn in_equation(equations: &[LabelEquation], source: &Range<usize>) -> bool {
    equations.iter().any(|equation| {
        equation.source.start <= source.start && source.end <= equation.source.end
    })
}

/// The line whose box holds `baseline`, or the line with the nearest baseline.
fn nearest_line(lines: &mut [Line], baseline: f32) -> Option<&mut Line> {
    let distance =
        |line: &Line| (line.top - baseline).max(baseline - line.bottom).max(0.0);
    lines.iter_mut().min_by(|a, b| distance(a).total_cmp(&distance(b)))
}

/// A text item's clusters, at their places in the label's box: its glyphs merged by their
/// source, with the text that they show. Glyphs without a source, such as those of a missing
/// glyph, have no positions.
fn clusters(ts: Transform, item: &TextItem, source: &str) -> Vec<Cluster> {
    let rtl = item.is_rtl();
    // Each cluster with the range of the item's text that its glyphs show.
    let mut clusters: Vec<(Cluster, Range<usize>)> = Vec::new();
    let mut pen = 0.0;
    for glyph in &item.glyphs {
        let advance = glyph.x_advance * item.size;
        let (a, b) = (ts.tx + ts.sx * pen, ts.tx + ts.sx * (pen + advance));
        pen += advance;
        if glyph.source.is_empty() {
            continue;
        }
        let (left, right) = (a.min(b), a.max(b));
        match clusters
            .iter_mut()
            .find(|(cluster, _)| cluster.source == glyph.source)
        {
            Some((cluster, range)) => {
                let right = cluster.right().max(right);
                cluster.left = cluster.left.min(left);
                cluster.width = right - cluster.left;
                *range =
                    range.start.min(glyph.range.start)..range.end.max(glyph.range.end);
            }
            None => clusters.push((
                Cluster {
                    source: glyph.source.clone(),
                    text: String::new(),
                    left,
                    width: right - left,
                    rtl,
                    verbatim: false,
                },
                glyph.range.clone(),
            )),
        }
    }
    clusters
        .into_iter()
        .map(|(mut cluster, range)| {
            cluster.text = item.text.get(range).unwrap_or_default().to_string();
            cluster.verbatim =
                source.get(cluster.source.clone()) == Some(cluster.text.as_str());
            cluster
        })
        .collect()
}

/// Merge side-by-side clusters from one source node, such as the items of one formatted
/// value, into one cluster.
fn merge_shared_sources(clusters: &mut Vec<Cluster>) {
    let mut merged: Vec<Cluster> = Vec::with_capacity(clusters.len());
    for cluster in clusters.drain(..) {
        match merged.last_mut() {
            Some(last) if last.source == cluster.source => {
                let right = last.right().max(cluster.right());
                last.width = right - last.left;
                last.text.push_str(&cluster.text);
            }
            _ => merged.push(cluster),
        }
    }
    *clusters = merged;
}
