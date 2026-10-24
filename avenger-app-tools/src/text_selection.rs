//! Text selection: a tool that selects and copies the text of the labels in the scenes below its
//! own.
//!
//! A press on a label starts a selection there when the label is the topmost thing under the
//! pointer, whether its text mark takes hits or not, and a drag extends it across the labels of
//! the same scene, in the order the scene draws them: by position after one press, by word after
//! two in a row, and by line after three. The selection takes such a press, so the scenes below
//! never hear it or the drag that follows, as a pan would. A press elsewhere and Escape clear it.
//! The tool draws the selection over the text, calls for a text cursor over text, and offers the
//! selected text to the copy shortcut, as the labels show it.

use std::collections::HashMap;
use std::ops::Range;
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use avenger_app::app::{Scenes, Tool, ToolHandler, ToolInstall};
use avenger_app::error::AvengerAppError;
use avenger_color::ColorOrGradient;
use avenger_common::cursor::CursorStyle;
use avenger_eventstream::manager::SceneTrees;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, Rebuild};
use avenger_eventstream::window::{Key, MouseButton, NamedKey};
use avenger_geometry::query::{clip_shows, Query, QueryShape, Relation};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::{MarkInstance, SceneMark};
use avenger_scenegraph::marks::path::ScenePathMark;
use avenger_scenegraph::marks::text::LabelPlacement;
use avenger_scenegraph::render::order::{SceneDisplayList, SceneDisplayMark};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::{
    Label, LabelEngine, LabelError, LabelOptions, LabelSource, TextLayout, TextPosition, TextRect,
};
use lyon_path::{geom::point, Path};
use rstar::{PointDistance, RTree, RTreeObject, AABB};

/// Selects and copies the text of the labels in the scenes below its own scene, and draws the
/// selection there.
#[derive(Clone)]
pub struct TextSelection {
    scene: String,
    z: i32,
    highlight: [f32; 4],
}

impl TextSelection {
    /// Create a selection that draws in a scene named `scene` at z-order `z`, above every scene
    /// whose text it selects.
    pub fn new(scene: &str, z: i32) -> Self {
        Self {
            scene: scene.to_string(),
            z,
            highlight: [0.25, 0.5, 1.0, 0.3],
        }
    }

    /// Draw the selection over its text in `color`, which should be translucent so that the
    /// text shows through.
    pub fn with_highlight(mut self, color: [f32; 4]) -> Self {
        self.highlight = color;
        self
    }

    /// The selection's highlights, one group for each clip of the labels it selects.
    fn highlights(&self, highlights: Vec<(Clip, Vec<[[f32; 2]; 4]>)>) -> Vec<SceneMark> {
        let highlight = |(clip, polygons): (Clip, Vec<[[f32; 2]; 4]>)| {
            let mut path = Path::builder();
            for polygon in &polygons {
                path.begin(point(polygon[0][0], polygon[0][1]));
                for corner in &polygon[1..] {
                    path.line_to(point(corner[0], corner[1]));
                }
                path.end(true);
            }
            let highlight = ScenePathMark {
                name: "text_selection".to_string(),
                clip: false,
                path: path.build().into(),
                fill: ColorOrGradient::Color(self.highlight).into(),
                ..Default::default()
            };
            SceneGroup {
                name: "text_selection".to_string(),
                interactive: Some(false),
                clip,
                marks: vec![highlight.into()],
                ..Default::default()
            }
            .into()
        };
        (highlights.into_iter())
            .filter(|(_, polygons)| !polygons.is_empty())
            .map(highlight)
            .collect()
    }
}

/// The selection's private state. Handlers see only hit-testing trees, which hold no text, so
/// the update indexes where each scene below draws its labels, and the handlers find labels in
/// that index.
#[derive(Debug, Default)]
pub struct SelectionState {
    /// Where each scene below the selection's draws its labels, from the top scene down.
    scenes: Vec<SceneLabels>,
    /// Whether the pointer is over text that a press would select.
    over_text: bool,
    /// The press that started the selection, if any, and where the drag from it went.
    press: Option<Press>,
    /// Whether the press's button is down, so moves extend the selection.
    dragging: bool,
    /// The selected text, as the last update found it, empty for none.
    text: String,
}

/// Where a scene draws its labels.
#[derive(Debug, Default)]
struct SceneLabels {
    labels: RTree<LabelBox>,
    /// The place of each of the scene's marks in its drawing order, by the mark's path, which
    /// tells whether a mark that takes hits draws over a label.
    orders: HashMap<Vec<usize>, usize>,
}

/// Where a scene draws a label, and the label.
#[derive(Debug, Clone)]
struct LabelBox {
    /// The label's text mark's path, and the label's instance.
    mark_path: Vec<usize>,
    instance: usize,
    /// The label's source, whether it's markup, and its options.
    source: String,
    markup: bool,
    options: LabelOptions,
    /// Where the label draws among the scene's labels, first to last, which is the order that a
    /// selection reads them in.
    order: usize,
    /// The place of the label's text mark in the scene's drawing order.
    draws: usize,
    placement: LabelPlacement,
    size: [f32; 2],
    clip: Clip,
    /// The box around the label, turned as it is.
    envelope: AABB<[f32; 2]>,
    /// The label's text layout, once a selection reads it, which the index keeps, so a selection
    /// across more labels than the engine remembers lays each out once.
    layout: Arc<OnceLock<Arc<TextLayout>>>,
}

impl RTreeObject for LabelBox {
    type Envelope = AABB<[f32; 2]>;

    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

impl PointDistance for LabelBox {
    /// The squared distance from `point` to the label's box, turned as it is.
    fn distance_2(&self, point: &[f32; 2]) -> f32 {
        let [x, y] = self.placement.to_label(*point);
        let [dx, dy] = [
            x - x.clamp(0.0, self.size[0]),
            y - y.clamp(0.0, self.size[1]),
        ];
        dx * dx + dy * dy
    }
}

impl LabelBox {
    /// Whether the label's box holds `point`, where its clip leaves it visible.
    fn holds(&self, point: [f32; 2]) -> bool {
        let [x, y] = self.placement.to_label(point);
        let inside = (0.0..=self.size[0]).contains(&x) && (0.0..=self.size[1]).contains(&y);
        inside && clip_shows(&self.clip, point)
    }

    /// Whether the point of the label's box nearest `point` shows through its clip.
    fn shows_near(&self, point: [f32; 2]) -> bool {
        let [x, y] = self.placement.to_label(point);
        let nearest = [x.clamp(0.0, self.size[0]), y.clamp(0.0, self.size[1])];
        clip_shows(&self.clip, self.placement.to_scene(nearest))
    }

    /// The label.
    fn label(&self) -> Label<'_> {
        let source = match self.markup {
            true => LabelSource::Markup(&self.source),
            false => LabelSource::Text(&self.source),
        };
        Label {
            source,
            options: self.options.clone(),
        }
    }

    /// The label's text layout, laid out once for the index.
    fn layout(&self, engine: &LabelEngine) -> Result<Arc<TextLayout>, LabelError> {
        if let Some(layout) = self.layout.get() {
            return Ok(layout.clone());
        }
        let layout = engine.text_layout(&self.label())?;
        Ok(self.layout.get_or_init(|| layout).clone())
    }

    /// Whether `other` is the same label of the same text mark, perhaps in another build.
    fn is(&self, other: &LabelBox) -> bool {
        self.mark_path == other.mark_path && self.instance == other.instance
    }
}

impl SceneLabels {
    /// Index where `scene` draws its labels, in the order it draws them. A label that doesn't lay
    /// out draws nothing, so it has no box.
    fn new(scene: &SceneGraph, engine: &LabelEngine) -> Self {
        let display = SceneDisplayList::from_scene_graph(scene);
        let mut orders = HashMap::new();
        let mut boxes = Vec::new();
        for (draws, item) in display.ordered_items().into_iter().enumerate() {
            orders.insert(item.mark_path.clone(), draws);
            let SceneDisplayMark::Borrowed(SceneMark::Text(mark)) = &item.mark else {
                continue;
            };
            for (instance, label) in mark.indices_iter().zip(mark.labels()) {
                let Ok(bounds) = engine.bounds(&label.label) else {
                    continue;
                };
                let placement = label.placement(&bounds, item.origin);
                let size = [bounds.width, bounds.height];
                let corners = placement.corners([0.0, 0.0, size[0], size[1]]);
                let (source, markup) = match label.label.source {
                    LabelSource::Text(text) => (text, false),
                    LabelSource::Markup(source) => (source, true),
                };
                boxes.push(LabelBox {
                    mark_path: item.mark_path.clone(),
                    instance,
                    source: source.to_string(),
                    markup,
                    options: label.label.options,
                    order: boxes.len(),
                    draws,
                    placement,
                    size,
                    clip: item.clip.clone(),
                    envelope: AABB::from_points(corners.iter()),
                    layout: Arc::default(),
                });
            }
        }
        Self {
            labels: RTree::bulk_load(boxes),
            orders,
        }
    }

    /// The topmost label at `point`.
    fn topmost(&self, point: [f32; 2]) -> Option<&LabelBox> {
        (self.labels)
            .locate_in_envelope_intersecting(AABB::from_point(point))
            .filter(|label| label.holds(point))
            .max_by_key(|label| label.order)
    }

    /// The label that a drag's end at `point` falls in: the topmost at the point, or else the
    /// nearest that shows there.
    fn near(&self, point: [f32; 2]) -> Option<&LabelBox> {
        self.topmost(point).or_else(|| {
            (self.labels.nearest_neighbor_iter(point)).find(|label| label.shows_near(point))
        })
    }

    /// Whether the instance `hit` draws over `label`: a later mark, or a later instance of the
    /// label's own mark. A mark the scene no longer draws, as a stale tree can hold, covers
    /// nothing.
    fn covers(&self, hit: &MarkInstance, label: &LabelBox) -> bool {
        let Some(&draws) = self.orders.get(&hit.mark_path) else {
            return false;
        };
        let instance = hit.instance_index.unwrap_or(usize::MAX);
        (draws, instance) > (label.draws, label.instance)
    }
}

/// An end of a selection: a point in the scene, and the label that it falls in.
#[derive(Debug, Clone)]
struct End {
    label: LabelBox,
    point: [f32; 2],
}

/// A press that selects text, and where the drag from it went.
#[derive(Debug, Clone)]
struct Press {
    /// The scene whose labels the selection reads, counted from the top scene down.
    scene: usize,
    /// Where the press went down, on a label.
    anchor: End,
    /// Where the drag from it is, in the label at the pointer, or else the nearest label.
    focus: End,
    /// How many presses in a row the press made: a second selects words and a third lines.
    count: u32,
    /// The anchor's label's source when an update first found it, so a changed label ends the
    /// selection.
    source: Option<String>,
}

impl SelectionState {
    /// The label at `point` that a press would select, with its scene, counted from the top
    /// scene down: the topmost label of the highest scene that holds one, where no mark that
    /// takes hits draws over it there. A mark that takes hits, over a label or with none beneath
    /// it, hides the labels of the scenes below.
    fn label_at(&self, trees: &SceneTrees<'_>, point: [f32; 2]) -> Option<(usize, &LabelBox)> {
        let trees = trees.below().iter().rev();
        for (index, (scene, tree)) in self.scenes.iter().zip(trees).enumerate() {
            let hit = hit_at(tree, point);
            match scene.topmost(point) {
                Some(label) if hit.is_none_or(|hit| !scene.covers(hit, label)) => {
                    return Some((index, label))
                }
                _ if hit.is_some() => return None,
                _ => {}
            }
        }
        None
    }

    /// Index the labels of each scene below that changed, or of every scene the first time, and
    /// return whether the anchor's scene changed.
    fn index(&mut self, scenes: &Scenes<'_>, below: &str) -> bool {
        let engine = scenes.text_engine();
        let views: Vec<_> = scenes.below(below).collect();
        let first = self.scenes.len() != views.len();
        if first {
            self.scenes = views.iter().map(|_| SceneLabels::default()).collect();
        }
        let mut anchor_changed = false;
        for (index, view) in views.iter().enumerate() {
            if first || view.changed() {
                self.scenes[index] = SceneLabels::new(view.graph(), engine);
                anchor_changed |= self
                    .press
                    .as_ref()
                    .is_some_and(|press| press.scene == index);
            }
        }
        anchor_changed
    }

    /// Find the pressed label again in what its scene draws now, or end the selection when the
    /// scene no longer draws it.
    fn find_again(&mut self) {
        let Some(press) = &self.press else {
            return;
        };
        let labels = &self.scenes[press.scene];
        let again = |end: &End| {
            labels
                .labels
                .iter()
                .find(|label| label.is(&end.label))
                .cloned()
        };
        let anchor = again(&press.anchor);
        // A focus whose label is gone falls in the label nearest its point.
        let focus = again(&press.focus).or_else(|| labels.near(press.focus.point).cloned());
        match (anchor, focus, &mut self.press) {
            (Some(anchor), Some(focus), Some(press)) => {
                press.anchor.label = anchor;
                press.focus.label = focus;
            }
            _ => {
                self.clear();
            }
        }
    }

    /// End the selection, and say whether there was one.
    fn clear(&mut self) -> bool {
        self.dragging = false;
        self.text.clear();
        self.press.take().is_some()
    }
}

/// The topmost instance of `tree` at `point`.
fn hit_at(tree: &SceneGraphRTree, point: [f32; 2]) -> Option<&MarkInstance> {
    let topmost = Query::new(QueryShape::Point(point), Relation::Intersects).limit(1);
    (tree.query(&topmost).into_iter().next()).map(|instance| &instance.mark_instance)
}

/// Drives the selection from its events: the stream that `claims` takes the presses on text, and
/// the other follows the pointer, the other presses, and Escape.
struct Driver {
    scene: String,
    claims: bool,
}

#[async_trait]
impl<State: Send + Sync + 'static> ToolHandler<State, SelectionState> for Driver {
    fn accepts(
        &self,
        event: &SceneGraphEvent,
        _app: &State,
        own: &SelectionState,
        trees: &SceneTrees<'_>,
    ) -> bool {
        let on_text = |down: &[f32; 2]| own.label_at(trees, *down).is_some();
        !self.claims
            || matches!(event, SceneGraphEvent::MouseDown(down)
                if down.button == MouseButton::Left && on_text(&down.position))
    }

    async fn handle(
        &self,
        event: &SceneGraphEvent,
        _app: &mut State,
        own: &mut SelectionState,
        trees: &SceneTrees<'_>,
    ) -> Rebuild {
        let redraw = |changed: bool| match changed {
            true => Rebuild::scene(&self.scene),
            false => Rebuild::none(),
        };
        match event {
            SceneGraphEvent::MouseDown(down) if self.claims => {
                own.clear();
                own.press = own.label_at(trees, down.position).map(|(scene, label)| {
                    let end = End {
                        label: label.clone(),
                        point: down.position,
                    };
                    Press {
                        scene,
                        anchor: end.clone(),
                        focus: end,
                        count: down.count,
                        source: None,
                    }
                });
                own.dragging = own.press.is_some();
                redraw(true)
            }
            SceneGraphEvent::MouseDown(down) if down.button == MouseButton::Left => {
                redraw(own.clear())
            }
            SceneGraphEvent::CursorMoved(moved) => {
                let point = moved.position;
                own.over_text = own.label_at(trees, point).is_some();
                let scene = (own.press.as_ref())
                    .filter(|_| own.dragging)
                    .map(|press| press.scene);
                let Some(scene) = scene else {
                    return Rebuild::none();
                };
                let label = own.scenes[scene].near(point).cloned();
                if let (Some(label), Some(press)) = (label, &mut own.press) {
                    press.focus = End { label, point };
                }
                redraw(true)
            }
            SceneGraphEvent::MouseUp(_)
            | SceneGraphEvent::MouseCancel(_)
            | SceneGraphEvent::WindowFocused(false) => {
                own.dragging = false;
                Rebuild::none()
            }
            SceneGraphEvent::CursorLeft => {
                own.over_text = false;
                Rebuild::none()
            }
            SceneGraphEvent::KeyPress(press) if press.key == Key::Named(NamedKey::Escape) => {
                redraw(own.clear())
            }
            _ => Rebuild::none(),
        }
    }
}

/// What a selection selects: its rectangles, as polygons in scene coordinates, with the clip of
/// their labels, its text, and its anchor label's source.
struct Found {
    highlights: Vec<(Clip, Vec<[[f32; 2]; 4]>)>,
    text: String,
    source: String,
}

/// What `press` selects among `labels`, its scene's: from its anchor to its focus, and every
/// label between them in the order they draw, whole.
fn find(press: &Press, labels: &SceneLabels, engine: &LabelEngine) -> Result<Found, LabelError> {
    let layout = |label: &LabelBox| label.layout(engine);
    let (anchor, focus) = (&press.anchor, &press.focus);
    let (anchor_layout, focus_layout) = (layout(&anchor.label)?, layout(&focus.label)?);
    let at = |end: &End, layout: &TextLayout| {
        let position = layout.position_at(end.label.placement.to_label(end.point));
        unit(layout, position, press.count)
    };
    let (from, to) = (at(anchor, &anchor_layout), at(focus, &focus_layout));
    let source = anchor_layout.source().to_string();
    // Each label that the selection reads, with its layout and the range it selects.
    let mut spans = Vec::new();
    if anchor.label.is(&focus.label) {
        let range = from.start.min(to.start)..from.end.max(to.end);
        spans.push((&anchor.label, anchor_layout, range));
    } else {
        let ((first, first_layout, start), (last, last_layout, end)) =
            if anchor.label.order < focus.label.order {
                ((anchor, anchor_layout, from), (focus, focus_layout, to))
            } else {
                ((focus, focus_layout, to), (anchor, anchor_layout, from))
            };
        let length = first_layout.source().len();
        spans.push((&first.label, first_layout, start.start..length));
        let mut between: Vec<&LabelBox> = (labels.labels.iter())
            .filter(|label| first.label.order < label.order && label.order < last.label.order)
            .collect();
        between.sort_by_key(|label| label.order);
        for label in between {
            let layout = layout(label)?;
            let length = layout.source().len();
            spans.push((label, layout, 0..length));
        }
        spans.push((&last.label, last_layout, 0..end.end));
    }

    let mut found = Found {
        highlights: Vec::new(),
        text: String::new(),
        source,
    };
    // The last label that gave text, and its selection's last line.
    let mut before: Option<(&LabelBox, Option<TextRect>)> = None;
    for (label, layout, range) in spans {
        let rects = layout.selection_rects(range.clone());
        let polygons = (rects.iter()).map(|rect| {
            label
                .placement
                .corners([rect.x, rect.y, rect.width, rect.height])
        });
        match found.highlights.last_mut() {
            Some((clip, shapes)) if *clip == label.clip => shapes.extend(polygons),
            _ => found
                .highlights
                .push((label.clip.clone(), polygons.collect())),
        }
        let text = layout.text_in(range);
        if text.is_empty() {
            continue;
        }
        if let Some((before, line)) = before {
            found
                .text
                .push(separator((before, line), (label, rects.first().copied())));
        }
        found.text.push_str(&text);
        before = Some((label, rects.last().copied()));
    }
    Ok(found)
}

/// The text between two labels in a selection's text, from the last selected line of the first
/// to the first selected line of the second: a tab where the second line goes on from the first,
/// lying across its middle and starting where it ends or to its right, as the first label reads,
/// and else a newline, so the text pastes into a spreadsheet as cells.
fn separator(
    (label, line): (&LabelBox, Option<TextRect>),
    (next, next_line): (&LabelBox, Option<TextRect>),
) -> char {
    let (Some(line), Some(next_line)) = (line, next_line) else {
        return '\n';
    };
    // The next line's box, in the first label's box.
    let rect = [next_line.x, next_line.y, next_line.width, next_line.height];
    let corners = (next.placement.corners(rect)).map(|corner| label.placement.to_label(corner));
    let (left, top, bottom) = corners.iter().fold(
        (f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY),
        |(left, top, bottom), [x, y]| (left.min(*x), top.min(*y), bottom.max(*y)),
    );
    let middle = (top + bottom) / 2.0;
    let across = (line.y..=line.y + line.height).contains(&middle);
    // A little overlap, as kerning or a tight gap leaves, still goes on from the line.
    let after = left >= line.x + line.width - line.height / 4.0;
    if across && after {
        '\t'
    } else {
        '\n'
    }
}

/// The least that a press at `position` selects: nothing after one press, its word after two, and
/// its line after three.
fn unit(layout: &TextLayout, position: TextPosition, count: u32) -> Range<usize> {
    match count {
        ..=1 => position.offset..position.offset,
        2 => layout.word_at(position),
        _ => layout.line_at(position),
    }
}

#[async_trait]
impl<State: Send + Sync + 'static> Tool<State> for TextSelection {
    type State = SelectionState;

    fn install(&self, tool: &mut ToolInstall<'_, State, SelectionState>) {
        let on = |types: &[SceneGraphEventType]| EventStreamConfig {
            types: types.to_vec(),
            ..Default::default()
        };
        // The press on text that starts a selection is the selection's alone.
        let claims = EventStreamConfig {
            consume: true,
            ..on(&[SceneGraphEventType::MouseDown])
        };
        let follows = on(&[
            SceneGraphEventType::CursorMoved,
            SceneGraphEventType::MouseDown,
            SceneGraphEventType::MouseUp,
            SceneGraphEventType::MouseCancel,
            SceneGraphEventType::KeyPress,
            SceneGraphEventType::WindowFocused,
            SceneGraphEventType::CursorLeft,
        ]);
        let driver = |claims| Driver {
            scene: self.scene.clone(),
            claims,
        };
        tool.scene(&self.scene, self.z)
            .stream(&self.scene, claims, driver(true))
            .stream(&self.scene, follows, driver(false));
    }

    /// Index the labels of the scenes below that changed, and draw the selection when it changed
    /// or its scene did. The selection ends when its scene no longer draws its anchor's label, or
    /// the label shows other text. Text that fails to lay out logs its error and ends the
    /// selection.
    async fn update(
        &self,
        _app: &mut State,
        own: &mut SelectionState,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        let anchor_changed = own.index(scenes, &self.scene);
        if anchor_changed {
            own.find_again();
        }
        if !scenes.requested(&self.scene) && !anchor_changed {
            return Ok(());
        }
        let found = (own.press.as_ref())
            .map(|press| find(press, &own.scenes[press.scene], scenes.text_engine()));
        let highlights = match found {
            None => Vec::new(),
            Some(Err(error)) => {
                log::warn!("Failed to lay out selected text: {error}");
                own.clear();
                Vec::new()
            }
            Some(Ok(found)) => {
                let press = own.press.as_mut().expect("a selection that found text");
                if (press.source.as_ref()).is_some_and(|source| *source != found.source) {
                    // The anchor's label shows other text, so the selection ends.
                    own.clear();
                    Vec::new()
                } else {
                    press.source = Some(found.source);
                    own.text = found.text;
                    found.highlights
                }
            }
        };
        let marks = self.highlights(highlights);
        if marks.is_empty() {
            return scenes.clear(&self.scene);
        }
        let [width, height] = scenes.size();
        let scene = SceneGraph {
            marks,
            width,
            height,
            origin: [0.0; 2],
        };
        scenes.set(&self.scene, scene.into())
    }

    fn cursor(&self, _app: &State, own: &SelectionState) -> Option<CursorStyle> {
        (own.over_text || own.dragging).then_some(CursorStyle::Text)
    }

    fn copy_text(&self, _app: &State, own: &SelectionState) -> Option<String> {
        (!own.text.is_empty()).then(|| own.text.clone())
    }
}
