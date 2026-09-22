use crate::{
    error,
    source::{Source, SourceTable},
    spec as vl, CompileError, Result,
};
use avenger_chart_definition as chart;
use avenger_datafusion_dataflow::{
    datafusion::{
        arrow::datatypes::DataType,
        common::{Column, ScalarValue},
        functions::core::expr_fn::{coalesce, get_field, named_struct},
        functions_aggregate::expr_fn::min,
        logical_expr::{
            lit, scalar_subquery, try_cast, when, Expr, LogicalPlan, LogicalPlanBuilder,
        },
    },
    DataflowBuilder, ScalarInput, ScalarNode, SemanticConfig,
};
use avenger_transform as transform;
use std::{collections::BTreeMap, str::FromStr, sync::Arc};
use vl::MissingNullOrValue::{Missing, Null, Value as Present};

impl From<avenger_datafusion_dataflow::datafusion::common::DataFusionError> for CompileError {
    fn from(e: avenger_datafusion_dataflow::datafusion::common::DataFusionError) -> Self {
        Self::at("$", e)
    }
}
impl From<avenger_datafusion_dataflow::Error> for CompileError {
    fn from(e: avenger_datafusion_dataflow::Error) -> Self {
        Self::at("$", e)
    }
}
impl From<chart::Error> for CompileError {
    fn from(e: chart::Error) -> Self {
        Self::at("$", e)
    }
}
fn col(name: &str) -> Expr {
    Expr::Column(Column::from_name(name))
}
fn text(value: &vl::Text) -> String {
    match value {
        vl::Text::String(s) => s.clone(),
        vl::Text::Lines(v) => v.join("\n"),
    }
}
fn finite(e: Expr) -> Expr {
    e.clone().gt_eq(lit(-f64::MAX)).and(e.lt_eq(lit(f64::MAX)))
}
fn numeric(e: Expr) -> Expr {
    try_cast(e, DataType::Float64)
}
fn extent_literal(a: f64, b: f64) -> Expr {
    named_struct(vec![lit("min"), lit(a), lit("max"), lit(b)])
}

// Literal escaped field names are supported; nested datum access belongs to a later frontend.
fn field(plan: &LogicalPlan, raw: &str, path: &str) -> Result<String> {
    let mut name = String::new();
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => name.push(
                chars
                    .next()
                    .ok_or_else(|| error(path, "unfinished field escape"))?,
            ),
            '.' | '[' | ']' => return Err(error(
                path,
                "nested field paths are not supported; escape punctuation for a literal field name",
            )),
            c => name.push(c),
        }
    }
    if !plan.schema().fields().iter().any(|f| f.name() == &name) {
        return Err(error(path, format!("unknown field {name}")));
    }
    Ok(name)
}
struct EncodingBin {
    field: String,
    options: vl::Bin,
    start: String,
    end: String,
    parameters: ScalarNode,
}

struct Compiler {
    flow: DataflowBuilder,
    prefix: String,
    ordinal: String,
    next: usize,
    params: BTreeMap<String, ScalarInput>,
    encoding_bins: Vec<EncodingBin>,
}
impl Compiler {
    fn name(&mut self, role: &str) -> String {
        let name = format!("{}{}_{}", self.prefix, role, self.next);
        self.next += 1;
        name
    }
    fn node(&mut self, plan: LogicalPlan, role: &str) -> Result<LogicalPlan> {
        let name = self.name(role);
        Ok(self.flow.add_plan(name, plan)?.plan_ref())
    }
    fn scalar(&mut self, expr: Expr, role: &str) -> Result<ScalarNode> {
        let name = self.name(role);
        Ok(self.flow.add_scalar(name, expr)?)
    }
    fn extent(&mut self, plan: LogicalPlan, expr: Expr) -> Result<ScalarNode> {
        let plan = self.node(transform::extent(plan, expr)?, "extent")?;
        self.scalar(scalar_subquery(Arc::new(plan)), "extent_value")
    }
    fn bin(
        &mut self,
        plan: LogicalPlan,
        field: &str,
        bin: &vl::Bin,
        names: [&str; 2],
    ) -> Result<(LogicalPlan, ScalarNode)> {
        let p = match bin {
            vl::Bin::Params(p) => p.clone(),
            _ => vl::BinParams::default(),
        };
        let extent = match p.extent {
            Some([a, b]) => extent_literal(a, b),
            None => self.extent(plan.clone(), numeric(col(field)))?.expr_ref(),
        };
        let list = |v: &Vec<f64>| {
            lit(ScalarValue::List(ScalarValue::new_list(
                &v.iter()
                    .map(|x| ScalarValue::Float64(Some(*x)))
                    .collect::<Vec<_>>(),
                &DataType::Float64,
                false,
            )))
        };
        let params = transform::bin_parameters(
            extent,
            transform::BinOptions {
                maxbins: Some(lit(p.maxbins.unwrap_or(10.0))),
                base: p.base.map(lit),
                divide: p.divide.as_ref().map(list),
                span: None,
                step: p.step.map(lit),
                steps: p.steps.as_ref().map(list),
                minstep: p.minstep.map(lit),
                nice: p.nice.map(lit),
                anchor: p.anchor.map(lit),
            },
        )?;
        let params = self.scalar(params, "bin_parameters")?;
        let plan = transform::bin(plan, numeric(col(field)), params.expr_ref(), names)?;
        Ok((self.node(plan, "bins")?, params))
    }
    fn aggregate(
        &self,
        plan: LogicalPlan,
        groups: Vec<String>,
        mut measures: Vec<Expr>,
    ) -> Result<LogicalPlan> {
        measures.push(min(col(&self.ordinal)).alias(&self.ordinal));
        Ok(transform::aggregate(
            plan,
            groups.iter().map(|s| col(s)).collect(),
            measures,
        )?)
    }
    fn authored(
        &mut self,
        mut plan: LogicalPlan,
        transforms: &[vl::Transform],
    ) -> Result<LogicalPlan> {
        for (i, t) in transforms.iter().enumerate() {
            let path = format!("transform[{i}]");
            plan = match t {
                vl::Transform::Filter(t) => {
                    let f = field(&plan, &t.filter.field, &format!("{path}.filter.field"))?;
                    let value_type = plan.schema().field_with_unqualified_name(&f)?.data_type();
                    if !value_type.is_numeric() && *value_type != DataType::Null {
                        return Err(error(
                            format!("{path}.filter.field"),
                            "gte currently requires a numeric column",
                        ));
                    }
                    let rhs = match &t.filter.gte {
                        vl::PredicateOperand::Number(v) => lit(*v),
                        vl::PredicateOperand::Expr(e) => self
                            .params
                            .get(e.expr.trim())
                            .ok_or_else(|| {
                                error(
                                    format!("{path}.filter.gte.expr"),
                                    "expected a declared numeric parameter name",
                                )
                            })?
                            .expr_ref(),
                    };
                    let a = coalesce(vec![numeric(col(&f)), lit(0.0)]);
                    let b = coalesce(vec![rhs, lit(0.0)]);
                    // DataFusion orders NaN above finite values; exclude it explicitly.
                    let pred = a
                        .clone()
                        .not_eq(lit(f64::NAN))
                        .and(b.clone().not_eq(lit(f64::NAN)))
                        .and(a.gt_eq(b));
                    transform::filter(plan, pred)?
                }
                vl::Transform::Bin(t) => {
                    let f = field(&plan, &t.field, &format!("{path}.field"))?;
                    let names = match &t.as_ {
                        vl::BinOutput::Name(s) => [s.clone(), format!("{s}_end")],
                        vl::BinOutput::Pair(p) => p.clone(),
                    };
                    self.bin(plan, &f, &t.bin, [&names[0], &names[1]])?.0
                }
                vl::Transform::Aggregate(t) => {
                    let groups = t
                        .groupby
                        .iter()
                        .flatten()
                        .map(|s| field(&plan, s, &format!("{path}.groupby")))
                        .collect::<Result<Vec<_>>>()?;
                    let measures = t
                        .aggregate
                        .iter()
                        .enumerate()
                        .map(|(j, a)| {
                            measure(
                                &plan,
                                a.op,
                                a.field.as_deref(),
                                &format!("{path}.aggregate[{j}]"),
                            )
                            .map(|e| e.alias(&a.as_))
                        })
                        .collect::<Result<Vec<_>>>()?;
                    self.aggregate(plan, groups, measures)?
                }
            };
            plan = self.node(plan, "transform")?;
        }
        Ok(plan)
    }
}
fn measure(
    plan: &LogicalPlan,
    op: vl::AggregateOp,
    name: Option<&str>,
    path: &str,
) -> Result<Expr> {
    use transform::expr_fn as a;
    use vl::AggregateOp::*;
    if op == Count {
        return Ok(a::count());
    }
    let value = col(&field(
        plan,
        name.ok_or_else(|| error(path, "aggregate requires a field"))?,
        &format!("{path}.field"),
    )?);
    Ok(match op {
        Count => unreachable!(),
        Valid => a::valid(value),
        Missing => a::missing(value),
        Sum => a::sum(numeric(value)),
        Min => a::min(value),
        Max => a::max(value),
        Mean | Average => a::mean(numeric(value)),
        Variance => a::variance(numeric(value)),
        Variancep => a::variancep(numeric(value)),
        Stdev => a::stdev(numeric(value)),
        Stdevp => a::stdevp(numeric(value)),
    })
}
#[derive(Clone)]
struct Channel {
    def: vl::PositionFieldDef,
    field: String,
    end: Option<String>,
    bin: Option<ScalarNode>,
    quantitative: bool,
    kind: Option<chart::ScaleKind>,
    hidden: bool,
    title: String,
    binned: bool,
    end_in_domain: bool,
}
fn channel(
    c: &mut Compiler,
    mut plan: LogicalPlan,
    def: Option<&vl::PositionFieldDef>,
    secondary: Option<&vl::SecondaryFieldDef>,
    axis: &str,
) -> Result<(LogicalPlan, Channel)> {
    let hidden = def.is_none();
    let def = def.cloned().unwrap_or_default();
    let path = format!("encoding.{axis}");
    if def.field_type == Some(vl::FieldType::Temporal) {
        return Err(error(
            format!("{path}.type"),
            "temporal encodings are not supported",
        ));
    }
    let binned = matches!(&def.bin, Present(vl::Bin::Binned))
        || matches!(&def.bin, Present(vl::Bin::Params(p)) if p.binned == Some(true));
    let binning = matches!(&def.bin, Present(vl::Bin::Bool(true)))
        || matches!(&def.bin, Present(vl::Bin::Params(p)) if p.binned != Some(true));
    let quantitative = def.field_type == Some(vl::FieldType::Quantitative)
        || (def.field_type.is_none()
            && (def.aggregate.is_some()
                || binned
                || binning
                || matches!(&def.scale, Present(s) if s.scale_type == Some(vl::ScaleType::Linear))));
    let mut f = if hidden {
        let f = c.name("single_category");
        plan = transform::formula(plan, lit(""), &f)?;
        f
    } else if def.aggregate == Some(vl::AggregateOp::Count) {
        String::new()
    } else {
        field(
            &plan,
            def.field
                .as_deref()
                .ok_or_else(|| error(format!("{path}.field"), "field is required"))?,
            &format!("{path}.field"),
        )?
    };
    let mut end = secondary
        .map(|s| field(&plan, &s.field, &format!("encoding.{axis}2.field")))
        .transpose()?;
    let mut bin = None;
    if binning {
        if end.is_some() {
            return Err(error(
                path,
                "generated bins cannot also specify a secondary boundary",
            ));
        }
        let options = match &def.bin {
            Present(b) => b,
            _ => unreachable!(),
        };
        // Encoding binning only appends columns, so both channels can share a bin calculation.
        if let Some(previous) = c
            .encoding_bins
            .iter()
            .find(|b| b.field == f && &b.options == options)
        {
            f = previous.start.clone();
            end = Some(previous.end.clone());
            bin = Some(previous.parameters.clone());
        } else {
            let start = c.name("bin_start");
            let stop = c.name("bin_end");
            let (p, b) = c.bin(plan, &f, options, [&start, &stop])?;
            c.encoding_bins.push(EncodingBin {
                field: f,
                options: options.clone(),
                start: start.clone(),
                end: stop.clone(),
                parameters: b.clone(),
            });
            plan = p;
            f = start;
            end = Some(stop);
            bin = Some(b);
        }
    } else if binned && end.is_none() {
        if let Present(vl::Bin::Params(p)) = &def.bin {
            if let Some(step) = p.step {
                let stop = c.name("bin_end");
                plan = transform::formula(plan, numeric(col(&f)) + lit(step), &stop)?;
                end = Some(stop);
            }
        }
        if end.is_none() {
            return Err(error(
                path,
                "already-binned data requires an end field or step",
            ));
        }
    }
    let kind = match &def.scale {
        Null => {
            if !quantitative {
                return Err(error(
                    format!("{path}.scale"),
                    "disabled scales require quantitative positions",
                ));
            }
            None
        }
        Present(s) => Some(
            match s.scale_type.unwrap_or(if quantitative {
                vl::ScaleType::Linear
            } else {
                vl::ScaleType::Band
            }) {
                vl::ScaleType::Linear => chart::ScaleKind::Linear,
                vl::ScaleType::Band => chart::ScaleKind::Band,
                vl::ScaleType::Point => chart::ScaleKind::Point,
            },
        ),
        Missing => Some(if quantitative {
            chart::ScaleKind::Linear
        } else {
            chart::ScaleKind::Band
        }),
    };
    if kind == Some(chart::ScaleKind::Linear) && !quantitative {
        return Err(error(
            format!("{path}.scale"),
            "linear scale requires a quantitative field",
        ));
    }
    if (binning || binned) && kind != Some(chart::ScaleKind::Linear) {
        return Err(error(
            format!("{path}.bin"),
            "binned positions currently require a quantitative linear scale",
        ));
    }
    if kind.is_some_and(|k| k != chart::ScaleKind::Linear)
        && matches!(&def.scale, Present(s) if s.zero == Some(true) || s.nice == Some(true))
    {
        return Err(error(
            format!("{path}.scale"),
            "zero and nice require a linear scale",
        ));
    }
    let title = match &def.title {
        Null => String::new(),
        Present(t) => text(t),
        Missing => {
            let f = def.field.clone().unwrap_or_default();
            match def.aggregate {
                Some(vl::AggregateOp::Count) => "Count of Records".into(),
                Some(op) => format!(
                    "{} of {f}",
                    match op {
                        vl::AggregateOp::Mean | vl::AggregateOp::Average => "Mean".into(),
                        _ => {
                            let s = serde_json::to_value(op)
                                .unwrap()
                                .as_str()
                                .unwrap()
                                .to_owned();
                            format!("{}{}", s[..1].to_uppercase(), &s[1..])
                        }
                    }
                ),
                None if binning || binned => format!("{f} (binned)"),
                None => f,
            }
        }
    };
    Ok((
        plan,
        Channel {
            def,
            field: f,
            end,
            bin,
            quantitative,
            kind,
            hidden,
            title,
            binned: binning || binned,
            end_in_domain: binning || secondary.is_some(),
        },
    ))
}

pub(crate) fn compile(spec: &vl::UnitSpec, source: Source) -> Result<chart::ChartDefinition> {
    let mut c = Compiler {
        flow: DataflowBuilder::with_semantics(SemanticConfig {
            function_versions: transform::function_versions(),
            ..Default::default()
        }),
        prefix: source.prefix,
        ordinal: source.ordinal,
        next: 0,
        params: BTreeMap::new(),
        encoding_bins: Vec::new(),
    };
    for p in spec.params.iter().flatten() {
        c.params.insert(
            p.name.clone(),
            c.flow.scalar_input(&p.name, DataType::Float64)?,
        );
    }
    let source = match source.table {
        SourceTable::Input { name, schema } => {
            let input = c.flow.table_input(name, schema)?;
            c.flow
                .add_plan("source", input.plan_ref_with_row_index(&c.ordinal)?)?
                .plan_ref()
        }
        SourceTable::Snapshot(snapshot) => c.flow.table_snapshot("source", snapshot)?.plan_ref(),
    };
    let plan = c.authored(source, spec.transform.as_deref().unwrap_or_default())?;
    let enc = spec.encoding.clone().unwrap_or_default();
    for (axis, primary, secondary) in [("x", &enc.x, &enc.x2), ("y", &enc.y, &enc.y2)] {
        if secondary.is_some() && primary.is_none() {
            return Err(error(
                format!("encoding.{axis}2"),
                "secondary boundary requires a primary channel",
            ));
        }
    }
    let (plan, mut x) = channel(&mut c, plan, enc.x.as_ref(), enc.x2.as_ref(), "x")?;
    let (mut plan, mut y) = channel(&mut c, plan, enc.y.as_ref(), enc.y2.as_ref(), "y")?;
    let mark = match &spec.mark {
        vl::Mark::Bar => vl::BarMark::default(),
        vl::Mark::Def(m) => m.clone(),
    };
    let horizontal = match mark.orient {
        Some(vl::Orient::Horizontal) => true,
        Some(vl::Orient::Vertical) => false,
        None => x.quantitative && !x.binned && (!y.quantitative || y.binned),
    };
    let aggregated = x.def.aggregate.is_some() || y.def.aggregate.is_some();
    if aggregated {
        let mut groups = vec![];
        let mut measures = vec![];
        for ch in [&mut x, &mut y] {
            if let Some(op) = ch.def.aggregate {
                if ch.binned || ch.end.is_some() {
                    return Err(error("encoding","aggregation cannot be combined with binning or a secondary field on the same channel"));
                }
                let name = c.name("aggregate");
                measures
                    .push(measure(&plan, op, ch.def.field.as_deref(), "encoding")?.alias(&name));
                ch.field = name;
            } else {
                for f in std::iter::once(&ch.field).chain(ch.end.iter()) {
                    if !groups.contains(f) {
                        groups.push(f.clone());
                    }
                }
            }
        }
        plan = c.aggregate(plan, groups, measures)?;
    }
    // Convert positional values after valid/missing/count have seen the original rows.
    for ch in [&mut x, &mut y] {
        if ch.quantitative {
            for f in std::iter::once(&mut ch.field).chain(ch.end.iter_mut()) {
                let name = c.name("position");
                let ty = plan.schema().field_with_unqualified_name(f)?.data_type();
                if !ty.is_numeric()
                    && !matches!(
                        ty,
                        DataType::Utf8
                            | DataType::LargeUtf8
                            | DataType::Utf8View
                            | DataType::Null
                            | DataType::Boolean
                    )
                {
                    return Err(error(
                        "encoding",
                        format!("unsupported quantitative type {ty}"),
                    ));
                }
                let e = if matches!(
                    ty,
                    DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View
                ) {
                    when(
                        avenger_datafusion_dataflow::datafusion::functions::string::expr_fn::btrim(
                            vec![col(f)],
                        )
                        .eq(lit("")),
                        lit(0.0),
                    )
                    .otherwise(numeric(col(f)))?
                } else {
                    numeric(col(f))
                };
                plan = transform::formula(plan, e, &name)?;
                *f = name;
                plan = transform::filter(plan, finite(col(f)))?;
            }
        } else if plan
            .schema()
            .field_with_unqualified_name(&ch.field)?
            .data_type()
            == &DataType::Null
        {
            plan = transform::formula(plan, try_cast(col(&ch.field), DataType::Utf8), &ch.field)?;
        }
    }
    let (dimension, measure_ch) = if horizontal {
        (&y, &mut x)
    } else {
        (&x, &mut y)
    };
    let stack = measure_ch.quantitative
        && !measure_ch.binned
        && measure_ch.end.is_none()
        && !matches!(measure_ch.def.stack, Null)
        && (!aggregated || matches!(measure_ch.def.stack, Present(_)));
    if stack {
        let start = c.name("stack_start");
        let end = c.name("stack_end");
        plan = transform::stack_zero(
            plan,
            vec![col(&dimension.field)],
            col(&measure_ch.field),
            vec![col(&c.ordinal).sort(true, true)],
            [start.clone(), end.clone()],
        )?;
        measure_ch.field = start;
        measure_ch.end = Some(end);
        measure_ch.end_in_domain = true;
    }
    plan = LogicalPlanBuilder::from(plan)
        .sort(vec![col(&c.ordinal).sort(true, true)])?
        .build()?;
    let rows = c.flow.add_plan("visible_rows", plan)?;
    let output = c.flow.table_output("rows", &rows)?;
    let xd = domain(&mut c, rows.plan_ref(), &x, "x")?;
    let yd = domain(&mut c, rows.plan_ref(), &y, "y")?;
    let params = c.params;
    let mut builder = chart::ChartDefinition::builder(c.flow.finish()?);
    builder.arrange(chart::Arrangement::column().margin(5.0));
    builder.background("white");
    if let Some(t) = &spec.title {
        builder.title(text(t));
    }
    for p in spec.params.iter().flatten() {
        builder.parameter(
            &p.name,
            &params[&p.name],
            ScalarValue::Float64(Some(p.value)),
        )?;
    }
    let color = mark.color.as_deref().unwrap_or("#4c78a8");
    let rgba =
        css_color_parser::Color::from_str(color).map_err(|e| error("mark.color", e.to_string()))?;
    let fill = format!(
        "rgba({}, {}, {}, {})",
        rgba.r,
        rgba.g,
        rgba.b,
        rgba.a as f64 * mark.opacity.unwrap_or(1.0)
    );
    builder.plot("plot", |p| {
        p.content_size(
            spec.width.unwrap_or(300.0) as f32,
            spec.height.unwrap_or(300.0) as f32,
        );
        p.clip(false);
        let xs = scale(p, &x, xd, chart::Range::PlotWidth, "x", horizontal)?;
        let ys = scale(
            p,
            &y,
            yd,
            if y.kind.is_some_and(|k| k != chart::ScaleKind::Linear) {
                chart::Range::PlotHeight
            } else {
                chart::Range::PlotHeightReversed
            },
            "y",
            !horizontal,
        )?;
        if spec.width.is_none() && x.kind.is_some_and(|k| k != chart::ScaleKind::Linear) {
            p.width_step(xs.as_ref().unwrap(), 20.0);
        }
        if spec.height.is_none() && y.kind.is_some_and(|k| k != chart::ScaleKind::Linear) {
            p.height_step(ys.as_ref().unwrap(), 20.0);
        }
        let mut rect = chart::RectEncoding::new().fill(fill);
        rect = geometry(rect, &x, xs.as_ref(), true, horizontal, mark.size);
        rect = geometry(rect, &y, ys.as_ref(), false, !horizontal, mark.size);
        p.rect("bars", &output, rect)?;
        axis(p, &x, xs.as_ref(), true)?;
        axis(p, &y, ys.as_ref(), false)?;
        Ok(())
    })?;
    Ok(builder.finish()?)
}
fn domain(c: &mut Compiler, plan: LogicalPlan, ch: &Channel, axis: &str) -> Result<chart::Domain> {
    if let Some(bin) = &ch.bin {
        let e = named_struct(vec![
            lit("min"),
            get_field(bin.expr_ref(), "start"),
            lit("max"),
            get_field(bin.expr_ref(), "stop"),
        ]);
        let v = c.scalar(e, "bin_domain")?;
        return Ok(chart::Domain::extent(
            &c.flow.scalar_output(format!("{axis}_domain"), &v)?,
        ));
    }
    if ch.kind == Some(chart::ScaleKind::Linear) || ch.kind.is_none() {
        let mut p = LogicalPlanBuilder::from(plan.clone())
            .project(vec![col(&ch.field).alias("value")])?
            .build()?;
        if let Some(end) = ch.end.as_ref().filter(|_| ch.end_in_domain) {
            let other = LogicalPlanBuilder::from(plan)
                .project(vec![col(end).alias("value")])?
                .build()?;
            p = LogicalPlanBuilder::from(p).union(other)?.build()?;
        }
        let v = c.extent(p, col("value"))?;
        Ok(chart::Domain::extent(
            &c.flow.scalar_output(format!("{axis}_domain"), &v)?,
        ))
    } else {
        let p = LogicalPlanBuilder::from(plan).aggregate(
            vec![col(&ch.field)],
            vec![min(col(&c.ordinal)).alias(&c.ordinal)],
        )?;
        let order = match ch.def.sort {
            Null => col(&c.ordinal).sort(true, true),
            Present(vl::SortOrder::Descending) => col(&ch.field).sort(false, false),
            _ => col(&ch.field).sort(true, true),
        };
        let p = c
            .flow
            .add_plan(format!("{axis}_categories"), p.sort(vec![order])?.build()?)?;
        Ok(chart::Domain::column(
            &c.flow.table_output(format!("{axis}_domain"), &p)?,
            &ch.field,
        ))
    }
}
fn scale(
    p: &mut chart::PlotBuilder,
    ch: &Channel,
    domain: chart::Domain,
    range: chart::Range,
    name: &str,
    measure: bool,
) -> chart::Result<Option<chart::ScaleHandle>> {
    let Some(kind) = ch.kind else { return Ok(None) };
    let opts = match &ch.def.scale {
        Present(s) => s.clone(),
        _ => vl::Scale::default(),
    };
    let s = match kind {
        chart::ScaleKind::Linear => chart::Scale::linear(domain, range)
            .zero(opts.zero.unwrap_or(measure && !ch.binned))
            .pixel_padding(if !measure && ch.end.is_none() {
                5.0
            } else {
                0.0
            })
            .nice(opts.nice.unwrap_or(!ch.binned)),
        chart::ScaleKind::Band => chart::Scale::band(domain, range)
            .padding_inner(0.1)
            .padding_outer(0.05)
            .include_null(true),
        chart::ScaleKind::Point => chart::Scale::point(domain, range)
            .padding_outer(0.5)
            .include_null(true),
    };
    Ok(Some(p.scale(name, s)?))
}
fn geometry(
    mut r: chart::RectEncoding,
    ch: &Channel,
    s: Option<&chart::ScaleHandle>,
    x: bool,
    measure: bool,
    size: Option<f64>,
) -> chart::RectEncoding {
    let value = |f: &str| s.map_or_else(|| chart::Value::field(f), |s| s.field(f));
    if let Some(end) = &ch.end {
        r = if x {
            r.x(value(&ch.field)).x2(value(end))
        } else {
            r.y(value(&ch.field)).y2(value(end))
        };
        if ch.binned {
            let span = chart::SpanAdjustment {
                spacing: 1.0,
                minimum: 0.25,
                offset: 0.5,
            };
            r = if x { r.x_span(span) } else { r.y_span(span) };
        }
    } else if measure && ch.quantitative {
        let baseline = s.map_or_else(|| chart::Value::Constant(0.0), |s| s.baseline());
        r = if x {
            r.x(baseline).x2(value(&ch.field))
        } else {
            r.y(baseline).y2(value(&ch.field))
        };
    } else if ch.kind == Some(chart::ScaleKind::Band) && size.is_none() {
        let s = s.unwrap();
        r = if x {
            r.x(s.field(&ch.field)).width(s.bandwidth())
        } else {
            r.y(s.field(&ch.field)).height(s.bandwidth())
        };
    } else {
        let center = if ch.kind == Some(chart::ScaleKind::Band) {
            s.unwrap().band_position(&ch.field, 0.5)
        } else {
            value(&ch.field)
        };
        let size = size.unwrap_or(if ch.quantitative { 5.0 } else { 18.0 });
        r = if x {
            r.xc(center).width(size)
        } else {
            r.yc(center).height(size)
        };
    }
    r
}
fn axis(
    p: &mut chart::PlotBuilder,
    ch: &Channel,
    s: Option<&chart::ScaleHandle>,
    x: bool,
) -> chart::Result<()> {
    let Some(s) = s else { return Ok(()) };
    if ch.hidden || matches!(ch.def.axis, Null) {
        return Ok(());
    }
    let a = match &ch.def.axis {
        Present(a) => a.clone(),
        _ => vl::Axis::default(),
    };
    let mut axis = if x {
        chart::Axis::bottom(s)
    } else {
        chart::Axis::left(s)
    };
    let title = match &a.title {
        Null => String::new(),
        Present(t) => text(t),
        Missing => ch.title.clone(),
    };
    axis = axis
        .title(title)
        .grid(a.grid.unwrap_or(ch.quantitative && !ch.binned));
    if let Some(f) = a.format {
        axis = axis.format(f);
    }
    axis = axis.label_angle(
        a.label_angle
            .unwrap_or(if x && !ch.quantitative { 270.0 } else { 0.0 }) as f32,
    );
    p.axis(axis)
}
