//! Text positions, carets, and selections across a label's lines.
//! Coordinates are in points relative to the top-left corner of the label's [`TextBounds`].
//! Positions use byte offsets in the label source. Equations select as whole units and copy as
//! Typst markup.

use std::ops::Range;

use unicode_segmentation::UnicodeSegmentation;

use super::bounds::{TextBounds, first_baseline};
use super::engine::{CompiledLabel, LabelEquation};
use super::frame::{TextItem, Transform};

/// The side of a source offset used to place a caret at a line wrap or text-direction boundary,
/// where one offset has two visual positions.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Affinity {
    /// Associate the position with the text after the offset.
    #[default]
    Downstream,
    /// Associate the position with the text before the offset.
    Upstream,
}

/// A source byte offset and its visual affinity. Valid positions lie on grapheme boundaries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct TextPosition {
    /// Byte offset in the label source.
    pub offset: usize,
    /// Visual side of the offset at a wrap or direction boundary.
    pub affinity: Affinity,
}

impl TextPosition {
    /// Create a position without validating the offset against a label source.
    pub fn new(offset: usize, affinity: Affinity) -> Self {
        Self { offset, affinity }
    }
}

/// A rectangle in points relative to the top-left corner of the label box.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TextRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

/// A compiled label's geometry and source ranges for hit testing and selection.
/// Lines are ordered top to bottom, with clusters ordered left to right within each line.
#[derive(Debug, Clone, PartialEq)]
pub struct TextLayout {
    source: String,
    lines: Vec<Line>,
    /// Width of the highlight for an empty selected line.
    empty_width: f32,
}

/// A line's bounds, source range, and clusters in label coordinates.
#[derive(Debug, Clone, PartialEq)]
struct Line {
    /// Vertical line bounds, padded to at least the font size.
    top: f32,
    bottom: f32,
    /// Horizontal line start, used to place the caret on an empty line.
    left: f32,
    /// Source byte range, including any trailing line break.
    source: Range<usize>,
    /// Whether an explicit break ends the line, rather than a wrap or the end of the text.
    breaks: bool,
    /// The line's clusters in visual order.
    clusters: Vec<Cluster>,
}

/// Glyphs with a shared source range, or a whole equation.
#[derive(Debug, Clone, PartialEq)]
struct Cluster {
    source: Range<usize>,
    /// Rendered glyph text or equation markup.
    text: String,
    left: f32,
    width: f32,
    rtl: bool,
    /// Whether the rendered text matches the source, allowing positions between graphemes.
    /// Other clusters, such as formatted numbers and equations, select as whole units.
    verbatim: bool,
}

impl Cluster {
    fn right(&self) -> f32 {
        self.left + self.width
    }

    /// Return valid source offsets within the cluster, in logical order.
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

    /// Return the horizontal position of a source offset within the cluster.
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

    /// Return the cluster boundary nearest `x`, with upstream affinity at the source end and
    /// downstream affinity elsewhere.
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

    /// Return the position at the cluster's left or right edge.
    fn edge(&self, right: bool) -> TextPosition {
        if right != self.rtl {
            TextPosition::new(self.source.end, Affinity::Upstream)
        } else {
            TextPosition::new(self.source.start, Affinity::Downstream)
        }
    }
}

impl TextLayout {
    /// Build selection geometry from a compiled label with a font size in points.
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
                    source: source.range.clone(),
                    breaks: source.breaks,
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

    /// Return the label source used by position and range offsets.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Return the position nearest `point` on the line at its height, or on the nearest line.
    /// Points beyond the line's text map to the text boundary on that side. An empty line maps
    /// to its source start.
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

    /// Return a zero-width caret with the height of the line at `position`.
    /// Affinity chooses the cluster at a shared source boundary, including the preceding line's
    /// end or the following line's start at a wrap.
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

    /// Return highlight rectangles for a source byte range.
    /// Each line has one rectangle per contiguous visual span of selected clusters. Mixed text
    /// directions can produce several rectangles. An empty selected line gets a narrow
    /// rectangle. A range that overlaps an equation or a formatted value covers all of it, as
    /// [`TextLayout::text_in`] copies all of it.
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
                // A range that overlaps a cluster that doesn't show its source takes it whole,
                // as the selected text does.
                let (start, end) = match cluster.verbatim {
                    true => (start, end),
                    false => (cluster.source.start, cluster.source.end),
                };
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

    /// Return the source byte range selected by a double press at `position`.
    /// Select a Unicode word, or a grapheme between words. Formatted values and equations
    /// select as whole units.
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

    /// Return the source byte range of the line at `position`, excluding its trailing break.
    /// An empty line returns an empty range at its source start.
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

    /// Extract selected text in reading order from a source byte range.
    /// Text that matches its source can be selected partially. Other clusters, including
    /// equations, are included once in full. Equations copy as Typst markup, even when they
    /// span lines. Each explicit line break that the range selects copies as a newline, and a
    /// wrap adds no separator.
    ///
    /// # Panics
    ///
    /// Panics if a range boundary splits a UTF-8 character in selected text that matches its
    /// source.
    pub fn text_in(&self, range: Range<usize>) -> String {
        let mut text = String::new();
        let mut taken_whole: Vec<&Range<usize>> = Vec::new();
        for line in &self.lines {
            let mut clusters: Vec<&Cluster> = line.clusters.iter().collect();
            clusters.sort_by_key(|cluster| cluster.source.start);
            for cluster in clusters {
                let start = range.start.max(cluster.source.start);
                let end = range.end.min(cluster.source.end);
                if start >= end {
                    continue;
                }
                if cluster.verbatim {
                    text.push_str(&self.source[start..end]);
                } else if !taken_whole.contains(&&cluster.source) {
                    taken_whole.push(&cluster.source);
                    text.push_str(&cluster.text);
                }
            }
            // An explicit break's source follows the line's last cluster, and takes at least one
            // byte, so that a range that runs past the line's text selects the break.
            let text_end = (line.clusters.iter().map(|c| c.source.end).max())
                .unwrap_or(line.source.start)
                .max(line.source.start);
            let breaking = text_end..line.source.end.max(text_end + 1);
            if line.breaks && range.start < breaking.end && breaking.start < range.end {
                text.push('\n');
            }
        }
        text
    }

    /// Find the line at height `y`, or the line with the nearest vertical bounds.
    fn line_at_y(&self, y: f32) -> Option<&Line> {
        let distance = |line: &Line| (line.top - y).max(y - line.bottom).max(0.0);
        self.lines.iter().min_by(|a, b| distance(a).total_cmp(&distance(b)))
    }

    /// Find the line whose source range contains `offset`, falling back to the last line.
    fn line_of(&self, offset: usize) -> Option<&Line> {
        self.lines
            .iter()
            .find(|line| line.source.start <= offset && offset < line.source.end)
            .or_else(|| self.lines.last())
    }

    /// Find the line and cluster containing `position`, using affinity to choose at a shared
    /// source boundary.
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
        // An equation that wraps has a cluster with its whole source on each line, so a
        // position at its start takes its first line, and one at its end its last.
        let first_start = || clusters().find(starts);
        let last_end = || clusters().rev().find(ends);
        clusters().find(inside).or_else(|| match position.affinity {
            Affinity::Downstream => first_start().or_else(last_end),
            Affinity::Upstream => last_end().or_else(first_start),
        })
    }
}

/// Return whether an equation's source range contains `source`.
fn in_equation(equations: &[LabelEquation], source: &Range<usize>) -> bool {
    equations.iter().any(|equation| {
        equation.source.start <= source.start && source.end <= equation.source.end
    })
}

/// Find the line containing `baseline`, or the line with the nearest vertical bounds.
fn nearest_line(lines: &mut [Line], baseline: f32) -> Option<&mut Line> {
    let distance =
        |line: &Line| (line.top - baseline).max(baseline - line.bottom).max(0.0);
    lines.iter_mut().min_by(|a, b| distance(a).total_cmp(&distance(b)))
}

/// Group glyphs by source range into clusters in label coordinates. Retain their rendered text
/// and skip glyphs without a source range.
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

/// Merge adjacent clusters with the same source range, such as parts of one formatted value.
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
