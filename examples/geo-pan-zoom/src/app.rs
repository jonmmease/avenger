//! The app: a globe and a world map in a wrap, each with its own projection that its gestures
//! change, and the layers' legends beside them.

use std::sync::Arc;

use avenger_app::app::{AvengerApp, SceneGraphBuilder};
use avenger_app::error::AvengerAppError;
use avenger_color::ColorOrGradient;
use avenger_common::types::{FontWeight, TextBaseline};
use avenger_eventstream::manager::EventStreamHandler;
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{EventStreamConfig, UpdateStatus};
use avenger_eventstream::window::{MouseButton, MouseScrollDelta};
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geo::{PanMode, Projection};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_guides::attribution::{Attribution, AttributionConfig};
use avenger_guides::{BackgroundStyle, GuideSide, PartExtent};
use avenger_layout::{
    Direction, Distribute, Edges, Gap, Layout, Leaf, Measure, Measurement, Overflow, Placement,
    Position, Rect, Side, Slab, SlabStack, Solution, SolveOptions, Track, Wrap,
};
use avenger_scenegraph::marks::{mark::SceneMark, rect::SceneRectMark, text::SceneTextMark};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;

use crate::maps::{self, Drawn, Layers, Legends, MapSpec, INK, LABEL_SIDES, MUTED};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// The shortest that a short window makes a map.
const MIN_MAP_HEIGHT: f32 = 160.0;

/// A map's state: its projection now, the size it was drawn at, and its drawing.
#[derive(Clone)]
struct MapState {
    spec: Arc<MapSpec>,
    /// The map's projection, clipped to its rectangle, or `None` until the map is fitted.
    projection: Option<Projection>,
    /// The map's rectangle in the window, from the last layout.
    rect: Rect,
    /// The drawing at `projection` and the rectangle's size, or `None` once either changes.
    drawn: Option<Arc<Drawn>>,
    /// What lies under the cursor, or `None` for the map's hint.
    readout: Option<String>,
}

/// A drag: the map it started on, that map's projection then, and where it started, in the
/// map's frame.
#[derive(Clone)]
struct Drag {
    map: usize,
    start: Projection,
    from: [f64; 2],
}

#[derive(Clone)]
pub struct AppState {
    canvas: [f32; 2],
    engine: LabelEngine,
    layers: Arc<Layers>,
    maps: Vec<MapState>,
    legends: Arc<Legends>,
    credits: Arc<Attribution>,
    drag: Option<Drag>,
}

impl AppState {
    fn new() -> Self {
        let engine = avenger_typst_label::bundled_label_engine()
            .with_number_formatting(Arc::new(D3NumberFormatProvider::new()));
        let layers = Layers::read().expect("the embedded layers read");
        let maps = maps::maps(&engine)
            .expect("the maps prepare")
            .into_iter()
            .map(|spec| MapState {
                spec: Arc::new(spec),
                projection: None,
                rect: Rect::new(0.0, 0.0, 0.0, 0.0),
                drawn: None,
                readout: None,
            })
            .collect();
        let credits = Attribution::new(
            &[
                "Countries: Natural Earth".to_string(),
                "Earthquakes: U.S. Geological Survey".to_string(),
                "Plate boundaries: Peter Bird (2003), Hugo Ahlenius and Nordpil, ODC-BY 1.0"
                    .to_string(),
            ],
            &AttributionConfig {
                font_family: "Lato".to_string(),
                font_size: 10.0,
                color: MUTED,
                background: BackgroundStyle {
                    padding: 6.0,
                    ..Default::default()
                },
                ..Default::default()
            },
            &engine,
        )
        .expect("the credits lay out");
        let legends = Legends::new(&engine).expect("the legends lay out");
        Self {
            canvas: [1240.0, 720.0],
            engine,
            layers: Arc::new(layers),
            maps,
            legends: Arc::new(legends),
            credits: Arc::new(credits),
            drag: None,
        }
    }

    /// The map under `position` in the window, with the position in the map's frame.
    fn map_at(&self, position: [f32; 2]) -> Option<(usize, [f64; 2])> {
        self.maps.iter().enumerate().find_map(|(index, map)| {
            map.rect
                .contains(position)
                .then(|| (index, local(map.rect, position)))
        })
    }

    /// Show `projection` on map `index`.
    fn set_projection(&mut self, index: usize, projection: Projection) {
        let map = &mut self.maps[index];
        map.projection = Some(projection);
        map.drawn = None;
    }
}

/// A window position in the frame of the map at `rect`.
fn local(rect: Rect, position: [f32; 2]) -> [f64; 2] {
    [
        f64::from(position[0] - rect.x),
        f64::from(position[1] - rect.y),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Id {
    Header,
    Map(usize),
    Labels(usize, GuideSide),
    Readout(usize),
    Continents,
    Magnitude,
    Depth,
    Plates,
    Credits,
}

/// The maps in a wrap under a header, each with its title in its padding, its graticule's
/// labels in slabs on its left and below it, and a readout below those. The legends stand in a
/// stack on the right, and the credits run along the bottom.
fn layout(maps: &[MapState]) -> Layout<Id> {
    let leaves = maps.iter().enumerate().map(|(index, map)| {
        let mut leaf = Leaf::new()
            .id(Id::Map(index))
            .aspect(map.spec.graticule.aspect())
            .padding(Edges::new(0.0, 24.0, 0.0, 0.0))
            .overflow_direction(Direction::Vertical, Overflow::Contain)
            .place(Direction::Horizontal, Placement::stretch(Position::Center))
            .place(Direction::Vertical, Placement::stretch(Position::Center));
        for side in LABEL_SIDES {
            leaf = leaf.slab(slab_side(side), Slab::new().id(Id::Labels(index, side)));
        }
        leaf.slab(
            Side::Bottom,
            Slab::new().id(Id::Readout(index)).min_height(22.0),
        )
    });
    Wrap::new(leaves)
        .min_column(320.0)
        .gap(Gap::new(28.0))
        .slab(
            Side::Top,
            Slab::new().id(Id::Header).min_height(62.0).full(),
        )
        .slab(Side::Right, Slab::new().min_width(24.0))
        .slab_stack(
            Side::Right,
            SlabStack::new([
                Slab::new().id(Id::Continents),
                Slab::new().id(Id::Magnitude),
                Slab::new().id(Id::Depth),
                Slab::new().id(Id::Plates),
            ])
            .track(0, Track::auto())
            .track(1, Track::auto())
            .track(2, Track::px(140.0))
            .track(3, Track::auto())
            .gap(Gap::new(18.0))
            .distribute(Distribute::Start),
        )
        .slab(Side::Bottom, Slab::new().id(Id::Credits).full())
        .into()
}

fn slab_side(side: GuideSide) -> Side {
    match side {
        GuideSide::Left => Side::Left,
        GuideSide::Right => Side::Right,
        GuideSide::Top => Side::Top,
        GuideSide::Bottom => Side::Bottom,
    }
}

fn answer(extent: PartExtent) -> Measurement {
    Measurement::over(extent.thickness, extent.holds).with_overhang(extent.overhang)
}

/// Measures the maps at their fitted aspects, no taller than `max_map_height`, the graticules'
/// labels at every view, the legends and the credits.
struct Guides<'a> {
    state: &'a AppState,
    max_map_height: f32,
}

impl Measure<Id> for Guides<'_> {
    fn height(&mut self, id: &Id, width: f32) -> Option<Measurement> {
        let maps = &self.state.maps;
        match id {
            Id::Map(index) => Some(Measurement::over(
                (width / maps[*index].spec.graticule.aspect()).min(self.max_map_height),
                width..=width,
            )),
            Id::Labels(index, side) => Some(answer(maps[*index].spec.graticule.view_extent(*side))),
            Id::Credits => Some(answer(self.state.credits.extent(width))),
            _ => None,
        }
    }

    fn width(&mut self, id: &Id, height: f32) -> Option<Measurement> {
        let legends = &self.state.legends;
        match id {
            Id::Labels(index, side) => Some(answer(
                self.state.maps[*index].spec.graticule.view_extent(*side),
            )),
            Id::Continents => Some(answer(legends.continents.extent(height))),
            Id::Magnitude => Some(answer(legends.magnitude.extent(height))),
            Id::Depth => Some(answer(legends.depth.extent(height))),
            Id::Plates => Some(answer(legends.plates.extent(height))),
            _ => None,
        }
    }

    fn max_content(&mut self, id: &Id, direction: Direction) -> Option<f32> {
        let legends = &self.state.legends;
        match (id, direction) {
            (Id::Continents, Direction::Vertical) => Some(legends.continents.max_content()),
            (Id::Magnitude, Direction::Vertical) => Some(legends.magnitude.max_content()),
            (Id::Plates, Direction::Vertical) => Some(legends.plates.max_content()),
            _ => None,
        }
    }
}

fn text(source: &str, x: f32, y: f32, size: f32, weight: FontWeight, color: [f32; 4]) -> SceneMark {
    SceneTextMark {
        text: source.to_string().into(),
        x: x.into(),
        y: y.into(),
        baseline: TextBaseline::Alphabetic.into(),
        font: "Lato".to_string().into(),
        font_size: size.into(),
        font_weight: weight.into(),
        color: color.into(),
        ..Default::default()
    }
    .into()
}

#[derive(Clone, Debug)]
struct MapsSceneGraphBuilder;

#[async_trait::async_trait]
impl SceneGraphBuilder<AppState> for MapsSceneGraphBuilder {
    async fn build(&self, state: &mut AppState) -> Result<SceneGraph, AvengerAppError> {
        make_scene_graph(state).map_err(|err| AvengerAppError::InternalError(err.to_string()))
    }
}

/// The layout across the window. Where the maps at their widths would make it taller than the
/// window, the maps' rows share the excess, and each map keeps its aspect in its shorter cell.
fn solve(state: &AppState) -> Result<Solution<Id>, Box<dyn std::error::Error>> {
    let options = SolveOptions {
        width: Some(state.canvas[0]),
        height: None,
        padding: Edges::all(24.0),
        ..SolveOptions::default()
    };
    let solve = |max_map_height: f32| {
        layout(&state.maps)
            .measure(
                &options,
                &mut Guides {
                    state,
                    max_map_height,
                },
            )
            .map(|measured| measured.into_solution())
    };
    let solution = solve(f32::INFINITY)?;
    let excess = solution.size().height - state.canvas[1];
    if excess <= 0.0 {
        return Ok(solution);
    }
    let cells: Vec<Rect> = (0..state.maps.len())
        .filter_map(|index| {
            solution
                .region(&Id::Map(index))
                .map(|region| region.content)
        })
        .collect();
    let mut rows: Vec<f32> = cells.iter().map(|cell| cell.y).collect();
    rows.sort_by(f32::total_cmp);
    rows.dedup_by(|a, b| (*a - *b).abs() < 0.5);
    let tallest = cells.iter().map(|cell| cell.height).fold(0.0, f32::max);
    Ok(solve(
        (tallest - excess / rows.len() as f32).max(MIN_MAP_HEIGHT),
    )?)
}

fn make_scene_graph(state: &mut AppState) -> Result<SceneGraph, Box<dyn std::error::Error>> {
    let [width, height] = state.canvas;
    let solution = solve(state)?;
    let region = |id: Id| solution.region(&id).expect("every part has a region");

    let mut marks: Vec<SceneMark> = vec![SceneRectMark {
        width: Some(width.into()),
        height: Some(height.into()),
        fill: ColorOrGradient::Color(maps::rgb(0xf7f8fa)).into(),
        ..Default::default()
    }
    .into()];
    let header = region(Id::Header).content;
    marks.push(text(
        "A globe and a world map, two ways to pan",
        header.x,
        header.y + 26.0,
        24.0,
        FontWeight::BOLD,
        INK,
    ));
    marks.push(text(
        "Earthquakes of magnitude 6 and larger, 2000 through 2024, on the plate boundaries. Drag to pan, scroll to zoom, and double-click to reset a map.",
        header.x,
        header.y + 50.0,
        13.0,
        FontWeight::NORMAL,
        MUTED,
    ));

    for index in 0..state.maps.len() {
        let rect = region(Id::Map(index)).content;
        let size = [rect.width, rect.height];
        let map = &mut state.maps[index];
        // A map without area draws nothing and keeps its view for when it has area again.
        if !(rect.width >= 1.0 && rect.height >= 1.0) {
            continue;
        }
        // A resized map scales its view with its rectangle, so it shows the same part of the
        // world. A map without a projection is fitted. The state changes only once the new
        // projection is made.
        let projection = match &map.projection {
            Some(projection) if [map.rect.width, map.rect.height] == size => projection.clone(),
            Some(projection) if map.rect.width > 0.0 => {
                let factor = f64::from(rect.width / map.rect.width);
                projection
                    .zoom(PanMode::Translate, [0.0, 0.0], factor)?
                    .with_clip_extent(Some([[0.0, 0.0], size.map(f64::from)]))
            }
            _ => map.spec.fitted(size)?,
        };
        if [map.rect.width, map.rect.height] != size {
            map.drawn = None;
        }
        map.rect = rect;
        let drawn = match &map.drawn {
            Some(drawn) => drawn.clone(),
            None => Arc::new(maps::draw(&map.spec, &state.layers, &projection, size)?),
        };
        map.projection = Some(projection);
        map.drawn = Some(drawn.clone());

        // The title stands in the cell's padding, over the map, which can be narrower than its
        // cell in a short window.
        marks.push(text(
            map.spec.title,
            rect.x,
            rect.y - 8.0,
            15.0,
            FontWeight::BOLD,
            INK,
        ));
        marks.push(
            avenger_scenegraph::marks::group::SceneGroup {
                origin: [rect.x, rect.y],
                marks: vec![
                    drawn.content.clone().into(),
                    SceneRectMark {
                        width: Some(rect.width.into()),
                        height: Some(rect.height.into()),
                        fill: ColorOrGradient::Color([0.0; 4]).into(),
                        stroke: ColorOrGradient::Color(maps::rgb(0xb8c4d0)).into(),
                        stroke_width: 1.0.into(),
                        ..Default::default()
                    }
                    .into(),
                ],
                ..Default::default()
            }
            .into(),
        );
        for (side, part) in LABEL_SIDES.iter().zip(&drawn.parts) {
            if let Some(mut part) = part.clone() {
                let slab = region(Id::Labels(index, *side)).content;
                part.origin = map
                    .spec
                    .graticule
                    .origin(*side, [slab.x, slab.y, slab.width, slab.height]);
                marks.push(part.into());
            }
        }
        let readout = region(Id::Readout(index)).content;
        let (line, color) = match &map.readout {
            Some(line) => (line.as_str(), INK),
            None => (map.spec.hint, MUTED),
        };
        marks.push(text(
            line,
            rect.x,
            readout.y + 16.0,
            12.0,
            FontWeight::NORMAL,
            color,
        ));
    }

    let slot = |rect: Rect| [rect.x, rect.y, rect.width, rect.height];
    let legends = &state.legends;
    for (id, legend) in [
        (Id::Continents, &legends.continents),
        (Id::Magnitude, &legends.magnitude),
        (Id::Plates, &legends.plates),
    ] {
        let rect = region(id).content;
        let mut group = legend.marks(rect.height).group;
        group.origin = legend.origin(slot(rect));
        marks.push(group.into());
    }
    let depth = region(Id::Depth).content;
    let mut group = legends.depth.marks(depth.height).group;
    group.origin = legends.depth.origin(slot(depth));
    marks.push(group.into());

    let credits = region(Id::Credits).content;
    if let Some(mut group) = state.credits.part(credits.width) {
        group.origin = state.credits.origin(slot(credits));
        marks.push(group.into());
    }
    Ok(SceneGraph {
        width,
        height,
        origin: [0.0, 0.0],
        marks,
    })
}

/// No change to draw.
const UNCHANGED: UpdateStatus = UpdateStatus {
    rerender: false,
    rebuild_geometry: false,
};

/// A change to draw. The maps read their rectangles from the state, so no gesture needs the
/// hit-testing tree.
const REDRAW: UpdateStatus = UpdateStatus {
    rerender: true,
    rebuild_geometry: false,
};

/// A press on a map starts a drag there.
struct DragStart;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for DragStart {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut AppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let SceneGraphEvent::MouseDown(event) = event else {
            return UNCHANGED;
        };
        if event.button != MouseButton::Left {
            return UNCHANGED;
        }
        state.drag = state.map_at(event.position).and_then(|(map, from)| {
            let start = state.maps[map].projection.clone()?;
            Some(Drag { map, start, from })
        });
        UNCHANGED
    }
}

/// A release ends a drag with a last pan to where the cursor let go, since the throttle can drop
/// the cursor's last move.
struct DragEnd;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for DragEnd {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut AppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        match (state.drag.take(), event.position()) {
            (Some(drag), Some(position)) => pan(state, &drag, position),
            _ => UNCHANGED,
        }
    }
}

/// Pan the map that `drag` started on so that the place under the drag's start lies under
/// `position`, in the window.
fn pan(state: &mut AppState, drag: &Drag, position: [f32; 2]) -> UpdateStatus {
    let map = &state.maps[drag.map];
    let to = local(map.rect, position);
    match drag.start.pan(map.spec.mode, drag.from, to) {
        Ok(projection) => {
            state.set_projection(drag.map, projection);
            REDRAW
        }
        Err(err) => {
            log::warn!("pan failed: {err}");
            UNCHANGED
        }
    }
}

/// A moving cursor pans the map that a drag started on, or else reads out what lies under it.
struct CursorMoved;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for CursorMoved {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut AppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let Some(position) = event.position() else {
            return UNCHANGED;
        };
        if let Some(drag) = state.drag.clone() {
            return pan(state, &drag, position);
        }
        let over = state.map_at(position);
        let mut changed = false;
        for (index, map) in state.maps.iter_mut().enumerate() {
            let readout = match (over, &map.projection, &map.drawn) {
                (Some((over, point)), Some(projection), Some(drawn)) if over == index => {
                    projection.build().ok().and_then(|projector| {
                        maps::readout(&projector, &state.layers, drawn, point.map(|v| v as f32))
                    })
                }
                _ => None,
            };
            if readout != map.readout {
                map.readout = readout;
                changed = true;
            }
        }
        if changed {
            REDRAW
        } else {
            UNCHANGED
        }
    }
}

/// The wheel zooms the map under the cursor, within its zoom limits.
struct WheelZoom;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for WheelZoom {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut AppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let SceneGraphEvent::MouseWheel(event) = event else {
            return UNCHANGED;
        };
        let Some((index, anchor)) = state.map_at(event.position) else {
            return UNCHANGED;
        };
        let map = &state.maps[index];
        let Some(projection) = &map.projection else {
            return UNCHANGED;
        };
        // Wheel steps: a desktop wheel reports a line a notch, a browser about 100 pixels a
        // notch, and a trackpad a few pixels an event. A step zooms 12%, and an event zooms at
        // most two steps.
        let steps = match event.delta {
            MouseScrollDelta::LineDelta(x, y) => f64::from(x + y),
            MouseScrollDelta::PixelDelta(x, y) => (x + y) / 100.0,
        };
        let scale = projection.view.map_or(1.0, |view| view.scale);
        let fitted = map.spec.fitted_scale([map.rect.width, map.rect.height]);
        let wanted = scale * 1.12_f64.powf(steps.clamp(-2.0, 2.0));
        let (zoom_out, zoom_in) = map.spec.zoom_limits;
        let factor = wanted.clamp(fitted * zoom_out, fitted * zoom_in) / scale;
        match projection.zoom(map.spec.mode, anchor, factor) {
            Ok(projection) => {
                state.set_projection(index, projection);
                REDRAW
            }
            Err(err) => {
                log::warn!("zoom failed: {err}");
                UNCHANGED
            }
        }
    }
}

/// A double click restores the map under the cursor to its fitted projection.
struct Reset;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for Reset {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut AppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let Some((index, _)) = event.position().and_then(|position| state.map_at(position)) else {
            return UNCHANGED;
        };
        let map = &mut state.maps[index];
        map.projection = None;
        map.drawn = None;
        REDRAW
    }
}

/// The layout fills the window at its new size, down to the maps' shortest.
struct WindowResized;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for WindowResized {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut AppState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        let SceneGraphEvent::WindowResize(resize) = event else {
            return UNCHANGED;
        };
        // A minimized window has no size to lay the maps out in.
        if resize.size.iter().any(|&length| length < 1.0) {
            return UNCHANGED;
        }
        state.canvas = resize.size;
        REDRAW
    }
}

fn on(types: SceneGraphEventType) -> EventStreamConfig {
    EventStreamConfig {
        types: vec![types],
        ..Default::default()
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen(start))]
pub async fn run() {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            std::panic::set_hook(Box::new(console_error_panic_hook::hook));
            console_log::init_with_level(log::Level::Warn).expect("Couldn't initialize logger");
        } else {
            env_logger::init();
        }
    }

    let state = AppState::new();
    let text_engine = state.engine.clone();
    let throttled = |types| EventStreamConfig {
        // At most one update a frame.
        throttle: Some(8),
        ..on(types)
    };
    let avenger_app = AvengerApp::try_new(
        state,
        Arc::new(MapsSceneGraphBuilder),
        vec![
            (on(SceneGraphEventType::MouseDown), Arc::new(DragStart)),
            (on(SceneGraphEventType::MouseUp), Arc::new(DragEnd)),
            (
                throttled(SceneGraphEventType::CursorMoved),
                Arc::new(CursorMoved),
            ),
            // Every wheel event zooms, so a throttle would drop steps.
            (on(SceneGraphEventType::MouseWheel), Arc::new(WheelZoom)),
            (on(SceneGraphEventType::DoubleClick), Arc::new(Reset)),
            (
                on(SceneGraphEventType::WindowResize),
                Arc::new(WindowResized),
            ),
        ],
        text_engine,
    )
    .await
    .expect("Failed to create AvengerApp");

    let (mut app, event_loop) = {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let tokio_runtime = tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap();
            WinitWgpuAvengerApp::new_and_event_loop(avenger_app, 2.0, tokio_runtime)
        }
        #[cfg(target_arch = "wasm32")]
        {
            WinitWgpuAvengerApp::new_and_event_loop(avenger_app, 2.0)
        }
    };

    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");
}
