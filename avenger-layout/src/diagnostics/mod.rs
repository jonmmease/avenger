//! Diagnostic images of a solution: where every rectangle is, and why tracks and gaps have
//! their sizes.
//!
//! [`diagnostic_scene`] draws a standalone image with rulers, a key and the solution's warnings.
//! [`diagnostic_layer`] draws the same layers as a light overlay for a chart built from the
//! solution, in the solution's coordinates. Each region's label is inside the region, on a light
//! background, or beside a region too small for it, with an arrow. Both name each region that has
//! an id with `name`, and other regions by their paths. Tracks and gaps appear only when the
//! solution was solved with [`SolveOptions::explain`](crate::SolveOptions::explain).

use std::hash::Hash;

use avenger_annotation::leader::{
    LeaderArrow, LeaderShape, LeaderStyle, LeaderTarget, PlacedLabel, leader, make_leader_marks,
};
use avenger_color::ColorOrGradient;
use avenger_common::types::{FontWeight, StrokeCap, TextAlign, TextBaseline};
use avenger_common::value::ScalarOrArray;
use avenger_scenegraph::marks::group::SceneGroup;
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::pattern::{PatternAnchor, PatternFill, PatternInk, StripeLayer};
use avenger_scenegraph::marks::rect::SceneRectMark;
use avenger_scenegraph::marks::rule::SceneRuleMark;
use avenger_scenegraph::marks::text::SceneTextMark;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::{LabelEngine, LabelError, LabelWidth, TextBounds};

use crate::explain::{GapExplanation, GapTerm, RepresentationKind, TrackOwnerExplanation};
use crate::geometry::{Direction, Rect, Side, Size};
use crate::node::Extent;
use crate::report::{Warning, in_overlay, number, overlaps, ranges};
use crate::solution::{PathStep, Region, RegionKind, Solution};

/// A standalone diagnostic image of `solution`, with rulers, a key and the solution's warnings.
pub fn diagnostic_scene<Id: Clone + Eq + Hash>(
    solution: &Solution<Id>,
    name: &dyn Fn(&Id) -> String,
    engine: &LabelEngine,
) -> Result<SceneGraph, LabelError> {
    let mut painter = Painter::new(solution, name, engine, true);
    painter.layers();
    painter.place_labels()?;
    painter.rulers();
    let size = solution.size();
    let left = MARGIN + RULER;
    let top = MARGIN + RULER;
    let canvas = rects_mark(
        "canvas".to_string(),
        &[Rect::new(0.0, 0.0, size.width, size.height)],
        [1.0, 1.0, 1.0, 1.0],
        Some((INK_FAINT, 1.0)),
        None,
    );
    let mut content = vec![canvas.into()];
    content.append(&mut painter.marks);
    let content = SceneGroup {
        name: "diagnostics".to_string(),
        interactive: Some(false),
        origin: [left, top],
        marks: content,
        ..Default::default()
    };
    let key_x = left + size.width + MARGIN * 2.0;
    let (key, key_size) = painter.key([key_x, top])?;
    let width = key_x + key_size.width + MARGIN;
    let height = top + size.height.max(key_size.height) + MARGIN;
    let background = rects_mark(
        "background".to_string(),
        &[Rect::new(0.0, 0.0, width, height)],
        PAPER,
        Some(([0.0; 4], 0.0)),
        None,
    );
    Ok(SceneGraph {
        marks: vec![background.into(), content.into(), key.into()],
        width,
        height,
        origin: [0.0, 0.0],
    })
}

/// A diagnostic overlay for a chart built from `solution`: the same layers as
/// [`diagnostic_scene`], with light fills, in the solution's coordinates. Push it into the group
/// that holds the chart. Its z-index is the highest there is, so it draws above every mark of
/// the scene, including guides with z-indexes of their own, and its marks are not interactive,
/// so it never takes the chart's picks.
pub fn diagnostic_layer<Id: Clone + Eq + Hash>(
    solution: &Solution<Id>,
    name: &dyn Fn(&Id) -> String,
    engine: &LabelEngine,
) -> Result<SceneGroup, LabelError> {
    let mut painter = Painter::new(solution, name, engine, false);
    painter.layers();
    painter.place_labels()?;
    Ok(SceneGroup {
        name: "diagnostics".to_string(),
        interactive: Some(false),
        zindex: Some(i32::MAX),
        marks: painter.marks,
        ..Default::default()
    })
}

const FONT_SIZE: f32 = 10.0;
const MARGIN: f32 = 8.0;
const RULER: f32 = 18.0;
/// The widest a label gets before it ends in an ellipsis.
const LABEL_WIDTH: f32 = 160.0;
/// The font sizes a region's label tries, largest first, to fit inside its region.
const LABEL_FONT_SIZES: [f32; 3] = [10.0, 9.0, 8.0];
/// The shortest length, in font sizes, that a label shortened with an ellipsis takes.
const ELLIPSIS_MIN: f32 = 3.0;
/// How far a label beside its region sits from the region: the nearer distance first.
const CALLOUT_GAPS: [f32; 2] = [6.0, 14.0];
/// The space between a label's text and the edge of its background, along the text and across
/// it.
const LABEL_PADDING: [f32; 2] = [3.0, 1.0];
const LABEL_RADIUS: f32 = 3.0;
const KEY_WIDTH: f32 = 340.0;
/// The distance between a hatch's stripes.
const HATCH_SPACING: f32 = 16.0 / 3.0;
/// How much of its color an overlay keeps, so the chart under it stays readable.
const OVERLAY_ALPHA: f32 = 0.35;

const PAPER: [f32; 4] = [0.97, 0.965, 0.95, 1.0];
const INK: [f32; 4] = [0.09, 0.13, 0.2, 1.0];
const INK_SOFT: [f32; 4] = [0.29, 0.33, 0.41, 1.0];
const INK_FAINT: [f32; 4] = [0.6, 0.64, 0.7, 1.0];
const CONTENT: [f32; 4] = [0.83, 0.89, 0.96, 1.0];
const CONTENT_STROKE: [f32; 4] = [0.18, 0.43, 0.71, 1.0];
const SLAB: [f32; 4] = [0.96, 0.85, 0.73, 1.0];
const SLAB_STROKE: [f32; 4] = [0.72, 0.36, 0.09, 1.0];
const TRACK_COLUMN: [f32; 4] = [0.18, 0.43, 0.71, 0.07];
const TRACK_ROW: [f32; 4] = [0.72, 0.36, 0.09, 0.06];
const GAP_MINIMUM: [f32; 4] = [0.5, 0.52, 0.56, 0.22];
const GAP_OVERFLOW: [f32; 4] = [0.93, 0.56, 0.16, 0.4];
const GAP_MATCHED: [f32; 4] = [0.52, 0.27, 0.72, 0.3];
const PADDING: [f32; 4] = [0.6, 0.64, 0.7, 0.18];
/// Warnings, overlapping slabs and stale answers, apart from the labels' color.
const ALERT: [f32; 4] = [0.0, 0.55, 0.55, 1.0];
/// The text color of a label, so that it stands apart from a chart's text.
const LABEL_INK: [f32; 4] = [0.86, 0.08, 0.24, 1.0];
const LABEL_BACKGROUND: [f32; 4] = [1.0, 1.0, 1.0, 0.75];

/// A label for one region, which it sits inside.
struct LabelRequest {
    name: String,
    text: String,
    target: Rect,
}

struct Painter<'a, Id> {
    solution: &'a Solution<Id>,
    name: &'a dyn Fn(&Id) -> String,
    engine: &'a LabelEngine,
    standalone: bool,
    marks: Vec<SceneMark>,
    labels: Vec<LabelRequest>,
    warnings: Vec<Warning>,
}

impl<'a, Id: Clone + Eq + Hash> Painter<'a, Id> {
    fn new(
        solution: &'a Solution<Id>,
        name: &'a dyn Fn(&Id) -> String,
        engine: &'a LabelEngine,
        standalone: bool,
    ) -> Self {
        Self {
            solution,
            name,
            engine,
            standalone,
            marks: Vec::new(),
            labels: Vec::new(),
            warnings: solution.warnings(),
        }
    }

    fn layers(&mut self) {
        self.padding();
        self.tracks();
        self.gaps();
        self.empty_cells();
        self.slots_and_content();
        self.slabs();
        self.overlays();
        self.stale();
    }

    /// The owners' tracks and gaps, when the solve explained.
    fn owners(&self) -> &'a [TrackOwnerExplanation] {
        self.solution
            .explanation()
            .sizes
            .as_ref()
            .map_or(&[], |sizes| &sizes.owners)
    }

    /// The color as drawn: lighter in an overlay, so the chart under it stays readable.
    fn tint(&self, color: [f32; 4]) -> [f32; 4] {
        if self.standalone {
            color
        } else {
            [color[0], color[1], color[2], color[3] * OVERLAY_ALPHA]
        }
    }

    /// A hatch as drawn: lighter in an overlay, as [`tint`](Self::tint) makes colors.
    fn tint_hatch(&self, mut pattern: PatternFill) -> PatternFill {
        if !self.standalone
            && let PatternInk::Solid { opacity, .. } = &mut pattern.ink
        {
            *opacity *= OVERLAY_ALPHA;
        }
        pattern
    }

    fn display_name(&self, region: &Region<Id>) -> String {
        match &region.id {
            Some(id) => (self.name)(id),
            None => compact_path(&region.path),
        }
    }

    /// A leaf's content together with its own slabs: what its slot holds, so the rest of the slot
    /// is slack.
    fn drawn_box(&self, region: &Region<Id>) -> Rect {
        let mut x = (region.content.x, region.content.right());
        let mut y = (region.content.y, region.content.bottom());
        for slab in &self.solution.regions {
            if slab.path.len() == region.path.len() + 1
                && slab.path.starts_with(&region.path)
                && matches!(slab.path.last(), Some(PathStep::Slab(..)))
            {
                x = (x.0.min(slab.slot.x), x.1.max(slab.slot.right()));
                y = (y.0.min(slab.slot.y), y.1.max(slab.slot.bottom()));
            }
        }
        Rect::from_ranges(x, y)
    }

    fn push_rect(
        &mut self,
        name: String,
        rect: Rect,
        fill: [f32; 4],
        stroke: [f32; 4],
        width: f32,
    ) {
        let mark = rects_mark(name, &[rect], self.tint(fill), Some((stroke, width)), None);
        self.marks.push(mark.into());
    }

    fn push_hatch(&mut self, name: String, rect: Rect, pattern: PatternFill) {
        let pattern = Some(self.tint_hatch(pattern));
        let mark = rects_mark(name, &[rect], [0.0; 4], Some(([0.0; 4], 0.0)), pattern);
        self.marks.push(mark.into());
    }

    fn push_outline(&mut self, name: String, rect: Rect, color: [f32; 4], width: f32, dash: bool) {
        self.marks
            .push(segments(name, &outline(rect), color, width, dash));
    }

    fn label(&mut self, name: String, text: String, target: Rect) {
        self.labels.push(LabelRequest { name, text, target });
    }

    /// The canvas padding: the parts of the canvas outside the root's slot.
    fn padding(&mut self) {
        for (side, band) in padding_bands(self.solution) {
            let name = format!("padding:{}", side_name(side));
            self.push_rect(name, band, PADDING, [0.0; 4], 0.0);
        }
    }

    fn tracks(&mut self) {
        let solution = self.solution;
        for owner in self.owners() {
            let Some(region) = solution.at_path(&owner.path) else {
                continue;
            };
            let (letter, color) = direction_style(owner.direction);
            for (index, track) in owner.tracks.iter().enumerate() {
                let rect = band(owner.direction, track.start, track.length, &region.content);
                let name = format!("track:{}/{letter}{index}", compact_path(&owner.path));
                self.push_rect(name, rect, color, [0.0; 4], 0.0);
            }
        }
    }

    fn gaps(&mut self) {
        let solution = self.solution;
        for owner in self.owners() {
            let Some(region) = solution.at_path(&owner.path) else {
                continue;
            };
            for (index, gap) in owner.gaps.iter().enumerate() {
                if gap.length > 0.0 {
                    self.gap_band(region, owner, index, gap);
                }
            }
        }
    }

    /// Draws a gap as the pieces of its band that no child spans across: the band is split at
    /// the owner's cross tracks, and a piece that some node inside the owner covers from one side
    /// of the gap to the other is left out, since there is no gap there.
    fn gap_band(
        &mut self,
        region: &Region<Id>,
        owner: &TrackOwnerExplanation,
        index: usize,
        gap: &GapExplanation,
    ) {
        let direction = owner.direction;
        let band = band(direction, gap.start, gap.length, &region.content);
        let pieces: Vec<Rect> = cross_pieces(region, direction)
            .into_iter()
            .map(|(from, to)| match direction {
                Direction::Horizontal => Rect::new(band.x, from, band.width, to - from),
                Direction::Vertical => Rect::new(from, band.y, to - from, band.height),
            })
            .filter(|piece| !self.spanned(region, direction, piece))
            .collect();
        if pieces.is_empty() {
            return;
        }
        let (fill, pattern) = if gap.raised_from.is_some() {
            (GAP_MATCHED, Some(self.tint_hatch(matched_hatch())))
        } else {
            let color = match gap.term {
                GapTerm::Minimum => GAP_MINIMUM,
                GapTerm::Overflow { .. } => GAP_OVERFLOW,
            };
            (color, None)
        };
        let (letter, _) = direction_style(direction);
        let name = format!("gap:{}/{letter}{index}", compact_path(&owner.path));
        let mark = rects_mark(name, &pieces, self.tint(fill), None, pattern);
        self.marks.push(mark.into());
    }

    /// Whether a node inside `owner`, other than a slab, covers `piece` from one side of a gap
    /// to the other. A container whose own tracks meet at the gap, as those of a Row or Column
    /// in a table do, keeps the gap.
    fn spanned(&self, owner: &Region<Id>, direction: Direction, piece: &Rect) -> bool {
        const EPSILON: f32 = 1e-3;
        let d = direction.index();
        let c = direction.cross().index();
        let (gap_start, gap_end) = ranges(piece)[d];
        let keeps_gap = |region: &Region<Id>| match region.kind.tracks() {
            Some(tracks) => {
                let spans = match direction {
                    Direction::Horizontal => &tracks.columns,
                    Direction::Vertical => &tracks.rows,
                };
                spans
                    .iter()
                    .any(|span| (span.start + span.length - gap_start).abs() < EPSILON)
                    && spans
                        .iter()
                        .any(|span| (span.start - gap_end).abs() < EPSILON)
            }
            None => false,
        };
        self.solution.regions.iter().any(|region| {
            let slot = ranges(&region.slot);
            let piece = ranges(piece);
            region.path.len() > owner.path.len()
                && region.path.starts_with(&owner.path)
                && !matches!(region.kind, RegionKind::Slab { .. })
                && !in_overlay(&region.path)
                && !keeps_gap(region)
                && slot[d].0 <= piece[d].0 + EPSILON
                && slot[d].1 >= piece[d].1 - EPSILON
                && slot[c].0 < piece[c].1 - EPSILON
                && slot[c].1 > piece[c].0 + EPSILON
        })
    }

    fn empty_cells(&mut self) {
        let solution = self.solution;
        for region in &solution.regions {
            let Some(tracks) = region.kind.tracks() else {
                continue;
            };
            let children: Vec<Rect> = solution
                .regions
                .iter()
                .filter(|child| {
                    child.path.len() == region.path.len() + 1
                        && child.path.starts_with(&region.path)
                        && matches!(child.path.last(), Some(PathStep::Child(_)))
                })
                .map(|child| child.slot)
                .collect();
            for (column, x) in tracks.columns.iter().enumerate() {
                for (row, y) in tracks.rows.iter().enumerate() {
                    let cell = Rect::new(x.start, y.start, x.length, y.length);
                    if cell.width <= 0.0 || cell.height <= 0.0 {
                        continue;
                    }
                    if !children.iter().any(|slot| overlaps(slot, &cell)) {
                        let name = format!("empty:{}/{column},{row}", compact_path(&region.path));
                        self.push_hatch(name, cell, empty_hatch());
                    }
                }
            }
        }
    }

    fn slots_and_content(&mut self) {
        let solution = self.solution;
        for region in &solution.regions {
            let kind = match region.kind {
                RegionKind::Leaf => {
                    self.leaf(region);
                    continue;
                }
                RegionKind::Slab { .. } => continue,
                RegionKind::Row { .. } => "row",
                RegionKind::Column { .. } => "column",
                RegionKind::Grid { .. } => "grid",
                RegionKind::Wrap { .. } => "wrap",
                RegionKind::Flow { .. } => "flow",
            };
            let name = format!("{kind}:{}", compact_path(&region.path));
            self.push_outline(name, region.content, INK_FAINT, 0.5, false);
        }
    }

    /// A leaf's content, and the slack that its slot leaves around its content and slabs.
    fn leaf(&mut self, region: &Region<Id>) {
        let name = self.display_name(region);
        let path = compact_path(&region.path);
        let drawn = self.drawn_box(region);
        if region.slot != drawn {
            for (index, slack) in slack(&region.slot, &drawn).into_iter().enumerate() {
                self.push_hatch(format!("slack:{path}/{index}"), slack, slack_hatch());
            }
            self.push_outline(format!("slot:{path}"), region.slot, INK_FAINT, 1.0, true);
        }
        let content_name = format!("content:{path}");
        self.push_rect(
            content_name.clone(),
            region.content,
            CONTENT,
            CONTENT_STROKE,
            1.0,
        );
        if !matches!(region.path.last(), Some(PathStep::Overlay(_))) {
            self.label(content_name, name, region.content);
        }
    }

    fn slabs(&mut self) {
        let solution = self.solution;
        for region in &solution.regions {
            let RegionKind::Slab {
                side,
                extent,
                tag,
                represented_by,
            } = &region.kind
            else {
                continue;
            };
            // A slab that another slab draws takes no space and draws nothing.
            if represented_by.is_some() || region.slot.width <= 0.0 || region.slot.height <= 0.0 {
                continue;
            }
            let path = compact_path(&region.path);
            let overlapping = self.warnings.iter().any(|warning| {
                matches!(warning, Warning::Overlap { slab, other }
                    if slab == &region.path || other == &region.path)
            });
            let stroke = if overlapping { ALERT } else { SLAB_STROKE };
            let width = if overlapping { 2.0 } else { 1.0 };
            let name = format!("slab:{path}");
            self.push_rect(name.clone(), region.slot, SLAB, stroke, width);
            if *extent == Extent::Full {
                let corners = corner_marks(&region.slot, *side);
                let mark = segments(format!("corner:{path}"), &corners, SLAB_STROKE, 2.0, false);
                self.marks.push(mark);
            }
            let what = match (tag, region.id.is_some()) {
                (Some(tag), _) => tag.as_str().to_string(),
                (None, true) => self.display_name(region),
                // A slab without a tag or an id, such as a spacer, has nothing to name.
                (None, false) => continue,
            };
            let merged = solution
                .explanation()
                .representations
                .iter()
                .find(|representation| {
                    representation.kind == RepresentationKind::Merge
                        && representation.representative == region.path
                });
            let what = match merged {
                Some(merge) => format!("{what}, {} merged", merge.members.len()),
                None => what,
            };
            self.label(name, what, region.slot);
        }
    }

    fn overlays(&mut self) {
        let solution = self.solution;
        for region in &solution.regions {
            if !matches!(region.path.last(), Some(PathStep::Overlay(_))) {
                continue;
            }
            let path = compact_path(&region.path);
            let name = format!("overlay:{path}");
            self.push_outline(name.clone(), region.slot, INK, 1.5, true);
            let text = format!("overlay {}", self.display_name(region));
            self.label(name, text, region.slot);
        }
    }

    /// Outlines for the measured leaves that took a stale answer.
    fn stale(&mut self) {
        let solution = self.solution;
        for id in solution.stale() {
            if let Some(region) = solution.region(id) {
                let rect = match region.kind {
                    RegionKind::Slab { .. } => region.slot,
                    _ => region.content,
                };
                self.push_outline(
                    format!("stale:{}", compact_path(&region.path)),
                    rect,
                    ALERT,
                    1.5,
                    true,
                );
            }
        }
    }

    /// A warning with regions named as the image names them.
    fn warning_text(&self, warning: &Warning) -> String {
        warning.describe(|path| {
            self.solution
                .at_path(path)
                .map_or_else(|| compact_path(path), |region| self.display_name(region))
        })
    }

    fn text_mark(&self, name: String, text: String) -> SceneTextMark {
        SceneTextMark {
            name,
            interactive: Some(false),
            clip: false,
            text: ScalarOrArray::new_scalar(text),
            x: 0.0.into(),
            y: 0.0.into(),
            align: ScalarOrArray::new_scalar(TextAlign::Left),
            baseline: ScalarOrArray::new_scalar(TextBaseline::Top),
            color: ScalarOrArray::new_scalar(INK),
            font_size: FONT_SIZE.into(),
            font_weight: ScalarOrArray::new_scalar(FontWeight::NORMAL),
            width: ScalarOrArray::new_scalar(LabelWidth::Max(LABEL_WIDTH)),
            wrap: true,
            max_lines: std::num::NonZeroUsize::new(1),
            ellipsis: true,
            ..Default::default()
        }
    }

    fn measure(&self, mark: &SceneTextMark) -> Result<TextBounds, LabelError> {
        match mark.labels().next() {
            Some(label) => Ok(self.engine.bounds(&label.label)?),
            None => Ok(TextBounds {
                width: 0.0,
                height: 0.0,
                ascent: 0.0,
                leading: 0.0,
            }),
        }
    }

    /// Draws each label on a light, rounded background: inside its region when it fits there at
    /// one of the label font sizes, flat or turned, or shortened with an ellipsis when a line fits
    /// across the region but the text does not fit along it. A region too small for a line gets
    /// its label beside it, in space that no other label takes, with an arrow to it. The
    /// backgrounds go under every label, so that no background covers another label's text.
    fn place_labels(&mut self) -> Result<(), LabelError> {
        let size = self.solution.size();
        let mut backgrounds = Vec::new();
        let mut texts = Vec::new();
        let mut beside = Vec::new();
        for request in std::mem::take(&mut self.labels) {
            match self.inside(&request, size)? {
                Some((mark, background)) => {
                    backgrounds.push(background);
                    texts.push(mark);
                }
                None => beside.push(request),
            }
        }
        let style = LeaderStyle {
            shape: LeaderShape::Straight,
            arrow: LeaderArrow::Triangle,
            arrow_length: 4.0,
            arrow_width: 3.5,
            stroke: LABEL_INK,
            stroke_width: 0.75,
            label_padding: LABEL_PADDING[0],
            ..Default::default()
        };
        let mut leaders = Vec::new();
        for request in beside {
            let Some((mark, bounds, background)) = self.callout(&request, size, &backgrounds)?
            else {
                continue;
            };
            let placed = PlacedLabel {
                bounds,
                position: center_of(&background),
                align: TextAlign::Center,
                baseline: TextBaseline::Middle,
                angle: 0.0,
            };
            let target = LeaderTarget {
                position: center_of(&request.target),
                radius: 0.0,
            };
            let line = leader(&placed, target, &style);
            leaders.push(make_leader_marks(&mark, &[line], &style).expect("one leader per label"));
            backgrounds.push(background);
            texts.push(mark);
        }
        self.marks.extend(leaders.into_iter().map(SceneMark::from));
        if !backgrounds.is_empty() {
            let mut mark = rects_mark(
                "label-backgrounds".to_string(),
                &backgrounds,
                LABEL_BACKGROUND,
                None,
                None,
            );
            mark.corner_radius = LABEL_RADIUS.into();
            self.marks.push(mark.into());
        }
        self.marks.extend(texts.into_iter().map(SceneMark::from));
        Ok(())
    }

    /// A label mark at `font_size`, no wider than `width`, and its text's box.
    fn label_mark(
        &self,
        request: &LabelRequest,
        font_size: f32,
        width: f32,
    ) -> Result<(SceneTextMark, TextBounds), LabelError> {
        let mut mark = self.text_mark(request.name.clone(), request.text.clone());
        mark.font_size = font_size.into();
        mark.width = ScalarOrArray::new_scalar(LabelWidth::Max(width));
        mark.align = ScalarOrArray::new_scalar(TextAlign::Center);
        mark.baseline = ScalarOrArray::new_scalar(TextBaseline::Middle);
        mark.color = ScalarOrArray::new_scalar(LABEL_INK);
        let bounds = self.measure(&mark)?;
        Ok((mark, bounds))
    }

    /// The label inside its region, centered and turned by `angle`, and its background, or
    /// `None` when no font size fits it there, even shortened.
    fn inside(
        &self,
        request: &LabelRequest,
        canvas: Size,
    ) -> Result<Option<(SceneTextMark, Rect)>, LabelError> {
        let target = request.target;
        let center = center_of(&target);
        let [along, across] = LABEL_PADDING;
        // A label that only fits turned reads up on the left half of the canvas and down on the
        // right half, facing out, as axis titles do.
        let turned = if center[0] <= canvas.width / 2.0 {
            -90.0
        } else {
            90.0
        };
        let place = |mut mark: SceneTextMark, angle: f32, bounds: &TextBounds| {
            let (width, height) = (bounds.width + 2.0 * along, bounds.height + 2.0 * across);
            let (width, height) = if angle == 0.0 {
                (width, height)
            } else {
                (height, width)
            };
            mark.x = center[0].into();
            mark.y = center[1].into();
            mark.angle = angle.into();
            let background = Rect::new(
                center[0] - width / 2.0,
                center[1] - height / 2.0,
                width,
                height,
            );
            (mark, background)
        };
        let mut smallest = None;
        for font_size in LABEL_FONT_SIZES {
            let (mark, bounds) = self.label_mark(request, font_size, LABEL_WIDTH)?;
            let (width, height) = (bounds.width + 2.0 * along, bounds.height + 2.0 * across);
            if width <= target.width && height <= target.height {
                return Ok(Some(place(mark, 0.0, &bounds)));
            }
            if height <= target.width && width <= target.height {
                return Ok(Some(place(mark, turned, &bounds)));
            }
            smallest = Some((font_size, bounds.height + 2.0 * across));
        }
        // Shorten the label at the smallest size, along the longer way that a line fits across.
        let Some((font_size, height)) = smallest else {
            return Ok(None);
        };
        let flat = (height <= target.height).then_some((0.0, target.width));
        let upright = (height <= target.width).then_some((turned, target.height));
        let Some((angle, length)) = [flat, upright]
            .into_iter()
            .flatten()
            .max_by(|a, b| a.1.total_cmp(&b.1))
        else {
            return Ok(None);
        };
        let room = length - 2.0 * along;
        if room < ELLIPSIS_MIN * font_size {
            return Ok(None);
        }
        let (mark, bounds) = self.label_mark(request, font_size, room)?;
        Ok(Some(place(mark, angle, &bounds)))
    }

    /// The label at the smallest font size beside its region, above, below, right or left of it,
    /// nearer first, moved inside the canvas, in the first place that covers neither the region
    /// nor a background in `taken`, with its text's box and its background. `None` when no place
    /// is free.
    fn callout(
        &self,
        request: &LabelRequest,
        canvas: Size,
        taken: &[Rect],
    ) -> Result<Option<(SceneTextMark, TextBounds, Rect)>, LabelError> {
        let font_size = LABEL_FONT_SIZES[LABEL_FONT_SIZES.len() - 1];
        let (mut mark, bounds) = self.label_mark(request, font_size, LABEL_WIDTH)?;
        let [along, across] = LABEL_PADDING;
        let (width, height) = (bounds.width + 2.0 * along, bounds.height + 2.0 * across);
        let target = request.target;
        let [x, y] = center_of(&target);
        let places = CALLOUT_GAPS.into_iter().flat_map(|gap| {
            [
                [x - width / 2.0, target.y - gap - height],
                [x - width / 2.0, target.bottom() + gap],
                [target.right() + gap, y - height / 2.0],
                [target.x - gap - width, y - height / 2.0],
            ]
        });
        let free = places
            .filter(|_| width <= canvas.width && height <= canvas.height)
            .map(|[x, y]| {
                let x = x.clamp(0.0, canvas.width - width);
                let y = y.clamp(0.0, canvas.height - height);
                Rect::new(x, y, width, height)
            })
            .find(|place| {
                !overlaps(&target, place) && !taken.iter().any(|other| overlaps(other, place))
            });
        let Some(background) = free else {
            return Ok(None);
        };
        let [x, y] = center_of(&background);
        mark.x = x.into();
        mark.y = y.into();
        Ok(Some((mark, bounds, background)))
    }

    /// Pixel rulers along the top and left edges of the canvas.
    fn rulers(&mut self) {
        let size = self.solution.size();
        let mut ticks = Vec::new();
        let mut texts = Vec::new();
        for direction in Direction::BOTH {
            let horizontal = direction == Direction::Horizontal;
            let mut step = 0.0f32;
            while step <= Size::new(size.width, size.height).along(direction) + 0.5 {
                let length = tick_length(step);
                ticks.push(if horizontal {
                    [step, -1.0, step, -1.0 - length]
                } else {
                    [-1.0, step, -1.0 - length, step]
                });
                // The y ruler leaves 0 to the x ruler.
                if step % 100.0 == 0.0 && (horizontal || step > 0.0) {
                    let (axis, position) = if horizontal {
                        ('x', [step + 2.0, -RULER])
                    } else {
                        ('y', [-RULER + 1.0, step + 1.0])
                    };
                    texts.push((
                        format!("ruler:{axis}{}", number(step)),
                        number(step),
                        position,
                    ));
                }
                step += 10.0;
            }
        }
        self.marks.push(segments(
            "ruler:ticks".to_string(),
            &ticks,
            INK_FAINT,
            0.75,
            false,
        ));
        for (name, text, [x, y]) in texts {
            let mut mark = self.text_mark(name, text);
            mark.x = x.into();
            mark.y = y.into();
            mark.font_size = (FONT_SIZE * 0.8).into();
            mark.color = ScalarOrArray::new_scalar(INK_SOFT);
            self.marks.push(mark.into());
        }
    }

    /// The key: what each color means, and the warnings. Returns the group and
    /// its size.
    fn key(&self, origin: [f32; 2]) -> Result<(SceneGroup, Size), LabelError> {
        let explanation = self.solution.explanation();
        // Each row's swatch, if any, its text and its text's color. A swatch has its element's
        // fill, hatch and edge.
        type Swatch = Option<([f32; 4], Option<PatternFill>, Edge)>;
        let mut rows: Vec<(Swatch, String, [f32; 4])> = vec![
            (
                Some((CONTENT, None, Edge::Solid(CONTENT_STROKE))),
                "content".to_string(),
                INK,
            ),
            (
                Some(([0.0; 4], Some(slack_hatch()), Edge::Dashed(INK_FAINT))),
                "slack in a slot".to_string(),
                INK,
            ),
            (
                Some((SLAB, None, Edge::Solid(SLAB_STROKE))),
                "slab".to_string(),
                INK,
            ),
            (
                Some((GAP_MINIMUM, None, Edge::None)),
                "gap: the minimum".to_string(),
                INK,
            ),
            (
                Some((GAP_OVERFLOW, None, Edge::None)),
                "gap: overflow and clearance".to_string(),
                INK,
            ),
            (
                Some((GAP_MATCHED, Some(matched_hatch()), Edge::None)),
                "gap: raised by a match".to_string(),
                INK,
            ),
            (
                Some(([0.0; 4], Some(empty_hatch()), Edge::None)),
                "empty cell".to_string(),
                INK,
            ),
        ];
        if !padding_bands(self.solution).is_empty() {
            rows.push((
                Some((PADDING, None, Edge::None)),
                "canvas padding".to_string(),
                INK,
            ));
        }
        if explanation.sizes.is_none() {
            rows.push((
                None,
                "geometry only: solve with explain".to_string(),
                INK_SOFT,
            ));
            rows.push((
                None,
                "to see tracks, gaps and matches".to_string(),
                INK_SOFT,
            ));
        }
        if explanation.solves > 1 {
            rows.push((None, format!("{} solves", explanation.solves), INK_SOFT));
        }
        for warning in &self.warnings {
            rows.push((None, self.warning_text(warning), ALERT));
        }
        let line = FONT_SIZE * 1.6;
        let count = rows.len();
        let mut marks = Vec::new();
        let mut width = 0.0f32;
        for (index, (swatch, text, color)) in rows.into_iter().enumerate() {
            let y = index as f32 * line;
            let text_x = if swatch.is_some() { 16.0 } else { 0.0 };
            if let Some((fill, pattern, edge)) = swatch {
                let rect = Rect::new(0.0, y + 1.0, 10.0, 10.0);
                let stroke = match edge {
                    Edge::Solid(color) => Some((color, 1.0)),
                    Edge::Dashed(_) | Edge::None => None,
                };
                let name = format!("key:swatch{index}");
                marks.push(rects_mark(name, &[rect], fill, stroke, pattern).into());
                if let Edge::Dashed(color) = edge {
                    let name = format!("key:edge{index}");
                    marks.push(segments(name, &outline(rect), color, 1.0, true));
                }
            }
            let mut mark = self.text_mark(format!("key:{index}"), text);
            mark.x = text_x.into();
            mark.y = y.into();
            mark.width = ScalarOrArray::new_scalar(LabelWidth::Max(KEY_WIDTH));
            mark.color = ScalarOrArray::new_scalar(color);
            let bounds = self.measure(&mark)?;
            width = width.max(text_x + bounds.width);
            marks.push(mark.into());
        }
        let height = count as f32 * line;
        Ok((
            SceneGroup {
                name: "key".to_string(),
                interactive: Some(false),
                origin,
                marks,
                ..Default::default()
            },
            Size::new(width, height),
        ))
    }
}

/// How a key's swatch draws its edge, as its element's edge is drawn.
enum Edge {
    None,
    Solid([f32; 4]),
    Dashed([f32; 4]),
}

/// Rectangles in one mark, with a stroke when one is given.
fn rects_mark(
    name: String,
    rects: &[Rect],
    fill: [f32; 4],
    stroke: Option<([f32; 4], f32)>,
    pattern: Option<PatternFill>,
) -> SceneRectMark {
    let column = |pick: fn(&Rect) -> f32| -> ScalarOrArray<f32> {
        ScalarOrArray::new_array(rects.iter().map(pick).collect())
    };
    let mut mark = SceneRectMark {
        name,
        interactive: Some(false),
        clip: false,
        len: rects.len() as u32,
        x: column(|r| r.x),
        y: column(|r| r.y),
        width: Some(column(|r| r.width.max(0.0))),
        height: Some(column(|r| r.height.max(0.0))),
        fill: ScalarOrArray::new_scalar(ColorOrGradient::Color(fill)),
        fill_pattern: pattern.into(),
        ..Default::default()
    };
    if let Some((color, width)) = stroke {
        mark.stroke = ScalarOrArray::new_scalar(ColorOrGradient::Color(color));
        mark.stroke_width = width.into();
    }
    mark
}

/// Line segments, `[x, y, x2, y2]` each, dashed when `dash` is set.
fn segments(
    name: String,
    segments: &[[f32; 4]],
    color: [f32; 4],
    width: f32,
    dash: bool,
) -> SceneMark {
    let column = |index: usize| -> ScalarOrArray<f32> {
        ScalarOrArray::new_array(segments.iter().map(|s| s[index]).collect())
    };
    SceneRuleMark {
        name,
        interactive: Some(false),
        clip: false,
        len: segments.len() as u32,
        stroke_dash: dash.then(|| ScalarOrArray::new_scalar(vec![4.0, 3.0])),
        x: column(0),
        y: column(1),
        x2: column(2),
        y2: column(3),
        stroke: ScalarOrArray::new_scalar(ColorOrGradient::Color(color)),
        stroke_width: width.into(),
        stroke_cap: ScalarOrArray::new_scalar(StrokeCap::Butt),
        ..Default::default()
    }
    .into()
}

fn tick_length(at: f32) -> f32 {
    if at % 100.0 == 0.0 {
        8.0
    } else if at % 50.0 == 0.0 {
        5.0
    } else {
        2.0
    }
}

/// Diagonal stripes of `color`.
fn stripes(color: [f32; 4]) -> PatternFill {
    PatternFill {
        anchor: PatternAnchor::Scene,
        ink: PatternInk::Solid {
            color: [color[0], color[1], color[2], 1.0],
            opacity: 0.6,
        },
        layers: vec![StripeLayer::new(45.0, HATCH_SPACING, 1.0).into()],
    }
}

/// The hatch of the slack in a slot.
fn slack_hatch() -> PatternFill {
    stripes(INK_FAINT)
}

/// The hatch of a gap that a match raised.
fn matched_hatch() -> PatternFill {
    stripes(GAP_MATCHED)
}

/// The crosshatch of an empty cell.
fn empty_hatch() -> PatternFill {
    let mut pattern = stripes(INK_FAINT);
    pattern
        .layers
        .push(StripeLayer::new(-45.0, HATCH_SPACING, 1.0).into());
    pattern
}

/// The band of a track or gap: its range in `direction`, across the owner's content.
fn band(direction: Direction, start: f32, length: f32, content: &Rect) -> Rect {
    match direction {
        Direction::Horizontal => Rect::new(start, content.y, length, content.height),
        Direction::Vertical => Rect::new(content.x, start, content.width, length),
    }
}

/// The owner's content across `direction`, split at its cross tracks: each track and each gap
/// between tracks is one piece.
fn cross_pieces<Id>(owner: &Region<Id>, direction: Direction) -> Vec<(f32, f32)> {
    let (start, end) = ranges(&owner.content)[direction.cross().index()];
    let tracks = owner.kind.tracks().expect("owners have tracks");
    let spans = match direction {
        Direction::Horizontal => &tracks.rows,
        Direction::Vertical => &tracks.columns,
    };
    let mut cuts = vec![start];
    for span in spans {
        cuts.push(span.start);
        cuts.push(span.start + span.length);
    }
    cuts.push(end);
    cuts.retain(|&cut| cut >= start && cut <= end);
    cuts.sort_by(f32::total_cmp);
    cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-3);
    cuts.windows(2).map(|pair| (pair[0], pair[1])).collect()
}

/// A direction's track letter and band color.
fn direction_style(direction: Direction) -> (char, [f32; 4]) {
    match direction {
        Direction::Horizontal => ('c', TRACK_COLUMN),
        Direction::Vertical => ('r', TRACK_ROW),
    }
}

fn side_letter(side: Side) -> char {
    match side {
        Side::Left => 'L',
        Side::Top => 'T',
        Side::Right => 'R',
        Side::Bottom => 'B',
    }
}

fn side_name(side: Side) -> String {
    format!("{side:?}").to_lowercase()
}

/// A path for names: `root`, then each step, such as `root/0/T1/o0` or `root/mB0`.
fn compact_path(path: &[PathStep]) -> String {
    let mut text = String::from("root");
    for step in path {
        text.push('/');
        match step {
            PathStep::Child(index) => text.push_str(&index.to_string()),
            PathStep::Slab(side, index) => {
                text.push(side_letter(*side));
                text.push_str(&index.to_string());
            }
            PathStep::Overlay(index) => {
                text.push('o');
                text.push_str(&index.to_string());
            }
            PathStep::Merged(side, index) => {
                text.push('m');
                text.push(side_letter(*side));
                text.push_str(&index.to_string());
            }
        }
    }
    text
}

/// The canvas padding on each side that has any: the canvas outside the root's slot.
fn padding_bands<Id: Clone + Eq + Hash>(solution: &Solution<Id>) -> Vec<(Side, Rect)> {
    let size = solution.size();
    let Some(root) = solution.at_path(&[]) else {
        return Vec::new();
    };
    let slot = root.slot;
    [
        (Side::Left, Rect::new(0.0, 0.0, slot.x, size.height)),
        (Side::Top, Rect::new(slot.x, 0.0, slot.width, slot.y)),
        (
            Side::Right,
            Rect::new(slot.right(), 0.0, size.width - slot.right(), size.height),
        ),
        (
            Side::Bottom,
            Rect::new(
                slot.x,
                slot.bottom(),
                slot.width,
                size.height - slot.bottom(),
            ),
        ),
    ]
    .into_iter()
    .filter(|(_, band)| band.width > 0.0 && band.height > 0.0)
    .collect()
}

fn center_of(rect: &Rect) -> [f32; 2] {
    [rect.x + rect.width / 2.0, rect.y + rect.height / 2.0]
}

/// The four edges of a rectangle as segments.
fn outline(rect: Rect) -> [[f32; 4]; 4] {
    let (x0, y0, x1, y1) = (rect.x, rect.y, rect.right(), rect.bottom());
    [
        [x0, y0, x1, y0],
        [x1, y0, x1, y1],
        [x1, y1, x0, y1],
        [x0, y1, x0, y0],
    ]
}

/// Brackets at both ends of a full slab, across its thickness.
fn corner_marks(slot: &Rect, side: Side) -> Vec<[f32; 4]> {
    match side.direction() {
        Direction::Vertical => vec![
            [slot.x + 1.0, slot.y, slot.x + 1.0, slot.bottom()],
            [
                slot.right() - 1.0,
                slot.y,
                slot.right() - 1.0,
                slot.bottom(),
            ],
        ],
        Direction::Horizontal => vec![
            [slot.x, slot.y + 1.0, slot.right(), slot.y + 1.0],
            [
                slot.x,
                slot.bottom() - 1.0,
                slot.right(),
                slot.bottom() - 1.0,
            ],
        ],
    }
}

/// The parts of a slot that its content leaves empty.
fn slack(slot: &Rect, content: &Rect) -> Vec<Rect> {
    let inner = Rect::from_ranges(
        (
            content.x.clamp(slot.x, slot.right()),
            content.right().clamp(slot.x, slot.right()),
        ),
        (
            content.y.clamp(slot.y, slot.bottom()),
            content.bottom().clamp(slot.y, slot.bottom()),
        ),
    );
    [
        Rect::new(slot.x, slot.y, slot.width, inner.y - slot.y),
        Rect::new(
            slot.x,
            inner.bottom(),
            slot.width,
            slot.bottom() - inner.bottom(),
        ),
        Rect::new(slot.x, inner.y, inner.x - slot.x, inner.height),
        Rect::new(
            inner.right(),
            inner.y,
            slot.right() - inner.right(),
            inner.height,
        ),
    ]
    .into_iter()
    .filter(|rect| rect.width > 0.0 && rect.height > 0.0)
    .collect()
}
