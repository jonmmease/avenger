use crate::dataflow::{
    datafusion::{
        arrow::{
            array::{Array, ArrayRef, Float32Array},
            compute::cast,
            datatypes::DataType,
        },
        common::ScalarValue,
    },
    SnapshotId, TableSnapshot,
};
use crate::{
    definition::*,
    error,
    evaluate::PlotInstance,
    scales::{column, number},
    Chart, GeometryReport, Result,
};
use avenger_color::ColorOrGradient;
use avenger_common::{
    types::{LinearScaleAdjustment, SymbolShape},
    value::ScalarOrArray,
};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::{mark::SceneMark, rect::SceneRectMark, symbol::SceneSymbolMark};
use std::{collections::HashSet, sync::Arc};

pub(crate) struct Positions {
    source: SnapshotId,
    fields: [String; 2],
    base: [ConfiguredScale; 2],
    xy: [ScalarOrArray<f32>; 2],
}
pub(crate) fn color(value: &str) -> Result<ScalarOrArray<ColorOrGradient>> {
    let c = value.parse::<css_color_parser::Color>().map_err(error)?;
    Ok(ScalarOrArray::new_scalar(ColorOrGradient::Color([
        c.r as f32 / 255.0,
        c.g as f32 / 255.0,
        c.b as f32 / 255.0,
        c.a,
    ])))
}
fn numeric(array: &ArrayRef) -> Result<Vec<f32>> {
    let a = cast(array, &DataType::Float32).map_err(error)?;
    let a = a.as_any().downcast_ref::<Float32Array>().unwrap();
    a.iter()
        .map(|v| match v {
            Some(v) if v.is_finite() => Ok(v),
            _ => Err(error(
                "mark coordinates must be finite and non-null; filter invalid rows in the dataflow",
            )),
        })
        .collect()
}
fn input(v: &Value, p: &PlotInstance, t: &TableSnapshot) -> Result<ArrayRef> {
    match v {
        Value::Field(name) => column(t, name),
        Value::Scalar(h) => p
            .ctx
            .scalar(h)?
            .to_array_of_size(t.num_rows())
            .map_err(error),
        Value::Constant(v) => ScalarValue::Float64(Some(*v))
            .to_array_of_size(t.num_rows())
            .map_err(error),
        _ => Err(error("nested scale applications are not supported")),
    }
}
fn channel(v: &Value, p: &PlotInstance, t: &TableSnapshot) -> Result<ScalarOrArray<f32>> {
    Ok(match v {
        Value::Constant(v) => ScalarOrArray::new_scalar(*v as f32),
        Value::Scalar(h) => ScalarOrArray::new_scalar(number(p.ctx.scalar(h)?)? as f32),
        Value::Field(name) => ScalarOrArray::new_array(numeric(&column(t, name)?)?),
        Value::Scaled(s, v) => ScalarOrArray::new_array(numeric(
            &p.scales[s.name()].scale(&input(v, p, t)?).map_err(error)?,
        )?),
        Value::BandPosition(s, v, fraction) => ScalarOrArray::new_array(numeric(
            &p.scales[s.name()]
                .clone()
                .with_option("band", *fraction)
                .scale(&input(v, p, t)?)
                .map_err(error)?,
        )?),
        Value::Baseline(s) => {
            let scale = &p.scales[s.name()];
            let (lo, hi) = scale.numeric_interval_domain_f64().map_err(error)?;
            let zero = 0.0_f64.clamp(lo.min(hi), lo.max(hi));
            let a = ScalarValue::Float64(Some(zero))
                .to_array_of_size(1)
                .map_err(error)?;
            ScalarOrArray::new_scalar(numeric(&scale.scale(&a).map_err(error)?)?[0])
        }
        Value::Bandwidth(s) => ScalarOrArray::new_scalar(
            avenger_scales::scales::band::bandwidth(&p.scales[s.name()].config).map_err(error)?,
        ),
        Value::PlotWidth => ScalarOrArray::new_scalar(p.plot.size.width),
        Value::PlotHeight => ScalarOrArray::new_scalar(p.plot.size.height),
    })
}
fn eligible<'a>(v: &'a Value, p: &'a PlotInstance) -> Option<(&'a str, &'a ConfiguredScale)> {
    if let Value::Scaled(s, v) = v {
        if let Value::Field(f) = v.as_ref() {
            let d = &p.plot.scales.iter().find(|(n, _)| n == s.name())?.1;
            if d.kind == ScaleKind::Linear && !d.clamp {
                return Some((f, &p.scales[s.name()]));
            }
        }
    }
    None
}
struct MappedPositions {
    xy: [ScalarOrArray<f32>; 2],
    adjustments: [Option<LinearScaleAdjustment>; 2],
}
fn positions(
    chart: &Chart,
    p: &PlotInstance,
    m: &Mark,
    t: &TableSnapshot,
    encoding: &SymbolEncoding,
    report: &mut GeometryReport,
    alive: &mut HashSet<String>,
) -> Result<MappedPositions> {
    let x = encoding.x.as_ref().unwrap();
    let y = encoding.y.as_ref().unwrap();
    let key = format!("{}:{}", p.id.as_str(), m.name);
    let eligibility = eligible(x, p).zip(eligible(y, p));
    if let Some(((xf, xs), (yf, ys))) = eligibility {
        alive.insert(key.clone());
        let fields = [xf.to_string(), yf.to_string()];
        let old = chart.0.positions.lock().map_err(error)?.get(&key).cloned();
        if let Some(old) = old.filter(|v| v.source == t.id() && v.fields == fields) {
            let ax = old.base[0].adjust(xs).map_err(error)?;
            let ay = old.base[1].adjust(ys).map_err(error)?;
            if [ax.scale, ax.offset, ay.scale, ay.offset]
                .iter()
                .all(|v| v.is_finite())
            {
                tracing::debug!(target:"avenger_chart::positions",plot=%p.id,mark=%m.name,"reused base positions");
                report.position_reuses += 1;
                return Ok(MappedPositions {
                    xy: old.xy.clone(),
                    adjustments: [Some(ax), Some(ay)],
                });
            }
        }
        let xy = [channel(x, p, t)?, channel(y, p, t)?];
        report.position_builds += 1;
        tracing::debug!(target:"avenger_chart::positions",plot=%p.id,mark=%m.name,rows=t.num_rows(),"built base positions");
        chart.0.positions.lock().map_err(error)?.insert(
            key,
            Arc::new(Positions {
                source: t.id(),
                fields,
                base: [xs.clone(), ys.clone()],
                xy: xy.clone(),
            }),
        );
        Ok(MappedPositions {
            xy,
            adjustments: [None, None],
        })
    } else {
        report.position_builds += 1;
        Ok(MappedPositions {
            xy: [channel(x, p, t)?, channel(y, p, t)?],
            adjustments: [None, None],
        })
    }
}
pub(crate) fn build(
    chart: &Chart,
    p: &PlotInstance,
    report: &mut GeometryReport,
    alive: &mut HashSet<String>,
) -> Result<Vec<SceneMark>> {
    p.plot
        .marks
        .iter()
        .map(|m| {
            let t = p.ctx.table(&m.table)?;
            let len: u32 = t.num_rows().try_into().map_err(error)?;
            if len == 0 {
                return Ok(SceneMark::Group(Default::default()));
            }
            Ok(match &m.encoding {
                Encoding::Symbol(s) => {
                    let MappedPositions {
                        xy: [x, y],
                        adjustments: [x_adjustment, y_adjustment],
                    } = positions(chart, p, m, t, s, report, alive)?;
                    SceneMark::Symbol(SceneSymbolMark {
                        name: m.name.clone(),
                        len,
                        x,
                        y,
                        x_adjustment,
                        y_adjustment,
                        size: channel(&s.size, p, t)?,
                        fill: color(&s.fill)?,
                        stroke_width: None,
                        shapes: vec![SymbolShape::from_vega_str("circle").map_err(error)?],
                        interactive: s.interactive,
                        clip: p.plot.clip,
                        ..Default::default()
                    })
                }
                Encoding::Rect(r) => {
                    let interval = |start: &Option<Value>,
                                    center: &Option<Value>,
                                    end: &Option<Value>,
                                    size: &Option<Value>,
                                    span: SpanAdjustment|
                     -> Result<(Vec<f32>, Vec<f32>)> {
                        let anchor = channel(start.as_ref().or(center.as_ref()).unwrap(), p, t)?
                            .as_vec(len as usize, None);
                        let sizes = size
                            .as_ref()
                            .map(|v| channel(v, p, t).map(|v| v.as_vec(len as usize, None)))
                            .transpose()?;
                        let ends = end
                            .as_ref()
                            .map(|v| channel(v, p, t).map(|v| v.as_vec(len as usize, None)))
                            .transpose()?;
                        let mut starts = Vec::with_capacity(len as usize);
                        let mut lengths = Vec::with_capacity(len as usize);
                        for (i, a) in anchor.into_iter().enumerate() {
                            let (lo, raw) = if let Some(ends) = &ends {
                                (a.min(ends[i]), (a - ends[i]).abs())
                            } else {
                                let size = sizes.as_ref().unwrap()[i];
                                if size < 0.0 {
                                    return Err(error("rectangle size must be nonnegative"));
                                }
                                (if center.is_some() { a - size / 2.0 } else { a }, size)
                            };
                            let length = (raw - span.spacing).max(span.minimum);
                            starts.push(lo + (raw - length) / 2.0 + span.offset);
                            lengths.push(length);
                        }
                        Ok((starts, lengths))
                    };
                    let (xs, widths) = interval(&r.x, &r.xc, &r.x2, &r.width, r.x_span)?;
                    let (ys, heights) = interval(&r.y, &r.yc, &r.y2, &r.height, r.y_span)?;
                    SceneMark::Rect(SceneRectMark {
                        name: m.name.clone(),
                        len,
                        x: ScalarOrArray::new_array(xs),
                        y: ScalarOrArray::new_array(ys),
                        width: Some(ScalarOrArray::new_array(widths)),
                        height: Some(ScalarOrArray::new_array(heights)),
                        fill: color(&r.fill)?,
                        interactive: r.interactive,
                        clip: p.plot.clip,
                        ..Default::default()
                    })
                }
            })
        })
        .collect()
}

// Plain text layout collapses whitespace. Titles preserve authored line breaks as separate instances.
pub(crate) fn multiline(
    mut mark: avenger_scenegraph::marks::text::SceneTextMark,
) -> avenger_scenegraph::marks::text::SceneTextMark {
    use avenger_text::types::{TextBaseline, TextSyntaxMode};
    if mark.len != 1 || mark.text_syntax != TextSyntaxMode::Plain {
        return mark;
    }
    let text = mark.text.as_vec(1, None);
    if !text[0].contains('\n') {
        return mark;
    }
    let lines = text[0].split('\n').map(str::to_owned).collect::<Vec<_>>();
    let step = mark.font_size.as_vec(1, None)[0] * 1.2;
    let angle = mark.angle.as_vec(1, None)[0].to_radians();
    let baseline = mark.baseline.as_vec(1, None)[0];
    let offset = if matches!(baseline, TextBaseline::Bottom | TextBaseline::LineBottom) {
        -(lines.len() as f32 - 1.0) * step
    } else {
        0.0
    };
    let x = mark.x.as_vec(1, None)[0];
    let y = mark.y.as_vec(1, None)[0];
    mark.x = ScalarOrArray::new_array(
        (0..lines.len())
            .map(|i| x - (offset + i as f32 * step) * angle.sin())
            .collect(),
    );
    mark.y = ScalarOrArray::new_array(
        (0..lines.len())
            .map(|i| y + (offset + i as f32 * step) * angle.cos())
            .collect(),
    );
    mark.len = lines.len() as u32;
    mark.text = ScalarOrArray::new_array(lines);
    mark
}
pub(crate) fn multiline_group(
    mut group: avenger_scenegraph::marks::group::SceneGroup,
) -> avenger_scenegraph::marks::group::SceneGroup {
    group.marks = group
        .marks
        .into_iter()
        .map(|m| match m {
            SceneMark::Text(t) => SceneMark::Text(Arc::new(multiline((*t).clone()))),
            SceneMark::Group(g) => SceneMark::Group(multiline_group(g)),
            m => m,
        })
        .collect();
    group
}
