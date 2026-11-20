//! The rules of the nodes of the tree: plots and their coordinate systems, concats, wraps, and
//! empty cells, with their layouts, titles, attributions, partitions, sharing, and tools.

use arrow::{datatypes::DataType, record_batch::RecordBatch};

use super::{
    classes, fail, keyed, label_settings, named, non_negative, positive, shown_labels, size, title,
    within, Checker, Result, REQUIRED,
};
use crate::{
    layer::{CoordKind, LayerRef},
    proto::{
        self, layer_tool, node::Node, plot::Coord, projection, select, spherical_clip, tool, track,
        Channel, ProjectionType,
    },
};

/// What a node inherits from its ancestors: the columns of their partitions, and whether the node
/// is a cell of a wrap.
#[derive(Clone, Default)]
pub(super) struct Scope {
    pub(super) partition: Vec<String>,
    cell: bool,
}

impl Scope {
    /// The scope of the children of a node that declares `partition`, which are cells of a wrap
    /// when `cell` is set.
    fn with(&self, partition: &[proto::PartitionValue], cell: bool) -> Self {
        let mut scope = self.clone();
        scope
            .partition
            .extend(partition.iter().map(|value| value.column.clone()));
        scope.cell = cell;
        scope
    }
}

impl Checker<'_> {
    pub(super) fn node(&self, node: &proto::Node, path: &str, scope: &Scope) -> Result<()> {
        match node.node.as_ref() {
            Some(Node::Plot(plot)) => self.plot(plot, &format!("{path}.plot"), scope),
            Some(Node::Concat(concat)) => self.concat(concat, &format!("{path}.concat"), scope),
            Some(Node::Wrap(wrap)) => self.wrap(wrap, &format!("{path}.wrap"), scope),
            Some(Node::Empty(empty)) => self.partition(&empty.partition, &format!("{path}.empty")),
            None => Err(fail(
                path,
                "a node is a plot, a concat, a wrap, or an empty cell",
            )),
        }
    }

    fn concat(&self, concat: &proto::Concat, path: &str, scope: &Scope) -> Result<()> {
        self.partition(&concat.partition, path)?;
        classes(&concat.classes, path)?;
        title(concat.title.as_ref(), &format!("{path}.title"), true)?;
        resolve(concat.resolve.as_ref(), path, false)?;
        non_negative(concat.gap.map(f64::from), &format!("{path}.gap"))?;
        if !concat.tracks.is_empty() && concat.tracks.len() != concat.children.len() {
            return Err(fail(
                format!("{path}.tracks"),
                format!(
                    "a concat has one track for each child or no tracks, and it has {} tracks for {} children",
                    concat.tracks.len(),
                    concat.children.len()
                ),
            ));
        }
        for (index, track) in concat.tracks.iter().enumerate() {
            track_size(track, &format!("{path}.tracks[{index}]"))?;
        }
        if let Some(facet) = &concat.facet {
            let path = format!("{path}.facet");
            facet_column(facet, &path)?;
            classes(&facet.classes, &path)?;
            title(facet.title.as_ref(), &format!("{path}.title"), false)?;
            facet_labels(facet.labels.as_ref(), &format!("{path}.labels"))?;
        }
        let scope = scope.with(&concat.partition, false);
        for (index, child) in concat.children.iter().enumerate() {
            self.node(child, &format!("{path}.children[{index}]"), &scope)?;
        }
        Ok(())
    }

    fn wrap(&self, wrap: &proto::Wrap, path: &str, scope: &Scope) -> Result<()> {
        self.partition(&wrap.partition, path)?;
        classes(&wrap.classes, path)?;
        title(wrap.title.as_ref(), &format!("{path}.title"), true)?;
        resolve(wrap.resolve.as_ref(), path, false)?;
        size(wrap.min_column_width, &format!("{path}.min_column_width"))?;
        if wrap.max_columns == Some(0) {
            return Err(fail(
                format!("{path}.max_columns"),
                "a wrap has a column or more",
            ));
        }
        if let Some(row) = &wrap.row_height {
            track_size(row, &format!("{path}.row_height"))?;
        }
        non_negative(wrap.gap.map(f64::from), &format!("{path}.gap"))?;
        if let Some(facet) = &wrap.facet {
            let path = format!("{path}.facet");
            facet_column(facet, &path)?;
            classes(&facet.classes, &path)?;
            title(facet.title.as_ref(), &format!("{path}.title"), false)?;
            facet_labels(facet.labels.as_ref(), &format!("{path}.labels"))?;
        }
        let scope = scope.with(&wrap.partition, true);
        for (index, child) in wrap.children.iter().enumerate() {
            self.node(child, &format!("{path}.children[{index}]"), &scope)?;
        }
        Ok(())
    }

    fn plot(&self, plot: &proto::Plot, path: &str, scope: &Scope) -> Result<()> {
        self.partition(&plot.partition, path)?;
        classes(&plot.classes, path)?;
        title(plot.title.as_ref(), &format!("{path}.title"), true)?;
        resolve(plot.resolve.as_ref(), path, true)?;
        size(plot.width, &format!("{path}.width"))?;
        size(plot.height, &format!("{path}.height"))?;
        if let Some(attribution) = &plot.attribution {
            let path = format!("{path}.attribution");
            if matches!(
                proto::Side::try_from(attribution.orient),
                Ok(proto::Side::Left | proto::Side::Right)
            ) {
                return Err(fail(
                    &path,
                    "an attribution stands at the top or the bottom of its plot",
                ));
            }
            if proto::TextAlign::try_from(attribution.text_align) == Ok(proto::TextAlign::Center) {
                return Err(fail(
                    format!("{path}.text_align"),
                    "an attribution stands at the start or the end of its side",
                ));
            }
            non_negative(
                attribution.separator_space.map(f64::from),
                &format!("{path}.separator_space"),
            )?;
            non_negative(
                attribution.line_gap.map(f64::from),
                &format!("{path}.line_gap"),
            )?;
        }
        let kind = CoordKind::of(plot).expect(REQUIRED);
        let coord_path = format!("{path}.{}", kind.field());
        match plot
            .coord
            .as_ref()
            .expect("the kind says that the plot sets one")
        {
            Coord::Cartesian(cartesian) => {
                if let Some(options) = &cartesian.options {
                    positive(
                        options.aspect_ratio.map(f64::from),
                        &format!("{coord_path}.options.aspect_ratio"),
                    )?;
                    // A reader ignores the size of a plot in a cell of a wrap.
                    if options.aspect_ratio.is_some()
                        && !scope.cell
                        && plot.width.is_some()
                        && plot.height.is_some()
                    {
                        return Err(fail(
                            path,
                            "a Cartesian plot with an aspect ratio that sets its width and its \
                             height is a conflict",
                        ));
                    }
                }
            }
            Coord::Polar(polar) => {
                if let Some(options) = &polar.options {
                    polar_options(options, &format!("{coord_path}.options"))?;
                }
            }
            Coord::Geo(geo) => {
                let options = geo.options.as_ref().expect(REQUIRED);
                self.geo_options(options, &format!("{coord_path}.options"))?;
            }
        }
        let layers = LayerRef::of(plot);
        // The partition columns of the plot: those of its ancestors and its own, and those that
        // every layer declares with one value.
        let mut scope = scope.with(&plot.partition, false);
        scope.partition.extend(self.shared_columns(&layers));
        let tables: Vec<Option<&RecordBatch>> = (layers.iter())
            .map(|layer| layer.table().map(|table| self.chart.table(table)))
            .collect();
        for (index, layer) in layers.iter().enumerate() {
            let path = format!("{coord_path}.layers[{index}]");
            let table = tables[index].expect(REQUIRED);
            self.partition(layer.partition(), &path)?;
            let layer_scope = scope.with(layer.partition(), false);
            for (position, tool) in layer.tools().iter().enumerate() {
                let path = format!("{path}.tools[{position}]");
                match tool.kind.as_ref() {
                    Some(layer_tool::Kind::Tooltip(tooltip)) => {
                        self.tooltip(tooltip, &[table], &layer_scope, &format!("{path}.tooltip"))?
                    }
                    Some(layer_tool::Kind::Select(select)) => self.select(
                        select,
                        kind,
                        &[table],
                        &layer_scope,
                        &format!("{path}.select"),
                    )?,
                    None => unreachable!("{REQUIRED}"),
                }
            }
            // A selection's clauses name their owner by id, and a layer without one is a part of
            // the chart by its path, which no clause names.
            let selects = (layer.tools().iter())
                .any(|tool| matches!(tool.kind, Some(layer_tool::Kind::Select(_))));
            if selects && layer.id().is_empty() {
                return Err(fail(
                    &path,
                    "a layer that declares a selection has an id, which its clauses name",
                ));
            }
            self.widgets(layer.widgets(), table, &layer_scope, &path)?;
            classes(layer.classes(), &path)?;
            self.layer(*layer, table, &path)?;
        }
        let tables: Vec<&RecordBatch> = tables.into_iter().flatten().collect();
        let mut navigations = 0;
        for (index, tool) in plot.tools.iter().enumerate() {
            let path = format!("{path}.tools[{index}]");
            match tool.kind.as_ref() {
                Some(tool::Kind::Navigate(navigate)) => {
                    self.tool_modes(&navigate.modes, &format!("{path}.navigate.modes"))?;
                    navigations += 1;
                    if kind == CoordKind::Polar {
                        return Err(fail(&path, "a navigation is invalid on a polar plot"));
                    }
                    if navigations > 1 {
                        return Err(fail(&path, "a plot has at most one navigation"));
                    }
                    for channel in &navigate.channels {
                        let channel = Channel::try_from(*channel).unwrap_or_default();
                        if !kind.navigated().contains(&channel) {
                            return Err(fail(
                                format!("{path}.navigate.channels"),
                                format!(
                                    "a navigation of a {} plot does not move the scale of `{}`",
                                    kind.name(),
                                    channel.field_name()
                                ),
                            ));
                        }
                    }
                }
                Some(tool::Kind::Tooltip(tooltip)) => {
                    self.tooltip(tooltip, &tables, &scope, &format!("{path}.tooltip"))?
                }
                Some(tool::Kind::Select(select)) => {
                    self.select(select, kind, &tables, &scope, &format!("{path}.select"))?
                }
                None => unreachable!("{REQUIRED}"),
            }
        }
        // A selection's clauses name their owner by id, and a plot without one is a part of the
        // chart by its path, which no clause names. A navigation that moves a scale whose domain
        // reads a selection writes a clause too.
        let selects =
            (plot.tools.iter()).any(|tool| matches!(tool.kind, Some(tool::Kind::Select(_))));
        if selects && plot.id.is_empty() {
            return Err(fail(
                path,
                "a plot that declares a selection has an id, which its clauses name",
            ));
        }
        if plot.id.is_empty() && navigates_selection(plot, kind, &layers) {
            return Err(fail(
                path,
                "a plot whose navigation moves a scale whose domain reads a selection has an id, \
                 which the clauses of its navigation name",
            ));
        }
        Ok(())
    }

    fn geo_options(&self, geo: &proto::Geo, path: &str) -> Result<()> {
        let projection = geo.projection.as_ref().expect(REQUIRED);
        projection_takes(geo, path)?;
        self.projection(projection, &format!("{path}.projection"))?;
        if let Some(proto::geo::AspectRatio::Ratio(ratio)) = geo.aspect_ratio {
            positive(Some(f64::from(ratio)), &format!("{path}.aspect_ratio"))?;
        }
        if let Some(navigation) = &geo.navigation {
            let path = format!("{path}.navigation");
            positive(navigation.min_zoom, &format!("{path}.min_zoom"))?;
            positive(navigation.max_zoom, &format!("{path}.max_zoom"))?;
            if let (Some(min), Some(max)) = (navigation.min_zoom, navigation.max_zoom) {
                if min > max {
                    return Err(fail(
                        &path,
                        "the zoom limits of a geo plot run from `min_zoom` up to `max_zoom`",
                    ));
                }
            }
        }
        if let Some(graticule) = &geo.graticule {
            let path = format!("{path}.graticule");
            for (step, at) in [
                (&graticule.step, "step"),
                (&graticule.step_major, "step_major"),
            ] {
                let at = format!("{path}.{at}");
                count(
                    step,
                    &[2],
                    &at,
                    "a graticule's step holds two numbers of degrees",
                )?;
                for value in step {
                    positive(Some(*value), &at)?;
                }
            }
            for (extent, at) in [
                (&graticule.extent, "extent"),
                (&graticule.extent_major, "extent_major"),
            ] {
                lon_lat_box(extent, &format!("{path}.{at}"))?;
            }
            positive(graticule.precision, &format!("{path}.precision"))?;
            if let Some(labels) = &graticule.labels {
                let path = format!("{path}.labels");
                if !labels.markup.is_empty() {
                    return Err(fail(
                        format!("{path}.markup"),
                        "the labels of a graticule take no `markup`",
                    ));
                }
                label_settings(
                    Some(labels),
                    &path,
                    "a graticule",
                    &["margin", "frame", "sides"],
                )?;
                if let Some(sides) = &labels.sides {
                    if sides.sides.contains(&(proto::Side::Unspecified as i32)) {
                        return Err(fail(format!("{path}.sides"), "a label side is a side"));
                    }
                }
                self.pattern(
                    &labels.format,
                    &DataType::Float64,
                    &format!("{path}.format"),
                )?;
                non_negative(labels.margin.map(f64::from), &format!("{path}.margin"))?;
            }
            non_negative(
                graticule.grid_stroke_width.map(f64::from),
                &format!("{path}.grid_stroke_width"),
            )?;
        }
        if let Some(tiles) = &geo.tiles {
            let path = format!("{path}.tiles");
            if tiles.tile_size == Some(0) {
                return Err(fail(
                    format!("{path}.tile_size"),
                    "a tile is more than 0 pixels wide",
                ));
            }
            if let (Some(min), Some(max)) = (tiles.min_zoom, tiles.max_zoom) {
                if min > max {
                    return Err(fail(
                        &path,
                        "the zoom levels of tiles run from `min_zoom` up to `max_zoom`",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Checks the values of the parameters of a projection, after `projection_takes` checked that
    /// its type takes them.
    fn projection(&self, projection: &proto::Projection, path: &str) -> Result<()> {
        let at = |field: &str| format!("{path}.{field}");
        count(
            &projection.parallels,
            &[1, 2],
            &at("parallels"),
            "a conic projection takes one or two standard parallels",
        )?;
        for parallel in &projection.parallels {
            within(Some(*parallel), -90.0, 90.0, &at("parallels"))?;
        }
        within(projection.parallel, -90.0, 90.0, &at("parallel"))?;
        if (projection.coefficient).is_some_and(|value| !(value.is_finite() && value >= 1.0)) {
            return Err(fail(
                at("coefficient"),
                "the coefficient of a Hammer projection is at least 1",
            ));
        }
        count(
            &projection.rotate,
            &[2, 3],
            &at("rotate"),
            "a rotation takes two or three angles: the yaw, the pitch, and an optional roll",
        )?;
        if !projection.rotate.iter().all(|angle| angle.is_finite()) {
            return Err(fail(at("rotate"), "the angles of a rotation are finite"));
        }
        count(
            &projection.center,
            &[2],
            &at("center"),
            "a center takes two numbers: its longitude and its latitude",
        )?;
        if let [longitude, latitude] = projection.center[..] {
            lon_lat(longitude, latitude, &at("center"))?;
        }
        if let Some(spherical_clip::Clip::CircleAngle(angle)) =
            projection.clip.as_ref().and_then(|clip| clip.clip.as_ref())
        {
            if !(angle.is_finite() && *angle > 0.0 && *angle <= 180.0) {
                return Err(fail(
                    at("clip.circle_angle"),
                    "the angle of a circle clip is more than 0 and at most 180 degrees",
                ));
            }
        }
        non_negative(projection.precision, &at("precision"))?;
        if let Some(projection::Fit::Box(fit)) = &projection.fit {
            lon_lat_box(&fit.bounds, &at("box.bounds"))?;
        }
        Ok(())
    }

    /// Checks partition values, keyed by column.
    pub(super) fn partition(&self, partition: &[proto::PartitionValue], path: &str) -> Result<()> {
        let path = format!("{path}.partition");
        let mut keys = Vec::with_capacity(partition.len());
        for (index, value) in partition.iter().enumerate() {
            let path = format!("{path}[{index}]");
            keys.push((
                value.column.clone(),
                self.values.key(value.value.as_ref(), &path)?,
            ));
        }
        keyed(
            &keys,
            |(column, _)| column.clone(),
            |a, b| a.1 == b.1,
            &path,
            "partition values",
        )
    }

    fn tooltip(
        &self,
        tooltip: &proto::Tooltip,
        tables: &[&RecordBatch],
        scope: &Scope,
        path: &str,
    ) -> Result<()> {
        self.tool_modes(&tooltip.modes, &format!("{path}.modes"))?;
        if !tooltip.nearest {
            let found = [
                ("max_radius", tooltip.max_radius.is_some()),
                ("metric", tooltip.metric != 0),
            ];
            if let Some((name, _)) = found.iter().find(|(_, set)| *set) {
                return Err(fail(
                    format!("{path}.{name}"),
                    "a tooltip without `nearest` finds the topmost instance under the pointer, \
                     so it has no `max_radius` and no `metric`",
                ));
            }
        }
        non_negative(
            tooltip.max_radius.map(f64::from),
            &format!("{path}.max_radius"),
        )?;
        if let Some(title) = &tooltip.title {
            let path = format!("{path}.title");
            let data_type = column_type(&title.column, tables, scope, &format!("{path}.column"))?;
            if let Some(data_type) = data_type {
                self.pattern(&title.format, &data_type, &format!("{path}.format"))?;
            }
        }
        for (index, field) in tooltip.fields.iter().enumerate() {
            let path = format!("{path}.fields[{index}]");
            let data_type = column_type(&field.column, tables, scope, &format!("{path}.column"))?;
            if let Some(data_type) = data_type {
                self.pattern(&field.format, &data_type, &format!("{path}.format"))?;
            }
        }
        Ok(())
    }

    /// The partition columns that every one of `layers` declares with one value. A value that does
    /// not refer to a value counts as no value, and the check of its layer reports it.
    fn shared_columns(&self, layers: &[LayerRef]) -> Vec<String> {
        let key = |value: &proto::PartitionValue| {
            (value.value.as_ref()).and_then(|value| self.values.key(Some(value), "").ok())
        };
        let Some((first, rest)) = layers.split_first() else {
            return Vec::new();
        };
        (first.partition().iter())
            .filter(|value| {
                let Some(known) = key(value) else {
                    return false;
                };
                rest.iter().all(|layer| {
                    (layer.partition().iter()).any(|other| {
                        other.column == value.column && key(other).as_ref() == Some(&known)
                    })
                })
            })
            .map(|value| value.column.clone())
            .collect()
    }

    fn select(
        &self,
        select: &proto::Select,
        kind: CoordKind,
        tables: &[&RecordBatch],
        scope: &Scope,
        path: &str,
    ) -> Result<()> {
        self.tool_modes(&select.modes, &format!("{path}.modes"))?;
        self.selection(&select.selection, &format!("{path}.selection"))?;
        for (index, column) in select.scope.iter().enumerate() {
            if !scope.partition.contains(column) {
                return Err(fail(
                    format!("{path}.scope[{index}]"),
                    format!("`{column}` is not a partition column of the plot or its ancestors"),
                ));
            }
        }
        match select.r#type.as_ref() {
            Some(select::Type::Interval(interval)) => {
                for channel in &interval.channels {
                    let channel = Channel::try_from(*channel).unwrap_or_default();
                    if !kind.positions().contains(&channel) {
                        return Err(fail(
                            format!("{path}.interval.channels"),
                            format!(
                                "a box of a {} plot selects on its positional channels, not `{}`",
                                kind.name(),
                                channel.field_name()
                            ),
                        ));
                    }
                }
                positive(interval.cell, &format!("{path}.interval.cell"))?;
            }
            Some(select::Type::Point(points)) => {
                if points.keys.is_empty() {
                    return Err(fail(
                        format!("{path}.point.keys"),
                        "a point selection has a key column or more",
                    ));
                }
                for (index, key) in points.keys.iter().enumerate() {
                    let path = format!("{path}.point.keys[{index}]");
                    let data_type = column_type(&key.column, tables, scope, &path)?;
                    // A source is a column of the query's input, which the chart's tables need
                    // not hold, so only its key is checked.
                    let Some(source) = &key.source else {
                        continue;
                    };
                    named(source, &format!("{path}.source"), "the source of a key")?;
                    if points.keys.len() != 1 {
                        return Err(fail(
                            format!("{path}.source"),
                            "a key with a source is the point selection's one key",
                        ));
                    }
                    if !data_type.as_ref().is_some_and(crate::lump::is_lump) {
                        return Err(fail(
                            format!("{path}.source"),
                            format!(
                                "`{}` is a column of lumps, the one kind of key with a source",
                                key.column
                            ),
                        ));
                    }
                }
            }
            Some(select::Type::Lasso(lasso)) => {
                for (index, key) in lasso.keys.iter().enumerate() {
                    column_type(key, tables, scope, &format!("{path}.lasso.keys[{index}]"))?;
                }
            }
            None => {}
        }
        Ok(())
    }
}

/// The type of the column `name` of the first of `tables` that has it, or `None` for a partition
/// column, and a failure for a column that neither has.
fn column_type(
    name: &str,
    tables: &[&RecordBatch],
    scope: &Scope,
    path: &str,
) -> Result<Option<DataType>> {
    if let Some(column) = tables.iter().find_map(|table| table.column_by_name(name)) {
        return Ok(Some(column.data_type().clone()));
    }
    if scope.partition.iter().any(|column| column == name) {
        return Ok(None);
    }
    Err(fail(
        path,
        format!("no table of the plot and no partition has a column `{name}`"),
    ))
}

fn polar_options(polar: &proto::Polar, path: &str) -> Result<()> {
    let [start, end, inner] = crate::resolve::polar_circle(polar);
    if !(start.is_finite() && end.is_finite() && end > start && end - start <= 360.0) {
        return Err(fail(
            path,
            "the end angle of a polar plot is more than its start angle and at most 360 degrees \
             past it",
        ));
    }
    if !(0.0..1.0).contains(&inner) {
        return Err(fail(
            format!("{path}.inner_radius"),
            "the inner radius of a polar plot is at least 0 and less than 1",
        ));
    }
    Ok(())
}

fn lon_lat(longitude: f64, latitude: f64, path: &str) -> Result<()> {
    if !(longitude.is_finite() && (-90.0..=90.0).contains(&latitude)) {
        return Err(fail(
            path,
            "a place has a finite longitude and a latitude from -90 to 90",
        ));
    }
    Ok(())
}

/// Checks a box of longitudes and latitudes, which an empty list leaves unset: four numbers, its
/// western, southern, eastern, and northern edges.
fn lon_lat_box(bounds: &[f64], path: &str) -> Result<()> {
    count(
        bounds,
        &[4],
        path,
        "a box takes four numbers: its western, southern, eastern, and northern edges",
    )?;
    if let [west, south, east, north] = bounds[..] {
        lon_lat(west, south, path)?;
        lon_lat(east, north, path)?;
        if south > north {
            return Err(fail(
                path,
                "the south edge of a box is at most its north edge",
            ));
        }
    }
    Ok(())
}

/// Fails for a list that is not empty and whose length is not one of `lengths`.
fn count(values: &[f64], lengths: &[usize], path: &str, rule: &str) -> Result<()> {
    if !values.is_empty() && !lengths.contains(&values.len()) {
        return Err(fail(path, format!("{rule}, not {}", values.len())));
    }
    Ok(())
}

/// The projection types that take a parameter.
#[derive(Clone, Copy)]
enum Takers {
    /// These types only.
    Only(&'static [ProjectionType]),
    /// The projections of the sphere: every type but `identity`, and the custom projections.
    Sphere,
}

/// The parameters of a geo plot that some projection types take: the path of each from the
/// options of the plot, whether the options set it, and the types that take it. A projection of
/// the type `identity` takes coordinates that are already projected, so it does not rotate,
/// center, clip, resample, or fit a box or the whole sphere, it shows no graticule, sphere, or
/// tiles, and a drag only slides its view.
#[allow(clippy::type_complexity)]
const PROJECTION_PARAMETERS: [(&str, fn(&proto::Geo) -> bool, Takers); 15] = {
    use ProjectionType as T;
    const CONIC: &[ProjectionType] = &[T::ConicConformal, T::ConicEqualArea, T::ConicEquidistant];
    fn of(geo: &proto::Geo) -> &proto::Projection {
        geo.projection.as_ref().expect(REQUIRED)
    }
    [
        (
            "projection.parallels",
            |geo| !of(geo).parallels.is_empty(),
            Takers::Only(CONIC),
        ),
        (
            "projection.parallel",
            |geo| of(geo).parallel.is_some(),
            Takers::Only(&[T::CylindricalEqualArea]),
        ),
        (
            "projection.coefficient",
            |geo| of(geo).coefficient.is_some(),
            Takers::Only(&[T::Hammer]),
        ),
        (
            "projection.reflect_x",
            |geo| of(geo).reflect_x,
            Takers::Only(&[T::Identity]),
        ),
        (
            "projection.reflect_y",
            |geo| of(geo).reflect_y,
            Takers::Only(&[T::Identity]),
        ),
        (
            "projection.rotate",
            |geo| !of(geo).rotate.is_empty(),
            Takers::Sphere,
        ),
        (
            "projection.center",
            |geo| !of(geo).center.is_empty(),
            Takers::Sphere,
        ),
        (
            "projection.clip",
            |geo| of(geo).clip.is_some(),
            Takers::Sphere,
        ),
        (
            "projection.precision",
            |geo| of(geo).precision.is_some(),
            Takers::Sphere,
        ),
        (
            "projection.whole_sphere",
            |geo| matches!(of(geo).fit, Some(projection::Fit::WholeSphere(_))),
            Takers::Sphere,
        ),
        (
            "projection.box",
            |geo| matches!(of(geo).fit, Some(projection::Fit::Box(_))),
            Takers::Sphere,
        ),
        (
            "graticule",
            |geo| {
                (geo.graticule.as_ref()).is_some_and(|graticule| graticule.display != Some(false))
            },
            Takers::Sphere,
        ),
        ("sphere", |geo| geo.sphere == Some(true), Takers::Sphere),
        ("tiles", |geo| geo.tiles.is_some(), Takers::Sphere),
        (
            "navigation.pan_mode",
            |geo| {
                (geo.navigation.as_ref()).is_some_and(|navigation| {
                    matches!(
                        proto::PanMode::try_from(navigation.pan_mode),
                        Ok(proto::PanMode::Longitude | proto::PanMode::Sphere)
                    )
                })
            },
            Takers::Sphere,
        ),
    ]
};

/// Checks that the options of a geo plot at `path` set only the parameters that its projection's
/// type takes, as `PROJECTION_PARAMETERS` lists them.
fn projection_takes(geo: &proto::Geo, path: &str) -> Result<()> {
    // The sphere's styles style the sphere that `sphere` draws, so they need it.
    let sphere_styles = [
        ("sphere_fill", !geo.sphere_fill.is_empty()),
        ("sphere_stroke", !geo.sphere_stroke.is_empty()),
        ("sphere_stroke_width", geo.sphere_stroke_width.is_some()),
    ];
    if let Some((field, _)) = sphere_styles.iter().find(|(_, set)| *set) {
        if geo.sphere != Some(true) {
            return Err(fail(
                format!("{path}.{field}"),
                "the styles of the sphere need `sphere`, which draws it",
            ));
        }
    }
    let projection = geo.projection.as_ref().expect(REQUIRED);
    // The type of a built-in projection, or `None` for a custom projection.
    let builtin = match projection.r#type.as_ref().expect(REQUIRED) {
        projection::Type::Builtin(builtin) => {
            Some(ProjectionType::try_from(*builtin).unwrap_or(ProjectionType::Unspecified))
        }
        projection::Type::Custom(_) => None,
    };
    for (parameter, sets, takers) in PROJECTION_PARAMETERS {
        if !sets(geo) {
            continue;
        }
        let (takes, which) = match takers {
            Takers::Only(types) => (
                builtin.is_some_and(|builtin| types.contains(&builtin)),
                types
                    .iter()
                    .map(|builtin| format!("`{}`", type_name(*builtin)))
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            Takers::Sphere => (
                builtin != Some(ProjectionType::Identity),
                "a projection of the sphere".to_string(),
            ),
        };
        if !takes {
            let found = match builtin {
                Some(builtin) => format!("`{}`", type_name(builtin)),
                None => "a custom projection".to_string(),
            };
            return Err(fail(
                format!("{path}.{parameter}"),
                format!("`{parameter}` applies only to {which}, not to {found}"),
            ));
        }
    }
    Ok(())
}

/// The name of a projection type, as a string names it: its value in lowercase without the prefix.
fn type_name(builtin: ProjectionType) -> String {
    let name = builtin.as_str_name();
    (name.strip_prefix("PROJECTION_TYPE_").unwrap_or(name)).to_lowercase()
}

/// Checks the size of a track: a length of 0 pixels or more, or a share of a positive weight,
/// and a positive maximum only with a share. An empty track holds the tracks of a nested concat.
fn track_size(track: &proto::Track, path: &str) -> Result<()> {
    match &track.size {
        Some(track::Size::Length(length)) => {
            non_negative(Some(f64::from(*length)), &format!("{path}.length"))?
        }
        Some(track::Size::Share(share)) => {
            positive(Some(f64::from(*share)), &format!("{path}.share"))?
        }
        None => {}
    }
    if track.max.is_some() && !matches!(track.size, Some(track::Size::Share(_))) {
        return Err(fail(
            format!("{path}.max"),
            "a track has a maximum only with a share",
        ));
    }
    positive(track.max.map(f64::from), &format!("{path}.max"))
}

/// Checks the sharing rules of a plot, a concat, or a wrap at `path`: each independent widget's
/// key is not empty and appears once, and a plot, whose layers share one projection, sets no
/// sharing of the projection.
fn resolve(resolve: Option<&proto::Resolve>, path: &str, plot: bool) -> Result<()> {
    let Some(resolve) = resolve else {
        return Ok(());
    };
    let path = format!("{path}.resolve");
    if plot && resolve.projection != 0 {
        return Err(fail(
            format!("{path}.projection"),
            "the layers of a plot share one projection, so its resolve sets no `projection`",
        ));
    }
    for (index, key) in resolve.independent_widgets.iter().enumerate() {
        let at = format!("{path}.independent_widgets[{index}]");
        if key.is_empty() {
            return Err(fail(at, "the key of an independent widget is not empty"));
        }
        if resolve.independent_widgets[..index].contains(key) {
            return Err(fail(
                at,
                format!("the key `{key}` of an independent widget appears once"),
            ));
        }
    }
    Ok(())
}

/// A facet names its column. The constructor of a facet takes no column, because the composition
/// that builds a facet sets it from its key, so the reader checks it here rather than through
/// `required`.
fn facet_column(facet: &proto::Facet, path: &str) -> Result<()> {
    match facet.column.is_empty() {
        true => Err(fail(
            format!("{path}.column"),
            "`column` is required and not empty",
        )),
        false => Ok(()),
    }
}

/// Whether a navigation of `plot` moves a channel whose scale domain, on an encoding of one of its
/// `layers`, reads a selection.
fn navigates_selection(plot: &proto::Plot, kind: CoordKind, layers: &[LayerRef]) -> bool {
    let moved: Vec<Channel> = (plot.tools.iter())
        .filter_map(|tool| match &tool.kind {
            Some(tool::Kind::Navigate(navigate)) => Some(navigate),
            _ => None,
        })
        .flat_map(|navigate| match navigate.channels.is_empty() {
            true => kind.navigated().to_vec(),
            false => (navigate.channels.iter())
                .map(|channel| Channel::try_from(*channel).unwrap_or_default())
                .collect(),
        })
        .collect();
    (layers.iter().filter_map(|layer| layer.mark())).any(|mark| {
        mark.encodings().any(|(channel, encoding)| {
            let domain = (encoding.scale.as_ref()).and_then(|scale| scale.domain.as_ref());
            moved.contains(&channel) && matches!(domain, Some(proto::scale::Domain::Selection(_)))
        })
    })
}

/// Checks the labels of a facet, which always show, and which take an angle and a margin.
fn facet_labels(labels: Option<&proto::Labels>, path: &str) -> Result<()> {
    shown_labels(labels, path, "a facet")?;
    label_settings(labels, path, "a facet", &["angle", "margin"])
}
