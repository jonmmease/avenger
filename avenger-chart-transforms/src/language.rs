//! Avenger-language schemas and lowerers owned by the transform crate.

use std::collections::{BTreeMap, BTreeSet};

use avenger_chart_core::DataTransform;
use avenger_chart_lang_types::{
    LoweredTransform, NativeLoweringError, ResolvedDeclaration, ResolvedValue,
    TransformLanguageDefinition, expr_property, resolved_expr, string_property,
};
use avenger_chart_schema::{
    DynamicOutputSource, DynamicTransformOutputSchema, EnumValueSchema, KindSchema, NativeKindKey,
    NativeKindNamespace, PropertySchema, TransformOutputSchema, ValueShape,
};
use datafusion::{
    common::ScalarValue,
    logical_expr::{Expr, col},
};
use indexmap::IndexMap;

use crate::{
    Aggregate, Bin, Calculate, Filter, Fold, Impute, JoinAggregate, Kde, KdeResolve, Lump, Select,
    Sql, Stack, StackOffset,
};

/// The transform definitions currently available to the native registry.
///
/// Phase 6 extends this inventory in this owner crate rather than introducing
/// transform-kind branches in the registry or compiler.
pub fn definitions() -> Vec<TransformLanguageDefinition> {
    vec![
        filter_definition(),
        aggregate_definition(),
        join_aggregate_definition(),
        calculate_definition(),
        select_definition(),
        fold_definition(),
        impute_definition(),
        kde_definition(),
        lump_definition(),
        bin_definition(),
        stack_definition(),
        sql_definition(),
    ]
}

fn filter_definition() -> TransformLanguageDefinition {
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "filter"),
        "Retain rows for which a predicate is true.",
    )
    .property(
        "predicate",
        PropertySchema::required(ValueShape::SqlExpression, "Boolean row predicate."),
    );
    TransformLanguageDefinition {
        schema,
        lowerer: |declaration, context| {
            let (transform, ()) = Filter::new(expr_property(declaration, "predicate")?)
                .into_compiled_and_output(context)?;
            Ok(LoweredTransform {
                transform,
                outputs: BTreeMap::new(),
            })
        },
    }
}

fn aggregate_definition() -> TransformLanguageDefinition {
    let operation = ValueShape::Atom {
        values: ["sum", "count", "mean", "min", "max", "median"]
            .into_iter()
            .map(|value| EnumValueSchema {
                value: value.to_string(),
                docs: format!("The `{value}` aggregation operation."),
            })
            .collect(),
    };
    let measure = ValueShape::Object(
        [
            (
                "name".to_string(),
                PropertySchema::required(ValueShape::String, "Output column name."),
            ),
            (
                "op".to_string(),
                PropertySchema::required(operation, "Aggregation operation."),
            ),
            (
                "expr".to_string(),
                PropertySchema::optional(
                    ValueShape::SqlExpression,
                    "Input expression; omitted for count.",
                ),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "aggregate"),
        "Group rows and compute named aggregate measures.",
    )
    .property(
        "group_by",
        PropertySchema::optional(
            ValueShape::OneOrMany(Box::new(ValueShape::SqlExpression)),
            "One grouping expression or an array of grouping expressions.",
        ),
    )
    .property(
        "measures",
        PropertySchema::optional(
            ValueShape::Array(Box::new(measure)),
            "Legacy structured named measures; user-named expression properties are preferred.",
        ),
    )
    .additional_properties(PropertySchema::optional(
        ValueShape::SqlExpression,
        "A user-named aggregate expression whose property name becomes the output handle.",
    ))
    .dynamic_output(DynamicTransformOutputSchema {
        source: DynamicOutputSource::PropertyNames {
            exclude: ["group_by".to_string(), "measures".to_string()]
                .into_iter()
                .collect(),
        },
        shape: ValueShape::SqlExpression,
        docs: "Each user-named aggregate expression exposes a same-named field handle.".to_string(),
    })
    .dynamic_output(DynamicTransformOutputSchema {
        source: DynamicOutputSource::ArrayObjectField {
            property: "measures".to_string(),
            field: "name".to_string(),
        },
        shape: ValueShape::SqlExpression,
        docs: "Each structured measure exposes the field named by its `name` member.".to_string(),
    });
    TransformLanguageDefinition {
        schema,
        lowerer: lower_aggregate,
    }
}

fn lower_aggregate(
    declaration: &ResolvedDeclaration,
    context: avenger_chart_core::DataTransformCompileContext,
) -> Result<LoweredTransform, NativeLoweringError> {
    let mut aggregate = Aggregate::new();
    aggregate = aggregate.group_by(resolved_exprs(declaration.properties.get("group_by"))?);
    let mut names = Vec::new();
    if let Some(ResolvedValue::Array(measures)) = declaration.properties.get("measures") {
        for measure in measures {
            let ResolvedValue::Object(fields) = measure else {
                unreachable!("schema validation checks aggregate measure objects")
            };
            let name = object_string(fields, "name")?;
            let op = object_string(fields, "op")?;
            let expr = optional_object_expr(fields, "expr")?;
            aggregate = apply_structured_measure(aggregate, &name, &op, expr, "aggregate")?;
            names.push(name);
        }
    }
    for (name, value) in declaration
        .properties
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "group_by" | "measures"))
    {
        let ResolvedValue::Expr(expr) = value else {
            unreachable!("schema validation checks aggregate expressions")
        };
        aggregate = apply_aggregate_expr(aggregate, name, expr.clone(), "aggregate")?;
        names.push(name.clone());
    }
    let (transform, _output) = aggregate.into_compiled_and_output(context)?;
    Ok(LoweredTransform {
        transform,
        outputs: names
            .into_iter()
            .map(|name| (name.clone(), col(name).into()))
            .collect(),
    })
}

fn join_aggregate_definition() -> TransformLanguageDefinition {
    let mut definition = aggregate_definition();
    definition.schema.key.kind = "join_aggregate".to_string();
    definition.schema.docs =
        "Compute grouped aggregate measures and join them back onto every input row.".to_string();
    definition.lowerer = lower_join_aggregate;
    definition
}

fn lower_join_aggregate(
    declaration: &ResolvedDeclaration,
    context: avenger_chart_core::DataTransformCompileContext,
) -> Result<LoweredTransform, NativeLoweringError> {
    let mut aggregate =
        JoinAggregate::new().group_by(resolved_exprs(declaration.properties.get("group_by"))?);
    let mut names = Vec::new();
    if let Some(ResolvedValue::Array(measures)) = declaration.properties.get("measures") {
        for measure in measures {
            let ResolvedValue::Object(fields) = measure else {
                unreachable!("schema validation checks join aggregate measure objects")
            };
            let name = object_string(fields, "name")?;
            aggregate = apply_structured_measure(
                aggregate,
                &name,
                &object_string(fields, "op")?,
                optional_object_expr(fields, "expr")?,
                "join_aggregate",
            )?;
            names.push(name);
        }
    }
    for (name, value) in declaration
        .properties
        .iter()
        .filter(|(name, _)| !matches!(name.as_str(), "group_by" | "measures"))
    {
        let ResolvedValue::Expr(expr) = value else {
            unreachable!("schema validation checks join aggregate expressions")
        };
        aggregate = apply_aggregate_expr(aggregate, name, expr.clone(), "join_aggregate")?;
        names.push(name.clone());
    }
    let (transform, ()) = aggregate.into_compiled_and_output(context)?;
    Ok(LoweredTransform {
        transform,
        outputs: names
            .into_iter()
            .map(|name| (name.clone(), col(name).into()))
            .collect(),
    })
}

fn calculate_definition() -> TransformLanguageDefinition {
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "calculate"),
        "Append user-named columns computed from row expressions.",
    )
    .additional_properties(PropertySchema::optional(
        ValueShape::SqlExpression,
        "A user-named row expression whose property name becomes the output column and handle.",
    ))
    .dynamic_output(DynamicTransformOutputSchema {
        source: DynamicOutputSource::PropertyNames {
            exclude: BTreeSet::new(),
        },
        shape: ValueShape::SqlExpression,
        docs: "Each expression exposes a same-named output field handle.".to_string(),
    });
    TransformLanguageDefinition {
        schema,
        lowerer: |declaration, context| {
            let mut calculate = Calculate::new();
            let mut outputs = BTreeMap::new();
            for (name, value) in &declaration.properties {
                let ResolvedValue::Expr(expr) = value else {
                    unreachable!("schema validation checks calculate expressions")
                };
                calculate = calculate.expr(name, expr.clone());
                outputs.insert(name.clone(), col(name).into());
            }
            let (transform, ()) = calculate.into_compiled_and_output(context)?;
            Ok(LoweredTransform { transform, outputs })
        },
    }
}

fn select_definition() -> TransformLanguageDefinition {
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "select"),
        "Project an ordered set of source columns and explicitly aliased expressions.",
    )
    .property(
        "expressions",
        PropertySchema::required(
            ValueShape::OneOrMany(Box::new(ValueShape::SqlExpression)),
            "One projection expression or an ordered array of projection expressions.",
        ),
    );
    TransformLanguageDefinition {
        schema,
        lowerer: |declaration, context| {
            let mut select = Select::new();
            for expression in resolved_exprs(declaration.properties.get("expressions"))? {
                select = select.expr(expression);
            }
            let (transform, ()) = select.into_compiled_and_output(context)?;
            Ok(LoweredTransform {
                transform,
                outputs: BTreeMap::new(),
            })
        },
    }
}

fn fold_definition() -> TransformLanguageDefinition {
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "fold"),
        "Turn a named set of source expressions into key/value rows.",
    )
    .property(
        "fields",
        PropertySchema::required(
            ValueShape::Map(Box::new(ValueShape::SqlExpression)),
            "A map from emitted key labels to source expressions.",
        ),
    )
    .property(
        "as_key",
        PropertySchema::optional(ValueShape::String, "Generated key column name."),
    )
    .property(
        "as_value",
        PropertySchema::optional(ValueShape::String, "Generated value column name."),
    )
    .property(
        "index",
        PropertySchema::optional(
            ValueShape::String,
            "Optional generated source-order column.",
        ),
    )
    .output(transform_output("key", "The generated field-key column."))
    .output(transform_output(
        "value",
        "The generated field-value column.",
    ))
    .output(conditional_transform_output(
        "index",
        "index",
        "The generated source-order column when `index` is configured.",
    ));
    TransformLanguageDefinition {
        schema,
        lowerer: |declaration, context| {
            let ResolvedValue::Object(fields) = declaration.get("fields")? else {
                unreachable!("schema validation checks fold fields")
            };
            let mut fold = Fold::new();
            for (key, value) in fields {
                let ResolvedValue::Expr(expr) = value else {
                    unreachable!("schema validation checks fold field expressions")
                };
                fold = fold.field(key, expr.clone());
            }
            if let Some(name) = optional_string(declaration, "as_key")? {
                fold = fold.as_key(name);
            }
            if let Some(name) = optional_string(declaration, "as_value")? {
                fold = fold.as_value(name);
            }
            let has_index = declaration.properties.contains_key("index");
            if let Some(name) = optional_string(declaration, "index")? {
                fold = fold.index(name);
            }
            let (transform, output) = fold.into_compiled_and_output(context)?;
            let mut outputs = BTreeMap::from([
                ("key".to_string(), output.key().into()),
                ("value".to_string(), output.value().into()),
            ]);
            if has_index {
                outputs.insert("index".to_string(), output.index().into());
            }
            Ok(LoweredTransform { transform, outputs })
        },
    }
}

fn impute_definition() -> TransformLanguageDefinition {
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "impute"),
        "Insert missing key rows and fill a value expression within groups.",
    )
    .property(
        "field",
        PropertySchema::required(ValueShape::SqlExpression, "The value expression to impute."),
    )
    .property(
        "key",
        PropertySchema::required(
            ValueShape::SqlExpression,
            "The key expression whose domain is completed.",
        ),
    )
    .property(
        "group_by",
        PropertySchema::optional(
            ValueShape::OneOrMany(Box::new(ValueShape::SqlExpression)),
            "Expressions defining independent imputation groups.",
        ),
    )
    .property(
        "method",
        PropertySchema::required(atom(&["value", "mean", "min", "max"]), "The fill strategy."),
    )
    .property(
        "fill_value",
        PropertySchema::optional(
            ValueShape::SqlExpression,
            "Fill expression required by the `value` method.",
        ),
    )
    .property(
        "as_value",
        PropertySchema::optional(ValueShape::String, "Generated value column name."),
    )
    .property(
        "flag",
        PropertySchema::optional(
            ValueShape::String,
            "Optional generated imputation flag column.",
        ),
    )
    .output(transform_output(
        "value",
        "The completed and imputed value column.",
    ))
    .output(conditional_transform_output(
        "flag",
        "flag",
        "The imputation flag when `flag` is configured.",
    ));
    TransformLanguageDefinition {
        schema,
        lowerer: |declaration, context| {
            let mut impute = Impute::new(expr_property(declaration, "field")?)
                .key(expr_property(declaration, "key")?)
                .group_by(resolved_exprs(declaration.properties.get("group_by"))?);
            let method = string_property(declaration, "method")?;
            impute =
                match method.as_str() {
                    "value" => impute.value(optional_expr(declaration, "fill_value")?.ok_or_else(
                        || NativeLoweringError::Lowering {
                            kind: "impute".to_string(),
                            message: "method `value` requires `fill_value`".to_string(),
                        },
                    )?),
                    "mean" => impute.mean(),
                    "min" => impute.min(),
                    "max" => impute.max(),
                    _ => unreachable!("schema validation checks impute method"),
                };
            if let Some(name) = optional_string(declaration, "as_value")? {
                impute = impute.as_value(name);
            }
            let has_flag = declaration.properties.contains_key("flag");
            if let Some(name) = optional_string(declaration, "flag")? {
                impute = impute.flag(name);
            }
            let (transform, output) = impute.into_compiled_and_output(context)?;
            let mut outputs = BTreeMap::from([("value".to_string(), output.value().into())]);
            if has_flag {
                outputs.insert("flag".to_string(), output.flag().into());
            }
            Ok(LoweredTransform { transform, outputs })
        },
    }
}

fn kde_definition() -> TransformLanguageDefinition {
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "kde"),
        "Estimate a one-dimensional kernel density, optionally by group.",
    )
    .property(
        "field",
        PropertySchema::required(
            ValueShape::SqlExpression,
            "The quantitative sample expression.",
        ),
    )
    .property(
        "group_by",
        PropertySchema::optional(
            ValueShape::OneOrMany(Box::new(ValueShape::SqlExpression)),
            "Simple columns defining independent density groups.",
        ),
    )
    .property(
        "bandwidth",
        PropertySchema::optional(
            ValueShape::SqlExpression,
            "Kernel bandwidth; zero selects an automatic value.",
        ),
    )
    .property(
        "counts",
        PropertySchema::optional(ValueShape::Boolean, "Scale density by group sample count."),
    )
    .property(
        "cumulative",
        PropertySchema::optional(ValueShape::Boolean, "Emit a cumulative density estimate."),
    )
    .property(
        "extent",
        PropertySchema::optional(
            ValueShape::Array(Box::new(ValueShape::SqlExpression)),
            "Two expressions defining the evaluation interval.",
        ),
    )
    .property(
        "resolve",
        PropertySchema::optional(
            atom(&["independent", "shared"]),
            "Whether groups use independent or shared evaluation domains.",
        ),
    )
    .property(
        "steps",
        PropertySchema::optional(ValueShape::SqlExpression, "Number of evaluation samples."),
    )
    .property(
        "as_fields",
        PropertySchema::optional(
            ValueShape::Array(Box::new(ValueShape::String)),
            "Two names for the generated value and density columns.",
        ),
    )
    .output(transform_output(
        "value",
        "The density evaluation position.",
    ))
    .output(transform_output(
        "density",
        "The estimated density at the evaluation position.",
    ));
    TransformLanguageDefinition {
        schema,
        lowerer: lower_kde,
    }
}

fn lower_kde(
    declaration: &ResolvedDeclaration,
    context: avenger_chart_core::DataTransformCompileContext,
) -> Result<LoweredTransform, NativeLoweringError> {
    let mut kde = Kde::new(expr_property(declaration, "field")?)
        .group_by(resolved_exprs(declaration.properties.get("group_by"))?);
    if let Some(value) = optional_expr(declaration, "bandwidth")? {
        kde = kde.bandwidth(value);
    }
    if let Some(ResolvedValue::Boolean(value)) = declaration.properties.get("counts") {
        kde = kde.counts(*value);
    }
    if let Some(ResolvedValue::Boolean(value)) = declaration.properties.get("cumulative") {
        kde = kde.cumulative(*value);
    }
    if let Some(values) = optional_expr_array(declaration, "extent")? {
        let [start, stop]: [Expr; 2] =
            values
                .try_into()
                .map_err(|_| NativeLoweringError::Lowering {
                    kind: "kde".to_string(),
                    message: "extent requires exactly two expressions".to_string(),
                })?;
        kde = kde.extent(start, stop);
    }
    if let Some(resolve) = optional_string(declaration, "resolve")? {
        kde = kde.resolve(match resolve.as_str() {
            "independent" => KdeResolve::Independent,
            "shared" => KdeResolve::Shared,
            _ => unreachable!("schema validation checks KDE resolution"),
        });
    }
    if let Some(value) = optional_expr(declaration, "steps")? {
        kde = kde.steps(value);
    }
    if let Some(names) = optional_strings(declaration, "as_fields")? {
        let [value, density]: [String; 2] =
            names
                .try_into()
                .map_err(|_| NativeLoweringError::Lowering {
                    kind: "kde".to_string(),
                    message: "as_fields requires exactly two names".to_string(),
                })?;
        kde = kde.as_fields(value, density);
    }
    let (transform, output) = kde.into_compiled_and_output(context)?;
    Ok(LoweredTransform {
        transform,
        outputs: [
            ("value".to_string(), output.value().into()),
            ("density".to_string(), output.density().into()),
        ]
        .into_iter()
        .collect(),
    })
}

fn lump_definition() -> TransformLanguageDefinition {
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "lump"),
        "Keep the highest-ranked categories and combine or drop the remainder.",
    )
    .property(
        "field",
        PropertySchema::required(ValueShape::SqlExpression, "The categorical value to rank."),
    )
    .property(
        "top_n",
        PropertySchema::required(
            ValueShape::SqlExpression,
            "Scalar number of categories to retain.",
        ),
    )
    .property(
        "order_by",
        PropertySchema::optional(ValueShape::SqlExpression, "Aggregate ranking expression."),
    )
    .property(
        "order",
        PropertySchema::optional(atom(&["asc", "desc"]), "Ranking direction."),
    )
    .property(
        "window",
        PropertySchema::optional(ValueShape::SqlExpression, "Window ranking expression."),
    )
    .property(
        "keep",
        PropertySchema::optional(
            ValueShape::SqlExpression,
            "Predicate identifying retained ranks.",
        ),
    )
    .property(
        "other",
        PropertySchema::optional(
            ValueShape::SqlExpression,
            "Replacement value for combined categories.",
        ),
    )
    .property(
        "drop_other",
        PropertySchema::optional(
            ValueShape::Boolean,
            "Drop categories outside the retained set.",
        ),
    )
    .property(
        "name",
        PropertySchema::optional(ValueShape::String, "Base name for generated columns."),
    )
    .output(transform_output(
        "value",
        "Scaled retained-or-combined category value.",
    ))
    .output(transform_output("rank", "Category rank."))
    .output(transform_output("measure", "Aggregate ranking measure."))
    .output(transform_output(
        "is_other",
        "Whether the row represents combined categories.",
    ))
    .output(transform_output("order", "Stable category ordering value."));
    TransformLanguageDefinition {
        schema,
        lowerer: |declaration, context| {
            let mut lump = Lump::top_n(
                expr_property(declaration, "field")?,
                expr_property(declaration, "top_n")?,
            );
            if let Some(value) = optional_expr(declaration, "order_by")? {
                lump = lump.order_by(value);
            }
            if let Some(order) = optional_string(declaration, "order")? {
                lump = if order == "asc" {
                    lump.order_asc()
                } else {
                    lump.order_desc()
                };
            }
            if let Some(value) = optional_expr(declaration, "window")? {
                lump = lump.window(value);
            }
            if let Some(value) = optional_expr(declaration, "keep")? {
                lump = lump.keep(value);
            }
            if let Some(value) = optional_expr(declaration, "other")? {
                lump = lump.other_value(value);
            }
            if matches!(
                declaration.properties.get("drop_other"),
                Some(ResolvedValue::Boolean(true))
            ) {
                lump = lump.drop_other();
            }
            if let Some(name) = optional_string(declaration, "name")? {
                lump = lump.name(name);
            }
            let (transform, output) = lump.into_compiled_and_output(context)?;
            Ok(LoweredTransform {
                transform,
                outputs: [
                    ("value".to_string(), output.value().into()),
                    ("rank".to_string(), output.rank().into()),
                    ("measure".to_string(), output.measure().into()),
                    ("is_other".to_string(), output.is_other().into()),
                    ("order".to_string(), output.order().into()),
                ]
                .into_iter()
                .collect(),
            })
        },
    }
}

fn bin_definition() -> TransformLanguageDefinition {
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "bin"),
        "Discretize a quantitative field into stable interval columns.",
    )
    .property(
        "field",
        PropertySchema::required(
            ValueShape::SqlExpression,
            "The quantitative input expression.",
        ),
    )
    .property(
        "maxbins",
        PropertySchema::optional(ValueShape::SqlExpression, "Requested maximum bin count."),
    )
    .property(
        "nice",
        PropertySchema::optional(
            ValueShape::Boolean,
            "Whether to choose pleasant boundaries.",
        ),
    )
    .property(
        "base",
        PropertySchema::optional(ValueShape::Integer, "Radix used to choose candidate steps."),
    )
    .property(
        "divide",
        PropertySchema::optional(
            ValueShape::Array(Box::new(ValueShape::Integer)),
            "Positive divisors used to refine candidate steps.",
        ),
    )
    .property(
        "steps",
        PropertySchema::optional(
            ValueShape::Array(Box::new(ValueShape::Number)),
            "Explicit positive candidate steps.",
        ),
    )
    .property(
        "minstep",
        PropertySchema::optional(ValueShape::SqlExpression, "Minimum allowed step."),
    )
    .property(
        "step",
        PropertySchema::optional(ValueShape::SqlExpression, "Exact requested step."),
    )
    .property(
        "extent",
        PropertySchema::optional(
            ValueShape::Array(Box::new(ValueShape::SqlExpression)),
            "Two expressions defining the input extent.",
        ),
    )
    .property(
        "span",
        PropertySchema::optional(ValueShape::SqlExpression, "Optional extent span override."),
    )
    .property(
        "anchor",
        PropertySchema::optional(ValueShape::SqlExpression, "Optional boundary anchor."),
    )
    .property(
        "name",
        PropertySchema::optional(
            ValueShape::String,
            "Base name for generated columns and state.",
        ),
    )
    .output(transform_output("start", "Scaled lower interval boundary."))
    .output(transform_output("end", "Scaled upper interval boundary."))
    .output(transform_output("index", "Zero-based interval index."));
    TransformLanguageDefinition {
        schema,
        lowerer: lower_bin,
    }
}

fn lower_bin(
    declaration: &ResolvedDeclaration,
    context: avenger_chart_core::DataTransformCompileContext,
) -> Result<LoweredTransform, NativeLoweringError> {
    let mut bin = Bin::new(expr_property(declaration, "field")?);
    if let Some(value) = optional_expr(declaration, "maxbins")? {
        bin = bin.maxbins(value);
    }
    if matches!(
        declaration.properties.get("nice"),
        Some(ResolvedValue::Boolean(false))
    ) {
        bin = bin.exact();
    }
    if let Some(value) = optional_integer(declaration, "base")? {
        bin = bin.base(value as usize);
    }
    if let Some(values) = optional_integers(declaration, "divide")? {
        bin = bin.divide(values.into_iter().map(|value| value as usize));
    }
    if let Some(values) = optional_numbers(declaration, "steps")? {
        bin = bin.steps(values);
    }
    if let Some(value) = optional_expr(declaration, "minstep")? {
        bin = bin.minstep(value);
    }
    if let Some(value) = optional_expr(declaration, "step")? {
        bin = bin.step(value);
    }
    if let Some(values) = optional_expr_array(declaration, "extent")? {
        let [start, stop]: [Expr; 2] =
            values
                .try_into()
                .map_err(|_| NativeLoweringError::Lowering {
                    kind: "bin".to_string(),
                    message: "extent requires exactly two expressions".to_string(),
                })?;
        bin = bin.extent(start, stop);
    }
    if let Some(value) = optional_expr(declaration, "span")? {
        bin = bin.span(value);
    }
    if let Some(value) = optional_expr(declaration, "anchor")? {
        bin = bin.anchor(value);
    }
    if let Some(value) = optional_string(declaration, "name")? {
        bin = bin.name(value);
    }
    let (transform, output) = bin.into_compiled_and_output(context)?;
    Ok(LoweredTransform {
        transform,
        outputs: [
            ("start".to_string(), output.start().into()),
            ("end".to_string(), output.end().into()),
            ("index".to_string(), output.index().into()),
        ]
        .into_iter()
        .collect(),
    })
}

fn stack_definition() -> TransformLanguageDefinition {
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "stack"),
        "Compute stacked start and end positions for a quantitative field.",
    )
    .property(
        "field",
        PropertySchema::required(
            ValueShape::SqlExpression,
            "The quantitative value to stack.",
        ),
    )
    .property(
        "group_by",
        PropertySchema::optional(
            ValueShape::OneOrMany(Box::new(ValueShape::SqlExpression)),
            "Expressions that partition independent stacks.",
        ),
    )
    .property(
        "sort_by",
        PropertySchema::optional(
            ValueShape::OneOrMany(Box::new(ValueShape::SqlExpression)),
            "Expressions that order rows within each stack.",
        ),
    )
    .property(
        "offset",
        PropertySchema::optional(
            atom(&["zero", "normalize", "center"]),
            "Stack baseline and normalization mode.",
        ),
    )
    .property(
        "name",
        PropertySchema::optional(
            ValueShape::String,
            "Base name for generated boundary columns.",
        ),
    )
    .property(
        "value_name",
        PropertySchema::optional(
            ValueShape::String,
            "Optional copied value output column name.",
        ),
    )
    .output(transform_output("start", "Scaled lower stacked position."))
    .output(transform_output("end", "Scaled upper stacked position."))
    .output(transform_output(
        "mid",
        "Scaled midpoint of the stacked interval.",
    ))
    .output(transform_output(
        "value",
        "The configured value output or upper boundary.",
    ));
    TransformLanguageDefinition {
        schema,
        lowerer: |declaration, context| {
            let mut stack = Stack::new(expr_property(declaration, "field")?)
                .group_by(resolved_exprs(declaration.properties.get("group_by"))?)
                .sort_by_exprs(resolved_exprs(declaration.properties.get("sort_by"))?);
            if let Some(offset) = optional_string(declaration, "offset")? {
                stack = stack.offset(match offset.as_str() {
                    "zero" => StackOffset::Zero,
                    "normalize" => StackOffset::Normalize,
                    "center" => StackOffset::Center,
                    _ => unreachable!("schema validation checks stack offset"),
                });
            }
            if let Some(name) = optional_string(declaration, "name")? {
                stack = stack.name(name);
            }
            if let Some(name) = optional_string(declaration, "value_name")? {
                stack = stack.value_name(name);
            }
            let (transform, output) = stack.into_compiled_and_output(context)?;
            Ok(LoweredTransform {
                transform,
                outputs: [
                    ("start".to_string(), output.start().into()),
                    ("end".to_string(), output.end().into()),
                    ("mid".to_string(), output.mid().into()),
                    ("value".to_string(), output.value().into()),
                ]
                .into_iter()
                .collect(),
            })
        },
    }
}

fn sql_definition() -> TransformLanguageDefinition {
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "sql"),
        "Run one DataFusion SQL query against the reserved `input` relation.",
    )
    .property(
        "query",
        PropertySchema::required(ValueShape::SqlQuery, "The SQL query."),
    )
    .output(TransformOutputSchema {
        name: "fields".to_string(),
        shape: ValueShape::Object(BTreeMap::new()),
        condition_property: None,
        docs: "Fields projected by the SQL query.".to_string(),
    });
    TransformLanguageDefinition {
        schema,
        lowerer: |declaration, context| {
            let (transform, _output) = Sql::new(string_property(declaration, "query")?)
                .into_compiled_and_output(context)?;
            Ok(LoweredTransform {
                transform,
                outputs: BTreeMap::new(),
            })
        },
    }
}

trait AggregateMeasureBuilder: Sized {
    fn count(self, name: &str) -> Self;
    fn sum(self, name: &str, expr: Expr) -> Self;
    fn mean(self, name: &str, expr: Expr) -> Self;
    fn min(self, name: &str, expr: Expr) -> Self;
    fn max(self, name: &str, expr: Expr) -> Self;
    fn median(self, name: &str, expr: Expr) -> Self;
    fn approx_percentile_cont(self, name: &str, expr: Expr, percentile: f64) -> Self;
    fn approx_percentile_cont_with_centroids(
        self,
        name: &str,
        expr: Expr,
        percentile: f64,
        centroids: u32,
    ) -> Self;
}

macro_rules! aggregate_measure_builder {
    ($type:ty) => {
        impl AggregateMeasureBuilder for $type {
            fn count(self, name: &str) -> Self {
                self.count(name)
            }
            fn sum(self, name: &str, expr: Expr) -> Self {
                self.sum(name, expr)
            }
            fn mean(self, name: &str, expr: Expr) -> Self {
                self.mean(name, expr)
            }
            fn min(self, name: &str, expr: Expr) -> Self {
                self.min(name, expr)
            }
            fn max(self, name: &str, expr: Expr) -> Self {
                self.max(name, expr)
            }
            fn median(self, name: &str, expr: Expr) -> Self {
                self.median(name, expr)
            }
            fn approx_percentile_cont(self, name: &str, expr: Expr, percentile: f64) -> Self {
                self.approx_percentile_cont(name, expr, percentile)
            }
            fn approx_percentile_cont_with_centroids(
                self,
                name: &str,
                expr: Expr,
                percentile: f64,
                centroids: u32,
            ) -> Self {
                self.approx_percentile_cont_with_centroids(name, expr, percentile, centroids)
            }
        }
    };
}

aggregate_measure_builder!(Aggregate);
aggregate_measure_builder!(JoinAggregate);

fn apply_structured_measure<B: AggregateMeasureBuilder>(
    builder: B,
    name: &str,
    operation: &str,
    expr: Option<Expr>,
    kind: &str,
) -> Result<B, NativeLoweringError> {
    match (operation, expr) {
        ("count", None) => Ok(builder.count(name)),
        ("sum", Some(expr)) => Ok(builder.sum(name, expr)),
        ("mean", Some(expr)) => Ok(builder.mean(name, expr)),
        ("min", Some(expr)) => Ok(builder.min(name, expr)),
        ("max", Some(expr)) => Ok(builder.max(name, expr)),
        ("median", Some(expr)) => Ok(builder.median(name, expr)),
        _ => Err(NativeLoweringError::Lowering {
            kind: kind.to_string(),
            message: format!("operation '{operation}' has an invalid expression shape"),
        }),
    }
}

fn apply_aggregate_expr<B: AggregateMeasureBuilder>(
    builder: B,
    name: &str,
    expr: Expr,
    kind: &str,
) -> Result<B, NativeLoweringError> {
    let Expr::AggregateFunction(function) = expr else {
        return Err(NativeLoweringError::Lowering {
            kind: kind.to_string(),
            message: format!("user-named measure '{name}' must be one aggregate function call"),
        });
    };
    if function.params.distinct
        || function.params.filter.is_some()
        || !function.params.order_by.is_empty()
        || function.params.null_treatment.is_some()
    {
        return Err(NativeLoweringError::Lowering {
            kind: kind.to_string(),
            message: format!(
                "aggregate modifiers on measure '{name}' are not supported by this native transform"
            ),
        });
    }
    let function_name = function.func.name().to_ascii_lowercase();
    let args = function.params.args;
    let first = || {
        args.first()
            .cloned()
            .ok_or_else(|| NativeLoweringError::Lowering {
                kind: kind.to_string(),
                message: format!("aggregate function '{function_name}' requires an argument"),
            })
    };
    match function_name.as_str() {
        "count" => Ok(builder.count(name)),
        "sum" => Ok(builder.sum(name, first()?)),
        "avg" | "mean" => Ok(builder.mean(name, first()?)),
        "min" => Ok(builder.min(name, first()?)),
        "max" => Ok(builder.max(name, first()?)),
        "median" => Ok(builder.median(name, first()?)),
        "approx_percentile_cont" => {
            let value = first()?;
            let percentile =
                args.get(1)
                    .and_then(literal_f64)
                    .ok_or_else(|| NativeLoweringError::Lowering {
                        kind: kind.to_string(),
                        message: format!(
                            "aggregate measure '{name}' requires a literal percentile argument"
                        ),
                    })?;
            if let Some(centroids) = args.get(2).and_then(literal_u32) {
                Ok(builder
                    .approx_percentile_cont_with_centroids(name, value, percentile, centroids))
            } else {
                Ok(builder.approx_percentile_cont(name, value, percentile))
            }
        }
        _ => Err(NativeLoweringError::Lowering {
            kind: kind.to_string(),
            message: format!("aggregate function '{function_name}' is not supported"),
        }),
    }
}

fn literal_f64(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Literal(ScalarValue::Float64(Some(value)), _) => Some(*value),
        Expr::Literal(ScalarValue::Float32(Some(value)), _) => Some(*value as f64),
        Expr::Literal(ScalarValue::Int64(Some(value)), _) => Some(*value as f64),
        Expr::Literal(ScalarValue::Int32(Some(value)), _) => Some(*value as f64),
        _ => None,
    }
}

fn literal_u32(expr: &Expr) -> Option<u32> {
    match expr {
        Expr::Literal(ScalarValue::UInt32(Some(value)), _) => Some(*value),
        Expr::Literal(ScalarValue::UInt64(Some(value)), _) => (*value).try_into().ok(),
        Expr::Literal(ScalarValue::Int64(Some(value)), _) => (*value).try_into().ok(),
        Expr::Literal(ScalarValue::Int32(Some(value)), _) => (*value).try_into().ok(),
        _ => None,
    }
}

fn resolved_exprs(value: Option<&ResolvedValue>) -> Result<Vec<Expr>, NativeLoweringError> {
    match value {
        None => Ok(Vec::new()),
        Some(ResolvedValue::Array(values)) => values
            .iter()
            .map(|value| {
                resolved_expr(value).ok_or_else(|| NativeLoweringError::InvalidPropertyType {
                    property: "expression array".to_string(),
                    expected: "SQL expression".to_string(),
                })
            })
            .collect(),
        Some(value) => resolved_expr(value)
            .map(|value| vec![value])
            .ok_or_else(|| NativeLoweringError::InvalidPropertyType {
                property: "expression".to_string(),
                expected: "SQL expression or array".to_string(),
            }),
    }
}

fn optional_expr(
    declaration: &ResolvedDeclaration,
    name: &str,
) -> Result<Option<Expr>, NativeLoweringError> {
    declaration
        .properties
        .get(name)
        .map(|value| {
            resolved_expr(value).ok_or_else(|| NativeLoweringError::InvalidPropertyType {
                property: name.to_string(),
                expected: "SQL expression".to_string(),
            })
        })
        .transpose()
}

fn optional_expr_array(
    declaration: &ResolvedDeclaration,
    name: &str,
) -> Result<Option<Vec<Expr>>, NativeLoweringError> {
    declaration
        .properties
        .get(name)
        .map(|value| resolved_exprs(Some(value)))
        .transpose()
}

fn optional_string(
    declaration: &ResolvedDeclaration,
    name: &str,
) -> Result<Option<String>, NativeLoweringError> {
    declaration
        .properties
        .get(name)
        .map(|value| match value {
            ResolvedValue::String(value) => Ok(value.clone()),
            _ => Err(NativeLoweringError::InvalidPropertyType {
                property: name.to_string(),
                expected: "string".to_string(),
            }),
        })
        .transpose()
}

fn optional_integer(
    declaration: &ResolvedDeclaration,
    name: &str,
) -> Result<Option<i64>, NativeLoweringError> {
    declaration
        .properties
        .get(name)
        .map(|value| match value {
            ResolvedValue::Integer(value) => Ok(*value),
            _ => Err(NativeLoweringError::InvalidPropertyType {
                property: name.to_string(),
                expected: "integer".to_string(),
            }),
        })
        .transpose()
}

fn optional_integers(
    declaration: &ResolvedDeclaration,
    name: &str,
) -> Result<Option<Vec<i64>>, NativeLoweringError> {
    declaration
        .properties
        .get(name)
        .map(|value| {
            let ResolvedValue::Array(values) = value else {
                return Err(NativeLoweringError::InvalidPropertyType {
                    property: name.to_string(),
                    expected: "integer array".to_string(),
                });
            };
            values
                .iter()
                .map(|value| match value {
                    ResolvedValue::Integer(value) => Ok(*value),
                    _ => Err(NativeLoweringError::InvalidPropertyType {
                        property: name.to_string(),
                        expected: "integer array".to_string(),
                    }),
                })
                .collect()
        })
        .transpose()
}

fn optional_numbers(
    declaration: &ResolvedDeclaration,
    name: &str,
) -> Result<Option<Vec<f64>>, NativeLoweringError> {
    declaration
        .properties
        .get(name)
        .map(|value| {
            let ResolvedValue::Array(values) = value else {
                return Err(NativeLoweringError::InvalidPropertyType {
                    property: name.to_string(),
                    expected: "number array".to_string(),
                });
            };
            values
                .iter()
                .map(|value| match value {
                    ResolvedValue::Number(value) => Ok(*value),
                    ResolvedValue::Integer(value) => Ok(*value as f64),
                    _ => Err(NativeLoweringError::InvalidPropertyType {
                        property: name.to_string(),
                        expected: "number array".to_string(),
                    }),
                })
                .collect()
        })
        .transpose()
}

fn optional_strings(
    declaration: &ResolvedDeclaration,
    name: &str,
) -> Result<Option<Vec<String>>, NativeLoweringError> {
    declaration
        .properties
        .get(name)
        .map(|value| {
            let ResolvedValue::Array(values) = value else {
                return Err(NativeLoweringError::InvalidPropertyType {
                    property: name.to_string(),
                    expected: "string array".to_string(),
                });
            };
            values
                .iter()
                .map(|value| match value {
                    ResolvedValue::String(value) => Ok(value.clone()),
                    _ => Err(NativeLoweringError::InvalidPropertyType {
                        property: name.to_string(),
                        expected: "string array".to_string(),
                    }),
                })
                .collect()
        })
        .transpose()
}

fn optional_object_expr(
    fields: &IndexMap<String, ResolvedValue>,
    name: &str,
) -> Result<Option<Expr>, NativeLoweringError> {
    fields
        .get(name)
        .map(|value| match value {
            ResolvedValue::Expr(expr) => Ok(expr.clone()),
            _ => Err(NativeLoweringError::InvalidPropertyType {
                property: name.to_string(),
                expected: "SQL expression".to_string(),
            }),
        })
        .transpose()
}

fn transform_output(name: &str, docs: &str) -> TransformOutputSchema {
    TransformOutputSchema {
        name: name.to_string(),
        shape: ValueShape::SqlExpression,
        condition_property: None,
        docs: docs.to_string(),
    }
}

fn conditional_transform_output(
    name: &str,
    condition_property: &str,
    docs: &str,
) -> TransformOutputSchema {
    TransformOutputSchema {
        name: name.to_string(),
        shape: ValueShape::SqlExpression,
        condition_property: Some(condition_property.to_string()),
        docs: docs.to_string(),
    }
}

fn atom(values: &[&str]) -> ValueShape {
    ValueShape::Atom {
        values: values
            .iter()
            .map(|value| EnumValueSchema {
                value: (*value).to_string(),
                docs: format!("Use the `{value}` mode."),
            })
            .collect(),
    }
}

fn object_string(
    fields: &IndexMap<String, ResolvedValue>,
    name: &str,
) -> Result<String, NativeLoweringError> {
    match fields.get(name) {
        Some(ResolvedValue::String(value)) => Ok(value.clone()),
        _ => Err(NativeLoweringError::InvalidPropertyType {
            property: name.to_string(),
            expected: "string".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_chart_core::{CoordinationScope, DataTransformCompileContext};
    use avenger_chart_lang_types::NativeOutputValue;
    use avenger_chart_schema::{NativeSchemaSnapshot, SchemaVersion};
    use datafusion::functions_aggregate::expr_fn::sum;
    use datafusion::logical_expr::lit;

    #[test]
    fn definitions_are_complete_for_the_owner_slice_and_documented() {
        let definitions = definitions();
        assert_eq!(
            definitions
                .iter()
                .map(|definition| definition.schema.key.kind.as_str())
                .collect::<Vec<_>>(),
            [
                "filter",
                "aggregate",
                "join_aggregate",
                "calculate",
                "select",
                "fold",
                "impute",
                "kde",
                "lump",
                "bin",
                "stack",
                "sql"
            ]
        );
        let entries = definitions
            .into_iter()
            .map(|definition| (definition.schema.key.clone(), definition.schema))
            .collect();
        NativeSchemaSnapshot {
            version: SchemaVersion::V1,
            profile_label: "transform-owner-test".to_string(),
            entries,
        }
        .validate_docs()
        .unwrap();
    }

    #[test]
    fn dynamic_and_channel_outputs_survive_owner_lowering() {
        let context = DataTransformCompileContext::new(CoordinationScope::Free);
        let definitions = definitions();
        let calculate = definitions
            .iter()
            .find(|definition| definition.schema.key.kind == "calculate")
            .unwrap();
        let lowered = (calculate.lowerer)(
            &ResolvedDeclaration::new("calculate")
                .property("double", ResolvedValue::Expr(col("value") * lit(2))),
            context,
        )
        .unwrap();
        assert!(matches!(
            lowered.outputs.get("double"),
            Some(NativeOutputValue::Expr(_))
        ));

        let aggregate = definitions
            .iter()
            .find(|definition| definition.schema.key.kind == "aggregate")
            .unwrap();
        let lowered = (aggregate.lowerer)(
            &ResolvedDeclaration::new("aggregate")
                .property("total", ResolvedValue::Expr(sum(col("value")))),
            context,
        )
        .unwrap();
        assert!(matches!(
            lowered.outputs.get("total"),
            Some(NativeOutputValue::Expr(_))
        ));

        let bin = definitions
            .iter()
            .find(|definition| definition.schema.key.kind == "bin")
            .unwrap();
        let lowered = (bin.lowerer)(
            &ResolvedDeclaration::new("bin")
                .property("field", ResolvedValue::Expr(col("value")))
                .property("maxbins", ResolvedValue::Expr(lit(20))),
            context,
        )
        .unwrap();
        assert!(matches!(
            lowered.outputs.get("start"),
            Some(NativeOutputValue::Channel(_))
        ));
        assert!(matches!(
            lowered.outputs.get("index"),
            Some(NativeOutputValue::Expr(_))
        ));
    }
}
