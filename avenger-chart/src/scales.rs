use crate::dataflow::{
    datafusion::{
        arrow::{
            array::{new_empty_array, Array, ArrayRef, Float64Array},
            compute::concat,
            datatypes::DataType,
        },
        common::ScalarValue,
    },
    TableSnapshot,
};
use crate::{definition::*, error, evaluate::PlotInstance, Result};
use avenger_geometry::marks::MarkGeometryUtils;
use avenger_guides::axis::{
    band::make_band_axis_marks_with_text_engine,
    numeric::make_numeric_axis_marks_with_text_engine,
    opts::{AxisConfig, AxisOrientation},
    point::make_point_axis_marks_with_text_engine,
};
use avenger_layout::LayoutSolution;
use avenger_panels::*;
use avenger_scales::scales::{
    band::BandScale, linear::LinearScale, point::PointScale, ConfiguredScale,
};
use avenger_scenegraph::marks::group::SceneGroup;
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

pub(crate) fn column(table: &TableSnapshot, name: &str) -> Result<ArrayRef> {
    let index = table.schema().index_of(name).map_err(error)?;
    let arrays: Vec<_> = table
        .batches()
        .iter()
        .map(|b| b.column(index).as_ref())
        .collect();
    if arrays.is_empty() {
        Ok(new_empty_array(table.schema().field(index).data_type()))
    } else if arrays.len() == 1 {
        Ok(table.batches()[0].column(index).clone())
    } else {
        concat(&arrays).map_err(error)
    }
}
pub(crate) fn number(v: &ScalarValue) -> Result<f64> {
    match v.cast_to(&DataType::Float64).map_err(error)? {
        ScalarValue::Float64(Some(v)) if v.is_finite() => Ok(v),
        _ => Err(error("expected a finite non-null number")),
    }
}
fn values(p: &PlotInstance, s: &Scale) -> Result<Vec<ScalarValue>> {
    match &s.domain {
        Domain::Values(v) => Ok(v.clone()),
        Domain::Bounds(a, b) => Ok(vec![p.ctx.scalar(a)?.clone(), p.ctx.scalar(b)?.clone()]),
        Domain::Extent(h) => {
            let ScalarValue::Struct(v) = p.ctx.scalar(h)? else {
                return Err(error("extent is not a struct"));
            };
            ["min", "max"]
                .iter()
                .map(|name| {
                    ScalarValue::try_from_array(
                        v.column_by_name(name)
                            .ok_or_else(|| error("extent requires min and max"))?,
                        0,
                    )
                    .map_err(error)
                })
                .collect()
        }
        Domain::Column(h, c) => {
            let a = column(p.ctx.table(h)?, c)?;
            let mut seen = HashSet::new();
            let mut values = vec![];
            for i in 0..a.len() {
                let v = ScalarValue::try_from_array(&a, i).map_err(error)?;
                if (!v.is_null() || s.include_null) && seen.insert(v.clone()) {
                    values.push(v)
                }
            }
            Ok(values)
        }
    }
}
fn configure_scale(s: &Scale, values: &[ScalarValue], size: Size) -> Result<ConfiguredScale> {
    let range = match s.range {
        Range::PlotWidth => (0.0, size.width),
        Range::PlotHeight => (0.0, size.height),
        Range::PlotHeightReversed => (size.height, 0.0),
        Range::Fixed(a, b) => (a, b),
    };
    match s.kind {
        ScaleKind::Linear => {
            let numbers = values
                .iter()
                .filter(|v| !v.is_null())
                .map(number)
                .collect::<Result<Vec<_>>>()?;
            let explicit = matches!(s.domain, Domain::Bounds(..) | Domain::Values(_));
            if explicit && (numbers.len() != values.len() || numbers.len() < 2) {
                return Err(error("explicit domains require finite, non-null endpoints"));
            }
            let (mut lo, mut hi) = if explicit && s.sharing.is_none() {
                if numbers[0] == numbers[1] {
                    return Err(error("explicit domain endpoints must differ"));
                }
                (numbers[0], numbers[1])
            } else if numbers.is_empty() {
                (s.empty_domain[0], s.empty_domain[1])
            } else {
                (
                    numbers.iter().copied().fold(f64::INFINITY, f64::min),
                    numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                )
            };
            if s.zero {
                if lo < hi {
                    lo = lo.min(0.0);
                    hi = hi.max(0.0);
                } else {
                    lo = lo.max(0.0);
                    hi = hi.min(0.0);
                }
            }
            if lo == hi {
                let d = lo.abs().max(1.0) * 0.05;
                lo -= d;
                hi += d;
            }
            let configured = LinearScale::configured((lo as f32, hi as f32), range)
                .with_domain(Arc::new(Float64Array::from(vec![lo, hi])))
                .with_option("clamp", s.clamp)
                .with_option("clip_padding_lower", s.pixel_padding)
                .with_option("clip_padding_upper", s.pixel_padding)
                .with_option("nice", s.nice);
            let domain = configured.normalized_domain().map_err(error)?;
            let configured = configured
                .with_domain(domain)
                .with_option("nice", false)
                .with_option("clip_padding_lower", 0.0_f32)
                .with_option("clip_padding_upper", 0.0_f32);
            let (lo, hi) = configured.numeric_interval_domain_f64().map_err(error)?;
            if !lo.is_finite() || !hi.is_finite() || lo == hi {
                return Err(error("domain cannot be represented by the scale"));
            }
            Ok(configured)
        }
        ScaleKind::Band | ScaleKind::Point => {
            let mut seen = HashSet::new();
            let ordered: Vec<_> = values
                .iter()
                .filter(|v| (!v.is_null() || s.include_null) && seen.insert((*v).clone()))
                .cloned()
                .collect();
            let domain = if ordered.is_empty() {
                new_empty_array(&DataType::Utf8)
            } else {
                ScalarValue::iter_to_array(ordered).map_err(error)?
            };
            let scale = if s.kind == ScaleKind::Point {
                PointScale::configured(domain, range).with_option("padding", s.padding_outer)
            } else {
                BandScale::configured(domain, range)
                    .with_option("padding_inner", s.padding_inner)
                    .with_option("padding_outer", s.padding_outer)
            };
            Ok(scale.with_option("include_null", s.include_null))
        }
    }
}
pub(crate) fn configure(
    plots: &mut [PlotInstance],
    tree: &PanelTree,
    formatting: &avenger_scales::formatter::ScaleFormatting,
) -> Result<()> {
    let mut domains = plots
        .iter()
        .map(|p| {
            p.plot
                .scales
                .iter()
                .map(|(_, s)| values(p, s))
                .collect::<Result<Vec<_>>>()
        })
        .collect::<Result<Vec<_>>>()?;
    let mut families: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for (pi, p) in plots.iter().enumerate() {
        for (si, (_, s)) in p.plot.scales.iter().enumerate() {
            if let Some((name, scope)) = &s.sharing {
                families
                    .entry(format!("{name}:{scope:?}"))
                    .or_default()
                    .push((pi, si));
            }
        }
    }
    for members in families.values() {
        let (first_pi, first_si) = members[0];
        let first = &plots[first_pi].plot.scales[first_si].1;
        let scope = first.sharing.as_ref().unwrap().1.clone();
        let mut participants = HashSet::new();
        for (pi, si) in members {
            let s = &plots[*pi].plot.scales[*si].1;
            if !participants.insert(plots[*pi].id.clone())
                || s.kind != first.kind
                || s.zero != first.zero
                || s.nice != first.nice
                || s.empty_domain != first.empty_domain
                || s.include_null != first.include_null
                || s.pixel_padding != first.pixel_padding
            {
                return Err(error("incompatible shared scales"));
            }
        }
        for group in tree
            .group(members.iter().map(|(pi, _)| plots[*pi].id.clone()), scope)
            .map_err(error)?
            .iter()
        {
            let active: Vec<_> = members
                .iter()
                .filter(|(pi, _)| group.members().contains(&plots[*pi].id))
                .copied()
                .collect();
            let union: Vec<_> = active
                .iter()
                .flat_map(|(pi, si)| domains[*pi][*si].clone())
                .collect();
            for (pi, si) in active {
                domains[pi][si] = union.clone();
            }
        }
    }
    for (pi, p) in plots.iter_mut().enumerate() {
        let dimension = |d: &StepDimension| -> Result<f32> {
            let (si, (_, scale)) = p
                .plot
                .scales
                .iter()
                .enumerate()
                .find(|(_, (name, _))| name == d.scale.name())
                .unwrap();
            let mut unique = HashSet::new();
            let count = domains[pi][si]
                .iter()
                .filter(|v| (scale.include_null || !v.is_null()) && unique.insert((*v).clone()))
                .count();
            let inner = if scale.kind == ScaleKind::Point {
                1.0
            } else {
                scale.padding_inner
            };
            let size = (count as f32 - inner + 2.0 * scale.padding_outer).max(1.0) * d.step;
            if !size.is_finite() {
                return Err(error("step dimension exceeds finite scene coordinates"));
            }
            Ok(size)
        };
        let width = p.plot.width_step.as_ref().map(&dimension).transpose()?;
        let height = p.plot.height_step.as_ref().map(&dimension).transpose()?;
        if let Some(width) = width {
            p.plot.size.width = width;
        }
        if let Some(height) = height {
            p.plot.size.height = height;
        }
        for (si, (name, s)) in p.plot.scales.iter().enumerate() {
            let mut scale = configure_scale(s, &domains[pi][si], p.plot.size)?;
            scale.config.context.formatting = formatting.clone();
            if let Some(number) = &formatting.number {
                let context = if s.kind == ScaleKind::Linear {
                    avenger_scales::formatter::NumberLabelContext::Continuous
                } else {
                    avenger_scales::formatter::NumberLabelContext::Categorical
                };
                scale.config.context.formatters.number =
                    Some(number.prepare(None, context).map_err(error)?);
            }
            if let Some(datetime) = &formatting.datetime {
                match scale.domain().data_type() {
                    DataType::Date32 | DataType::Date64 | DataType::Timestamp(_, None) => {
                        scale.config.context.formatters.civil_datetime =
                            Some(datetime.prepare_naive(None).map_err(error)?);
                    }
                    DataType::Timestamp(_, Some(_)) => {
                        scale.config.context.formatters.instant =
                            Some(datetime.prepare_zoned(None).map_err(error)?);
                    }
                    _ => {}
                }
            }
            p.scales.insert(name.clone(), scale);
        }
    }
    Ok(())
}
fn guide_key(p: &PlotInstance, index: usize) -> GuideKey {
    format!("{:?}:{index}", p.path).into()
}
fn title_key(p: &PlotInstance, index: usize) -> GuideKey {
    format!("{:?}:{index}:title", p.path).into()
}
fn equivalent(p: &PlotInstance, a: &Axis) -> Result<EquivalenceKey> {
    let s = &p.scales[a.scale.name()];
    let band = p
        .plot
        .scales
        .iter()
        .any(|(name, scale)| name == a.scale.name() && scale.kind != ScaleKind::Linear);
    let ticks = if band {
        s.domain().clone()
    } else {
        s.ticks(Some(a.tick_count)).map_err(error)?
    };
    let labels = s.format(&ticks).map_err(error)?;
    let positions = if ticks.is_empty() {
        avenger_common::value::ScalarOrArray::new_array(Vec::<f32>::new())
    } else {
        (if band {
            s.clone().with_option("band", 0.5)
        } else {
            s.clone()
        })
        .scale_to_numeric(&ticks)
        .map_err(error)?
    };
    use prost::Message;
    let mut bytes = Vec::new();
    for array in [s.domain(), s.range(), &ticks] {
        bytes.extend_from_slice(&(array.len() as u64).to_le_bytes());
        for index in 0..array.len() {
            let value = ScalarValue::try_from_array(array, index).map_err(error)?;
            let wire: crate::dataflow::protobuf::common::ScalarValue =
                (&value).try_into().map_err(error)?;
            wire.encode_length_delimited(&mut bytes).map_err(error)?;
        }
    }
    let options: BTreeMap<_, _> = s.config.options.iter().collect();
    Ok(format!(
        "{}:{:?}:{:?}:{:?}:{:?}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>(),
        labels.value(),
        positions.value(),
        a.format,
        options
    )
    .into())
}
pub(crate) fn guides(
    plots: &[PlotInstance],
    tree: &PanelTree,
    arranged: &PanelArrangement,
    solution: &LayoutSolution<NodeId>,
) -> Result<GuidePlan> {
    let frames = PanelFrames::from_layout(arranged, solution).map_err(error)?;
    let mut families: BTreeMap<GuideKey, Vec<(&PlotInstance, usize)>> = BTreeMap::new();
    for p in plots {
        for i in 0..p.plot.axes.len() {
            families.entry(guide_key(p, i)).or_default().push((p, i));
        }
    }
    let mut requests = vec![];
    for (key, members) in families {
        let (p, i) = members[0];
        let a = &p.plot.axes[i];
        let contributions = members
            .iter()
            .map(|(p, i)| {
                Ok(GuideContribution::new(p.id.clone())
                    .equivalent(equivalent(p, &p.plot.axes[*i])?))
            })
            .collect::<Result<Vec<_>>>()?;
        requests.push(
            AxisLabels::new(key, a.side, a.sharing.clone(), contributions)
                .visibility(a.labels)
                .into(),
        );
        if a.shared_title {
            requests.push(
                SharedGuide::new(
                    title_key(p, i),
                    SharedGuideKind::AxisTitle,
                    a.side,
                    a.sharing.clone(),
                    members.iter().map(|(p, i)| {
                        GuideContribution::new(p.id.clone())
                            .equivalent(p.plot.axes[*i].title.clone().into())
                    }),
                )
                .into(),
            );
        }
    }
    tree.plan_guides(&frames, requests, GuideOptions::default())
        .map_err(error)
}
pub(crate) fn axes(
    p: &PlotInstance,
    text: &crate::TextEngine,
    plan: Option<&GuidePlan>,
) -> Result<Vec<SceneGroup>> {
    p.plot
        .axes
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let labels = plan.map_or(a.labels != LabelVisibility::None, |g| {
                g.decision(&guide_key(p, i), &p.id)
                    .and_then(|d| d.instance())
                    .and_then(|id| g.instance(id))
                    .is_some_and(|instance| instance.source_panel() == &p.id)
            });
            let title = !a.shared_title;
            let config = AxisConfig {
                dimensions: [p.plot.size.width, p.plot.size.height],
                orientation: match a.side {
                    Side::Top => AxisOrientation::Top,
                    Side::Bottom => AxisOrientation::Bottom,
                    Side::Left => AxisOrientation::Left,
                    Side::Right => AxisOrientation::Right,
                },
                grid: false,
                label_angle: a.label_angle,
                format_number: a.format.clone(),
                labels_visible: Some(labels),
                title_visible: Some(title),
                tick_count: Some(a.tick_count),
                ..Default::default()
            };
            let s = &p.scales[a.scale.name()];
            if s.config.domain.is_empty() {
                return Ok(SceneGroup::default());
            }
            let kind = p
                .plot
                .scales
                .iter()
                .find(|(n, _)| n == a.scale.name())
                .unwrap()
                .1
                .kind;
            match kind {
                ScaleKind::Linear => {
                    make_numeric_axis_marks_with_text_engine(s, &a.title, [0.0, 0.0], &config, text)
                }
                ScaleKind::Point => make_point_axis_marks_with_text_engine(
                    s.clone(),
                    &a.title,
                    [0.0, 0.0],
                    &config,
                    text,
                ),
                ScaleKind::Band => {
                    make_band_axis_marks_with_text_engine(s, &a.title, [0.0, 0.0], &config, text)
                }
            }
            .map(crate::marks::multiline_group)
            .map_err(error)
        })
        .collect()
}
pub(crate) fn measure(
    plots: &[PlotInstance],
    text: &crate::TextEngine,
    plan: Option<&GuidePlan>,
) -> Result<Vec<Edges<f32>>> {
    plots
        .iter()
        .map(|p| {
            if let Some(edges) = p.plot.guide_reservations {
                return Ok(edges);
            }
            let mut e = Edges::<f32>::default();
            for g in axes(p, text, plan)? {
                let b = g.bounding_box_with_text_engine(text);
                let lo = b.lower();
                let hi = b.upper();
                e = e.max(Edges {
                    left: (-lo[0]).max(0.0),
                    top: (-lo[1]).max(0.0),
                    right: (hi[0] - p.plot.size.width).max(0.0),
                    bottom: (hi[1] - p.plot.size.height).max(0.0),
                });
            }
            Ok(Edges {
                top: e.top.ceil() + 4.0,
                right: e.right.ceil() + 4.0,
                bottom: e.bottom.ceil() + 4.0,
                left: e.left.ceil() + 4.0,
            })
        })
        .collect()
}
pub(crate) fn shared_titles(
    plots: &[PlotInstance],
    plan: &GuidePlan,
    solution: &LayoutSolution<NodeId>,
) -> Vec<avenger_scenegraph::marks::mark::SceneMark> {
    use avenger_common::value::ScalarOrArray as V;
    use avenger_scenegraph::marks::{mark::SceneMark, text::SceneTextMark};
    plan.instances()
        .filter(|g| g.kind() == GuideKind::AxisTitle)
        .filter_map(|g| {
            let p = plots.iter().find(|p| p.id == *g.source_panel())?;
            let (_, a) = p
                .plot
                .axes
                .iter()
                .enumerate()
                .find(|(i, _)| title_key(p, *i) == *g.key())?;
            let r = solution.region(g.anchor())?;
            let siblings: Vec<_> = plan
                .instances()
                .filter(|other| {
                    other.kind() == GuideKind::AxisTitle
                        && other.anchor() == g.anchor()
                        && other.side() == g.side()
                })
                .collect();
            let ordinal = siblings
                .iter()
                .position(|other| other.id() == g.id())
                .unwrap();
            let slabs: Vec<_> = r
                .slabs
                .iter()
                .filter(|s| s.layer == avenger_layout::ChromeLayer::Strip && s.side == a.side)
                .collect();
            let slab = slabs[slabs.len() - siblings.len() + ordinal].rect;
            let (x, y, angle) = match a.side {
                Side::Bottom | Side::Top => (slab.x + slab.width / 2.0, slab.y + 16.0, 0.0),
                Side::Left => (slab.x + 14.0, slab.y + slab.height / 2.0, -90.0),
                Side::Right => (slab.x + 10.0, slab.y + slab.height / 2.0, 90.0),
            };
            Some(SceneMark::Text(
                SceneTextMark {
                    text: V::new_scalar(a.title.clone()),
                    x: V::new_scalar(x),
                    y: V::new_scalar(y),
                    angle: V::new_scalar(angle),
                    align: V::new_scalar(avenger_text::types::TextAlign::Center),
                    font_size: V::new_scalar(13.0),
                    interactive: false,
                    ..Default::default()
                }
                .into(),
            ))
        })
        .collect()
}

pub(crate) fn reserve_titles(
    mut layout: avenger_layout::Layout<NodeId, String>,
    id: &NodeId,
    plan: Option<&GuidePlan>,
) -> avenger_layout::Layout<NodeId, String> {
    if let Some(plan) = plan {
        for guide in plan
            .instances()
            .filter(|g| g.kind() == GuideKind::AxisTitle && g.anchor() == id)
        {
            layout = layout.strip(guide.side(), 24.0);
        }
    }
    layout
}

pub(crate) fn grids(p: &PlotInstance) -> Result<Vec<SceneGroup>> {
    p.plot
        .axes
        .iter()
        .filter(|a| a.grid)
        .map(|a| {
            let orientation = match a.side {
                Side::Top => AxisOrientation::Top,
                Side::Bottom => AxisOrientation::Bottom,
                Side::Left => AxisOrientation::Left,
                Side::Right => AxisOrientation::Right,
            };
            let dimensions = [p.plot.size.width, p.plot.size.height];
            let scale = &p.scales[a.scale.name()];
            if scale.config.domain.is_empty() {
                return Ok(SceneGroup::default());
            }
            let kind = p
                .plot
                .scales
                .iter()
                .find(|(n, _)| n == a.scale.name())
                .unwrap()
                .1
                .kind;
            if kind == ScaleKind::Linear {
                avenger_guides::axis::numeric::make_tick_grid_marks(
                    &scale.ticks(Some(a.tick_count)).map_err(error)?,
                    scale,
                    &orientation,
                    &dimensions,
                    None,
                    None,
                )
                .map_err(error)
            } else {
                avenger_guides::axis::band::make_tick_grid_marks(
                    &scale.clone().with_option("band", 0.5),
                    &orientation,
                    &dimensions,
                    None,
                    None,
                )
                .map_err(error)
            }
        })
        .collect()
}
