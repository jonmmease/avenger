//! Geo plots. When the evaluator prepares a chart, each group of geo plots that share a projection
//! builds its projection from the spec of its first plot, the summary of what it fits, and the
//! inset of its fit, the largest reach of its marks past their places. At each size, the group
//! fits its projection to its plots less the inset, and each plot draws its sphere, its tiles, the
//! lines of its graticule, and its marks through the fitted projector.

use std::{collections::BTreeMap, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray},
    compute::cast,
    datatypes::{DataType, Float64Type},
};
use avenger_chart::{
    layer::{LayerRef, MarkType},
    proto::{self, encoding, geo, geo_layer, option_value, spherical, spherical_clip, Channel},
    resolve::ProjectionGroup,
};
use avenger_coords::{pieces, GeoProjection, PlotCoords, PlotProjection};
use avenger_geo::{
    paths::row_parts, projections as p, FitSummary, Projection, ProjectionImpl, Projector,
    RegionFrame, SphericalClip, Surface,
};
use avenger_guides::GuideSide;
use avenger_guides_geo::{
    geometry_marks, make_sphere_marks, GeoFrame, GeometryStyle, Graticule, GraticuleConfig,
    GraticuleStyle, SphereStyle, TileLayer, TileSource,
};
use avenger_image::ImageResolver;
use avenger_scenegraph::marks::{group::SceneGroup, mark::SceneMark};
use avenger_style::{ComputedStyle, Property};
use geo_traits::to_geo::ToGeoGeometry;
use geo_types::{Geometry, LineString, MultiLineString, MultiPoint, MultiPolygon, Point, Polygon};
use geoarrow_array::{array::from_arrow_array, GeoArrowArray};

use super::{GeoGuide, GeoInputs, Located};
use crate::{
    encode::{numbers, LayerRows, Result},
    guides::{paint_color, text_color, visible},
    marks::{
        font_family, font_weight, image_mark, path_mark, pattern_fills, row_patterns, shape_names,
        shapes, symbol_mark, text_mark, Drawn, TextFormat,
    },
    polyline::{polyline, rule_lines, series_lines, series_trails, Polyline},
    reach::Reach,
    style::{color as style_color, number as style_number},
    EvalError, EvalWarning,
};

fn geo_error(error: impl std::fmt::Display) -> EvalError {
    EvalError::Geo(error.to_string())
}

/// The steps between the points of a parallel or of the edge of a box at one latitude, in
/// degrees, which the projection resamples.
const PARALLEL_STEP: f64 = 1.0;

/// The geo plots of a chart, prepared without a size.
pub(crate) struct GeoPlots {
    groups: Vec<GeoGroup>,
    /// The geo plots, by their position in `Tree::plots`.
    plots: BTreeMap<usize, GeoPlot>,
}

/// The geo plots that share a projection.
struct GeoGroup {
    plots: Vec<usize>,
    /// The projection of the spec, without a view.
    projection: Projection,
    summary: FitSummary,
    /// The largest reach of the marks of the group past their places, toward the left, the
    /// right, the top, and the bottom, by which the fit insets the plot.
    inset: Reach,
    /// Whether the group fits a box or its data, whose projections run past the plot, so its plots
    /// clip to their rectangles. A group that fits the whole sphere fits inside its plots.
    clipped: bool,
}

/// A geo plot, prepared without a size.
struct GeoPlot {
    path: String,
    guide: GeoGuide,
    graticule: Option<Graticule>,
    /// Whether the plot labels its graticule on its rectangle, so its projector clips to it.
    rectangle: bool,
    sphere: Option<SphereStyle>,
    tiles: Option<proto::TileLayer>,
    /// The tile layer, from the tiles of the spec and the image resolver of the host.
    layer: Option<TileLayer>,
}

/// A geo plot at a size: its fitted projection and projector, in the plot's frame.
pub(crate) struct GeoFit {
    /// The fitted projection with its projector at the plot's size.
    projection: Arc<GeoProjection>,
    size: [f32; 2],
    clip: bool,
}

/// The spec of the geo plot at `plot` of `inputs`.
fn spec<'a>(inputs: &GeoInputs<'a>, plot: usize) -> Option<&'a proto::Geo> {
    match inputs.resolved.tree.plots[plot].plot.coord.as_ref()? {
        proto::plot::Coord::Geo(geo) => geo.options.as_ref(),
        _ => None,
    }
}

/// The spherical part of a spec, or `None` for a planar geo plot.
fn spherical(spec: &proto::Geo) -> Option<&proto::Spherical> {
    match spec.surface.as_ref()? {
        geo::Surface::Spherical(spherical) => Some(spherical),
        geo::Surface::Planar(_) => None,
    }
}

impl GeoPlots {
    /// The geo plots of the chart and the projection of each group of them, from the rows of
    /// their layers, `layer_rows`, with the reach of `reach`.
    pub(crate) fn prepare<'r>(
        inputs: &GeoInputs,
        layer_rows: impl Fn(usize) -> LayerRows<'r>,
        reach: &[Option<Vec<Reach>>],
    ) -> Result<Self> {
        let tree = &inputs.resolved.tree;
        let mut groups = Vec::new();
        let mut plots = BTreeMap::new();
        for group in &inputs.resolved.projections {
            let spec = group.spec;
            let projection = projection(spec)?;
            let layers: Vec<usize> = (group.plots.iter())
                .flat_map(|plot| tree.plots[*plot].layers.clone())
                .collect();
            let fit = spherical(spec).and_then(|spherical| spherical.fit.as_ref());
            let clipped = !matches!(fit, Some(spherical::Fit::FitSphere(_)));
            let mut summary = FitSummary::default();
            let mut inset = [0.0_f32; 4];
            let fits_data = matches!(fit, None | Some(spherical::Fit::FitData(_)));
            for layer in &layers {
                let rows = layer_rows(*layer);
                if fits_data {
                    summary = summary.merge(&layer_summary(&projection, &rows)?);
                }
                let reach = match rows.mark.mark_type() {
                    MarkType::Shape => point_reach(&rows)?,
                    _ => {
                        (reach[*layer].as_ref()).map(|reach| reach.iter().fold([0.0; 4], max_reach))
                    }
                };
                if let Some(reach) = reach {
                    inset = max_reach(inset, &reach);
                }
            }
            let summary = group_summary(&projection, fit, summary)?;
            let geo_group = GeoGroup {
                plots: group.plots.clone(),
                projection,
                summary,
                inset,
                clipped,
            };
            for plot in &group.plots {
                let geo = geo_group.plot(inputs, group, *plot)?;
                plots.insert(*plot, geo);
            }
            groups.push(geo_group);
        }
        Ok(Self { groups, plots })
    }

    /// Loads the tiles of the geo plots through `resolver`.
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

    /// What the layout keeps for the geo plot `plot`.
    pub(crate) fn guide(&self, plot: usize) -> Option<&GeoGuide> {
        self.plots.get(&plot).map(|geo| &geo.guide)
    }

    /// Each geo plot fitted at the size of its rectangle in `sizes`, by plot. The plots of a group
    /// take one size, so the group fits one projection at the size of its first plot. A plot too
    /// small for the inset fits its whole rectangle, with a warning.
    pub(crate) fn fit(
        &self,
        sizes: &[[f32; 2]],
        warnings: &mut Vec<EvalWarning>,
    ) -> Result<BTreeMap<usize, GeoFit>> {
        let mut fits = BTreeMap::new();
        for group in &self.groups {
            let Some(first) = group.plots.first() else {
                continue;
            };
            let size = sizes[*first];
            let (fitted, fits_inset) = group.fit(size)?;
            for plot in &group.plots {
                let geo = &self.plots[plot];
                if !fits_inset {
                    warnings.push(EvalWarning::Fit {
                        path: geo.path.clone(),
                        reason: "the plot is too small for its marks, so they reach past its edges"
                            .to_string(),
                    });
                }
                let clip = group.clipped || geo.rectangle;
                let mut projection = fitted.clone();
                if clip {
                    projection.clip_extent =
                        Some([[0.0, 0.0], [f64::from(size[0]), f64::from(size[1])]]);
                }
                let projector = projection.build().map_err(geo_error)?;
                fits.insert(
                    *plot,
                    GeoFit {
                        projection: Arc::new(GeoProjection::new(projector, size)),
                        size,
                        clip,
                    },
                );
            }
        }
        Ok(fits)
    }

    /// What the geo plot `plot` draws behind its data at `fit`: its sphere, its tiles, and the
    /// lines of its graticule. A plot with tiles and no image resolver draws without its tiles,
    /// with a warning.
    pub(crate) fn backdrop(
        &self,
        plot: usize,
        fit: &GeoFit,
        warnings: &mut Vec<EvalWarning>,
    ) -> Result<Vec<SceneMark>> {
        let geo = &self.plots[&plot];
        let mut marks = Vec::new();
        if let Some(style) = &geo.sphere {
            marks.push(
                SceneGroup {
                    interactive: Some(false),
                    ..make_sphere_marks(fit.projector(), style)
                }
                .into(),
            );
        }
        match (&geo.tiles, &geo.layer) {
            (Some(_), Some(layer)) => {
                let zoom = layer.source().zoom(fit.projector());
                marks.push(
                    layer
                        .group(fit.projector(), zoom)
                        .map_err(geo_error)?
                        .into(),
                );
            }
            (Some(_), None) => warnings.push(EvalWarning::NotDrawn {
                path: geo.path.clone(),
                reason:
                    "the plot's tiles load through an image resolver, which the evaluator does \
                         not have, so the plot draws without them"
                        .to_string(),
            }),
            (None, _) => {}
        }
        if let Some(graticule) = &geo.graticule {
            marks.push(graticule.view(fit.projector(), fit.size).lines().into());
        }
        Ok(marks)
    }

    /// The labels of the graticule of the geo plot `plot` at `fit` on `side`, in the slab with the
    /// rectangle `slot`.
    pub(crate) fn labels(
        &self,
        plot: usize,
        fit: &GeoFit,
        side: GuideSide,
        slot: [f32; 4],
    ) -> Option<SceneGroup> {
        let graticule = self.plots.get(&plot)?.graticule.as_ref()?;
        let part = graticule.view(fit.projector(), fit.size).part(side)?;
        Some(SceneGroup {
            origin: graticule.origin(side, slot),
            ..part
        })
    }
}

impl GeoGroup {
    /// The projection fitted to plots of `size` less the inset, and whether the inset leaves the
    /// plots an area. A plot without one fits its whole rectangle.
    fn fit(&self, size: [f32; 2]) -> Result<(Projection, bool)> {
        let [width, height] = size.map(|value| f64::from(value).max(1e-6));
        let [left, right, top, bottom] = self.inset.map(f64::from);
        let inset = [[left, top], [width - right, height - bottom]];
        let fits = inset[1][0] > inset[0][0] && inset[1][1] > inset[0][1];
        let range = match fits {
            true => inset,
            false => [[0.0, 0.0], [width, height]],
        };
        let mut projection = self.projection.clone();
        projection.fit(range, &self.summary).map_err(geo_error)?;
        Ok((projection, fits))
    }

    /// The geo plot `plot` of the group `group`, prepared with the group's projection and
    /// summary, which give its aspect and its graticule, and the computed styles of its nodes.
    fn plot(&self, inputs: &GeoInputs, group: &ProjectionGroup, plot: usize) -> Result<GeoPlot> {
        let plot_ref = &inputs.resolved.tree.plots[plot];
        let spec = spec(inputs, plot).unwrap_or(group.spec);
        let aspect = match spec.aspect {
            Some(geo::Aspect::AspectRatio(ratio)) => Some(ratio),
            Some(geo::Aspect::FreeAspect(_)) => None,
            None => self.summary.aspect().map(|aspect| aspect as f32),
        };
        let nodes = inputs.nodes.plot(&plot_ref.path);
        let styles = inputs.styles;
        let spherical = spherical(spec);
        let (graticule, rectangle, labels) = match (
            spherical.and_then(|spherical| spherical.graticule.as_ref()),
            &nodes.graticule,
        ) {
            (Some(spec), Some(graticule_nodes)) => {
                let config = graticule_config(
                    spec,
                    self.clipped,
                    &styles[graticule_nodes.graticule],
                    &styles[graticule_nodes.grid],
                    &styles[graticule_nodes.label],
                );
                let rectangle = config.frame == GeoFrame::Rectangle;
                let sides = config.label_sides.clone();
                let graticule =
                    Graticule::new(&self.projection, &self.summary, &config, inputs.engine)
                        .map_err(geo_error)?;
                let labels = (sides.into_iter())
                    .map(|side| (side, graticule.view_extent(side)))
                    .filter(|(_, extent)| extent.thickness > 0.0)
                    .collect();
                (Some(graticule), rectangle, labels)
            }
            _ => (None, false, Vec::new()),
        };
        let sphere = (spherical.and_then(|spherical| spherical.sphere.as_ref()))
            .map(|_| sphere_style(&styles[nodes.plot]));
        Ok(GeoPlot {
            path: plot_ref.path.clone(),
            guide: GeoGuide { aspect, labels },
            graticule,
            rectangle,
            sphere,
            tiles: spherical.and_then(|spherical| spherical.tiles.clone()),
            layer: None,
        })
    }
}

impl GeoFit {
    /// Whether the plot clips its marks to its rectangle, because it fits a box or its data, or
    /// its graticule labels the rectangle.
    pub(crate) fn clip(&self) -> bool {
        self.clip
    }

    /// The fitted projector of the plot.
    fn projector(&self) -> &Projector {
        self.projection.projector()
    }

    /// The plot's coordinates: its projection, whose positions are longitudes and latitudes, or
    /// the coordinates of a planar geo plot, without scales.
    pub(crate) fn coords(&self) -> PlotCoords {
        PlotCoords::new(self.projection.clone(), [None, None])
    }

    /// The rows of a mark at places that the projection's clip keeps, with their projected
    /// places, or `None` for a mark that projects as it draws, such as a line.
    pub(crate) fn locate(&self, rows: &LayerRows) -> Result<Option<Located>> {
        if !matches!(
            rows.mark.mark_type(),
            MarkType::Symbol | MarkType::Text | MarkType::Image | MarkType::Path
        ) {
            return Ok(None);
        }
        let Some((lon, lat)) = places(rows)? else {
            return Ok(None);
        };
        let mut kept = Vec::with_capacity(rows.len());
        let (mut x, mut y) = (
            Vec::with_capacity(rows.len()),
            Vec::with_capacity(rows.len()),
        );
        for row in 0..rows.len() {
            if let Some([px, py]) = project_point(self.projector(), lon[row], lat[row]) {
                kept.push(row as u32);
                x.push(px);
                y.push(py);
            }
        }
        Ok(Some((kept, [x, y])))
    }
}

/// The scene marks of a layer of a geo plot, whose mark is `mark`, in the plot fitted at its size,
/// with the projected places of a mark at places from [`GeoFit::locate`].
pub(crate) fn marks(
    rows: &LayerRows,
    mark: &geo_layer::Mark,
    fit: &GeoFit,
    points: Option<[Vec<f32>; 2]>,
) -> Result<Vec<Drawn>> {
    use avenger_common::value::ScalarOrArray;
    use geo_layer::Mark;
    let one = |mark: SceneMark| {
        vec![Drawn {
            mark,
            rows: rows.table.clone(),
        }]
    };
    let at_points = || {
        let [x, y] = points.clone().unwrap_or_default();
        (ScalarOrArray::new_array(x), ScalarOrArray::new_array(y))
    };
    let canvas = |route: i32| proto::Route::try_from(route) == Ok(proto::Route::Canvas);
    Ok(match mark {
        Mark::Symbol(_) => {
            let (x, y) = at_points();
            one(symbol_mark(rows, x, y)?.into())
        }
        Mark::Text(text) => {
            let (x, y) = at_points();
            let format = TextFormat {
                format: &text.format,
                syntax: text.syntax,
            };
            one(text_mark(rows, format, x, y)?.into())
        }
        Mark::Image(_) => {
            let (x, y) = at_points();
            match image_mark(rows, x, y, None)? {
                Some(mark) => one(mark.into()),
                None => Vec::new(),
            }
        }
        Mark::Path(_) => {
            let (x, y) = at_points();
            one(path_mark(rows, x, y)?.into())
        }
        Mark::Line(line) => series_lines(rows, series_polylines(rows, fit, canvas(line.route))?)?,
        Mark::Trail(trail) => {
            series_trails(rows, series_polylines(rows, fit, canvas(trail.route))?)?
        }
        Mark::Rule(rule) => rules(rows, fit, canvas(rule.route))?,
        Mark::Shape(_) => shapes_of(rows, fit)?,
    })
}

/// The pieces in pixels of the segment between the places `from` and `to`: a straight line between
/// their pixels for the canvas route, and the projection's route otherwise, such as a great
/// circle.
fn segment(
    projection: &GeoProjection,
    from: [f64; 2],
    to: [f64; 2],
    canvas: bool,
) -> Vec<Vec<[f32; 2]>> {
    if canvas {
        return pieces([projection.pixel(from), projection.pixel(to)]);
    }
    projection.route(from, to)
}

/// The polyline of each series of a line or a trail through its rows along its route.
fn series_polylines(
    rows: &LayerRows,
    fit: &GeoFit,
    canvas: bool,
) -> Result<Vec<(Vec<u32>, Polyline)>> {
    let Some((lon, lat)) = places(rows)? else {
        return Ok(Vec::new());
    };
    let projection = &fit.projection;
    Ok((rows.series()?.into_iter())
        .map(|series| {
            let polyline = polyline(
                &series,
                |row| [lon[row], lat[row]],
                |place| projection.pixel(place),
                |a, b| segment(projection, a, b, canvas),
            );
            (series, polyline)
        })
        .collect())
}

/// One line for each rule, along its route, or along its parallel or its meridian when it misses
/// a second longitude or latitude.
fn rules(rows: &LayerRows, fit: &GeoFit, canvas: bool) -> Result<Vec<Drawn>> {
    let Some((lon, lat)) = places(rows)? else {
        return Ok(Vec::new());
    };
    let lon2 = degrees(rows, Channel::Longitude2)?;
    let lat2 = degrees(rows, Channel::Latitude2)?;
    let projection = &fit.projection;
    let mut lines = Vec::new();
    for row in 0..rows.len() {
        let start = [lon[row], lat[row]];
        let ends = (
            lon2.as_ref().map(|lon2| lon2[row]),
            lat2.as_ref().map(|lat2| lat2[row]),
        );
        let pieces = match ends {
            (None, None) => continue,
            (Some(lon2), Some(lat2)) => segment(projection, start, [lon2, lat2], canvas),
            (Some(lon2), None) => projection.iso_line(1, start[1], Some([start[0], lon2])),
            (None, Some(lat2)) => projection.iso_line(0, start[0], Some([start[1], lat2])),
        };
        let mut polyline = Polyline::default();
        for piece in pieces {
            polyline.push_piece(&piece, [row as u32; 2]);
        }
        lines.push((row, polyline));
    }
    rule_lines(rows, lines)
}

/// The polygons, the lines, and the points of the geometry of each row, as a path mark of
/// polygons, a path mark of lines, and a symbol mark of points, from the computed styles of the
/// layer's `mark` node and its channels.
fn shapes_of(rows: &LayerRows, fit: &GeoFit) -> Result<Vec<Drawn>> {
    use avenger_common::value::ScalarOrArray;
    let geometry = geometry(rows)?;
    let projected = fit
        .projector()
        .project_geometry(geometry.as_ref())
        .map_err(geo_error)?;
    let (fill, stroke) = rows.paint()?;
    let (shape_list, _) = shapes(&shape_names(rows)?)?;
    let stroke_width = match rows.numbers(Channel::StrokeWidth)? {
        Some(widths) => crate::encode::at(&widths, 0),
        None => style_number(rows.style, Property::StrokeWidth),
    };
    let point_size = rows.numbers_or(Channel::Size, Property::Size)?;
    let style = GeometryStyle {
        fill,
        stroke,
        stroke_width,
        point_size: match point_size.value() {
            avenger_common::value::ScalarOrArrayValue::Array(_) => point_size,
            avenger_common::value::ScalarOrArrayValue::Scalar(size) => {
                ScalarOrArray::new_scalar(*size)
            }
        },
        point_shape: shape_list.into_iter().next().expect("a layer has a shape"),
    };
    let group = geometry_marks(projected.as_ref(), &style).map_err(geo_error)?;
    let parts = row_parts(projected.as_ref()).map_err(geo_error)?;
    let has_polygons = (parts.polygons.iter()).any(|path| path.iter().next().is_some());
    let has_lines = (parts.lines.iter()).any(|path| path.iter().next().is_some());
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
    let patterns = pattern_fills(row_patterns(rows)?);
    let mut drawn = Vec::new();
    for (mark, kind) in group.marks.into_iter().zip(names) {
        let name = format!("{}/{kind}", rows.name);
        let (mark, mark_rows) = match mark {
            SceneMark::Path(mut path) => {
                path.name = name;
                if kind == "polygons" {
                    path.fill_pattern = patterns.clone();
                }
                (SceneMark::from(path), rows.table.clone())
            }
            SceneMark::Symbol(mut symbol) => {
                symbol.name = name;
                let mark_rows = match symbol.len as usize == rows.len() {
                    true => rows.table.clone(),
                    false => {
                        let points: Vec<u32> =
                            parts.point_rows.iter().map(|row| *row as u32).collect();
                        rows.rows(&points)?
                    }
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

/// The projection of a geo spec, without a view.
fn projection(spec: &proto::Geo) -> Result<Projection> {
    let spherical = match spec.surface.as_ref() {
        Some(geo::Surface::Planar(planar)) => return Ok(Projection::planar(planar.reflect_y)),
        Some(geo::Surface::Spherical(spherical)) => spherical,
        None => return Err(geo_error("a geo plot has no surface")),
    };
    let implementation = projection_impl(
        (spherical.projection.as_ref()).ok_or_else(|| geo_error("a geo plot has no projection"))?,
    )?;
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

/// The avenger-geo projection of a typed projection, or of a custom projection through its
/// typetag name and the fields of its serde form.
fn projection_impl(projection: &spherical::Projection) -> Result<Box<dyn ProjectionImpl>> {
    use spherical::Projection as P;
    let conic = |conic: &proto::ConicProjection| [conic.parallel1, conic.parallel2];
    Ok(match projection {
        P::EqualEarth(_) => Box::new(p::EqualEarth),
        P::NaturalEarth1(_) => Box::new(p::NaturalEarth1),
        P::Robinson(_) => Box::new(p::Robinson),
        P::Mollweide(_) => Box::new(p::Mollweide),
        P::Eckert4(_) => Box::new(p::Eckert4),
        P::WinkelTripel(_) => Box::new(p::WinkelTripel),
        P::Aitoff(_) => Box::new(p::Aitoff),
        P::Sinusoidal(_) => Box::new(p::Sinusoidal),
        P::Equirectangular(_) => Box::new(p::Equirectangular),
        P::Mercator(_) => Box::new(p::Mercator),
        P::TransverseMercator(_) => Box::new(p::TransverseMercator),
        P::Miller(_) => Box::new(p::Miller),
        P::Orthographic(_) => Box::new(p::Orthographic),
        P::Stereographic(_) => Box::new(p::Stereographic),
        P::Gnomonic(_) => Box::new(p::Gnomonic),
        P::AzimuthalEqualArea(_) => Box::new(p::AzimuthalEqualArea),
        P::AzimuthalEquidistant(_) => Box::new(p::AzimuthalEquidistant),
        P::ConicConformal(spec) => Box::new(p::ConicConformal::new(conic(spec))),
        P::ConicEqualArea(spec) => Box::new(p::ConicEqualArea::new(conic(spec))),
        P::ConicEquidistant(spec) => Box::new(p::ConicEquidistant::new(conic(spec))),
        P::CylindricalEqualArea(spec) => {
            Box::new(p::CylindricalEqualArea::new(spec.parallel.unwrap_or(38.58)))
        }
        P::Hammer(spec) => Box::new(p::Hammer::new(spec.coefficient.unwrap_or(2.0))),
        P::Custom(custom) => custom_projection(custom)?,
    })
}

/// The avenger-geo projection with the typetag name of `custom` and the fields of its serde form
/// from its parameters.
fn custom_projection(custom: &proto::CustomProjection) -> Result<Box<dyn ProjectionImpl>> {
    let mut fields = serde_json::Map::new();
    for parameter in &custom.parameters {
        let value = match (parameter.value.as_ref()).and_then(|value| value.value.as_ref()) {
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
            serde_json::json!({ "type": custom.name, "fields": fields }),
        )
    };
    // A projection without parameters is a unit struct, whose fields are null, or a struct whose
    // fields all have defaults.
    let read = match fields.is_empty() {
        true => read(serde_json::Value::Null).or_else(|_| read(serde_json::json!({}))),
        false => read(serde_json::Value::Object(fields)),
    };
    read.map_err(|error| geo_error(format!("the projection `{}`: {error}", custom.name)))
}

/// The multipolygon of WKB bytes.
fn wkb_polygon(bytes: &[u8]) -> Result<MultiPolygon<f64>> {
    let wkb = wkb::reader::read_wkb(bytes).map_err(geo_error)?;
    match wkb.to_geometry() {
        Geometry::MultiPolygon(polygons) => Ok(polygons),
        Geometry::Polygon(polygon) => Ok(MultiPolygon(vec![polygon])),
        _ => Err(geo_error("a region clip is a polygon or a multipolygon")),
    }
}

/// The values of the encoding of `channel` of the layer for each of its rows, as degrees, or `None`
/// when the layer does not encode it.
fn degrees(rows: &LayerRows, channel: Channel) -> Result<Option<Vec<f64>>> {
    let Some(value) = (rows.encoding(channel)).and_then(|encoding| encoding.kind.as_ref()) else {
        return Ok(None);
    };
    let values: ArrayRef = match value {
        encoding::Kind::DirectField(name) | encoding::Kind::ScaledField(name) => rows
            .column(name)
            .expect("a valid layer's table or partition has its columns"),
        encoding::Kind::DirectValue(value) | encoding::Kind::ScaledValue(value) => {
            rows.chart.value(value).clone()
        }
    };
    let values = cast(&values, &DataType::Float64)?;
    let values = values.as_primitive::<Float64Type>();
    let len = rows.len();
    Ok(Some(match values.len() == 1 && len != 1 {
        true => vec![values.value(0); len],
        false => values.values().to_vec(),
    }))
}

/// The longitude and the latitude of each row of the layer, or `None` for a layer without them.
fn places(rows: &LayerRows) -> Result<Option<(Vec<f64>, Vec<f64>)>> {
    let lon = degrees(rows, Channel::Longitude)?;
    let lat = degrees(rows, Channel::Latitude)?;
    Ok(lon.zip(lat))
}

/// The GeoArrow geometry of the `geometry` column of a layer.
fn geometry(rows: &LayerRows) -> Result<Arc<dyn GeoArrowArray>> {
    let name = match (rows.encoding(Channel::Geometry)).and_then(|encoding| encoding.kind.as_ref())
    {
        Some(encoding::Kind::DirectField(name) | encoding::Kind::ScaledField(name)) => name,
        _ => return Err(geo_error("a shape mark takes its `geometry` from a column")),
    };
    let schema = rows.table.schema();
    let field = schema.field_with_name(name)?;
    let column = (rows.table.column_by_name(name)).expect("a valid layer's table has its columns");
    from_arrow_array(column.as_ref(), field).map_err(geo_error)
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
    let east = match lon_lat.east < lon_lat.west {
        true => lon_lat.east + 360.0,
        false => lon_lat.east,
    };
    let south = parallel(lon_lat.south, lon_lat.west, east);
    let north = parallel(lon_lat.north, east, lon_lat.west);
    let mut ring: Vec<(f64, f64)> = (south.0.iter().chain(&north.0))
        .map(|coord| (coord.x, coord.y))
        .collect();
    ring.push((lon_lat.west, lon_lat.south));
    Polygon::new(LineString::from(ring), Vec::new())
}

/// The summary of what a group of geo plots fits: the whole sphere, a box, or the data of its
/// layers, `data`. A spherical group whose data spans no area fits the whole sphere, and a planar
/// one fails.
fn group_summary(
    projection: &Projection,
    fit: Option<&spherical::Fit>,
    data: FitSummary,
) -> Result<FitSummary> {
    let summary = match fit {
        Some(spherical::Fit::FitSphere(_)) => projection.summarize_sphere(),
        Some(spherical::Fit::FitBox(lon_lat)) => projection.summarize(&lon_lat_box(lon_lat)),
        Some(spherical::Fit::FitData(_)) | None => {
            match data.aspect().is_none() && matches!(projection.surface, Surface::Spherical(_)) {
                true => projection.summarize_sphere(),
                false => Ok(data),
            }
        }
    }
    .map_err(geo_error)?;
    if summary.aspect().is_none() {
        return Err(geo_error("a planar geo plot fits data that spans an area"));
    }
    Ok(summary)
}

/// The summary of the data of a layer of a geo plot, as it draws: the geometry of a shape, the
/// line strings of lines and trails, and of rules along their routes, and the places of the other
/// marks.
fn layer_summary(projection: &Projection, rows: &LayerRows) -> Result<FitSummary> {
    let summarize = |geometry: &Geometry<f64>| projection.summarize(geometry).map_err(geo_error);
    if rows.mark.mark_type() == MarkType::Shape {
        return (projection.summarize_geometry(geometry(rows)?.as_ref())).map_err(geo_error);
    }
    let Some((lon, lat)) = places(rows)? else {
        return Ok(FitSummary::default());
    };
    let points = || {
        MultiPoint::from(
            (lon.iter().zip(&lat))
                .map(|(lon, lat)| (*lon, *lat))
                .collect::<Vec<_>>(),
        )
    };
    let route = match rows.layer {
        LayerRef::Geo(layer) => match &layer.mark {
            Some(geo_layer::Mark::Line(line)) => line.route,
            Some(geo_layer::Mark::Trail(trail)) => trail.route,
            Some(geo_layer::Mark::Rule(rule)) => rule.route,
            _ => 0,
        },
        _ => 0,
    };
    let canvas = proto::Route::try_from(route) == Ok(proto::Route::Canvas);
    match rows.mark.mark_type() {
        MarkType::Line | MarkType::Trail if !canvas => {
            let lines: Vec<LineString<f64>> = (rows.series()?.iter())
                .map(|series| {
                    LineString::from(
                        (series.iter())
                            .map(|row| (lon[*row as usize], lat[*row as usize]))
                            .collect::<Vec<_>>(),
                    )
                })
                .collect();
            summarize(&Geometry::MultiLineString(MultiLineString(lines)))
        }
        MarkType::Rule => {
            let lon2 = degrees(rows, Channel::Longitude2)?;
            let lat2 = degrees(rows, Channel::Latitude2)?;
            let mut lines = Vec::new();
            let mut ends = Vec::new();
            for row in 0..rows.len() {
                let start = [lon[row], lat[row]];
                let line = rule_line(
                    start,
                    lon2.as_ref().map(|lon2| lon2[row]),
                    lat2.as_ref().map(|lat2| lat2[row]),
                );
                match line {
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

/// How far the symbols of the point geometries of a shape layer reach past their places, at the
/// largest size that its encodings give them, with half its stroke, or `None` for a layer
/// without points.
fn point_reach(rows: &LayerRows) -> Result<Option<Reach>> {
    let parts = row_parts(geometry(rows)?.as_ref()).map_err(geo_error)?;
    if parts.point_rows.is_empty() {
        return Ok(None);
    }
    let sizes = rows.alternatives(Channel::Size, &numbers)?;
    let size = match sizes.is_empty() {
        true => style_number(rows.style, Property::Size),
        false => (sizes.iter())
            .flat_map(|sizes| sizes.as_vec(rows.len(), None))
            .fold(0.0, f32::max),
    };
    let stroke = style_number(rows.style, Property::StrokeWidth) / 2.0;
    Ok(Some([size.max(0.0).sqrt() / 2.0 + stroke; 4]))
}

fn max_reach(a: Reach, b: &Reach) -> Reach {
    [0, 1, 2, 3].map(|side| a[side].max(b[side]))
}

/// The sides of a graticule's spec that label its lines: its own, or the left and the bottom.
fn label_sides(spec: &proto::Graticule) -> Vec<GuideSide> {
    match &spec.label_sides {
        Some(sides) => {
            let mut sides: Vec<GuideSide> = (sides.sides.iter())
                .filter_map(|side| match proto::Side::try_from(*side).ok()? {
                    proto::Side::Top => Some(GuideSide::Top),
                    proto::Side::Bottom => Some(GuideSide::Bottom),
                    proto::Side::Left => Some(GuideSide::Left),
                    proto::Side::Right => Some(GuideSide::Right),
                    proto::Side::Unspecified => None,
                })
                .collect();
            // A side that repeats counts once.
            let mut seen = Vec::new();
            sides.retain(|side| match seen.contains(side) {
                true => false,
                false => {
                    seen.push(*side);
                    true
                }
            });
            sides
        }
        None => vec![GuideSide::Left, GuideSide::Bottom],
    }
}

/// The configuration of a graticule from its spec and the computed styles of its `graticule`, its
/// `grid`, and its `label`. Unless the spec sets their frame, its labels sit on the edges of the
/// plot for a plot that `clipped` says fits a box or its data, and on the outline of the projection
/// for a plot that fits the whole sphere. A hidden `label` labels no side.
fn graticule_config(
    spec: &proto::Graticule,
    clipped: bool,
    graticule: &ComputedStyle,
    grid: &ComputedStyle,
    label: &ComputedStyle,
) -> GraticuleConfig {
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
    let frame = match proto::GeoFrame::try_from(spec.frame) {
        Ok(proto::GeoFrame::Outline) => GeoFrame::Outline,
        Ok(proto::GeoFrame::Rectangle) => GeoFrame::Rectangle,
        _ if clipped => GeoFrame::Rectangle,
        _ => GeoFrame::Outline,
    };
    let shown = visible(grid);
    GraticuleConfig {
        graticule: lines,
        frame,
        label_sides: match visible(label) {
            true => label_sides(spec),
            false => Vec::new(),
        },
        label_format: spec.format.clone(),
        style: GraticuleStyle {
            stroke: match shown {
                true => paint_color(grid, Property::Stroke),
                false => [0.0; 4],
            },
            stroke_width: style_number(grid, Property::StrokeWidth),
            label_color: text_color(label),
            label_font_family: font_family(label),
            label_font_size: label.font_size(),
            label_font_weight: font_weight(label),
            label_padding: style_number(graticule, Property::LabelPadding),
        },
    }
}

/// The style of a sphere from the computed `background` and border of its `plot[type=geo]`.
fn sphere_style(plot: &ComputedStyle) -> SphereStyle {
    let border = (plot.keyword(Property::BorderStyle))
        .is_some_and(|line| line != "none")
        .then(|| plot.color(Property::BorderColor))
        .flatten();
    SphereStyle {
        fill: style_color(plot, Property::Background),
        stroke: border.unwrap_or([0.0; 4]),
        stroke_width: match border {
            Some(_) => style_number(plot, Property::BorderWidth),
            None => 0.0,
        },
    }
}
