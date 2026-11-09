//! The rules of the nodes of the tree: plots and their coordinate systems, concats, wraps, and
//! empty cells, with their layouts, titles, attributions, partitions, sharing, and tools.

use arrow::{datatypes::DataType, record_batch::RecordBatch};

use super::{
    classes, fail, keyed, named, non_negative, positive, size, title, within, Checker, Result,
    REQUIRED,
};
use crate::{
    layer::{CoordKind, LayerRef},
    proto::{
        self, geo, layer_tool, node::Node, plot::Coord, select, spherical, spherical_clip, tool,
        tooltip, track, Channel,
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
        self.resolve(&concat.resolve, path)?;
        if let Some(layout) = &concat.layout {
            let path = format!("{path}.layout");
            non_negative(layout.gap.map(f64::from), &format!("{path}.gap"))?;
            if !layout.tracks.is_empty() && layout.tracks.len() != concat.children.len() {
                return Err(fail(
                    format!("{path}.tracks"),
                    format!(
                        "a concat has one track for each child or no tracks, and it has {} tracks for {} children",
                        layout.tracks.len(),
                        concat.children.len()
                    ),
                ));
            }
            for (index, track) in layout.tracks.iter().enumerate() {
                track_size(track, &format!("{path}.tracks[{index}]"))?;
            }
            part_rules(&layout.part_rules, &path)?;
        }
        if let Some(facet) = &concat.facet {
            let path = format!("{path}.facet");
            classes(&facet.classes, &path)?;
            title(facet.title.as_ref(), &format!("{path}.title"), false)?;
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
            part_rules(&layout.part_rules, &path)?;
        }
        if let Some(facet) = &wrap.facet {
            let path = format!("{path}.facet");
            classes(&facet.classes, &path)?;
            title(facet.title.as_ref(), &format!("{path}.title"), false)?;
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
                self.tool_modes(&tool.modes, &format!("{path}.modes"))?;
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
            if !layer.tools().is_empty() && layer.id().is_empty() {
                return Err(fail(&path, "a layer that declares a tool has an id"));
            }
            self.widgets(layer.widgets(), table, &layer_scope, &path)?;
            classes(layer.classes(), &path)?;
            self.layer(*layer, table, &path)?;
        }
        let tables: Vec<&RecordBatch> = tables.into_iter().flatten().collect();
        let mut navigations = 0;
        for (index, tool) in plot.tools.iter().enumerate() {
            let path = format!("{path}.tools[{index}]");
            self.tool_modes(&tool.modes, &format!("{path}.modes"))?;
            match tool.kind.as_ref() {
                Some(tool::Kind::Navigate(navigate)) => {
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
            None => unreachable!("{REQUIRED}"),
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
                        "the zoom limits of a geo plot run from `min_zoom` up to `max_zoom`",
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
                keyed(
                    &custom.parameters,
                    |parameter| parameter.name.clone(),
                    |a, b| a == b,
                    &format!("{path}.custom.parameters"),
                    "parameters",
                )?;
            }
            Some(_) => {}
            None => unreachable!("{REQUIRED}"),
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

    /// Checks resolve rules, keyed by what they share: a channel that takes a scale, or a widget's
    /// key.
    fn resolve(&self, resolve: &[proto::Resolve], path: &str) -> Result<()> {
        let path = format!("{path}.resolve");
        let mut keys = Vec::with_capacity(resolve.len());
        for (index, rule) in resolve.iter().enumerate() {
            let at = format!("{path}[{index}]");
            match rule.key.as_ref() {
                Some(proto::resolve::Key::Channel(channel)) => {
                    let channel = Channel::try_from(*channel).unwrap_or_default();
                    if !super::encoding::takes_scale(channel) {
                        return Err(fail(
                            format!("{at}.channel"),
                            format!(
                                "`{}` never has a scale, so it has no resolve rule",
                                channel.field_name()
                            ),
                        ));
                    }
                }
                Some(proto::resolve::Key::Widget(_)) => {}
                None => unreachable!("{REQUIRED}"),
            }
            keys.push((rule.key.clone(), rule.sharing));
        }
        keyed(
            &keys,
            |(key, _)| key.clone(),
            |a, b| a.1 == b.1,
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
        if let Some(field) = &tooltip.title {
            if field.title.is_some() {
                return Err(fail(
                    format!("{path}.title.title"),
                    "the title of a tooltip shows only its value, so its field has no title",
                ));
            }
        }
        let fields = tooltip
            .title
            .iter()
            .map(|field| ("title".to_string(), field));
        let fields = fields.chain(
            (tooltip.fields.iter().enumerate())
                .map(|(index, field)| (format!("fields[{index}]"), field)),
        );
        for (at, field) in fields {
            let path = format!("{path}.{at}");
            let data_type = column_type(&field.column, tables, scope, &format!("{path}.column"))?;
            let Some(labels) = &field.labels else {
                continue;
            };
            if labels.style.is_some() {
                return Err(fail(
                    format!("{path}.labels.style"),
                    "the labels of a tooltip field have no style, since a tooltip's text takes the tooltip's style",
                ));
            }
            if let Some(data_type) = data_type {
                self.pattern(&labels.format, &data_type, &format!("{path}.labels.format"))?;
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
                if !points.sources.is_empty() && points.keys.len() != 1 {
                    return Err(fail(
                        format!("{path}.points.sources"),
                        "a point selection with the source of a lump has the lump as its one key",
                    ));
                }
                // The source of a lump is a column of the query's input, which the chart's tables
                // need not hold, so only its key is checked.
                for (index, source) in points.sources.iter().enumerate() {
                    let path = format!("{path}.points.sources[{index}]");
                    named(
                        &source.source,
                        &format!("{path}.source"),
                        "the source of a lump",
                    )?;
                    if !points.keys.contains(&source.key) {
                        return Err(fail(
                            format!("{path}.key"),
                            format!("`{}` is a key of the point selection", source.key),
                        ));
                    }
                    let data_type =
                        column_type(&source.key, tables, scope, &format!("{path}.key"))?;
                    if !data_type.as_ref().is_some_and(crate::lump::is_lump) {
                        return Err(fail(
                            format!("{path}.key"),
                            format!("`{}` is a column of lumps", source.key),
                        ));
                    }
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

/// Checks part rules, keyed by part.
fn part_rules(rules: &[proto::PartRule], path: &str) -> Result<()> {
    keyed(
        rules,
        |rule| rule.part,
        |a, b| a.rule == b.rule,
        &format!("{path}.part_rules"),
        "part rules",
    )
}
