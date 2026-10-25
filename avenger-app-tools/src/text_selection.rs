//! Select and copy label text from scenes below the tool's scene.
//!
//! A left-button press starts a selection on the topmost label under the pointer, unless an
//! interactive mark covers it. Text marks can be selected even when they are not interactive.
//! The tool consumes the press, preventing lower scenes from starting a drag.
//!
//! Dragging extends the selection across labels in the same scene, in drawing order. One press
//! selects by position, two consecutive presses select by word, and three or more select by
//! line. A left-button press elsewhere or Escape clears the selection. A selection keeps its
//! text when its labels move, and a scene with a stale hit-testing tree selects no text.
//!
//! The tool draws highlights, requests a text cursor over selectable text and during a drag,
//! and provides the selected text to the host's copy shortcut. The cursor is vertical for a
//! label turned within 45 degrees of vertical, and a drag keeps its anchor label's cursor.

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

/// Selects text from labels in lower scenes and draws highlights in its own scene.
#[derive(Clone)]
pub struct TextSelection {
    scene: String,
    z: i32,
    highlight: [f32; 4],
}

impl TextSelection {
    /// Create a tool with a highlight scene named `scene` at z-order `z`.
    /// Choose a z-order above every scene whose text should be selectable.
    pub fn new(scene: &str, z: i32) -> Self {
        Self {
            scene: scene.to_string(),
            z,
            highlight: [0.25, 0.5, 1.0, 0.3],
        }
    }

    /// Set the RGBA highlight color. Use a translucent color to keep the selected text visible.
    pub fn with_highlight(mut self, color: [f32; 4]) -> Self {
        self.highlight = color;
        self
    }

    /// Build highlight marks, grouping consecutive labels that share a clip.
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

/// Private selection state and label indices.
/// Handlers use the indices to find label text and positions because hit-testing trees contain
/// no text. The tool updates the indices when lower scenes change.
#[derive(Debug, Default)]
pub struct SelectionState {
    /// Label indices for lower scenes, ordered from top to bottom.
    scenes: Vec<SceneLabels>,
    /// The pointer's last position over the window, if it is over the window.
    pointer: Option<[f32; 2]>,
    /// The cursor for the text under the pointer that a press would select, if any.
    hover: Option<CursorStyle>,
    /// The selection anchor and current drag endpoint, if a selection exists.
    press: Option<Press>,
    /// Whether pointer movement extends the selection.
    dragging: bool,
    /// Selected text from the last update, or an empty string if nothing is selected.
    text: String,
}

/// A scene's label positions and mark drawing order.
#[derive(Debug, Default)]
struct SceneLabels {
    labels: RTree<LabelBox>,
    /// Drawing order by mark path, used to detect interactive marks that cover a label.
    orders: HashMap<Vec<usize>, usize>,
    /// Whether the scene's hit-testing tree shows the graph that the labels come from. A stale
    /// tree, as during a pan, can't tell which marks cover a label.
    current: bool,
}

/// A label's content, position, and drawing order.
#[derive(Debug, Clone)]
struct LabelBox {
    /// Path to the text mark that contains the label.
    mark_path: Vec<usize>,
    /// Instance index within the text mark.
    instance: usize,
    /// Label source text.
    source: String,
    /// Whether the source contains Typst markup.
    markup: bool,
    options: LabelOptions,
    /// Label drawing order, which also determines selection order.
    order: usize,
    /// Drawing order of the label's text mark.
    draws: usize,
    placement: LabelPlacement,
    size: [f32; 2],
    clip: Clip,
    /// Axis-aligned bounding box around the rotated label.
    envelope: AABB<[f32; 2]>,
    /// Layout cached on first use and retained until the index is rebuilt. This avoids repeated
    /// layout when a selection exceeds the engine's cache capacity.
    layout: Arc<OnceLock<Arc<TextLayout>>>,
}

impl RTreeObject for LabelBox {
    type Envelope = AABB<[f32; 2]>;

    fn envelope(&self) -> Self::Envelope {
        self.envelope
    }
}

impl PointDistance for LabelBox {
    /// Return the squared distance from `point` to the rotated label box.
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
    /// Return whether `point` is inside the label box and visible through its clip.
    fn holds(&self, point: [f32; 2]) -> bool {
        let [x, y] = self.placement.to_label(point);
        let inside = (0.0..=self.size[0]).contains(&x) && (0.0..=self.size[1]).contains(&y);
        inside && clip_shows(&self.clip, point)
    }

    /// Return whether the nearest point on the label box is visible through its clip.
    fn shows_near(&self, point: [f32; 2]) -> bool {
        let [x, y] = self.placement.to_label(point);
        let nearest = [x.clamp(0.0, self.size[0]), y.clamp(0.0, self.size[1])];
        clip_shows(&self.clip, self.placement.to_scene(nearest))
    }

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

    /// Return the label's layout, caching the first successful result in the index.
    fn layout(&self, engine: &LabelEngine) -> Result<Arc<TextLayout>, LabelError> {
        if let Some(layout) = self.layout.get() {
            return Ok(layout.clone());
        }
        let layout = engine.text_layout(&self.label())?;
        Ok(self.layout.get_or_init(|| layout).clone())
    }

    /// Return whether `other` has the same mark path and instance index, even across builds.
    fn is(&self, other: &LabelBox) -> bool {
        self.mark_path == other.mark_path && self.instance == other.instance
    }

    /// Return the text cursor for the label: vertical when the label is turned within 45 degrees
    /// of vertical, and horizontal otherwise.
    fn cursor(&self) -> CursorStyle {
        let turn = self.placement.angle.rem_euclid(180.0);
        if turn > 45.0 && turn < 135.0 {
            CursorStyle::VerticalText
        } else {
            CursorStyle::Text
        }
    }
}

impl SceneLabels {
    /// Index label positions and drawing order in `scene`, whose tree is `current` or stale.
    /// Skip labels whose bounds cannot be computed.
    fn new(scene: &SceneGraph, current: bool, engine: &LabelEngine) -> Self {
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
            current,
        }
    }

    /// Return the topmost label whose visible box contains `point`.
    fn topmost(&self, point: [f32; 2]) -> Option<&LabelBox> {
        (self.labels)
            .locate_in_envelope_intersecting(AABB::from_point(point))
            .filter(|label| label.holds(point))
            .max_by_key(|label| label.order)
    }

    /// Return the topmost label at `point`, or the nearest label whose nearest point is not
    /// clipped.
    fn near(&self, point: [f32; 2]) -> Option<&LabelBox> {
        self.topmost(point).or_else(|| {
            (self.labels.nearest_neighbor_iter(point)).find(|label| label.shows_near(point))
        })
    }

    /// Return whether `hit` draws over `label` by comparing mark and instance order. Ignore
    /// hits for marks no longer in the scene.
    fn covers(&self, hit: &MarkInstance, label: &LabelBox) -> bool {
        let Some(&draws) = self.orders.get(&hit.mark_path) else {
            return false;
        };
        let instance = hit.instance_index.unwrap_or(usize::MAX);
        (draws, instance) > (label.draws, label.instance)
    }
}

/// A selection endpoint: its label and its position in the label's source.
#[derive(Debug, Clone)]
struct End {
    label: LabelBox,
    /// The pointer's position in the scene when it set the end, which finds the nearest label
    /// when the end's label disappears.
    point: [f32; 2],
    /// The position in the label's source, or `None` until the next update resolves `point`.
    position: Option<TextPosition>,
}

impl End {
    /// An end at `point` in `label`, which the next update resolves.
    fn at(label: LabelBox, point: [f32; 2]) -> Self {
        Self {
            label,
            point,
            position: None,
        }
    }

    /// Resolve `point` to a position in the label's source, through the label's placement when
    /// the pointer set the end. A resolved end keeps its position when the label moves.
    fn resolve(&mut self, engine: &LabelEngine) -> Result<TextPosition, LabelError> {
        if let Some(position) = self.position {
            return Ok(position);
        }
        let layout = self.label.layout(engine)?;
        let position = layout.position_at(self.label.placement.to_label(self.point));
        self.position = Some(position);
        Ok(position)
    }
}

/// A selection's anchor, focus, and press count.
#[derive(Debug, Clone)]
struct Press {
    /// Selected scene index, counted from the topmost scene below the tool.
    scene: usize,
    /// The label and point where the selection started.
    anchor: End,
    /// Current drag endpoint, associated with the label under the pointer or the nearest label.
    focus: End,
    /// Consecutive press count: two presses select words, three or more select lines.
    count: u32,
}

impl Press {
    /// Resolve both ends to positions in their labels' sources.
    fn resolve(&mut self, engine: &LabelEngine) -> Result<(), LabelError> {
        self.anchor.resolve(engine)?;
        self.focus.resolve(engine)?;
        Ok(())
    }
}

impl SelectionState {
    /// Find the topmost selectable label at `point` and its scene index, counted from top to
    /// bottom, with the scenes' `trees` from top to bottom.
    /// An interactive hit blocks labels drawn beneath it in the same scene or a lower scene. A
    /// scene with a stale tree selects none of its labels, and one at `point` blocks the scenes
    /// below.
    fn label_at<'t>(
        &self,
        trees: impl Iterator<Item = &'t SceneGraphRTree>,
        point: [f32; 2],
    ) -> Option<(usize, &LabelBox)> {
        for (index, (scene, tree)) in self.scenes.iter().zip(trees).enumerate() {
            let hit = hit_at(tree, point);
            match scene.topmost(point) {
                Some(label) if scene.current && hit.is_none_or(|hit| !scene.covers(hit, label)) => {
                    return Some((index, label))
                }
                Some(_) => return None,
                None if hit.is_some() => return None,
                None => {}
            }
        }
        None
    }

    /// Reindex changed scenes below `below`, or all lower scenes on the first call.
    /// Return the indices of the reindexed scenes, counted from top to bottom.
    fn index(&mut self, scenes: &Scenes<'_>, below: &str) -> Vec<usize> {
        let engine = scenes.text_engine();
        let views: Vec<_> = scenes.below(below).collect();
        let first = self.scenes.len() != views.len();
        if first {
            self.scenes = views.iter().map(|_| SceneLabels::default()).collect();
        }
        let mut reindexed = Vec::new();
        for (index, view) in views.iter().enumerate() {
            if first || view.changed() {
                self.scenes[index] = SceneLabels::new(view.graph(), view.is_current(), engine);
                reindexed.push(index);
            }
        }
        reindexed
    }

    /// Refresh selection endpoints after a rebuild. An end whose label the scene still draws,
    /// with the same source, keeps its position in the label wherever the label moves. Clear
    /// the selection if the anchor label is gone or shows other text, and move a focus whose
    /// label is gone to the label nearest its point.
    fn find_again(&mut self) {
        let Some(press) = &self.press else {
            return;
        };
        let labels = &self.scenes[press.scene];
        let again = |end: &End| {
            let same = |label: &&LabelBox| label.is(&end.label) && label.source == end.label.source;
            let label = labels.labels.iter().find(same)?.clone();
            Some(End {
                label,
                ..end.clone()
            })
        };
        let anchor = again(&press.anchor);
        let focus = again(&press.focus).or_else(|| {
            let label = labels.near(press.focus.point)?.clone();
            Some(End::at(label, press.focus.point))
        });
        match (anchor, focus, &mut self.press) {
            (Some(anchor), Some(focus), Some(press)) => {
                press.anchor = anchor;
                press.focus = focus;
            }
            _ => {
                self.clear();
            }
        }
    }

    /// Clear the selection and return whether one existed.
    fn clear(&mut self) -> bool {
        self.dragging = false;
        self.text.clear();
        self.press.take().is_some()
    }
}

/// Return the topmost mark instance at `point` in `tree`.
fn hit_at(tree: &SceneGraphRTree, point: [f32; 2]) -> Option<&MarkInstance> {
    let topmost = Query::new(QueryShape::Point(point), Relation::Intersects).limit(1);
    (tree.query(&topmost).into_iter().next()).map(|instance| &instance.mark_instance)
}

/// Handles selection events. A consuming stream claims left-button presses on selectable text.
/// A second stream tracks pointer movement, other presses, and Escape.
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
        let below = || trees.below().iter().rev();
        let on_text = |down: &[f32; 2]| own.label_at(below(), *down).is_some();
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
        let below = || trees.below().iter().rev();
        match event {
            SceneGraphEvent::MouseDown(down) if self.claims => {
                own.clear();
                own.press = own.label_at(below(), down.position).map(|(scene, label)| {
                    let end = End::at(label.clone(), down.position);
                    Press {
                        scene,
                        anchor: end.clone(),
                        focus: end,
                        count: down.count,
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
                own.pointer = Some(point);
                own.hover = own
                    .label_at(below(), point)
                    .map(|(_, label)| label.cursor());
                let scene = (own.press.as_ref())
                    .filter(|_| own.dragging)
                    .map(|press| press.scene);
                let Some(scene) = scene else {
                    return Rebuild::none();
                };
                let label = own.scenes[scene].near(point).cloned();
                if let (Some(label), Some(press)) = (label, &mut own.press) {
                    press.focus = End::at(label, point);
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
                own.pointer = None;
                own.hover = None;
                Rebuild::none()
            }
            SceneGraphEvent::KeyPress(press) if press.key == Key::Named(NamedKey::Escape) => {
                redraw(own.clear())
            }
            _ => Rebuild::none(),
        }
    }
}

/// Selected text, and highlight polygons and clips in scene coordinates.
struct Found {
    highlights: Vec<(Clip, Vec<[[f32; 2]; 4]>)>,
    text: String,
}

/// Compute the text and highlights between the anchor and focus of `press`, whose ends
/// [`Press::resolve`] resolved. Include every intermediate label in full, in drawing order.
fn find(press: &Press, labels: &SceneLabels, engine: &LabelEngine) -> Result<Found, LabelError> {
    let layout = |label: &LabelBox| label.layout(engine);
    let (anchor, focus) = (&press.anchor, &press.focus);
    let (anchor_layout, focus_layout) = (layout(&anchor.label)?, layout(&focus.label)?);
    let at = |end: &End, layout: &TextLayout| {
        unit(layout, end.position.unwrap_or_default(), press.count)
    };
    let (from, to) = (at(anchor, &anchor_layout), at(focus, &focus_layout));
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

/// Choose the separator between selected text from consecutive labels.
/// Use a tab when the next selection starts beside the previous selection's last line. In the
/// previous label's coordinates, the next rectangle's vertical midpoint must lie within that
/// line, and its left edge must reach the line's right edge, allowing overlap of one quarter of
/// the line height. Otherwise, use a newline.
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

/// Return the selection unit at `position`: an empty range for one press, a word for two, or a
/// line for three or more.
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

    /// Reindex changed lower scenes, check the hover again where they changed, and rebuild
    /// highlights when the selection scene is requested or the anchor scene changes.
    /// Clear the selection if the anchor label disappears or its source changes. Log layout
    /// errors and clear the selection when selected text cannot be laid out.
    async fn update(
        &self,
        _app: &mut State,
        own: &mut SelectionState,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        let engine = scenes.text_engine();
        // Ends resolve through the placements that the pointer saw, before reindexing moves them.
        let resolved = own.press.as_mut().map(|press| press.resolve(engine));
        if let Some(Err(error)) = resolved {
            log::warn!("Failed to lay out selected text: {error}");
            own.clear();
        }
        let reindexed = own.index(scenes, &self.scene);
        if let Some(pointer) = own.pointer.filter(|_| !reindexed.is_empty()) {
            let trees = scenes.below(&self.scene).map(|view| view.tree());
            own.hover = own
                .label_at(trees, pointer)
                .map(|(_, label)| label.cursor());
        }
        let anchor_changed =
            (own.press.as_ref()).is_some_and(|press| reindexed.contains(&press.scene));
        if anchor_changed {
            own.find_again();
        }
        if !scenes.requested(&self.scene) && !anchor_changed {
            return Ok(());
        }
        let found = own.press.as_mut().map(|press| {
            press.resolve(engine)?;
            find(press, &own.scenes[press.scene], engine)
        });
        let highlights = match found {
            None => Vec::new(),
            Some(Err(error)) => {
                log::warn!("Failed to lay out selected text: {error}");
                own.clear();
                Vec::new()
            }
            Some(Ok(found)) => {
                own.text = found.text;
                found.highlights
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
        match &own.press {
            Some(press) if own.dragging => Some(press.anchor.label.cursor()),
            _ => own.hover,
        }
    }

    fn copy_text<'a>(&'a self, _app: &'a State, own: &'a SelectionState) -> Option<&'a str> {
        (!own.text.is_empty()).then_some(own.text.as_str())
    }
}
