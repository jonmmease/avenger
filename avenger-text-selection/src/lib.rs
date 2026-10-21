//! Text selection for an avenger app: an overlay that selects the text of any label in the
//! chart under a drag, with the event streams that find it, and copies it as the label shows it.
//! Add it to the app as an overlay:
//!
//! ```ignore
//! let app = app.with_overlay(Arc::new(TextSelection::new()))?;
//! ```
//!
//! A press on the label of any text mark, whether it takes hits or not, in the scene or in the
//! overlays added before this one, such as a pinned tooltip, starts a selection there, and a drag extends it within the label: by position after one press, by word after
//! two in a row, and by line after three. A press elsewhere and Escape clear it. The overlay
//! draws the selection over the text, calls for a text cursor over text, and offers the selected
//! text to copy. A press on text consumes nothing, so the app's own streams still see it.

use std::ops::Range;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use async_trait::async_trait;
use avenger_app::app::OverlayBuilder;
use avenger_app::error::AvengerAppError;
use avenger_color::ColorOrGradient;
use avenger_common::cursor::CursorStyle;
use avenger_eventstream::manager::EventStreamHandler;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::{Key, MouseButton, NamedKey};
use avenger_geometry::data::SceneData;
use avenger_geometry::rtree::{clip_shows, SceneGraphRTree};
use avenger_scenegraph::marks::group::{Clip, SceneGroup};
use avenger_scenegraph::marks::mark::SceneMark;
use avenger_scenegraph::marks::path::ScenePathMark;
use avenger_scenegraph::marks::text::{LabelPlacement, SceneTextMark, TextLabel};
use avenger_scenegraph::render::order::{SceneDisplayList, SceneDisplayMark};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::{LabelEngine, LabelError, TextLayout, TextPosition};
use itertools::izip;
use lyon_path::{geom::point, Path};
use rstar::{RTree, RTreeObject, AABB};

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

    /// The selected text, as its label shows it, or `None` without a selection.
    pub fn text(&self) -> Option<String> {
        let selecting = lock(&self.selecting);
        (!selecting.text.is_empty()).then(|| selecting.text.clone())
    }
}

/// The selection's state. Handlers see only the hit-testing tree, which holds only the marks
/// that take hits, so the overlay indexes where each new scene and each build of the overlays
/// beneath draw their labels, which handlers find the pointer's label in, and a press keeps its
/// label from the index.
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
    /// The press that started the selection, if any.
    press: Option<Press>,
    /// Whether the press's button is down, so moves extend the selection.
    dragging: bool,
    /// The selected text, as the last build found it, empty for none.
    text: String,
}

/// Where the scene draws a label.
#[derive(Debug, Clone)]
struct LabelBox {
    /// The label's text mark, its path, and the label's instance.
    mark: Arc<SceneTextMark>,
    mark_path: Vec<usize>,
    instance: usize,
    /// Whether an overlay beneath draws the label, rather than the scene.
    overlay: bool,
    /// Where the label draws among the scene's labels, bottom first.
    order: usize,
    placement: LabelPlacement,
    size: [f32; 2],
    clip: Clip,
    /// The box around the label, turned as it is.
    envelope: AABB<[f32; 2]>,
}

impl RTreeObject for LabelBox {
    type Envelope = AABB<[f32; 2]>;

    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

impl LabelBox {
    /// Whether the label's box holds `point`, where its clip leaves it visible.
    fn holds(&self, point: [f32; 2]) -> bool {
        let [x, y] = self.placement.to_label(point);
        let inside = (0.0..=self.size[0]).contains(&x) && (0.0..=self.size[1]).contains(&y);
        inside && clip_shows(&self.clip, &point)
    }

    /// The label, from its text mark.
    fn label(&self) -> Option<TextLabel<'_>> {
        izip!(self.mark.indices_iter(), self.mark.labels())
            .find_map(|(instance, label)| (instance == self.instance).then_some(label))
    }

    /// Whether `other` is the same label of the same text mark, perhaps in another build.
    fn is(&self, other: &LabelBox) -> bool {
        self.overlay == other.overlay
            && self.mark_path == other.mark_path
            && self.instance == other.instance
    }
}

/// A press that selects text, and where the drag from it went.
#[derive(Debug, Clone)]
struct Press {
    /// The label that the press went down on.
    label: LabelBox,
    /// Where the press went down, and where the drag from it is, in scene coordinates.
    from: [f32; 2],
    to: [f32; 2],
    /// How many presses in a row the press made: a second selects words and a third lines.
    count: u32,
    /// The label's source when the build first found it, so a changed label ends the selection.
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
        let Some(press) = &mut self.press else {
            return;
        };
        let labels = match press.label.overlay {
            true => &self.overlay_labels,
            false if new_scene => &self.labels,
            false => return,
        };
        let label = labels.iter().find(|label| label.is(&press.label)).cloned();
        match label {
            Some(label) => press.label = label,
            None => {
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
                selecting.over_text = selecting.label_at(moved.position).is_some();
                let dragging = selecting.dragging;
                match selecting.press.as_mut().filter(|_| dragging) {
                    Some(press) => {
                        press.to = moved.position;
                        UpdateStatus::OVERLAY
                    }
                    None => UpdateStatus::UNCHANGED,
                }
            }
            SceneGraphEvent::MouseDown(down) if down.button == MouseButton::Left => {
                let had = selecting.clear();
                selecting.press = selecting.label_at(down.position).map(|label| Press {
                    label: label.clone(),
                    from: down.position,
                    to: down.position,
                    count: down.count,
                    source: None,
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
            boxes.push(LabelBox {
                mark: mark.clone(),
                mark_path: item.mark_path.clone(),
                instance,
                overlay,
                order: boxes.len(),
                placement,
                size,
                clip: item.clip.clone(),
                envelope: AABB::from_points(corners.iter()),
            });
        }
    }
    RTree::bulk_load(boxes)
}

/// A selection that the build found in its label.
struct Found {
    /// The selection's rectangles, as polygons in scene coordinates.
    polygons: Vec<[[f32; 2]; 4]>,
    text: String,
    /// The label's source.
    source: String,
}

/// The selection that `press` makes in its label.
fn find(press: &Press, engine: &LabelEngine) -> Result<Option<Found>, LabelError> {
    let Some(label) = press.label.label() else {
        return Ok(None);
    };
    let layout = engine.text_layout(&label.label)?;
    let placement = &press.label.placement;
    let from = layout.position_at(placement.to_label(press.from));
    let to = layout.position_at(placement.to_label(press.to));
    let range = selected(&layout, from, to, press.count);
    let polygons = layout
        .selection_rects(range.clone())
        .into_iter()
        .map(|rect| placement.corners([rect.x, rect.y, rect.width, rect.height]))
        .collect();
    Ok(Some(Found {
        polygons,
        text: layout.text_in(range),
        source: layout.source().to_string(),
    }))
}

/// The range that a drag from `from` to `to` selects: the text between them after one press,
/// the words that they touch after two, and their lines after three.
fn selected(layout: &TextLayout, from: TextPosition, to: TextPosition, count: u32) -> Range<usize> {
    let span = |a: Range<usize>, b: Range<usize>| a.start.min(b.start)..a.end.max(b.end);
    match count {
        ..=1 => from.offset.min(to.offset)..from.offset.max(to.offset),
        2 => span(layout.word_at(from), layout.word_at(to)),
        _ => span(layout.line_at(from), layout.line_at(to)),
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
        let found = find(press, engine).map_err(error)?;
        let clip = press.label.clip.clone();
        let Some(found) = found
            .filter(|found| (press.source.as_ref()).is_none_or(|source| *source == found.source))
        else {
            // The label shows other text, so the selection ends.
            selecting.clear();
            return Ok(Vec::new());
        };
        if let Some(press) = &mut selecting.press {
            press.source = Some(found.source);
        }
        selecting.text = found.text;
        if found.polygons.is_empty() {
            return Ok(Vec::new());
        }
        let mut path = Path::builder();
        for polygon in &found.polygons {
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
        let group = SceneGroup {
            name: "text_selection".to_string(),
            clip,
            marks: vec![highlight.into()],
            ..Default::default()
        };
        Ok(vec![group.into()])
    }

    fn streams(&self) -> Vec<(EventStreamConfig, Arc<dyn EventStreamHandler<State>>)> {
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
        vec![(config, Arc::new(Driver(self.selecting.clone())))]
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
