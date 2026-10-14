use crate::Result;
use crate::{dataflow::*, *};
use dataflow::datafusion::{arrow::datatypes::DataType, common::DFSchema};
use std::collections::{HashMap, HashSet};

#[derive(Default)]
struct Check {
    tables: HashSet<TableOutput>,
    scalars: HashSet<ScalarOutput>,
}
impl ChartDefinition {
    /// Validate native handles, scope visibility, property types, and visual structure.
    pub fn validate(&self) -> Result<()> {
        self.check()?;
        Ok(())
    }
    /// Collect distinct outputs required to evaluate every visual template.
    pub fn outputs(&self) -> Result<(Vec<TableOutput>, Vec<ScalarOutput>)> {
        let check = self.check()?;
        let mut tables: Vec<_> = check.tables.into_iter().collect();
        let mut scalars: Vec<_> = check.scalars.into_iter().collect();
        let interface = self.dataflow.interface();
        tables.sort_by_key(|h| {
            let r = interface.table_metadata(h).expect("validated").reference;
            (r.scope, r.name)
        });
        scalars.sort_by_key(|h| {
            let r = interface.scalar_metadata(h).expect("validated").reference;
            (r.scope, r.name)
        });
        Ok((tables, scalars))
    }
    fn check(&self) -> Result<Check> {
        let interface = self.dataflow.interface();
        let mut names = HashMap::new();
        let mut inputs = HashMap::new();
        for p in &self.parameters {
            nonempty(&p.name, "parameter")?;
            let address = interface.scalar_input_reference(&p.input)?;
            if let Some(v) = &p.initial {
                if v.data_type() != *p.input.field().data_type() {
                    return Err(invalid(
                        &p.name,
                        "initial value does not match scalar input type",
                    ));
                }
            }
            let key = (address.scope.clone(), p.name.clone());
            if names.insert(key, address.clone()).is_some() {
                return Err(invalid(&p.name, "duplicate parameter name"));
            }
            if inputs.insert(address, &p.name).is_some() {
                return Err(invalid(
                    &p.name,
                    "input has more than one parameter binding",
                ));
            }
        }
        let mut check = Check::default();
        check.group(&interface, &self.root, &[], "figure", 0)?;
        Ok(check)
    }
}
fn nonempty(name: &str, path: &str) -> Result<()> {
    if name.is_empty() {
        Err(invalid(path, "name cannot be empty"))
    } else {
        Ok(())
    }
}
fn finite(value: f32, path: &str) -> Result<()> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(invalid(path, "expected a finite nonnegative value"))
    }
}
fn visible(scope: &[String], reference: &Reference, path: &str) -> Result<()> {
    if scope.starts_with(&reference.scope) {
        Ok(())
    } else {
        Err(invalid(
            path,
            format!(
                "output {}/{} is outside the current scope",
                reference.scope.join("/"),
                reference.name
            ),
        ))
    }
}
pub(crate) fn column_type<'a>(
    schema: &'a DFSchema,
    field: &str,
    path: &str,
) -> Result<&'a DataType> {
    schema
        .field_with_unqualified_name(field)
        .map(|f| f.data_type())
        .map_err(|e| invalid(path, e.to_string()))
}
impl Check {
    fn scalar(
        &mut self,
        i: &DataflowInterface,
        h: &ScalarOutput,
        scope: &[String],
        path: &str,
    ) -> Result<DataType> {
        let m = i.scalar_metadata(h)?;
        visible(scope, &m.reference, path)?;
        self.scalars.insert(*h);
        Ok(m.schema.field(0).data_type().clone())
    }
    fn number(
        &mut self,
        i: &DataflowInterface,
        h: &ScalarOutput,
        scope: &[String],
        path: &str,
    ) -> Result<()> {
        if !self.scalar(i, h, scope, path)?.is_numeric() {
            return Err(invalid(path, "expected a numeric scalar output"));
        }
        Ok(())
    }
    fn table(
        &mut self,
        i: &DataflowInterface,
        h: &TableOutput,
        scope: &[String],
        path: &str,
    ) -> Result<OutputMetadata> {
        let m = i.table_metadata(h)?;
        visible(scope, &m.reference, path)?;
        self.tables.insert(*h);
        Ok(m)
    }
    fn group(
        &mut self,
        i: &DataflowInterface,
        g: &Group,
        scope: &[String],
        path: &str,
        depth: usize,
    ) -> Result<()> {
        nonempty(&g.name, path)?;
        arrangement(
            &g.arrangement,
            &g.children.iter().map(|n| n.name()).collect::<Vec<_>>(),
            path,
        )?;
        match &g.title {
            Some(Text::Scalar(h)) => {
                self.scalar(i, h, scope, path)?;
            }
            Some(Text::Key(key)) => {
                let s = i.scope_at(scope)?;
                let h = s
                    .handle()
                    .ok_or_else(|| invalid(path, "key title requires a facet"))?;
                h.key_schema()
                    .index_of(key)
                    .map_err(|e| invalid(path, e.to_string()))?;
            }
            _ => {}
        }
        if let Some(h) = &g.discovery {
            let m = i.scalar_metadata(h)?;
            if m.reference.scope != scope {
                return Err(invalid(
                    path,
                    "discovery anchor must be local to this scope",
                ));
            }
            self.scalar(i, h, scope, path)?;
        }
        let mut names = HashSet::new();
        for node in &g.children {
            let name = node.name();
            nonempty(name, path)?;
            if !names.insert(name) {
                return Err(invalid(path, format!("duplicate child {name}")));
            }
            let p = format!("{path}/{name}");
            match node {
                Node::Plot(plot) => self.plot(i, plot, scope, &p, depth + 1)?,
                Node::Group(group) => self.group(i, group, scope, &p, depth + 1)?,
                Node::Facet {
                    scope: child,
                    template,
                    arrangement: a,
                    ..
                } => {
                    let child_path = i.scope_path(child)?;
                    if child_path.len() != scope.len() + 1 || !child_path.starts_with(scope) {
                        return Err(invalid(&p, "facet must reference an immediate child scope"));
                    }
                    if matches!(a.kind, ArrangementKind::Grid { .. }) {
                        return Err(invalid(
                            &p,
                            "dynamic facets support row, column, or wrap arrangements",
                        ));
                    }
                    arrangement(a, &[], &p)?;
                    if let KeyOrder::Explicit(keys) = &template.key_order {
                        let mut seen = HashSet::new();
                        for key in keys {
                            child.key(key.values().iter().cloned())?;
                            if !seen.insert(key) {
                                return Err(invalid(&p, "duplicate explicit facet key"));
                            }
                        }
                    }
                    let mut local = Check::default();
                    local.group(i, template, &child_path, &p, depth + 2)?;
                    let has_local = local.tables.iter().any(|h| {
                        i.table_metadata(h)
                            .is_ok_and(|m| m.reference.scope.starts_with(&child_path))
                    }) || local.scalars.iter().any(|h| {
                        i.scalar_metadata(h)
                            .is_ok_and(|m| m.reference.scope.starts_with(&child_path))
                    });
                    if !has_local {
                        return Err(invalid(
                            &p,
                            "facet requires a local/descendant output or discover_with anchor",
                        ));
                    }
                    self.tables.extend(local.tables);
                    self.scalars.extend(local.scalars);
                }
            }
        }
        Ok(())
    }
    fn plot(
        &mut self,
        i: &DataflowInterface,
        plot: &Plot,
        scope: &[String],
        path: &str,
        depth: usize,
    ) -> Result<()> {
        finite(plot.size.width, path)?;
        finite(plot.size.height, path)?;
        if let Some(e) = &plot.guide_reservations {
            for v in [e.left, e.right, e.top, e.bottom] {
                finite(v, path)?;
            }
        }
        let mut names = HashSet::new();
        for (name, s) in &plot.scales {
            nonempty(name, path)?;
            if !names.insert(name) {
                return Err(invalid(path, "duplicate scale"));
            }
            let p = format!("{path}/{name}.domain");
            if s.kind == ScaleKind::Band && (s.zero || s.nice || s.clamp) {
                return Err(invalid(
                    path,
                    "zero, nice, and clamp apply to linear scales",
                ));
            }
            if s.kind == ScaleKind::Linear && (s.padding_inner != 0.0 || s.padding_outer != 0.0) {
                return Err(invalid(path, "padding applies to band scales"));
            }
            match &s.domain {
                Domain::Column(h, c) => {
                    let m = self.table(i, h, scope, &p)?;
                    let t = column_type(&m.schema, c, &p)?;
                    if s.kind != ScaleKind::Band || !band_type(t) {
                        return Err(invalid(&p, "column domains require a band scale and Boolean, Int32, Float32, or Utf8 values"));
                    }
                }
                Domain::Extent(h) => {
                    let t = self.scalar(i, h, scope, &p)?;
                    let DataType::Struct(fields) = t else {
                        return Err(invalid(&p, "extent must be a min/max struct"));
                    };
                    for name in ["min", "max"] {
                        if !fields
                            .iter()
                            .any(|f| f.name() == name && f.data_type().is_numeric())
                        {
                            return Err(invalid(&p, "extent requires numeric min/max fields"));
                        }
                    }
                    if s.kind != ScaleKind::Linear {
                        return Err(invalid(&p, "extent requires a linear scale"));
                    }
                }
                Domain::Bounds(a, b) => {
                    self.number(i, a, scope, &p)?;
                    self.number(i, b, scope, &p)?;
                    if s.kind != ScaleKind::Linear {
                        return Err(invalid(&p, "bounds require a linear scale"));
                    }
                }
                Domain::Values(v) => {
                    if s.kind == ScaleKind::Linear
                        && (v.len() != 2
                            || v.iter().any(|v| !v.data_type().is_numeric() || v.is_null()))
                    {
                        return Err(invalid(
                            &p,
                            "linear domain needs two non-null numeric values",
                        ));
                    }
                    if let Some(first) = v.first() {
                        if v.iter().any(|v| v.data_type() != first.data_type())
                            || (s.kind == ScaleKind::Band && !band_type(&first.data_type()))
                        {
                            return Err(invalid(&p, "incompatible literal domain types"));
                        }
                    }
                }
            }
            if s.empty_domain.iter().any(|v| !v.is_finite())
                || s.empty_domain[0] >= s.empty_domain[1]
            {
                return Err(invalid(
                    &p,
                    "empty-domain fallback must be finite and increasing",
                ));
            }
            if !(0.0..=1.0).contains(&s.padding_inner)
                || !s.padding_outer.is_finite()
                || s.padding_outer < 0.0
            {
                return Err(invalid(path, "invalid band padding"));
            }
            if let Range::Fixed(a, b) = s.range {
                if !a.is_finite() || !b.is_finite() {
                    return Err(invalid(path, "range must be finite"));
                }
            }
            if let Some((key, within)) = &s.sharing {
                nonempty(key, path)?;
                sharing(within, depth, path)?;
            }
        }
        let mut names = HashSet::new();
        for mark in &plot.marks {
            let p = format!("{path}/{}", mark.name);
            nonempty(&mark.name, &p)?;
            if !names.insert(&mark.name) {
                return Err(invalid(&p, "duplicate mark"));
            }
            let table = self.table(i, &mark.table, scope, &p)?;
            let (values, fill) = match &mark.encoding {
                Encoding::Rect(e) => {
                    if e.x.is_none()
                        || e.y.is_none()
                        || e.x2.is_some() == e.width.is_some()
                        || e.y2.is_some() == e.height.is_some()
                    {
                        return Err(invalid(
                            &p,
                            "rectangles need x/y and exactly one of end or size on each axis",
                        ));
                    }
                    (
                        vec![&e.x, &e.y, &e.x2, &e.y2, &e.width, &e.height]
                            .into_iter()
                            .flatten()
                            .collect::<Vec<_>>(),
                        &e.fill,
                    )
                }
                Encoding::Symbol(e) => {
                    if e.x.is_none() || e.y.is_none() {
                        return Err(invalid(&p, "symbols require x and y"));
                    }
                    (
                        vec![e.x.as_ref().unwrap(), e.y.as_ref().unwrap(), &e.size],
                        &e.fill,
                    )
                }
            };
            fill.parse::<css_color_parser::Color>()
                .map_err(|e| invalid(&p, e.to_string()))?;
            for v in values {
                if !self
                    .value(i, plot, v, &table.schema, scope, &p)?
                    .is_numeric()
                {
                    return Err(invalid(&p, "mark properties must evaluate to numbers"));
                }
            }
        }
        for axis in &plot.axes {
            local_scale(plot, &axis.scale, path)?;
            sharing(&axis.sharing, depth, path)?;
            if !axis.tick_count.is_finite() || axis.tick_count <= 0.0 {
                return Err(invalid(path, "tick count must be finite and positive"));
            }
        }
        Ok(())
    }
    fn value(
        &mut self,
        i: &DataflowInterface,
        plot: &Plot,
        v: &Value,
        schema: &DFSchema,
        scope: &[String],
        path: &str,
    ) -> Result<DataType> {
        let ty = match v {
            Value::Field(c) => column_type(schema, c, path)?.clone(),
            Value::Scalar(h) => self.scalar(i, h, scope, path)?,
            Value::Constant(n) => {
                if !n.is_finite() {
                    return Err(invalid(path, "numeric binding must be finite"));
                }
                DataType::Float64
            }
            Value::Scaled(handle, input) => {
                if !matches!(
                    input.as_ref(),
                    Value::Field(_) | Value::Scalar(_) | Value::Constant(_)
                ) {
                    return Err(invalid(
                        path,
                        "a scale input must be a field, scalar output, or constant",
                    ));
                }
                let s = local_scale(plot, handle, path)?;
                let expected = if s.kind == ScaleKind::Band {
                    Some(match &s.domain {
                        Domain::Column(h, c) => {
                            column_type(&i.table_metadata(h)?.schema, c, path)?.clone()
                        }
                        Domain::Values(v) => {
                            v.first().map(|v| v.data_type()).unwrap_or(DataType::Utf8)
                        }
                        _ => return Err(invalid(path, "invalid band domain")),
                    })
                } else {
                    None
                };
                let actual = self.value(i, plot, input, schema, scope, path)?;
                if expected
                    .as_ref()
                    .map_or(!actual.is_numeric(), |expected| expected != &actual)
                {
                    return Err(invalid(
                        path,
                        format!("incompatible scale input type {actual}"),
                    ));
                }
                DataType::Float32
            }
            Value::Bandwidth(h) => {
                if local_scale(plot, h, path)?.kind != ScaleKind::Band {
                    return Err(invalid(path, "bandwidth requires a band scale"));
                }
                DataType::Float32
            }
            Value::PlotWidth | Value::PlotHeight => DataType::Float32,
        };
        Ok(ty)
    }
}
fn band_type(t: &DataType) -> bool {
    matches!(
        t,
        DataType::Utf8 | DataType::Boolean | DataType::Int32 | DataType::Float32
    )
}
fn local_scale<'a>(p: &'a Plot, h: &ScaleHandle, path: &str) -> Result<&'a Scale> {
    if h.owner != p.identity {
        return Err(invalid(path, "scale belongs to another plot"));
    }
    p.scales
        .iter()
        .find(|(n, _)| n == &h.name)
        .map(|(_, s)| s)
        .ok_or_else(|| invalid(path, "unknown scale"))
}
fn sharing(scope: &PanelScope, depth: usize, path: &str) -> Result<()> {
    match scope {
        PanelScope::Group(_) => Err(invalid(
            path,
            "runtime GroupId is not a portable sharing scope",
        )),
        PanelScope::Ancestor(n) if n.get() > depth => {
            Err(invalid(path, "sharing ancestor is above the root"))
        }
        _ => Ok(()),
    }
}
fn arrangement(a: &Arrangement, names: &[&str], path: &str) -> Result<()> {
    finite(a.gap, path)?;
    finite(a.margin, path)?;
    if matches!(a.kind, ArrangementKind::Wrap(0)) {
        return Err(invalid(path, "wrap requires a positive column count"));
    }
    for track in a.columns.iter().chain(&a.rows) {
        match track {
            TrackSize::Fixed(v) | TrackSize::Flex(v) => finite(*v, path)?,
            _ => {}
        }
    }
    if let ArrangementKind::Grid {
        rows,
        columns,
        slots,
    } = &a.kind
    {
        let mut seen = HashSet::new();
        for (name, s) in slots {
            if !names.contains(&name.as_str())
                || !seen.insert(name)
                || s.row_span == 0
                || s.column_span == 0
                || s.row.checked_add(s.row_span).is_none_or(|n| n > *rows)
                || s.column
                    .checked_add(s.column_span)
                    .is_none_or(|n| n > *columns)
            {
                return Err(invalid(path, "invalid or duplicate grid slot"));
            }
        }
        if slots.len() != names.len() {
            return Err(invalid(path, "every child needs a grid slot"));
        }
        for (index, (_, a)) in slots.iter().enumerate() {
            for (_, b) in &slots[index + 1..] {
                if a.row < b.row + b.row_span
                    && b.row < a.row + a.row_span
                    && a.column < b.column + b.column_span
                    && b.column < a.column + a.column_span
                {
                    return Err(invalid(path, "overlapping grid slots"));
                }
            }
        }
    }
    Ok(())
}
