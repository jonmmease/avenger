//! Usage contexts and immutable bindings for caller-supplied row expressions.
use std::sync::{Arc, LazyLock};

use datafusion::{
    arrow::datatypes::{DataType, FieldRef},
    common::{
        tree_node::{Transformed, TreeNode, TreeNodeRecursion},
        Column, DFSchema, DFSchemaRef, Result as DFResult, ScalarValue,
    },
    logical_expr::{
        ColumnarValue, Expr, ExprSchemable, LogicalPlan, ReturnFieldArgs, ScalarFunctionArgs,
        ScalarUDF, ScalarUDFImpl, Signature, Volatility,
    },
    optimizer::analyzer::type_coercion::TypeCoercionRewriter,
};

use crate::{
    cache::BindingKey,
    graph::{reference::ScalarRef, GraphDef, InputKind},
    inputs::MaterializedValue,
    Error, Result,
};

#[derive(Clone, Debug)]
pub(crate) struct ExprSite {
    pub schema: DFSchemaRef,
    pub label: String,
}

/// Literal payloads use IPC so nested Arrow equality cannot erase floating-point bits.
#[derive(Debug, PartialEq, Eq, Hash)]
pub(crate) struct ExprKey {
    structure: Expr,
    literals: Vec<Arc<[u8]>>,
}
impl ExprKey {
    pub(crate) fn size(&self) -> usize {
        expression_size(&self.structure)
            + self
                .literals
                .iter()
                .map(|v| v.len() + std::mem::size_of::<Arc<[u8]>>())
                .sum::<usize>()
            + std::mem::size_of::<Self>()
    }
}

#[derive(Debug)]
pub(crate) struct BoundExpr {
    pub key: Arc<ExprKey>,
    forms: Vec<(DFSchemaRef, Expr)>,
}
impl BoundExpr {
    pub fn new(expr: Expr, sites: &[ExprSite], field: &FieldRef, name: &str) -> Result<Self> {
        validate_structure(&expr).map_err(|source| binding_error(name, "expression", source))?;
        let mut forms = Vec::new();
        for site in sites {
            let form = resolve(expr.clone(), &site.schema)
                .and_then(|expr| {
                    let actual = expr.get_type(site.schema.as_ref())?;
                    if &actual != field.data_type() {
                        return datafusion::common::plan_err!(
                            "expected {}, received {actual}",
                            field.data_type()
                        );
                    }
                    Ok(ScalarUDF::from(DeclaredField(field.clone())).call(vec![expr]))
                })
                .map_err(|source| binding_error(name, &site.label, source))?;
            forms.push((site.schema.clone(), form));
        }
        let mut literals = Vec::new();
        let structure = expr
            .transform_up(|expr| {
                if let Expr::Literal(value, metadata) = expr {
                    let BindingKey::Scalar(bytes) = MaterializedValue::Scalar(value)
                        .key()
                        .map_err(|e| datafusion::common::DataFusionError::External(Box::new(e)))?
                    else {
                        unreachable!()
                    };
                    literals.push(bytes);
                    Ok(Transformed::yes(Expr::Literal(ScalarValue::Null, metadata)))
                } else {
                    Ok(Transformed::no(expr))
                }
            })?
            .data;
        Ok(Self {
            key: Arc::new(ExprKey {
                structure,
                literals,
            }),
            forms,
        })
    }

    pub fn at(&self, schema: &DFSchema) -> DFResult<Expr> {
        self.forms
            .iter()
            .find(|(s, _)| s.as_ref() == schema)
            .map(|(_, expr)| expr.clone())
            .ok_or_else(|| {
                datafusion::common::DataFusionError::Internal(
                    "expression input lost its validated usage context".into(),
                )
            })
    }

    pub fn size(&self) -> usize {
        self.key.size()
            + self
                .forms
                .iter()
                .map(|(_, e)| expression_size(e))
                .sum::<usize>()
    }
}

fn binding_error(name: &str, context: &str, source: impl std::fmt::Display) -> Error {
    Error::InvalidExprInput {
        name: name.into(),
        context: context.into(),
        reason: source.to_string(),
    }
}

pub(crate) fn validate_structure(expr: &Expr) -> DFResult<()> {
    #[allow(deprecated)]
    expr.apply(|expr| {
        match expr {
            Expr::ScalarFunction(f) if f.func.signature().volatility != Volatility::Immutable => {
                return datafusion::common::plan_err!(
                    "function {} must be Immutable, received {:?}",
                    f.func.name(),
                    f.func.signature().volatility
                );
            }
            Expr::Alias(_)
            | Expr::Column(_)
            | Expr::Literal(..)
            | Expr::BinaryExpr(_)
            | Expr::Like(_)
            | Expr::SimilarTo(_)
            | Expr::Not(_)
            | Expr::IsNotNull(_)
            | Expr::IsNull(_)
            | Expr::IsTrue(_)
            | Expr::IsFalse(_)
            | Expr::IsUnknown(_)
            | Expr::IsNotTrue(_)
            | Expr::IsNotFalse(_)
            | Expr::IsNotUnknown(_)
            | Expr::Negative(_)
            | Expr::Between(_)
            | Expr::Case(_)
            | Expr::Cast(_)
            | Expr::TryCast(_)
            | Expr::ScalarFunction(_)
            | Expr::InList(_) => {}
            Expr::ScalarVariable(..)
            | Expr::AggregateFunction(_)
            | Expr::WindowFunction(_)
            | Expr::Exists(_)
            | Expr::InSubquery(_)
            | Expr::SetComparison(_)
            | Expr::ScalarSubquery(_)
            | Expr::Wildcard { .. }
            | Expr::GroupingSet(_)
            | Expr::Placeholder(_)
            | Expr::OuterReferenceColumn(..)
            | Expr::Unnest(_)
            | Expr::HigherOrderFunction(_)
            | Expr::Lambda(_)
            | Expr::LambdaVariable(_) => {
                return datafusion::common::plan_err!("unsupported expression input form: {expr}");
            }
        }
        Ok(TreeNodeRecursion::Continue)
    })?;
    Ok(())
}

fn resolve(expr: Expr, schema: &DFSchema) -> DFResult<Expr> {
    let expr = expr
        .transform_up(|expr| {
            if let Expr::Column(column) = expr {
                let (qualifier, field) = schema.qualified_field_from_column(&column)?;
                Ok(Transformed::yes(Expr::Column(Column::new(
                    qualifier.cloned(),
                    field.name(),
                ))))
            } else {
                Ok(Transformed::no(expr))
            }
        })?
        .data;
    Ok(expr.rewrite(&mut TypeCoercionRewriter::new(schema))?.data)
}

/// This wrapper keeps each placeholder's declared field stable after substitution.
#[derive(Debug, PartialEq, Eq, Hash)]
struct DeclaredField(FieldRef);
impl ScalarUDFImpl for DeclaredField {
    fn name(&self) -> &str {
        "__avenger_expr_input_field"
    }
    fn signature(&self) -> &Signature {
        static SIGNATURE: LazyLock<Signature> =
            LazyLock::new(|| Signature::any(1, Volatility::Immutable));
        &SIGNATURE
    }
    fn return_type(&self, _: &[DataType]) -> DFResult<DataType> {
        Ok(self.0.data_type().clone())
    }
    fn return_field_from_args(&self, _: ReturnFieldArgs<'_>) -> DFResult<FieldRef> {
        Ok(self.0.clone())
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> DFResult<ColumnarValue> {
        Ok(args.args[0].clone())
    }
}

pub(crate) fn collect_sites(graph: &GraphDef) -> Result<Vec<Vec<ExprSite>>> {
    let mut sites: Vec<Vec<ExprSite>> = vec![Vec::new(); graph.inputs.len()];
    for node in &graph.nodes {
        if !node
            .analysis
            .inputs
            .iter()
            .any(|i| matches!(graph.inputs[*i].kind, InputKind::Expr(_)))
        {
            continue;
        }
        let label = format!("{}::{}", graph.scope_name(node.scope), node.name);
        node.plan.apply_with_subqueries(|plan| {
            map_context_expressions(plan.clone(), |expr, schema| {
                expr.apply(|e| {
                    if let Expr::Placeholder(p) = e {
                        if let Some((ScalarRef::Input(index), _)) = graph.placeholders.get(&p.id) {
                            if matches!(graph.inputs[*index].kind, InputKind::Expr(_)) {
                                let schema = schema.ok_or_else(|| {
                                    datafusion::common::DataFusionError::Plan(format!(
                                        "expression input {} has an unsupported usage context at {label}",
                                        graph.inputs[*index].name
                                    ))
                                })?;
                                if !sites[*index].iter().any(|site| site.schema == *schema) {
                                    sites[*index].push(ExprSite {
                                        schema: schema.clone(),
                                        label: label.clone(),
                                    });
                                }
                            }
                        }
                    }
                    Ok(TreeNodeRecursion::Continue)
                })?;
                Ok(Transformed::no(expr))
            })?;
            Ok(TreeNodeRecursion::Continue)
        })?;
    }
    Ok(sites)
}

/// Use the same operator contexts during validation and substitution.
pub(crate) fn map_context_expressions(
    plan: LogicalPlan,
    mut f: impl FnMut(Expr, Option<&DFSchemaRef>) -> DFResult<Transformed<Expr>>,
) -> DFResult<Transformed<LogicalPlan>> {
    if let LogicalPlan::Join(mut join) = plan {
        let mut changed = false;
        for (left, right) in &mut join.on {
            let l = f(left.clone(), Some(join.left.schema()))?;
            let r = f(right.clone(), Some(join.right.schema()))?;
            changed |= l.transformed || r.transformed;
            *left = l.data;
            *right = r.data;
        }
        if let Some(filter) = join.filter.take() {
            let schema = Arc::new(join.left.schema().join(join.right.schema())?);
            let filter = f(filter, Some(&schema))?;
            changed |= filter.transformed;
            join.filter = Some(filter.data);
        }
        return Ok(Transformed::new(
            LogicalPlan::Join(join),
            changed,
            TreeNodeRecursion::Continue,
        ));
    }
    let schema = match &plan {
        LogicalPlan::Projection(p) => Some(p.input.schema().clone()),
        LogicalPlan::Filter(p) => Some(p.input.schema().clone()),
        LogicalPlan::Window(p) => Some(p.input.schema().clone()),
        LogicalPlan::Aggregate(p) => Some(p.input.schema().clone()),
        LogicalPlan::Sort(p) => Some(p.input.schema().clone()),
        LogicalPlan::Repartition(p) => Some(p.input.schema().clone()),
        LogicalPlan::Distinct(datafusion::logical_expr::Distinct::On(p)) => {
            Some(p.input.schema().clone())
        }
        LogicalPlan::TableScan(p) => Some(Arc::new(DFSchema::try_from_qualified_schema(
            p.table_name.clone(),
            &p.source.schema(),
        )?)),
        LogicalPlan::Values(_) | LogicalPlan::Limit(_) => Some(Arc::new(DFSchema::empty())),
        _ => None,
    };
    plan.map_expressions(|expr| f(expr, schema.as_ref()))
}

fn expression_size(expr: &Expr) -> usize {
    let mut size = 0;
    let _ = expr.apply(|node| {
        size += std::mem::size_of::<Expr>()
            + match node {
                Expr::Column(c) => {
                    c.name.len() + c.relation.as_ref().map_or(0, |r| r.to_string().len())
                }
                Expr::Alias(a) => {
                    a.name.len() + a.relation.as_ref().map_or(0, |r| r.to_string().len())
                }
                Expr::Literal(v, _) => v.size(),
                _ => 0,
            };
        Ok(TreeNodeRecursion::Continue)
    });
    size
}
