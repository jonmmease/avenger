//! Text selection for an avenger app: an overlay that selects the text of any label in the
//! chart under a drag, with the event streams that find it, and copies it as the label shows it.
//! Add it to the app as an overlay:
//!
//! ```ignore
//! let app = app.with_overlay(Arc::new(TextSelection::new()))?;
//! ```
//!
//! A press on the label of any text mark, whether it takes hits or not, in the scene or in the
//! overlays added before this one, such as a pinned tooltip, starts a selection there, and a drag
//! extends it across the labels that the chart draws, in the order it draws them: by position
//! after one press, by word after two in a row, and by line after three. A press elsewhere and
//! Escape clear it. The overlay draws the selection over the text, calls for a text cursor over
//! text, and offers the selected text to copy. A press on an overlay's text is the selection's
//! alone, and a press on the scene's text still reaches the app's own streams.

use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};

use async_trait::async_trait;
use avenger_app::app::OverlayBuilder;
use avenger_app::error::AvengerAppError;
use avenger_color::ColorOrGradient;
use avenger_common::cursor::CursorStyle;
use avenger_eventstream::manager::EventStreamHandler;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, EventStreamFilter, UpdateStatus};
use avenger_eventstream::window::{Key, MouseButton, NamedKey};
use avenger_geometry::data::SceneData;
use avenger_geometry::rtree::{clip_shows, SceneGraphRTree};
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::path::ScenePathMark;
use avenger_scenegraph::marks::text::LabelPlacement;
use avenger_scenegraph::render::order::{SceneDisplayList, SceneDisplayMark};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::{
    Label, LabelEngine, LabelError, LabelOptions, LabelSource, TextLayout, TextPosition, TextRect,
};
use itertools::izip;
use lyon_path::{geom::point, Path};
use rstar::{PointDistance, RTree, RTreeObject, AABB};

/// Text selection: an overlay that selects the text of any label in the chart under a drag,
/// and the event streams that drive it. Clones share the selection.
#[derive(Clone)]
pub struct TextSelection {
    selecting: Arc<Mutex<Selecting>>,
    highlight: [f32; 4],
}

impl Default for TextSelection {
    fn default() -> Self {
        Self::new()
    }
}

impl TextSelection {
    pub fn new() -> Self {
        Self {
            selecting: Arc::default(),
            highlight: [0.25, 0.5, 1.0, 0.3],
        }
    }

    /// The selection, drawn over its text in `color`, which should be translucent so that the
    /// text shows through.
    pub fn with_highlight(mut self, color: [f32; 4]) -> Self {
        self.highlight = color;
        self
    }

    /// The selected text, as its labels show it, or `None` without a selection.
    pub fn text(&self) -> Option<String> {
        let selecting = lock(&self.selecting);
        (!selecting.text.is_empty()).then(|| selecting.text.clone())
    }
}

/// The selection's state. Handlers see only the hit-testing tree, which holds only the marks
/// that take hits, so the overlay indexes where each new scene and each build of the overlays
/// beneath draw their labels, which handlers find the pointer's label in, and a selection keeps
/// its ends' labels from the index.
#[derive(Debug, Default)]
struct Selecting {
    /// Where the scene draws its labels.
    labels: RTree<LabelBox>,
    /// The scene that `labels` index. Holding it keeps its address from going to a newer scene.
    scene: Weak<SceneGraph>,
    /// Where the overlays beneath draw their labels, over the scene's.
    overlay_labels: RTree<LabelBox>,
    /// Where the overlays beneath draw marks that take hits, which hide the scene's labels
    /// beneath them, or none when they draw nothing.
    overlays: Option<SceneGraphRTree>,
    /// Whether the pointer is over a label.
    over_text: bool,
    /// The press that started the selection, if any, and where the drag from it went.
    press: Option<Press>,
    /// Whether the press's button is down, so moves extend the selection.
    dragging: bool,
    /// The selected text, as the last build found it, empty for none.
    text: String,
}

/// Where the scene draws a label, and the label.
#[derive(Debug, Clone)]
struct LabelBox {
    /// The label's text mark's path, and the label's instance.
    mark_path: Vec<usize>,
    instance: usize,
    /// The label's source, whether it's markup, and its options.
    source: String,
    markup: bool,
    options: LabelOptions,
    /// Whether an overlay beneath draws the label, rather than the scene.
    overlay: bool,
    /// Where the label draws among its layer's labels, first to last, which is the order that a
    /// selection reads them in.
    order: usize,
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
        inside && clip_shows(&self.clip, &point)
    }

    /// Whether the point of the label's box nearest `point` shows through its clip.
    fn shows_near(&self, point: [f32; 2]) -> bool {
        let [x, y] = self.placement.to_label(point);
        let nearest = [x.clamp(0.0, self.size[0]), y.clamp(0.0, self.size[1])];
        clip_shows(&self.clip, &self.placement.to_scene(nearest))
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
        self.overlay == other.overlay
            && self.mark_path == other.mark_path
            && self.instance == other.instance
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
    /// Where the press went down, on a label.
    anchor: End,
    /// Where the drag from it is, in the label at the pointer, or else the nearest label, among
    /// the labels of the anchor's layer.
    focus: End,
    /// How many presses in a row the press made: a second selects words and a third lines.
    count: u32,
    /// The anchor's label's source when the build first found it, so a changed label ends the
    /// selection.
    source: Option<String>,
}

impl Selecting {
    /// The topmost label at `point`: an overlay's, or else the scene's where no overlay draws
    /// over it, as a pinned tooltip's box does.
    fn label_at(&self, point: [f32; 2]) -> Option<&LabelBox> {
        topmost(&self.overlay_labels, point).or_else(|| {
            let covered = (self.overlays.as_ref())
                .is_some_and(|overlays| overlays.pick_top_mark_at_point(&point).is_some());
            topmost(&self.labels, point).filter(|_| !covered)
        })
    }

    /// The labels of the overlays beneath when `overlay`, and else the scene's.
    fn layer(&self, overlay: bool) -> &RTree<LabelBox> {
        match overlay {
            true => &self.overlay_labels,
            false => &self.labels,
        }
    }

    /// Index where `scene`, when it's new, and the overlays `beneath` draw their labels, and
    /// find the pressed label again in what they draw now, or end the selection when nothing
    /// draws the label.
    fn index(&mut self, scene: &Arc<SceneGraph>, beneath: &[SceneMark], engine: &LabelEngine) {
        let new_scene = !std::ptr::eq(self.scene.as_ptr(), Arc::as_ptr(scene));
        if new_scene {
            let display = SceneDisplayList::from_scene_graph(scene);
            self.labels = label_boxes(&display, false, engine);
            self.scene = Arc::downgrade(scene);
        }
        // The overlays beneath draw anew with each build, and draw over the scene, as the app
        // composes them.
        (self.overlay_labels, self.overlays) = if beneath.is_empty() {
            (RTree::new(), None)
        } else {
            let overlays = SceneGraph {
                marks: beneath.to_vec(),
                ..**scene
            };
            let display = SceneDisplayList::from_scene_graph(&overlays);
            let hits = SceneGraphRTree::from_scene_graph(&overlays, engine);
            (label_boxes(&display, true, engine), Some(hits))
        };
        let Some(press) = &self.press else {
            return;
        };
        let overlay = press.anchor.label.overlay;
        if !overlay && !new_scene {
            return;
        }
        let labels = self.layer(overlay);
        let again = |end: &End| labels.iter().find(|label| label.is(&end.label)).cloned();
        let anchor = again(&press.anchor);
        // A focus whose label is gone falls in the label nearest its point.
        let focus = again(&press.focus).or_else(|| near(labels, press.focus.point).cloned());
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

/// The topmost of `labels` at `point`.
fn topmost(labels: &RTree<LabelBox>, point: [f32; 2]) -> Option<&LabelBox> {
    labels
        .locate_in_envelope_intersecting(AABB::from_point(point))
        .filter(|label| label.holds(point))
        .max_by_key(|label| label.order)
}

/// The label of `labels` that a drag's end at `point` falls in: the topmost at the point, or else
/// the nearest that shows there.
fn near(labels: &RTree<LabelBox>, point: [f32; 2]) -> Option<&LabelBox> {
    topmost(labels, point)
        .or_else(|| (labels.nearest_neighbor_iter(point)).find(|label| label.shows_near(point)))
}

/// The selection, through a lock that a handler that panicked leaves usable.
fn lock(selecting: &Mutex<Selecting>) -> MutexGuard<'_, Selecting> {
    selecting
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

/// Drives the selection from its events.
struct Driver(Arc<Mutex<Selecting>>);

#[async_trait]
impl<State: Clone + Send + Sync + 'static> EventStreamHandler<State> for Driver {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        _state: &mut State,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let mut selecting = lock(&self.0);
        match event {
            SceneGraphEvent::CursorMoved(moved) => {
                let point = moved.position;
                selecting.over_text = selecting.label_at(point).is_some();
                let overlay = (selecting.press.as_ref())
                    .filter(|_| selecting.dragging)
                    .map(|press| press.anchor.label.overlay);
                let Some(overlay) = overlay else {
                    return UpdateStatus::UNCHANGED;
                };
                let label = near(selecting.layer(overlay), point).cloned();
                if let (Some(label), Some(press)) = (label, &mut selecting.press) {
                    press.focus = End { label, point };
                }
                UpdateStatus::OVERLAY
            }
            SceneGraphEvent::MouseDown(down) if down.button == MouseButton::Left => {
                let had = selecting.clear();
                selecting.press = selecting.label_at(down.position).map(|label| {
                    let end = End {
                        label: label.clone(),
                        point: down.position,
                    };
                    Press {
                        anchor: end.clone(),
                        focus: end,
                        count: down.count,
                        source: None,
                    }
                });
                selecting.dragging = selecting.press.is_some();
                if had || selecting.press.is_some() {
                    UpdateStatus::OVERLAY
                } else {
                    UpdateStatus::UNCHANGED
                }
            }
            SceneGraphEvent::MouseUp(_) | SceneGraphEvent::WindowFocused(false) => {
                selecting.dragging = false;
                UpdateStatus::UNCHANGED
            }
            SceneGraphEvent::CursorLeft => {
                selecting.over_text = false;
                UpdateStatus::UNCHANGED
            }
            SceneGraphEvent::KeyPress(press) if press.key == Key::Named(NamedKey::Escape) => {
                if selecting.clear() {
                    UpdateStatus::OVERLAY
                } else {
                    UpdateStatus::UNCHANGED
                }
            }
            _ => UpdateStatus::UNCHANGED,
        }
    }
}

/// Where a scene, or the overlays when `overlay`, draws its labels, in the order it draws them.
/// A label that doesn't lay out draws nothing, so it has no box.
fn label_boxes(display: &SceneDisplayList, overlay: bool, engine: &LabelEngine) -> RTree<LabelBox> {
    let mut boxes = Vec::new();
    for item in display.ordered_items() {
        let SceneDisplayMark::Borrowed(SceneMark::Text(mark)) = &item.mark else {
            continue;
        };
        for (instance, label) in izip!(mark.indices_iter(), mark.labels()) {
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
                overlay,
                order: boxes.len(),
                placement,
                size,
                clip: item.clip.clone(),
                envelope: AABB::from_points(corners.iter()),
                layout: Arc::default(),
            });
        }
    }
    RTree::bulk_load(boxes)
}

/// What a selection selects: its rectangles, as polygons in scene coordinates, with the clip of
/// their labels, its text, and its anchor label's source.
struct Found {
    highlights: Vec<(Clip, Vec<[[f32; 2]; 4]>)>,
    text: String,
    source: String,
}

/// What `press` selects among `labels`, its anchor's layer's: from its anchor to its focus, and
/// every label between them in the order they draw, whole.
fn find(
    press: &Press,
    labels: &RTree<LabelBox>,
    engine: &LabelEngine,
) -> Result<Found, LabelError> {
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
        let mut between: Vec<&LabelBox> = (labels.iter())
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

impl<State: Clone + Send + Sync + 'static> OverlayBuilder<State> for TextSelection {
    fn build(
        &self,
        _state: &State,
        scene: &Arc<SceneGraph>,
        beneath: &[SceneMark],
        _data: &SceneData,
        engine: &LabelEngine,
    ) -> Result<Vec<SceneMark>, AvengerAppError> {
        let error = |err: LabelError| AvengerAppError::InternalError(err.to_string());
        let mut selecting = lock(&self.selecting);
        selecting.index(scene, beneath, engine);
        let Some(press) = &selecting.press else {
            return Ok(Vec::new());
        };
        let labels = selecting.layer(press.anchor.label.overlay);
        let found = find(press, labels, engine).map_err(error)?;
        if (press.source.as_ref()).is_some_and(|source| *source != found.source) {
            // The anchor's label shows other text, so the selection ends.
            selecting.clear();
            return Ok(Vec::new());
        }
        if let Some(press) = &mut selecting.press {
            press.source = Some(found.source);
        }
        selecting.text = found.text;
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
                clip,
                marks: vec![highlight.into()],
                ..Default::default()
            }
            .into()
        };
        Ok((found.highlights.into_iter())
            .filter(|(_, polygons)| !polygons.is_empty())
            .map(highlight)
            .collect())
    }

    fn streams(&self) -> Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> {
        // A press on the text of an overlay beneath, which lies over the scene, and the clicks
        // that end a press that selects it, are the selection's alone, so a stream of the
        // scene's, such as a pan, doesn't act on them. A press on the scene's own text still
        // reaches the scene's streams, which can act on their text, as a legend's toggle does.
        let selecting = self.selecting.clone();
        let on_overlay_text = EventStreamFilter(Arc::new(move |event: &SceneGraphEvent| {
            let selecting = lock(&selecting);
            match event {
                SceneGraphEvent::MouseDown(down) => {
                    let label = selecting.label_at(down.position);
                    down.button == MouseButton::Left && label.is_some_and(|label| label.overlay)
                }
                SceneGraphEvent::Click(click) if click.button != MouseButton::Left => false,
                SceneGraphEvent::Click(_) | SceneGraphEvent::DoubleClick(_) => {
                    (selecting.press.as_ref()).is_some_and(|press| press.anchor.label.overlay)
                }
                _ => false,
            }
        }));
        let claims = EventStreamConfig {
            types: vec![
                SceneGraphEventType::MouseDown,
                SceneGraphEventType::Click,
                SceneGraphEventType::DoubleClick,
            ],
            filter: Some(vec![on_overlay_text]),
            consume: true,
            ..Default::default()
        };
        let config = EventStreamConfig {
            types: vec![
                SceneGraphEventType::CursorMoved,
                SceneGraphEventType::MouseDown,
                SceneGraphEventType::MouseUp,
                SceneGraphEventType::KeyPress,
                SceneGraphEventType::WindowFocused,
                SceneGraphEventType::CursorLeft,
            ],
            ..Default::default()
        };
        let driver = Arc::new(Driver(self.selecting.clone()));
        vec![(claims, driver.clone()), (config, driver)]
    }

    fn cursor(&self, _state: &State) -> Option<CursorStyle> {
        let selecting = lock(&self.selecting);
        let selecting_text = selecting.dragging && !selecting.text.is_empty();
        (selecting.over_text || selecting_text).then_some(CursorStyle::Text)
    }

    fn copy_text(&self, _state: &State) -> Option<String> {
        self.text()
    }
}
