//! The app: a globe and a world map in a wrap, each with its own projection that its gestures
//! change, over countries or raster tiles, and the layers' legends beside them.

use std::sync::Arc;

use avenger_app::app::{AppBuild, AppBuilder, AvengerApp, BuiltScene, Scene, Scenes};
use avenger_app::error::AvengerAppError;
use avenger_app_tools::navigation::MapPanZoom;
use avenger_app_tools::text_selection::TextSelection;
use avenger_app_tools::tooltips::{InstanceTarget, TooltipAnchor, TooltipField, Tooltips};
use avenger_color::ColorOrGradient;
use avenger_common::types::{FontWeight, TextBaseline, TextSyntaxMode};
use avenger_eventstream::manager::{EventStreamHandler, SceneTrees};
use avenger_eventstream::scene::{SceneGraphEvent, SceneGraphEventType};
use avenger_eventstream::stream::{DebounceConfig, EventStreamConfig, Rebuild};
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_number_d3::D3NumberFormatProvider;
use avenger_geo::tiles::tile_zoom;
use avenger_geo::{PanMode, Projection, Projector};
use avenger_geometry::data::SceneData;
use avenger_guides::attribution::{Attribution, AttributionConfig};
use avenger_guides::{BackgroundStyle, GuideSide, PartExtent};
use avenger_guides_geo::{TileLayer, TileSource};
use avenger_image::ImageCache;
use avenger_layout::{
    Direction, Distribute, Edges, Gap, Layout, Leaf, Measure, Measurement, Overflow, Placement,
    Position, Rect, Side, Slab, SlabStack, Solution, SolveOptions, Track, Wrap,
};
use avenger_scenegraph::marks::{mark::SceneMark, rect::SceneRectMark, text::SceneTextMark};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_typst_label::LabelEngine;
use avenger_winit_wgpu::WinitWgpuAvengerApp;

use crate::maps::{self, Drawn, Layers, Legends, MapSpec, INK, LABEL_SIDES, MUTED};

/// OpenStreetMap's tiles.
pub const OSM_TILES: &str = "https://tile.openstreetmap.org/{z}/{x}/{y}.png";

/// How far a map of countries zooms in from its fitted scale.
const COUNTRIES_ZOOM_IN: f64 = 400.0;

/// What the maps draw beneath their data.
#[derive(Debug, Clone)]
pub enum Basemap {
    /// Natural Earth's countries, colored by continent.
    Countries,
    /// The raster tiles at a URL template with `{z}`, `{x}`, and `{y}`, such as [`OSM_TILES`].
    Tiles(String),
}

/// The User-Agent of the tile requests, as OpenStreetMap's tile usage policy asks.
#[cfg(not(target_arch = "wasm32"))]
const USER_AGENT: &str = concat!(
    "avenger-geo-pan-zoom/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/jonmmease/avenger)"
);

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

/// The shortest that a short window makes a map.
const MIN_MAP_HEIGHT: f32 = 160.0;

/// A map's state: its projection now, the size it was drawn at, and its drawing.
struct MapState {
    spec: Arc<MapSpec>,
    /// The map's projection, clipped to its rectangle, or `None` until the map is fitted.
    projection: Option<Projection>,
    /// The map's rectangle in the window, from the last layout.
    rect: Rect,
    /// The cached drawing and its projection.
    drawn: Option<(Projection, Arc<Drawn>)>,
    /// Where the readout stands, from the last layout, or `None` while the map has no area.
    readout_at: Option<[f32; 2]>,
    /// The map's tiles, or `None` for a map of countries.
    tiles: Option<TileLayer>,
}

impl MapState {
    /// The scales that the map zooms between at `projection`: from its zoom-out limit, to where
    /// its tiles reach their source's deepest zoom at the view's center, or for a map of
    /// countries, 400 times its fitted scale.
    fn scales(&self, projection: &Projection) -> (f64, f64) {
        let fitted = self.spec.fitted_scale([self.rect.width, self.rect.height]);
        let min = fitted * self.spec.zoom_out;
        let max = match &self.tiles {
            Some(tiles) => {
                let scale = projection.view.map_or(1.0, |view| view.scale);
                let source = tiles.source();
                let deepest = f64::from(*source.zooms().end()) + MAX_ZOOM_ROOM;
                projection
                    .build()
                    .ok()
                    .and_then(|projector| tile_zoom(&projector, f64::from(source.tile_size())))
                    .map_or(f64::INFINITY, |zoom| scale * (deepest - zoom).exp2())
            }
            None => fitted * COUNTRIES_ZOOM_IN,
        };
        (min, max.max(min))
    }
}

/// How far past the deepest tile zoom a map zooms in, short of rounding to the next zoom.
const MAX_ZOOM_ROOM: f64 = 0.4;

pub struct AppState {
    engine: LabelEngine,
    layers: Arc<Layers>,
    maps: Vec<MapState>,
    legends: Arc<Legends>,
    credits: Arc<Attribution>,
    /// Window cursor position used by the readouts, or `None` outside the window.
    cursor: Option<[f32; 2]>,
    /// Whether the maps draw countries, which a continent legend explains, instead of tiles.
    countries: bool,
}

impl AppState {
    /// The app's state, whose maps draw countries, or the tiles at a template through a cache.
    fn new(tiles: Option<(&str, &ImageCache)>) -> Self {
        let engine = avenger_typst_label::bundled_label_engine()
            .with_number_formatting(Arc::new(D3NumberFormatProvider::new()))
            .with_datetime_formatting(Arc::new(D3DateTimeFormatProvider::new()));
        let layers = Layers::read().expect("the embedded layers read");
        let maps = maps::maps(&engine)
            .expect("the maps prepare")
            .into_iter()
            .map(|spec| MapState {
                spec: Arc::new(spec),
                projection: None,
                rect: Rect::new(0.0, 0.0, 0.0, 0.0),
                drawn: None,
                readout_at: None,
                tiles: tiles.map(|(template, cache)| {
                    TileLayer::new(
                        TileSource::new(template, Arc::new(cache.clone()))
                            .expect("the tile URL template names z, x, and y"),
                    )
                }),
            })
            .collect();
        let basemap = match tiles {
            None => "Countries: Natural Earth".to_string(),
            Some((OSM_TILES, _)) => "Basemap: © OpenStreetMap contributors".to_string(),
            Some((template, _)) => {
                let host = template.split("://").nth(1).unwrap_or(template);
                format!("Basemap: {}", host.split('/').next().unwrap_or(host))
            }
        };
        let credits = Attribution::new(
            &[
                basemap,
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
            engine,
            layers: Arc::new(layers),
            maps,
            legends: Arc::new(legends),
            credits: Arc::new(credits),
            cursor: None,
            countries: tiles.is_none(),
        }
    }

    /// Prefetch the tiles that zooming map `index` one tile zoom in or out at `anchor` would
    /// show, within the map's scales, from the map's view now.
    fn prefetch_zooms(&self, index: usize, anchor: [f64; 2]) {
        let map = &self.maps[index];
        let (Some(tiles), Some(projection)) = (&map.tiles, &map.projection) else {
            return;
        };
        let (min, max) = map.scales(projection);
        let scale = projection.view.map_or(1.0, |view| view.scale);
        let views: Vec<Projector> = [2.0, 0.5]
            .into_iter()
            .map(|factor| (scale * factor).min(max).max(min) / scale)
            .filter(|factor| (factor - 1.0).abs() > 1e-6)
            .filter_map(|factor| {
                projection
                    .zoom(map.spec.mode, anchor, factor)
                    .and_then(|zoomed| zoomed.build())
                    .ok()
            })
            .collect();
        let source = tiles.source();
        let views: Vec<(&Projector, u8)> = views
            .iter()
            .map(|projector| (projector, source.zoom(projector)))
            .collect();
        tiles.prefetch(&views);
    }

    /// The map under `position` in the window, with the position in the map's frame.
    fn map_at(&self, position: [f32; 2]) -> Option<(usize, [f64; 2])> {
        self.maps.iter().enumerate().find_map(|(index, map)| {
            map.rect
                .contains(position)
                .then(|| (index, local(map.rect, position)))
        })
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
/// stack on the right, with the continents' first for maps of countries, and the credits run
/// along the bottom.
fn layout(state: &AppState) -> Layout<Id> {
    let legends: Vec<Id> = state
        .countries
        .then_some(Id::Continents)
        .into_iter()
        .chain([Id::Magnitude, Id::Depth, Id::Plates])
        .collect();
    let stack = SlabStack::new(legends.iter().map(|&id| Slab::new().id(id)));
    let stack = legends
        .iter()
        .enumerate()
        .fold(stack, |stack, (track, id)| match id {
            Id::Depth => stack.track(track, Track::px(140.0)),
            _ => stack.track(track, Track::auto()),
        });
    let leaves = state.maps.iter().enumerate().map(|(index, map)| {
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
            Slab::new().id(Id::Header).min_height(78.0).full(),
        )
        .slab(Side::Right, Slab::new().min_width(24.0))
        .slab_stack(
            Side::Right,
            stack.gap(Gap::new(18.0)).distribute(Distribute::Start),
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

/// Create a label with explicit newline breaks.
fn text(source: &str, x: f32, y: f32, size: f32, weight: FontWeight, color: [f32; 4]) -> SceneMark {
    SceneTextMark {
        text: source.to_string().into(),
        text_syntax: TextSyntaxMode::PlainLines,
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

/// The map scene. Navigation defers tree rebuilding until a gesture settles.
/// Tooltips skip the stale tree during movement.
const MAPS: &str = "maps";

/// The coordinate readouts above the maps. A separate scene lets cursor movement
/// update readouts without rebuilding the maps or their hit-testing tree.
const READOUTS: &str = "readouts";

/// Tooltip scene above the maps and readouts.
const TOOLTIPS: &str = "tooltips";

/// Selection scene above all other scenes.
const SELECTION: &str = "selection";

/// Draws maps and coordinate readouts from the current projections and cursor position.
struct Maps;

#[async_trait::async_trait]
impl AppBuild<AppState> for Maps {
    /// Declare map and readout scenes with tile-prefetch and cursor-tracking streams.
    fn scenes(&self) -> Vec<Scene<AppState>> {
        let rest = EventStreamConfig {
            types: vec![
                SceneGraphEventType::CursorMoved,
                SceneGraphEventType::MouseWheel,
            ],
            // The cursor rests once it stays still, and the wheel stops, for 150 ms.
            debounce: Some(DebounceConfig::new(150)),
            ..Default::default()
        };
        let follow = EventStreamConfig {
            types: vec![
                SceneGraphEventType::CursorMoved,
                SceneGraphEventType::CursorLeft,
            ],
            ..Default::default()
        };
        vec![
            Scene::new(MAPS, 0).stream(rest, PrefetchAtRest),
            Scene::new(READOUTS, 10).stream(follow, FollowCursor),
        ]
    }

    /// Build maps before readouts so both use the latest projections and layout.
    async fn build(
        &self,
        state: &mut AppState,
        scenes: &mut Scenes<'_>,
    ) -> Result<(), AvengerAppError> {
        if scenes.requested(MAPS) {
            let scene = make_scene_graph(state, scenes.size())
                .map_err(|err| AvengerAppError::InternalError(err.to_string()))?;
            // Every map draws the same earthquakes and countries, so each map's marks share their
            // rows. A map of tiles draws no countries, and their rows pair with nothing.
            let mut data = SceneData::new();
            for index in 0..state.maps.len() {
                data.insert(maps::quakes(index), state.layers.quake_rows.clone());
                data.insert(maps::countries(index), state.layers.country_rows.clone());
            }
            scenes.set(MAPS, BuiltScene { scene, data })?;
        }
        if scenes.requested(READOUTS) || scenes.changed(MAPS) {
            scenes.set(READOUTS, readouts(state, scenes.size()).into())?;
        }
        Ok(())
    }
}

/// The readouts in a window of `size`.
fn readouts(state: &AppState, size: [f32; 2]) -> SceneGraph {
    let over = state.cursor.and_then(|position| state.map_at(position));
    let marks = (state.maps.iter().enumerate())
        .filter_map(|(index, map)| {
            let [x, y] = map.readout_at?;
            let place =
                (over.filter(|&(over, _)| over == index)).and_then(|(_, point)| place(map, point));
            let (line, color) = match &place {
                Some(line) => (line.as_str(), INK),
                None => (map.spec.hint, MUTED),
            };
            Some(text(line, x, y, 12.0, FontWeight::NORMAL, color))
        })
        .collect();
    let [width, height] = size;
    SceneGraph {
        marks,
        width,
        height,
        origin: [0.0; 2],
    }
}

/// The earthquakes' and the countries' tooltips over the maps of `count`: an earthquake's
/// magnitude, depth, and date beside it, and a country's name and continent beside the cursor.
fn tooltips(count: usize) -> Tooltips {
    let quakes = InstanceTarget::nearest(10.0)
        .with_marks((0..count).map(maps::quakes))
        .with_header(
            TooltipField::new("magnitude")
                .with_value_format(".1f")
                .with_value_label("Magnitude #value"),
        )
        .with_fields([
            TooltipField::new("depth_km")
                .with_title("Depth")
                .with_value_format(".0f")
                .with_value_label("#value km"),
            TooltipField::new("time")
                .with_title("Date")
                .with_value_format("%-d %B %Y"),
        ]);
    let countries = InstanceTarget::exact()
        .with_marks((0..count).map(maps::countries))
        .with_anchor(TooltipAnchor::Pointer)
        .with_header("name")
        .with_fields([TooltipField::new("continent").with_title("Continent")]);
    Tooltips::new(TOOLTIPS, 100).over([quakes, countries])
}

/// Fit maps into a window of `size`. Rows share any excess height, and each map
/// preserves its aspect ratio within the shorter cell.
fn solve(state: &AppState, size: [f32; 2]) -> Result<Solution<Id>, Box<dyn std::error::Error>> {
    let options = SolveOptions {
        width: Some(size[0]),
        height: None,
        padding: Edges::all(24.0),
        ..SolveOptions::default()
    };
    let solve = |max_map_height: f32| {
        layout(state)
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
    let excess = solution.size().height - size[1];
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

/// Compare the projection fields that gestures and resize can change.
fn same_view(a: &Projection, b: &Projection) -> bool {
    (a.rotate, a.center, a.view, a.clip_extent) == (b.rotate, b.center, b.view, b.clip_extent)
}

/// The maps, their legends, and the credits, laid out to fill a window of `size`.
fn make_scene_graph(
    state: &mut AppState,
    size: [f32; 2],
) -> Result<SceneGraph, Box<dyn std::error::Error>> {
    let [width, height] = size;
    let solution = solve(state, size)?;
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
        "Earthquakes of magnitude 6 and larger, 2000 through 2024, on the plate boundaries.\nDrag to pan, scroll to zoom, and double-click to reset a map.",
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
            map.readout_at = None;
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
        map.rect = rect;
        let drawn = match &map.drawn {
            Some((drawn_at, drawn)) if same_view(drawn_at, &projection) => drawn.clone(),
            _ => Arc::new(maps::draw(
                index,
                &map.spec,
                &state.layers,
                map.tiles.as_ref(),
                &projection,
                size,
            )?),
        };
        map.projection = Some(projection.clone());
        map.drawn = Some((projection, drawn.clone()));

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
                    // The frame takes no hits, so the earthquakes beneath it do.
                    SceneRectMark {
                        width: Some(rect.width.into()),
                        height: Some(rect.height.into()),
                        fill: ColorOrGradient::Color([0.0; 4]).into(),
                        stroke: ColorOrGradient::Color(maps::rgb(0xb8c4d0)).into(),
                        stroke_width: 1.0.into(),
                        interactive: Some(false),
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
        // The readouts' scene draws the readout, since it changes with nearly every move.
        let readout = region(Id::Readout(index)).content;
        state.maps[index].readout_at = Some([rect.x, readout.y + 16.0]);
    }

    let slot = |rect: Rect| [rect.x, rect.y, rect.width, rect.height];
    let legends = &state.legends;
    let continents = state
        .countries
        .then_some((Id::Continents, &legends.continents));
    for (id, legend) in continents.into_iter().chain([
        (Id::Magnitude, &legends.magnitude),
        (Id::Plates, &legends.plates),
    ]) {
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

/// The longitude and latitude under `point` in `map`'s frame, or `None` off the map.
fn place(map: &MapState, point: [f64; 2]) -> Option<String> {
    let projector = map.projection.as_ref()?.build().ok()?;
    maps::readout(&projector, point.map(|v| v as f32))
}

/// Tracks cursor changes and requests readout updates.
/// Ignoring repeated positions avoids scheduling a redundant render before a wheel
/// event, which the host could then drop while that render is pending.
struct FollowCursor;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for FollowCursor {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut AppState,
        _trees: &SceneTrees<'_>,
    ) -> Rebuild {
        let position = event.position();
        if state.cursor == position {
            return Rebuild::none();
        }
        state.cursor = position;
        Rebuild::scene(READOUTS)
    }
}

/// Once the cursor rests over a map, after it moves or the wheel zooms there, prefetch the tiles
/// that zooming there would show.
struct PrefetchAtRest;

#[async_trait::async_trait]
impl EventStreamHandler<AppState> for PrefetchAtRest {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut AppState,
        _trees: &SceneTrees<'_>,
    ) -> Rebuild {
        if let Some((index, anchor)) = event.position().and_then(|position| state.map_at(position))
        {
            state.prefetch_zooms(index, anchor);
        }
        Rebuild::none()
    }
}

/// Start the web app, whose maps draw OpenStreetMap's tiles when the page's query names `tiles`,
/// as in `index.html?tiles`, and countries otherwise.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub async fn start() {
    let query = web_sys::window()
        .and_then(|window| window.location().search().ok())
        .unwrap_or_default();
    let tiles = query
        .trim_start_matches('?')
        .split('&')
        .any(|param| param.split('=').next() == Some("tiles"));
    run(if tiles {
        Basemap::Tiles(OSM_TILES.to_string())
    } else {
        Basemap::Countries
    })
    .await;
}

/// Configure map and readout scenes, navigation for each map, and tooltip and text selection
/// scenes above them.
fn builder(state: AppState) -> AppBuilder<AppState> {
    let modes: Vec<PanMode> = state.maps.iter().map(|map| map.spec.mode).collect();
    let count = modes.len();
    let engine = state.engine.clone();
    let mut builder = AvengerApp::builder(state, Maps, engine).size([1240.0, 720.0]);
    // Each map pans and zooms in its rectangle, within its scales, and a double-click fits it to
    // its rectangle again.
    for (index, mode) in modes.into_iter().enumerate() {
        let map = MapPanZoom::new(
            MAPS,
            move |s: &AppState| s.maps[index].rect,
            move |s: &mut AppState| s.maps[index].projection.as_mut(),
            mode,
        )
        .with_home(move |s| {
            let map = &s.maps[index];
            map.spec.fitted([map.rect.width, map.rect.height]).ok()
        })
        .with_scale_limits(move |s, projection| s.maps[index].scales(projection));
        builder = builder.tool(map);
    }
    builder
        .tool(tooltips(count))
        .tool(TextSelection::new(SELECTION, 1000))
}

/// Run the app in a window, with its maps over `basemap`.
pub async fn run(basemap: Basemap) {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            std::panic::set_hook(Box::new(console_error_panic_hook::hook));
            console_log::init_with_level(log::Level::Warn).expect("Couldn't initialize logger");
        } else {
            env_logger::init();
        }
    }

    // Both maps share one cache of tiles.
    let tiles = match basemap {
        Basemap::Countries => None,
        Basemap::Tiles(template) => Some((template, tile_cache())),
    };
    let state = AppState::new(
        tiles
            .as_ref()
            .map(|(template, cache)| (template.as_str(), cache)),
    );
    let avenger_app = builder(state)
        .build()
        .await
        .expect("Failed to create AvengerApp");
    let avenger_app = match tiles {
        Some((_, cache)) => avenger_app.with_images(cache),
        None => avenger_app,
    };

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

/// The cache of the tiles. On native targets, it fetches them with the example's User-Agent and
/// keeps them on disk under `target/`, as OpenStreetMap's tile usage policy asks.
fn tile_cache() -> ImageCache {
    cfg_if::cfg_if! {
        if #[cfg(target_arch = "wasm32")] {
            ImageCache::new()
        } else {
            let fetcher = avenger_image::reqwest_fetcher::ReqwestImageFetcher::new(
                USER_AGENT,
                std::time::Duration::from_secs(30),
            )
            .expect("the tile fetcher builds");
            ImageCache::with_options(avenger_image::ImageCacheOptions {
                fetcher: Arc::new(fetcher),
                disk: Some(avenger_image::DiskCacheOptions::new(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/../../target/geo-pan-zoom-tiles"
                ))),
                ..Default::default()
            })
        }
    }
}

#[cfg(test)]
mod tests;
