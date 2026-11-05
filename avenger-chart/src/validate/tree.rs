//! The rules of the nodes of the tree: plots and their coordinate systems, concats, wraps, and
//! empty cells, with their layouts, titles, attributions, partitions, sharing, and tools.

use arrow::{datatypes::DataType, record_batch::RecordBatch};

use super::{fail, keyed, named, non_negative, positive, size, within, Checker, Result};
use crate::{
    layer::{CoordKind, LayerRef},
    proto::{
        self, geo, layer_tool, node::Node, plot::Coord, select, spherical, spherical_clip, tool,
        tooltip, track, Channel,
    },
};

/// What a node inherits from its ancestors: the columns of their partitions.
#[derive(Clone, Default)]
pub(super) struct Scope {
    pub(super) partition: Vec<String>,
}

impl Scope {
    /// The scope of the children of a node that declares `partition`.
    fn with(&self, partition: &[proto::PartitionValue]) -> Self {
        let mut scope = self.clone();
        scope
            .partition
            .extend(partition.iter().map(|value| value.column.clone()));
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
        self.resolve(&concat.resolve, path)?;
        if let Some(layout) = &concat.layout {
            let path = format!("{path}.layout");
            non_negative(layout.gap.map(f64::from), &format!("{path}.gap"))?;
            for (index, track) in layout.tracks.iter().enumerate() {
                track_size(track, &format!("{path}.tracks[{index}]"))?;
            }
            guide_rules(&layout.guide_rules, &path)?;
        }
        if let Some(facet) = &concat.facet {
            named(
                &facet.column,
                &format!("{path}.facet.column"),
                "the column of a facet",
            )?;
        }
        let scope = scope.with(&concat.partition);
        for (index, child) in concat.children.iter().enumerate() {
            self.node(child, &format!("{path}.children[{index}]"), &scope)?;
        }
        Ok(())
    }

    fn wrap(&self, wrap: &proto::Wrap, path: &str, scope: &Scope) -> Result<()> {
        self.partition(&wrap.partition, path)?;
        self.resolve(&wrap.resolve, path)?;
        if let Some(layout) = &wrap.layout {
            let path = format!("{path}.layout");
            size(layout.min_column_width, &format!("{path}.min_column_width"))?;
            if layout.max_columns == Some(0) {
                return Err(fail(
                    format!("{path}.max_columns"),
                    "a wrap has a column or more",
                ));
            }
            if let Some(row) = &layout.row {
                track_size(row, &format!("{path}.row"))?;
            }
            non_negative(layout.gap.map(f64::from), &format!("{path}.gap"))?;
            guide_rules(&layout.guide_rules, &path)?;
        }
        if let Some(facet) = &wrap.facet {
            named(
                &facet.column,
                &format!("{path}.facet.column"),
                "the column of a facet",
            )?;
        }
        let scope = scope.with(&wrap.partition);
        for (index, child) in wrap.children.iter().enumerate() {
            self.node(child, &format!("{path}.children[{index}]"), &scope)?;
        }
        Ok(())
    }

    fn plot(&self, plot: &proto::Plot, path: &str, scope: &Scope) -> Result<()> {
        self.partition(&plot.partition, path)?;
        self.resolve(&plot.resolve, path)?;
        size(plot.width, &format!("{path}.width"))?;
        size(plot.height, &format!("{path}.height"))?;
        if let Some(attribution) = &plot.attribution {
            let path = format!("{path}.attribution");
            if matches!(
                proto::Side::try_from(attribution.side),
                Ok(proto::Side::Left | proto::Side::Right)
            ) {
                return Err(fail(
                    &path,
                    "an attribution stands at the top or the bottom of its plot",
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
        let kind = CoordKind::of(plot)
            .ok_or_else(|| fail(path, "a plot always sets its coordinate system"))?;
        let coord_path = format!("{path}.{}", coord_name(kind));
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
                    if options.aspect_ratio.is_some()
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
                if let Some(options) = &geo.options {
                    self.geo_options(options, &format!("{coord_path}.options"))?;
                }
            }
        }
        let layers = LayerRef::of(plot);
        if layers.is_empty() {
            return Err(fail(&coord_path, "a plot has a layer or more"));
        }
        let scope = scope.with(&plot.partition);
        let tables: Vec<Option<&RecordBatch>> = (layers.iter())
            .map(|layer| layer.table().map(|table| self.chart.table(table)))
            .collect();
        for (index, layer) in layers.iter().enumerate() {
            let path = format!("{coord_path}.layers[{index}]");
            let table = tables[index].ok_or_else(|| {
                fail(
                    format!("{path}.table"),
                    "a layer always refers to its table",
                )
            })?;
            self.partition(layer.partition(), &path)?;
            let layer_scope = scope.with(layer.partition());
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
                    None => return Err(fail(&path, "a tool always sets its kind")),
                }
            }
            if !layer.tools().is_empty() && layer.id().is_empty() {
                return Err(fail(&path, "a layer that declares a tool has an id"));
            }
            self.layer(*layer, table, &path)?;
        }
        let tables: Vec<&RecordBatch> = tables.into_iter().flatten().collect();
        let mut navigations = 0;
        for (index, tool) in plot.tools.iter().enumerate() {
            let path = format!("{path}.tools[{index}]");
            match tool.kind.as_ref() {
                Some(tool::Kind::Navigate(navigate)) => {
                    navigations += 1;
                    if kind == CoordKind::Polar {
                        return Err(fail(&path, "a polar plot does not take a navigation"));
                    }
                    if navigations > 1 {
                        return Err(fail(&path, "a plot takes at most one navigation"));
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
                None => return Err(fail(&path, "a tool always sets its kind")),
            }
        }
        if !plot.tools.is_empty() && plot.id.is_empty() {
            return Err(fail(path, "a plot that declares a tool has an id"));
        }
        Ok(())
    }

    fn geo_options(&self, geo: &proto::Geo, path: &str) -> Result<()> {
        match geo.surface.as_ref() {
            Some(geo::Surface::Spherical(spherical)) => {
                self.spherical(spherical, &format!("{path}.spherical"))?
            }
            Some(geo::Surface::Planar(_)) => {}
            None => return Err(fail(path, "a map takes spherical or planar coordinates")),
        }
        if let Some(geo::Aspect::AspectRatio(ratio)) = geo.aspect {
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
                        "the zoom limits of a map run from `min_zoom` up to `max_zoom`",
                    ));
                }
            }
        }
        Ok(())
    }

    fn spherical(&self, spherical: &proto::Spherical, path: &str) -> Result<()> {
        let parallel =
            |value: f64, at: &str| within(Some(value), -90.0, 90.0, &format!("{path}.{at}"));
        match spherical.projection.as_ref() {
            Some(
                spherical::Projection::ConicConformal(conic)
                | spherical::Projection::ConicEqualArea(conic)
                | spherical::Projection::ConicEquidistant(conic),
            ) => {
                parallel(conic.parallel1, "parallel1")?;
                parallel(conic.parallel2, "parallel2")?;
            }
            Some(spherical::Projection::CylindricalEqualArea(cylindrical)) => {
                within(
                    cylindrical.parallel,
                    -90.0,
                    90.0,
                    &format!("{path}.parallel"),
                )?;
            }
            Some(spherical::Projection::Hammer(hammer)) => {
                if hammer
                    .coefficient
                    .is_some_and(|value| !(value.is_finite() && value >= 1.0))
                {
                    return Err(fail(
                        format!("{path}.coefficient"),
                        "the coefficient of a Hammer projection is at least 1",
                    ));
                }
            }
            Some(spherical::Projection::Custom(custom)) => {
                named(
                    &custom.name,
                    &format!("{path}.custom.name"),
                    "the name of a custom projection",
                )?;
                keyed(
                    &custom.parameters,
                    |parameter| parameter.name.clone(),
                    |a, b| a == b,
                    &format!("{path}.custom.parameters"),
                    "parameters",
                )?;
            }
            Some(_) => {}
            None => return Err(fail(path, "a spherical map always sets its projection")),
        }
        if let Some(rotation) = &spherical.rotate {
            if ![rotation.lambda, rotation.phi, rotation.gamma]
                .iter()
                .all(|angle| angle.is_finite())
            {
                return Err(fail(
                    format!("{path}.rotate"),
                    "the angles of a rotation are finite",
                ));
            }
        }
        if let Some(center) = &spherical.center {
            lon_lat(center.longitude, center.latitude, &format!("{path}.center"))?;
        }
        if let Some(spherical_clip::Clip::CircleAngle(angle)) =
            spherical.clip.as_ref().and_then(|clip| clip.clip.as_ref())
        {
            if !(angle.is_finite() && *angle > 0.0 && *angle <= 180.0) {
                return Err(fail(
                    format!("{path}.clip.circle_angle"),
                    "the angle of a circle clip is more than 0 and at most 180 degrees",
                ));
            }
        }
        non_negative(spherical.precision, &format!("{path}.precision"))?;
        if let Some(spherical::Fit::FitBox(fit)) = &spherical.fit {
            lon_lat_box(fit, &format!("{path}.fit_box"))?;
        }
        if let Some(graticule) = &spherical.graticule {
            let path = format!("{path}.graticule");
            for (value, at) in [
                (graticule.longitude_step, "longitude_step"),
                (graticule.latitude_step, "latitude_step"),
                (graticule.major_longitude_step, "major_longitude_step"),
                (graticule.major_latitude_step, "major_latitude_step"),
                (graticule.precision, "precision"),
            ] {
                positive(value, &format!("{path}.{at}"))?;
            }
            for (extent, at) in [
                (&graticule.extent, "extent"),
                (&graticule.major_extent, "major_extent"),
            ] {
                if let Some(extent) = extent {
                    lon_lat_box(extent, &format!("{path}.{at}"))?;
                }
            }
            if let Some(sides) = &graticule.label_sides {
                if sides.sides.contains(&(proto::Side::Unspecified as i32)) {
                    return Err(fail(
                        format!("{path}.label_sides"),
                        "a label side is a side",
                    ));
                }
            }
            self.pattern(
                &graticule.format,
                &DataType::Float64,
                &format!("{path}.format"),
            )?;
            non_negative(
                graticule.label_padding.map(f64::from),
                &format!("{path}.label_padding"),
            )?;
            non_negative(graticule.width.map(f64::from), &format!("{path}.width"))?;
        }
        if let Some(tiles) = &spherical.tiles {
            let path = format!("{path}.tiles");
            named(
                &tiles.url_template,
                &format!("{path}.url_template"),
                "the URL template of tiles",
            )?;
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

    /// Checks partition values, keyed by column.
    pub(super) fn partition(&self, partition: &[proto::PartitionValue], path: &str) -> Result<()> {
        let path = format!("{path}.partition");
        let mut keys = Vec::with_capacity(partition.len());
        for (index, value) in partition.iter().enumerate() {
            let path = format!("{path}[{index}]");
            named(
                &value.column,
                &format!("{path}.column"),
                "the column of a partition value",
            )?;
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

    /// Checks resolve rules, keyed by channel.
    fn resolve(&self, resolve: &[proto::Resolve], path: &str) -> Result<()> {
        let path = format!("{path}.resolve");
        for (index, rule) in resolve.iter().enumerate() {
            let channel = Channel::try_from(rule.channel).unwrap_or_default();
            if !super::encoding::takes_scale(channel) {
                return Err(fail(
                    format!("{path}[{index}].channel"),
                    format!(
                        "`{}` takes no scale, so it has no resolve rule",
                        channel.field_name()
                    ),
                ));
            }
        }
        keyed(
            resolve,
            |rule| rule.channel,
            |a, b| a.sharing == b.sharing,
            &path,
            "resolve rules",
        )
    }

    fn tooltip(
        &self,
        tooltip: &proto::Tooltip,
        tables: &[&RecordBatch],
        scope: &Scope,
        path: &str,
    ) -> Result<()> {
        if let Some(tooltip::Find::Nearest(nearest)) = &tooltip.find {
            non_negative(
                Some(f64::from(nearest.max_radius)),
                &format!("{path}.nearest.max_radius"),
            )?;
        }
        let fields = tooltip
            .header
            .iter()
            .map(|field| ("header".to_string(), field));
        let fields = fields.chain(
            (tooltip.fields.iter().enumerate())
                .map(|(index, field)| (format!("fields[{index}]"), field)),
        );
        for (at, field) in fields {
            let path = format!("{path}.{at}");
            let data_type = column_type(&field.column, tables, scope, &format!("{path}.column"))?;
            if let Some(data_type) = data_type {
                self.pattern(&field.format, &data_type, &format!("{path}.format"))?;
            }
        }
        Ok(())
    }

    fn select(
        &self,
        select: &proto::Select,
        kind: CoordKind,
        tables: &[&RecordBatch],
        scope: &Scope,
        path: &str,
    ) -> Result<()> {
        self.selection(&select.selection, &format!("{path}.selection"))?;
        for (index, column) in select.scope.iter().enumerate() {
            if !scope.partition.contains(column) {
                return Err(fail(
                    format!("{path}.scope[{index}]"),
                    format!("`{column}` is not a partition column of the plot or its ancestors"),
                ));
            }
        }
        match select.form.as_ref() {
            Some(select::Form::Interval(interval)) => {
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
            Some(select::Form::Points(points)) => {
                if points.keys.is_empty() {
                    return Err(fail(
                        format!("{path}.points.keys"),
                        "a point selection has a key column or more",
                    ));
                }
                for (index, key) in points.keys.iter().enumerate() {
                    column_type(key, tables, scope, &format!("{path}.points.keys[{index}]"))?;
                }
            }
            Some(select::Form::Lasso(lasso)) => {
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

/// The name of the coordinate system's member of `Plot.coord`.
fn coord_name(kind: CoordKind) -> &'static str {
    match kind {
        CoordKind::Cartesian => "cartesian",
        CoordKind::Polar => "polar",
        CoordKind::Geo => "geo",
    }
}

fn polar_options(polar: &proto::Polar, path: &str) -> Result<()> {
    let start = polar.start_angle.unwrap_or(0.0);
    let end = polar.end_angle.unwrap_or(start + 360.0);
    if !(start.is_finite() && end.is_finite() && end > start && end - start <= 360.0) {
        return Err(fail(
            path,
            "the end angle of a polar plot is more than its start angle and at most 360 degrees \
             past it",
        ));
    }
    if !(0.0..1.0).contains(&polar.inner_radius.unwrap_or(0.0)) {
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

fn lon_lat_box(lon_lat_box: &proto::LonLatBox, path: &str) -> Result<()> {
    lon_lat(lon_lat_box.west, lon_lat_box.south, path)?;
    lon_lat(lon_lat_box.east, lon_lat_box.north, path)?;
    if lon_lat_box.south > lon_lat_box.north {
        return Err(fail(
            path,
            "the south edge of a box is at most its north edge",
        ));
    }
    Ok(())
}

/// Checks the size of a track: a length of 0 pixels or more, or a share of a positive weight with
/// a positive maximum. An empty track holds the tracks of a nested concat.
fn track_size(track: &proto::Track, path: &str) -> Result<()> {
    match &track.size {
        Some(track::Size::Length(length)) => {
            non_negative(Some(f64::from(*length)), &format!("{path}.length"))
        }
        Some(track::Size::Share(share)) => {
            positive(share.weight.map(f64::from), &format!("{path}.share.weight"))?;
            positive(share.max.map(f64::from), &format!("{path}.share.max"))
        }
        None => Ok(()),
    }
}

/// Checks guide rules, keyed by part.
fn guide_rules(rules: &[proto::GuideRule], path: &str) -> Result<()> {
    keyed(
        rules,
        |rule| rule.part,
        |a, b| a.rule == b.rule,
        &format!("{path}.guide_rules"),
        "guide rules",
    )
}
