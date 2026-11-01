//! Map plots, behind the `geo` feature. When the evaluator prepares a chart, each group of map
//! plots that share a projection builds its projection from its spec, the summary of what it
//! fits, and the inset of its fit, the largest reach of its marks past their places. At each size,
//! the group fits its view to its plots less the inset, and each plot draws its sphere, its tiles,
//! the lines of its graticule, and the marks of its units through the fitted projector.

use std::{collections::BTreeMap, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray, RecordBatch, UInt32Array},
    compute::{cast, take_record_batch},
    datatypes::{DataType, Float64Type},
};
use avenger_common::value::ScalarOrArray;
use avenger_coords::{MapProjection, PlotProjection};
use avenger_geo::{
    paths::row_parts, FitSummary, Projection, ProjectionImpl, Projector, RegionFrame,
    SphericalClip, Surface,
};
use avenger_guides::{GuideSide, PartExtent};
use avenger_guides_geo::{
    geometry_marks, make_sphere_marks, GeometryStyle, Graticule, GraticuleConfig, GraticuleStyle,
    MapFrame, SphereStyle, TileLayer, TileSource,
};
use avenger_image::ImageResolver;
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{
    group::SceneGroup, line::SceneLineMark, mark::SceneMark, trail::SceneTrailMark,
};
use geo_traits::to_geo::ToGeoGeometry;
use geo_types::{Geometry, LineString, MultiLineString, MultiPoint, MultiPolygon, Point, Polygon};
use geoarrow_array::{array::from_arrow_array, GeoArrowArray};

use avenger_chart::{
    proto::{self, encoding, geo, mark, option_value, spherical_clip, Channel},
    resolve, Chart,
};

use crate::{
    coord::{CoordSystem, FittedPlot, PlotUnit, ProjectionGuide, SystemInputs},
    encode::{self, at, pattern_fills, scalar, shape_names, shapes, Drawn, Projected, UnitRows},
    evaluator::{EvaluationWarning, PlotCoord},
    fit::{self, Position, Reach},
    guides::{css, side, text_style},
    scale::eval_error,
    EvalError, Theme,
};

type Result<T> = std::result::Result<T, EvalError>;

fn geo_error(error: impl std::fmt::Display) -> EvalError {
    eval_error("a map plot", error)
}

/// The steps between the points of a parallel or of the edge of a box at one latitude, in
/// degrees, which the projection resamples.
const PARALLEL_STEP: f64 = 1.0;

/// The map plots of a chart, prepared without a size, with each map plot at its size at the last
/// fit.
pub(crate) struct MapPlots {
    groups: Vec<MapGroup>,
    /// The map plots, by their index in the plots of the tree.
    plots: BTreeMap<usize, MapPlot>,
    /// Each map plot at its size at the last fit, by plot.
    fitted: BTreeMap<usize, MapFit>,
}

/// The map plots that share a projection.
struct MapGroup {
    plots: Vec<usize>,
    /// The projection of the spec, without a view.
    projection: Projection,
    summary: FitSummary,
    /// The largest reach of the marks of the group past their places, toward the left, the
    /// right, the top, and the bottom, by which the fit insets the plot.
    inset: Reach,
    /// Whether the group fits a box or its data, whose maps run past the plot, so its plots
    /// clip to their rectangles. A group that fits the whole sphere fits inside its plots.
    clipped: bool,
}

/// A map plot, prepared without a size.
struct MapPlot {
    path: String,
    guide: ProjectionGuide,
    graticule: Option<Graticule>,
    /// Whether the plot labels its graticule on its rectangle, so its projector clips to it.
    rectangle: bool,
    sphere: Option<SphereStyle>,
    tiles: Option<proto::TileLayer>,
    /// The tile layer, from the tiles of the spec and the image resolver of the host.
    layer: Option<TileLayer>,
}

/// A map plot at a size: its fitted projection and projector, in the plot's frame.
struct MapFit {
    projection: Projection,
    projector: Projector,
    size: [f32; 2],
    clip: bool,
}

/// A map plot fitted at its size at the last fit.
struct FittedMap<'a> {
    map: &'a MapPlot,
    fit: &'a MapFit,
}

impl MapPlots {
    /// Loads the tiles of the map plots through `resolver`.
    pub(crate) fn set_image_resolver(&mut self, resolver: Arc<dyn ImageResolver>) -> Result<()> {
        for plot in self.plots.values_mut() {
            let Some(tiles) = &plot.tiles else {
                continue;
            };
            let zoom = |zoom: Option<u32>, default: u8| zoom.map_or(default, |zoom| zoom as u8);
            let mut source = TileSource::new(tiles.url_template.clone(), resolver.clone())
                .and_then(|source| {
                    source.with_zooms(zoom(tiles.min_zoom, 0)..=zoom(tiles.max_zoom, 19))
                })
                .map_err(geo_error)?;
            if let Some(size) = tiles.tile_size {
                source = source.with_tile_size(size);
            }
            plot.layer = Some(TileLayer::new(source));
        }
        Ok(())
    }
}

impl CoordSystem for MapPlots {
    /// The map plots of the tree and the projection of each group of them, from the units of the
    /// map plots.
    fn prepare(inputs: &SystemInputs) -> Result<Self> {
        let (tree, theme, engine) = (inputs.tree, inputs.theme, inputs.engine);
        let projections = resolve::projections(tree)?;
        let mut groups = Vec::with_capacity(projections.len());
        let mut plots = BTreeMap::new();
        for group in &projections {
            let spec = group.spec;
            let projection = projection(spec)?;
            let group_units: Vec<&PlotUnit> = inputs
                .units
                .iter()
                .filter(|unit| group.plots.contains(&unit.plot))
                .collect();
            let clipped = !matches!(spec.fit, Some(geo::Fit::FitSphere(_)));
            let summary = summary(&projection, spec, &group_units)?;
            let mut inset = [0.0_f32; 4];
            for unit in &group_units {
                let reach = match unit.rows.mark {
                    mark::Mark::Geoshape(shape) => point_reach(&unit.rows, shape)?,
                    _ => unit
                        .reach
                        .map(|reach| reach.iter().fold([0.0; 4], max_reach)),
                };
                if let Some(reach) = reach {
                    inset = max_reach(inset, &reach);
                }
            }
            for plot in &group.plots {
                let path = tree.plots[*plot].path.clone();
                let Some(proto::coord::Coord::Geo(plot_spec)) = tree.plots[*plot]
                    .coord
                    .and_then(|coord| coord.coord.as_ref())
                else {
                    continue;
                };
                let aspect = match plot_spec.aspect {
                    Some(geo::Aspect::AspectRatio(ratio)) => Some(ratio),
                    Some(geo::Aspect::FreeAspect(_)) => None,
                    None => summary.aspect().map(|aspect| aspect as f32),
                };
                let (graticule, rectangle) = match &plot_spec.graticule {
                    Some(spec) => {
                        let config = graticule_config(spec, clipped, theme)?;
                        let rectangle = config.frame == MapFrame::Rectangle;
                        let graticule = Graticule::new(&projection, &summary, &config, engine)
                            .map_err(geo_error)?;
                        (Some(graticule), rectangle)
                    }
                    None => (None, false),
                };
                let labels = graticule
                    .as_ref()
                    .map(|graticule| {
                        label_sides(plot_spec.graticule.as_ref())
                            .into_iter()
                            .map(|side| (side, graticule.view_extent(side)))
                            .filter(|(_, extent): &(GuideSide, PartExtent)| extent.thickness > 0.0)
                            .collect()
                    })
                    .unwrap_or_default();
                let sphere = plot_spec
                    .sphere
                    .as_ref()
                    .map(|sphere| sphere_style(sphere, theme))
                    .transpose()?;
                plots.insert(
                    *plot,
                    MapPlot {
                        path,
                        guide: ProjectionGuide {
                            aspect,
                            slabs: labels,
                        },
                        graticule,
                        rectangle,
                        sphere,
                        tiles: plot_spec.tiles.clone(),
                        layer: None,
                    },
                );
            }
            groups.push(MapGroup {
                plots: group.plots.clone(),
                projection,
                summary,
                inset,
                clipped,
            });
        }
        Ok(Self {
            groups,
            plots,
            fitted: BTreeMap::new(),
        })
    }

    fn guide(&self, plot: usize) -> ProjectionGuide {
        (self.plots.get(&plot))
            .map(|map| map.guide.clone())
            .unwrap_or_default()
    }

    /// Fits each map plot at the size of its rectangle in `sizes`. The plots of a group take one
    /// size, so the group fits one view at the size of its first plot. A plot too small for the
    /// inset fits its whole rectangle, with a warning.
    fn fit(
        &mut self,
        _positions: &BTreeMap<usize, Position>,
        sizes: &[[f32; 2]],
        warnings: &mut Vec<EvaluationWarning>,
    ) -> Result<()> {
        let mut plots = BTreeMap::new();
        for group in &self.groups {
            let Some(first) = group.plots.first() else {
                continue;
            };
            let size = sizes[*first];
            let (fitted, fits) = group.fit(size)?;
            for plot in &group.plots {
                let map = &self.plots[plot];
                if !fits {
                    warnings.push(EvaluationWarning {
                        path: map.path.clone(),
                        message: "the plot is too small for the marks of its map, so they \
                                  reach past its edges"
                            .to_string(),
                    });
                }
                let clip = group.clipped || map.rectangle;
                let mut projection = fitted.clone();
                if clip {
                    projection.clip_extent =
                        Some([[0.0, 0.0], [f64::from(size[0]), f64::from(size[1])]]);
                }
                let projector = projection.build().map_err(geo_error)?;
                plots.insert(
                    *plot,
                    MapFit {
                        projection,
                        projector,
                        size,
                        clip,
                    },
                );
            }
        }
        self.fitted = plots;
        Ok(())
    }

    fn fitted(&self, plot: usize) -> Option<Box<dyn FittedPlot + '_>> {
        Some(Box::new(FittedMap {
            map: self.plots.get(&plot)?,
            fit: self.fitted.get(&plot)?,
        }))
    }
}

impl MapGroup {
    /// The projection fitted to plots of `size` less the inset, and whether the inset leaves the
    /// plots an area. A plot without one fits its whole rectangle.
    fn fit(&self, size: [f32; 2]) -> Result<(Projection, bool)> {
        let [width, height] = size.map(|value| f64::from(value).max(1e-6));
        let [left, right, top, bottom] = self.inset.map(f64::from);
        let inset = [[left, top], [width - right, height - bottom]];
        let fits = inset[1][0] > inset[0][0] && inset[1][1] > inset[0][1];
        let range = if fits {
            inset
        } else {
            [[0.0, 0.0], [width, height]]
        };
        let mut projection = self.projection.clone();
        projection.fit(range, &self.summary).map_err(geo_error)?;
        Ok((projection, fits))
    }
}

impl FittedPlot for FittedMap<'_> {
    /// A map plot has no positional scales: its projection places longitudes and latitudes.
    fn scale(&self, _group: usize) -> Option<&ConfiguredScale> {
        None
    }

    /// Whether the plot clips its marks to its rectangle, because it fits a box or its data, or
    /// its graticule labels the rectangle.
    fn clips(&self) -> bool {
        self.fit.clip
    }

    /// The coordinate system of the plot: its fitted projection.
    fn coord(&self) -> PlotCoord {
        PlotCoord::Geo {
            projection: self.fit.projection.clone(),
        }
    }

    /// The fitted projection's projector, whose range positions are longitudes and latitudes.
    fn projection(&self, _size: [f32; 2]) -> Option<Arc<dyn PlotProjection>> {
        let projector = self.fit.projection.build().ok()?;
        Some(Arc::new(MapProjection::new(projector)))
    }

    /// What the plot draws behind its data: its sphere, its tiles, and the lines of its
    /// graticule.
    fn backdrop(&self) -> Result<Vec<SceneMark>> {
        let map = self.map;
        let mut marks = Vec::new();
        if let Some(style) = &map.sphere {
            marks.push(
                SceneGroup {
                    interactive: Some(false),
                    ..make_sphere_marks(&self.fit.projector, style)
                }
                .into(),
            );
        }
        if map.tiles.is_some() {
            let layer = map.layer.as_ref().ok_or_else(|| {
                EvalError::Evaluation(format!(
                    "the map plot `{}` draws tiles, which load through an image resolver, so the \
                     evaluator needs one from `ChartEvaluator::set_image_resolver`",
                    map.path
                ))
            })?;
            let zoom = layer.source().zoom(&self.fit.projector);
            marks.push(
                layer
                    .group(&self.fit.projector, zoom)
                    .map_err(geo_error)?
                    .into(),
            );
        }
        if let Some(graticule) = &map.graticule {
            marks.push(
                graticule
                    .view(&self.fit.projector, self.fit.size)
                    .lines()
                    .into(),
            );
        }
        Ok(marks)
    }

    /// The labels of the plot's graticule on `side`, in the slab with rectangle `slot`.
    fn slab(&self, side: GuideSide, slot: [f32; 4]) -> Option<SceneGroup> {
        let graticule = self.map.graticule.as_ref()?;
        let part = graticule
            .view(&self.fit.projector, self.fit.size)
            .part(side)?;
        Some(SceneGroup {
            origin: graticule.origin(side, slot),
            ..part
        })
    }

    /// The drawn rows of a unit that draw an instance and their positions: for a mark at a place,
    /// the rows whose place the clip keeps, at their projected places, and for another mark,
    /// every row, which it projects as it draws.
    fn locate(
        &self,
        chart: &Chart,
        unit: &proto::Unit,
        table: &RecordBatch,
    ) -> Result<(RecordBatch, Option<Projected>)> {
        let mark = unit
            .mark
            .as_ref()
            .and_then(|mark| mark.mark.as_ref())
            .ok_or_else(|| EvalError::Evaluation("a unit has no mark".to_string()))?;
        if matches!(
            mark,
            mark::Mark::Line(_)
                | mark::Mark::Trail(_)
                | mark::Mark::Rule(_)
                | mark::Mark::Geoshape(_)
        ) {
            return Ok((table.clone(), None));
        }
        let Some((lon, lat)) = places(chart, unit, table)? else {
            return Ok((table.clone(), None));
        };
        let mut rows = Vec::with_capacity(table.num_rows());
        let mut x = Vec::with_capacity(table.num_rows());
        let mut y = Vec::with_capacity(table.num_rows());
        for row in 0..table.num_rows() {
            if let Some([px, py]) = project_point(&self.fit.projector, lon[row], lat[row]) {
                rows.push(row as u32);
                x.push(px);
                y.push(py);
            }
        }
        let table = if rows.len() == table.num_rows() {
            table.clone()
        } else {
            take_record_batch(table, &UInt32Array::from(rows))?
        };
        Ok((table, Some(Projected { x, y })))
    }

    /// The scene marks of a unit of the plot.
    fn marks(&self, rows: &UnitRows) -> Result<Vec<Drawn>> {
        match rows.mark {
            mark::Mark::Line(line) => self.lines(rows, line.route),
            mark::Mark::Trail(trail) => self.trails(rows, trail.route),
            mark::Mark::Rule(rule) => self.rules(rows, rule.route),
            mark::Mark::Geoshape(shape) => self.geoshapes(rows, shape),
            _ => encode::marks(rows),
        }
    }
}

impl FittedMap<'_> {
    /// The polyline of each series of a line or a trail through its rows along `route`.
    fn series_polylines(&self, rows: &UnitRows, route: i32) -> Result<Vec<(Vec<u32>, Polyline)>> {
        let Some((lon, lat)) = places(rows.chart, rows.unit, &rows.table)? else {
            return Ok(Vec::new());
        };
        let canvas = proto::Route::try_from(route) == Ok(proto::Route::Canvas);
        Ok(rows
            .series()?
            .into_iter()
            .map(|series| {
                let mut polyline = Polyline::default();
                if let [row] = series[..] {
                    let row = row as usize;
                    if let Some(point) = project_point(&self.fit.projector, lon[row], lat[row]) {
                        polyline.push_piece(&[point], [row as u32; 2]);
                    }
                }
                for pair in series.windows(2) {
                    let [a, b] = [pair[0] as usize, pair[1] as usize];
                    let pieces = segment(
                        &self.fit.projector,
                        [lon[a], lat[a]],
                        [lon[b], lat[b]],
                        canvas,
                    );
                    for piece in pieces {
                        polyline.push_piece(&piece, [pair[0], pair[1]]);
                    }
                }
                (series, polyline)
            })
            .collect())
    }

    fn lines(&self, rows: &UnitRows, route: i32) -> Result<Vec<Drawn>> {
        let (_, stroke) = rows.paint(false)?;
        let width = rows
            .numbers(Channel::StrokeWidth)?
            .unwrap_or_else(|| scalar(rows.theme.stroke_width));
        let dash = rows.dashes(Channel::StrokeDash)?;
        self.series_polylines(rows, route)?
            .into_iter()
            .enumerate()
            .map(|(index, (series, polyline))| {
                let first = series[0] as usize;
                let mark = SceneLineMark {
                    name: format!("{}/{index}", rows.name),
                    len: polyline.x.len() as u32,
                    x: ScalarOrArray::new_array(polyline.x),
                    y: ScalarOrArray::new_array(polyline.y),
                    defined: ScalarOrArray::new_array(polyline.defined),
                    stroke: at(&stroke, first),
                    stroke_width: at(&width, first),
                    stroke_dash: dash
                        .as_ref()
                        .map(|dash| at(dash, first))
                        .filter(|dash| !dash.is_empty()),
                    ..Default::default()
                };
                Ok(Drawn {
                    mark: mark.into(),
                    rows: rows.rows(&polyline.rows)?,
                })
            })
            .collect()
    }

    fn trails(&self, rows: &UnitRows, route: i32) -> Result<Vec<Drawn>> {
        let (_, stroke) = rows.paint(false)?;
        let size = rows
            .numbers(Channel::Size)?
            .unwrap_or_else(|| scalar(rows.theme.trail_size));
        self.series_polylines(rows, route)?
            .into_iter()
            .enumerate()
            .map(|(index, (series, polyline))| {
                let first = series[0] as usize;
                // The width of each vertex runs from the size of its segment's first row to the
                // size of its last.
                let sizes = polyline
                    .ends
                    .iter()
                    .zip(&polyline.along)
                    .map(|([a, b], along)| {
                        let (a, b) = (at(&size, *a as usize), at(&size, *b as usize));
                        a + (b - a) * along
                    })
                    .collect();
                let mark = SceneTrailMark {
                    name: format!("{}/{index}", rows.name),
                    len: polyline.x.len() as u32,
                    x: ScalarOrArray::new_array(polyline.x),
                    y: ScalarOrArray::new_array(polyline.y),
                    size: ScalarOrArray::new_array(sizes),
                    defined: ScalarOrArray::new_array(polyline.defined),
                    stroke: at(&stroke, first),
                    ..Default::default()
                };
                Ok(Drawn {
                    mark: mark.into(),
                    rows: rows.rows(&polyline.rows)?,
                })
            })
            .collect()
    }

    /// One line for each rule, along its route, or along its parallel or its meridian when it
    /// misses a second position.
    fn rules(&self, rows: &UnitRows, route: i32) -> Result<Vec<Drawn>> {
        let Some((lon, lat)) = places(rows.chart, rows.unit, &rows.table)? else {
            return Ok(Vec::new());
        };
        let lon2 = degrees(rows.chart, rows.unit, Channel::Longitude2, &rows.table)?;
        let lat2 = degrees(rows.chart, rows.unit, Channel::Latitude2, &rows.table)?;
        let canvas = proto::Route::try_from(route) == Ok(proto::Route::Canvas);
        let (_, stroke) = rows.paint(false)?;
        let width = rows
            .numbers(Channel::StrokeWidth)?
            .unwrap_or_else(|| scalar(1.0));
        let dash = rows.dashes(Channel::StrokeDash)?;
        let mut drawn = Vec::new();
        for row in 0..rows.len() {
            let start = [lon[row], lat[row]];
            let line = rule_line(
                start,
                lon2.as_ref().map(|lon2| lon2[row]),
                lat2.as_ref().map(|lat2| lat2[row]),
            );
            let pieces = match line {
                RuleLine::Point => continue,
                RuleLine::Segment(end) => segment(&self.fit.projector, start, end, canvas),
                RuleLine::Along(line) => project_lines(&self.fit.projector, &line),
            };
            let mut polyline = Polyline::default();
            for piece in pieces {
                polyline.push_piece(&piece, [row as u32; 2]);
            }
            if polyline.x.is_empty() {
                continue;
            }
            let mark = SceneLineMark {
                name: format!("{}/{row}", rows.name),
                len: polyline.x.len() as u32,
                x: ScalarOrArray::new_array(polyline.x),
                y: ScalarOrArray::new_array(polyline.y),
                defined: ScalarOrArray::new_array(polyline.defined),
                stroke: at(&stroke, row),
                stroke_width: at(&width, row),
                stroke_dash: dash
                    .as_ref()
                    .map(|dash| at(dash, row))
                    .filter(|dash| !dash.is_empty()),
                ..Default::default()
            };
            drawn.push(Drawn {
                mark: mark.into(),
                rows: rows.rows(&polyline.rows)?,
            });
        }
        Ok(drawn)
    }

    /// The polygons, the lines, and the points of the geometry of each row, as a path mark of
    /// polygons, a path mark of lines, and a symbol mark of points.
    fn geoshapes(&self, rows: &UnitRows, shape: &proto::GeoshapeMark) -> Result<Vec<Drawn>> {
        let geometry = geometry(rows)?;
        let projected = (self.fit.projector)
            .project_geometry(geometry.as_ref())
            .map_err(geo_error)?;
        let (fill, stroke) = rows.paint(true)?;
        let stroke = match rows.has(Channel::Stroke) {
            true => stroke,
            false => scalar(avenger_color::ColorOrGradient::Color(
                rows.theme.map.geoshape_stroke,
            )),
        };
        let (shape_list, shape_index) = shapes(&shape_names(rows)?)?;
        let style = GeometryStyle {
            fill,
            stroke,
            stroke_width: shape
                .stroke_width
                .unwrap_or(rows.theme.map.geoshape_stroke_width),
            point_size: rows
                .numbers(Channel::Size)?
                .unwrap_or_else(|| scalar(rows.theme.symbol_size)),
            point_shape: shape_index.map(|index| shape_list[*index].clone()),
        };
        let group = geometry_marks(projected.as_ref(), &style).map_err(geo_error)?;
        let parts = row_parts(projected.as_ref()).map_err(geo_error)?;
        let has_polygons = parts
            .polygons
            .iter()
            .any(|path| path.iter().next().is_some());
        let has_lines = parts.lines.iter().any(|path| path.iter().next().is_some());
        let mut names = Vec::new();
        if has_polygons {
            names.push("polygons");
        }
        if has_lines {
            names.push("lines");
        }
        if !parts.points.is_empty() {
            names.push("points");
        }
        let patterns = rows.patterns(Channel::Pattern)?;
        let mut drawn = Vec::new();
        for (mark, kind) in group.marks.into_iter().zip(names) {
            let name = format!("{}/{kind}", rows.name);
            let (mark, mark_rows) = match mark {
                SceneMark::Path(mut path) => {
                    path.name = name;
                    if kind == "polygons" {
                        path.fill_pattern = pattern_fills(patterns.clone());
                    }
                    (SceneMark::from(path), rows.table.clone())
                }
                SceneMark::Symbol(mut symbol) => {
                    symbol.name = name;
                    let mark_rows = if symbol.len as usize == rows.len() {
                        rows.table.clone()
                    } else {
                        let points: Vec<u32> =
                            parts.point_rows.iter().map(|row| *row as u32).collect();
                        rows.rows(&points)?
                    };
                    (SceneMark::from(symbol), mark_rows)
                }
                other => (other, rows.table.clone()),
            };
            drawn.push(Drawn {
                mark,
                rows: mark_rows,
            });
        }
        Ok(drawn)
    }
}

/// The vertices of a polyline through the projected pieces of segments, with an undefined vertex
/// between pieces that do not meet, so the line breaks there.
#[derive(Default)]
struct Polyline {
    x: Vec<f32>,
    y: Vec<f32>,
    defined: Vec<bool>,
    /// The row of each vertex: the first row of its segment.
    rows: Vec<u32>,
    /// The first and the last row of the segment of each vertex.
    ends: Vec<[u32; 2]>,
    /// How far along its piece each vertex is, from 0 at its start to 1 at its end.
    along: Vec<f32>,
}

impl Polyline {
    fn push_vertex(&mut self, [x, y]: [f32; 2], defined: bool, ends: [u32; 2], along: f32) {
        self.x.push(x);
        self.y.push(y);
        self.defined.push(defined);
        self.rows.push(ends[0]);
        self.ends.push(ends);
        self.along.push(along);
    }

    /// Adds `piece`, the projection of a segment between the rows `ends`, continuing the line
    /// where the piece starts at its last vertex, and breaking it elsewhere.
    fn push_piece(&mut self, piece: &[[f32; 2]], ends: [u32; 2]) {
        let Some(first) = piece.first() else {
            return;
        };
        let meets = match (self.x.last(), self.y.last(), self.defined.last()) {
            (Some(x), Some(y), Some(true)) => {
                (x - first[0]).abs() < 1e-3 && (y - first[1]).abs() < 1e-3
            }
            _ => false,
        };
        if !self.x.is_empty() && !meets {
            self.push_vertex([f32::NAN, f32::NAN], false, ends, 0.0);
        }
        let mut lengths = vec![0.0_f32];
        for pair in piece.windows(2) {
            let length = lengths.last().copied().unwrap_or(0.0)
                + (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]);
            lengths.push(length);
        }
        let total = lengths.last().copied().unwrap_or(0.0);
        for (index, (point, length)) in piece.iter().zip(&lengths).enumerate() {
            if index == 0 && meets {
                continue;
            }
            let along = if total > 0.0 { length / total } else { 0.0 };
            self.push_vertex(*point, true, ends, along);
        }
    }
}

/// The projection of a geo spec, without a view.
fn projection(spec: &proto::Geo) -> Result<Projection> {
    let spherical = match spec.surface.as_ref() {
        Some(geo::Surface::Planar(planar)) => return Ok(Projection::planar(planar.reflect_y)),
        Some(geo::Surface::Spherical(spherical)) => spherical,
        None => {
            return Err(EvalError::Evaluation(
                "a map plot has no surface".to_string(),
            ))
        }
    };
    let implementation = projection_impl(&spherical.projection, &spherical.parameters)?;
    let mut projection = Projection::planar(false);
    projection.surface = Surface::Spherical(Arc::from(implementation));
    if let Some(rotate) = &spherical.rotate {
        projection.rotate = [rotate.lambda, rotate.phi, rotate.gamma];
    }
    if let Some(center) = &spherical.center {
        projection.center = [center.longitude, center.latitude];
    }
    if let Some(precision) = spherical.precision {
        projection.precision = precision;
    }
    projection.clip = match spherical.clip.as_ref().and_then(|clip| clip.clip.as_ref()) {
        None => None,
        Some(spherical_clip::Clip::Antimeridian(_)) => Some(SphericalClip::Antimeridian),
        Some(spherical_clip::Clip::CircleAngle(angle)) => {
            Some(SphericalClip::Circle { angle: *angle })
        }
        Some(spherical_clip::Clip::Region(region)) => Some(SphericalClip::Region {
            polygon: wkb_polygon(&region.polygon)?,
            frame: match proto::RegionFrame::try_from(region.frame) {
                Ok(proto::RegionFrame::View) => RegionFrame::View,
                _ => RegionFrame::Earth,
            },
        }),
    };
    Ok(projection)
}

/// The avenger-geo projection with the typetag name `name` and the fields of its serde form
/// from `parameters`.
fn projection_impl(
    name: &str,
    parameters: &[proto::NamedOption],
) -> Result<Box<dyn ProjectionImpl>> {
    let mut fields = serde_json::Map::new();
    for parameter in parameters {
        let value = match parameter
            .value
            .as_ref()
            .and_then(|value| value.value.as_ref())
        {
            Some(option_value::Value::Boolean(value)) => serde_json::json!(value),
            Some(option_value::Value::Float(value)) => serde_json::json!(value),
            Some(option_value::Value::Integer(value)) => serde_json::json!(value),
            Some(option_value::Value::String(value)) => serde_json::json!(value),
            Some(option_value::Value::Floats(list)) => serde_json::json!(list.values),
            Some(option_value::Value::Strings(list)) => serde_json::json!(list.values),
            None => serde_json::Value::Null,
        };
        fields.insert(parameter.name.clone(), value);
    }
    let read = |fields: serde_json::Value| {
        serde_json::from_value::<Box<dyn ProjectionImpl>>(
            serde_json::json!({ "type": name, "fields": fields }),
        )
    };
    // A projection without parameters is a unit struct, whose fields are null, or a struct whose
    // fields all have defaults.
    let read = if fields.is_empty() {
        read(serde_json::Value::Null).or_else(|_| read(serde_json::json!({})))
    } else {
        read(serde_json::Value::Object(fields))
    };
    read.map_err(|error| eval_error(&format!("the projection `{name}`"), error))
}

/// The multipolygon of WKB bytes.
fn wkb_polygon(bytes: &[u8]) -> Result<MultiPolygon<f64>> {
    let wkb = wkb::reader::read_wkb(bytes).map_err(|error| eval_error("a region clip", error))?;
    match wkb.to_geometry() {
        Geometry::MultiPolygon(polygons) => Ok(polygons),
        Geometry::Polygon(polygon) => Ok(MultiPolygon(vec![polygon])),
        _ => Err(EvalError::Evaluation(
            "a region clip is a polygon or a multipolygon".to_string(),
        )),
    }
}

/// The values of the encoding of `channel` of `unit` for each row of `table`, as degrees, or
/// `None` when the unit does not encode it.
fn degrees(
    chart: &Chart,
    unit: &proto::Unit,
    channel: Channel,
    table: &RecordBatch,
) -> Result<Option<Vec<f64>>> {
    let Some(value) = unit
        .encodings
        .iter()
        .find(|encoding| encoding.channel == channel as i32)
        .and_then(|encoding| encoding.value.as_ref())
    else {
        return Ok(None);
    };
    let values: ArrayRef = match value {
        encoding::Value::Column(name) => table.column_by_name(name).cloned().ok_or_else(|| {
            EvalError::Evaluation(format!("the unit's table has no column `{name}`"))
        })?,
        encoding::Value::Constant(index) | encoding::Value::ScaledConstant(index) => {
            chart.value(*index).clone()
        }
    };
    let values = cast(&values, &DataType::Float64)?;
    let values = values.as_primitive::<Float64Type>();
    let len = table.num_rows();
    Ok(Some(if values.len() == 1 && len != 1 {
        vec![values.value(0); len]
    } else {
        values.values().to_vec()
    }))
}

/// The longitude and the latitude of each row of `table`, or `None` for a unit without them.
fn places(
    chart: &Chart,
    unit: &proto::Unit,
    table: &RecordBatch,
) -> Result<Option<(Vec<f64>, Vec<f64>)>> {
    let lon = degrees(chart, unit, Channel::Longitude, table)?;
    let lat = degrees(chart, unit, Channel::Latitude, table)?;
    Ok(lon.zip(lat))
}

/// The GeoArrow geometry of the `geometry` column of a unit.
fn geometry(rows: &UnitRows) -> Result<Arc<dyn GeoArrowArray>> {
    let name = match rows
        .unit
        .encodings
        .iter()
        .find(|encoding| encoding.channel == Channel::Geometry as i32)
        .and_then(|encoding| encoding.value.as_ref())
    {
        Some(encoding::Value::Column(name)) => name,
        _ => {
            return Err(EvalError::Evaluation(
                "a geoshape mark needs a `geometry` column".to_string(),
            ))
        }
    };
    let schema = rows.table.schema();
    let field = schema.field_with_name(name)?;
    let column = rows
        .table
        .column_by_name(name)
        .ok_or_else(|| EvalError::Evaluation(format!("the unit's table has no column `{name}`")))?;
    from_arrow_array(column.as_ref(), field).map_err(|error| eval_error("a geometry column", error))
}

/// The place of the point at `lon` and `lat` in the frame of the projector, or `None` when the
/// clip leaves it out, as the far side of a globe.
fn project_point(projector: &Projector, lon: f64, lat: f64) -> Option<[f32; 2]> {
    match projector.project(&Point::new(lon, lat)) {
        Geometry::Point(point) if point.x().is_finite() && point.y().is_finite() => {
            Some([point.x() as f32, point.y() as f32])
        }
        _ => None,
    }
}

/// The projected pieces of `line`: the resampled line, cut by the antimeridian and the clip.
fn project_lines(projector: &Projector, line: &LineString<f64>) -> Vec<Vec<[f32; 2]>> {
    let pieces = match projector.project(line) {
        Geometry::MultiLineString(lines) => lines.0,
        Geometry::LineString(line) => vec![line],
        _ => Vec::new(),
    };
    pieces
        .into_iter()
        .map(|piece| {
            piece
                .0
                .iter()
                .map(|coord| [coord.x as f32, coord.y as f32])
                .collect::<Vec<_>>()
        })
        .filter(|piece| piece.len() >= 2)
        .collect()
}

/// The projected pieces of the segment from `a` to `b`, in degrees: the great circle between
/// them for the coordinate route of a spherical map, or a straight line between their projected
/// places for the canvas route.
fn segment(projector: &Projector, a: [f64; 2], b: [f64; 2], canvas: bool) -> Vec<Vec<[f32; 2]>> {
    if canvas {
        return match (
            project_point(projector, a[0], a[1]),
            project_point(projector, b[0], b[1]),
        ) {
            (Some(a), Some(b)) => vec![vec![a, b]],
            _ => Vec::new(),
        };
    }
    project_lines(
        projector,
        &LineString::from(vec![(a[0], a[1]), (b[0], b[1])]),
    )
}

/// The line of a rule from `start`.
enum RuleLine {
    /// A rule without a second position, which draws nothing.
    Point,
    /// A rule to a second place, along its route.
    Segment([f64; 2]),
    /// A rule along its parallel or its meridian.
    Along(LineString<f64>),
}

/// The line of a rule from `start` to its second longitude and latitude: along the parallel of
/// its latitude without a second latitude, along the meridian of its longitude without a second
/// longitude, and along its route with both.
fn rule_line(start: [f64; 2], lon2: Option<f64>, lat2: Option<f64>) -> RuleLine {
    let [lon, lat] = start;
    match (lon2, lat2) {
        (None, None) => RuleLine::Point,
        (Some(lon2), Some(lat2)) => RuleLine::Segment([lon2, lat2]),
        (Some(lon2), None) => RuleLine::Along(parallel(lat, lon, lon2)),
        (None, Some(lat2)) => RuleLine::Along(LineString::from(vec![(lon, lat), (lon, lat2)])),
    }
}

/// The parallel at `lat` from `west` to `east`, with a point at each [`PARALLEL_STEP`], because
/// a parallel is not a great circle.
fn parallel(lat: f64, west: f64, east: f64) -> LineString<f64> {
    let steps = ((east - west).abs() / PARALLEL_STEP).ceil().max(1.0) as usize;
    LineString::from(
        (0..=steps)
            .map(|step| (west + (east - west) * step as f64 / steps as f64, lat))
            .collect::<Vec<_>>(),
    )
}

/// The box from `west` to `east` and from `south` to `north` as a polygon whose north and south
/// edges follow their parallels, wound counterclockwise, so its inside is the box. It crosses
/// the antimeridian when `east` is less than `west`.
fn lon_lat_box(lon_lat: &proto::LonLatBox) -> Polygon<f64> {
    let east = if lon_lat.east < lon_lat.west {
        lon_lat.east + 360.0
    } else {
        lon_lat.east
    };
    let south = parallel(lon_lat.south, lon_lat.west, east);
    let north = parallel(lon_lat.north, east, lon_lat.west);
    let mut ring: Vec<(f64, f64)> = south.0.iter().chain(&north.0).map(|c| (c.x, c.y)).collect();
    ring.push((lon_lat.west, lon_lat.south));
    Polygon::new(LineString::from(ring), Vec::new())
}

/// The summary of what a group of map plots fits: the whole sphere, a box, or the data of its
/// units, as each unit draws it. A group whose data spans no area fits the whole sphere.
fn summary(projection: &Projection, spec: &proto::Geo, units: &[&PlotUnit]) -> Result<FitSummary> {
    let summary = match &spec.fit {
        Some(geo::Fit::FitSphere(_)) => projection.summarize_sphere(),
        Some(geo::Fit::FitBox(lon_lat)) => projection.summarize(&lon_lat_box(lon_lat)),
        Some(geo::Fit::FitData(_)) | None => {
            let mut summary = FitSummary::default();
            for unit in units {
                summary = summary.merge(&unit_summary(projection, &unit.rows)?);
            }
            if summary.aspect().is_none() && matches!(projection.surface, Surface::Spherical(_)) {
                projection.summarize_sphere()
            } else {
                Ok(summary)
            }
        }
    }
    .map_err(geo_error)?;
    if summary.aspect().is_none() {
        return Err(EvalError::Evaluation(
            "a planar map plot fits data that spans an area".to_string(),
        ));
    }
    Ok(summary)
}

/// The summary of the data of a unit of a map plot, as it draws: the geometry of a geoshape, the
/// line strings of lines, trails, and rules along their routes, and the places of other marks.
fn unit_summary(projection: &Projection, rows: &UnitRows) -> Result<FitSummary> {
    let summarize = |geometry: &Geometry<f64>| projection.summarize(geometry).map_err(geo_error);
    if let mark::Mark::Geoshape(_) = rows.mark {
        return projection
            .summarize_geometry(geometry(rows)?.as_ref())
            .map_err(geo_error);
    }
    let Some((lon, lat)) = places(rows.chart, rows.unit, &rows.table)? else {
        return Ok(FitSummary::default());
    };
    let points = || {
        MultiPoint::from(
            lon.iter()
                .zip(&lat)
                .map(|(lon, lat)| (*lon, *lat))
                .collect::<Vec<_>>(),
        )
    };
    let route = match rows.mark {
        mark::Mark::Line(line) => Some(line.route),
        mark::Mark::Trail(trail) => Some(trail.route),
        mark::Mark::Rule(rule) => Some(rule.route),
        _ => None,
    };
    let canvas =
        route.is_some_and(|route| proto::Route::try_from(route) == Ok(proto::Route::Canvas));
    match rows.mark {
        mark::Mark::Line(_) | mark::Mark::Trail(_) if !canvas => {
            let lines: Vec<LineString<f64>> = rows
                .series()?
                .iter()
                .map(|series| {
                    LineString::from(
                        series
                            .iter()
                            .map(|row| (lon[*row as usize], lat[*row as usize]))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect();
            summarize(&Geometry::MultiLineString(MultiLineString(lines)))
        }
        mark::Mark::Rule(_) => {
            let lon2 = degrees(rows.chart, rows.unit, Channel::Longitude2, &rows.table)?;
            let lat2 = degrees(rows.chart, rows.unit, Channel::Latitude2, &rows.table)?;
            let mut lines = Vec::new();
            let mut ends = Vec::new();
            for row in 0..rows.len() {
                let start = [lon[row], lat[row]];
                match rule_line(
                    start,
                    lon2.as_ref().map(|lon2| lon2[row]),
                    lat2.as_ref().map(|lat2| lat2[row]),
                ) {
                    RuleLine::Point => ends.push((start[0], start[1])),
                    RuleLine::Segment(end) if canvas => {
                        ends.push((start[0], start[1]));
                        ends.push((end[0], end[1]));
                    }
                    RuleLine::Segment(end) => lines.push(LineString::from(vec![
                        (start[0], start[1]),
                        (end[0], end[1]),
                    ])),
                    RuleLine::Along(line) => lines.push(line),
                }
            }
            Ok(
                summarize(&Geometry::MultiLineString(MultiLineString(lines)))?
                    .merge(&summarize(&Geometry::MultiPoint(MultiPoint::from(ends)))?),
            )
        }
        _ => summarize(&Geometry::MultiPoint(points())),
    }
}

/// The reach of the largest symbol of the point geometries of a geoshape unit, or `None` for a
/// unit without points.
fn point_reach(rows: &UnitRows, shape: &proto::GeoshapeMark) -> Result<Option<Reach>> {
    let geometry = geometry(rows)?;
    let parts = row_parts(geometry.as_ref()).map_err(geo_error)?;
    if parts.point_rows.is_empty() {
        return Ok(None);
    }
    let symbol = mark::Mark::Symbol(proto::SymbolMark {
        filled: Some(true),
        stroke_width: None,
    });
    let symbols = UnitRows {
        chart: rows.chart,
        unit: rows.unit,
        mark: &symbol,
        table: rows.table.clone(),
        positions: BTreeMap::new(),
        projected: None,
        scales: rows.scales,
        masks: BTreeMap::new(),
        theme: rows.theme,
        engine: rows.engine,
        size: rows.size,
        name: String::new(),
    };
    let Some(reach) = fit::reach(&symbols)? else {
        return Ok(None);
    };
    let stroke = shape
        .stroke_width
        .unwrap_or(rows.theme.map.geoshape_stroke_width)
        / 2.0;
    Ok(Some(
        parts
            .point_rows
            .iter()
            .fold([0.0; 4], |largest, row| max_reach(largest, &reach[*row]))
            .map(|side| side + stroke),
    ))
}

fn max_reach(a: Reach, b: &Reach) -> Reach {
    [
        a[0].max(b[0]),
        a[1].max(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// The sides that label the lines of a graticule: its own, or the left and the bottom.
fn label_sides(spec: Option<&proto::Graticule>) -> Vec<GuideSide> {
    match spec.and_then(|spec| spec.label_sides.as_ref()) {
        Some(sides) => sides
            .sides
            .iter()
            .map(|labeled| side(*labeled, GuideSide::Left))
            .collect(),
        None => vec![GuideSide::Left, GuideSide::Bottom],
    }
}

/// The configuration of a graticule from its spec over the theme. Unless the spec sets their
/// frame, its labels sit on the edges of the plot for a map that `clipped` says fits a box or its
/// data, and on the outline of the projection for a map that fits the whole sphere.
fn graticule_config(
    spec: &proto::Graticule,
    clipped: bool,
    theme: &Theme,
) -> Result<GraticuleConfig> {
    let mut lines = avenger_geo::Graticule::default();
    if let Some(step) = spec.longitude_step {
        lines.step_minor[0] = step;
    }
    if let Some(step) = spec.latitude_step {
        lines.step_minor[1] = step;
    }
    if let Some(step) = spec.major_longitude_step {
        lines.step_major[0] = step;
    }
    if let Some(step) = spec.major_latitude_step {
        lines.step_major[1] = step;
    }
    let extent =
        |lon_lat: &proto::LonLatBox| [[lon_lat.west, lon_lat.south], [lon_lat.east, lon_lat.north]];
    if let Some(minor) = &spec.extent {
        lines.extent_minor = extent(minor);
    }
    if let Some(major) = &spec.major_extent {
        lines.extent_major = extent(major);
    }
    if let Some(precision) = spec.precision {
        lines.precision = precision;
    }
    let frame = match proto::MapFrame::try_from(spec.frame) {
        Ok(proto::MapFrame::Outline) => MapFrame::Outline,
        Ok(proto::MapFrame::Rectangle) => MapFrame::Rectangle,
        _ if clipped => MapFrame::Rectangle,
        _ => MapFrame::Outline,
    };
    let map = &theme.map;
    let mut family = theme.font_family.clone();
    let mut size = map.graticule_label_font_size;
    let mut weight = map.graticule_label_font_weight;
    let mut color = map.graticule_label_color;
    text_style(
        spec.label_style.as_ref(),
        &mut family,
        &mut size,
        &mut weight,
        &mut color,
    )?;
    Ok(GraticuleConfig {
        graticule: lines,
        frame,
        label_sides: label_sides(Some(spec)),
        label_format: spec.label_format.clone(),
        style: GraticuleStyle {
            stroke: if spec.stroke.is_empty() {
                map.graticule_stroke
            } else {
                css(&spec.stroke)?
            },
            stroke_width: spec.stroke_width.unwrap_or(map.graticule_stroke_width),
            label_color: color,
            label_font_family: family,
            label_font_size: size,
            label_font_weight: weight,
            label_padding: spec.label_padding.unwrap_or(map.graticule_label_padding),
        },
    })
}

/// The style of a sphere from its spec over the theme.
fn sphere_style(spec: &proto::SphereStyle, theme: &Theme) -> Result<SphereStyle> {
    let map = &theme.map;
    Ok(SphereStyle {
        fill: if spec.fill.is_empty() {
            map.sphere_fill
        } else {
            css(&spec.fill)?
        },
        stroke: if spec.stroke.is_empty() {
            map.sphere_stroke
        } else {
            css(&spec.stroke)?
        },
        stroke_width: spec.stroke_width.unwrap_or(map.sphere_stroke_width),
    })
}
