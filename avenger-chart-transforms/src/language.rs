//! Avenger-language schemas and lowerers owned by the transform crate.

use std::collections::{BTreeMap, BTreeSet};

use avenger_chart_core::{DataTransform, TimeContext, WeekStart};
use avenger_chart_lang_types::{
    LoweredTransform, NativeLoweringError, NativeOutputValue, ResolvedDeclaration, ResolvedValue,
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
    Sql, Stack, StackOffset, TimeFill, TimeLevel, TimeLevelKeys, TimeLevelLabel, TimeLevels,
    TimeUnit, TimeUnitPart,
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
        time_unit_definition(),
        time_levels_definition(),
        time_fill_definition(),
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

fn time_unit_definition() -> TransformLanguageDefinition {
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "time_unit"),
        "Discretize timestamps into calendar-aware interval boundaries.",
    )
    .property(
        "field",
        PropertySchema::required(ValueShape::SqlExpression, "The temporal input expression."),
    )
    .property(
        "maxbins",
        PropertySchema::optional(
            ValueShape::SqlExpression,
            "Requested maximum interval count.",
        ),
    )
    .property(
        "units",
        PropertySchema::optional(
            ValueShape::OneOrMany(Box::new(time_unit_shape())),
            "One calendar unit or an ordered array of units; overrides `maxbins`.",
        ),
    )
    .property(
        "time_context",
        PropertySchema::optional(time_context_shape(), "Timezone and week-start overrides."),
    )
    .property(
        "interval",
        PropertySchema::optional(
            ValueShape::Boolean,
            "Whether to emit interval end boundaries.",
        ),
    )
    .property(
        "name",
        PropertySchema::optional(
            ValueShape::String,
            "Base name for generated columns and state.",
        ),
    )
    .output(transform_output(
        "start",
        "Scaled interval start with temporal axis defaults.",
    ))
    .output(transform_output(
        "end",
        "Scaled interval end with temporal axis defaults.",
    ));
    TransformLanguageDefinition {
        schema,
        lowerer: |declaration, context| {
            let mut transform = TimeUnit::new(expr_property(declaration, "field")?);
            if let Some(value) = optional_expr(declaration, "maxbins")? {
                transform = transform.maxbins(value);
            }
            if let Some(value) = declaration.properties.get("units") {
                transform = transform.units(time_unit_parts(value)?);
            }
            if let Some(value) = declaration.properties.get("time_context") {
                transform = transform.time_context(time_context(value)?);
            }
            if let Some(ResolvedValue::Boolean(value)) = declaration.properties.get("interval") {
                transform = transform.interval(*value);
            }
            if let Some(name) = optional_string(declaration, "name")? {
                transform = transform.name(name);
            }
            let (transform, output) = transform.into_compiled_and_output(context)?;
            Ok(LoweredTransform {
                transform,
                outputs: [
                    ("start".to_string(), output.start().into()),
                    ("end".to_string(), output.end().into()),
                ]
                .into_iter()
                .collect(),
            })
        },
    }
}

fn time_levels_definition() -> TransformLanguageDefinition {
    let level_config = ValueShape::Object(
        [
            (
                "level".to_string(),
                PropertySchema::required(time_level_shape(), "Calendar hierarchy level."),
            ),
            (
                "label".to_string(),
                PropertySchema::optional(time_level_label_shape(), "Generated label style."),
            ),
            (
                "output_name".to_string(),
                PropertySchema::optional(ValueShape::String, "Generated key column name."),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "time_levels"),
        "Derive an ordered categorical calendar hierarchy from timestamps.",
    )
    .property(
        "field",
        PropertySchema::required(ValueShape::SqlExpression, "The temporal input expression."),
    )
    .property(
        "levels",
        PropertySchema::required(
            ValueShape::Array(Box::new(ValueShape::Union(vec![
                time_level_shape(),
                level_config,
            ]))),
            "Ordered calendar levels, either atoms or configured level objects.",
        ),
    )
    .property(
        "time_context",
        PropertySchema::optional(time_context_shape(), "Timezone and week-start overrides."),
    )
    .property(
        "name",
        PropertySchema::optional(ValueShape::String, "Base name for generated key columns."),
    )
    .output(TransformOutputSchema {
        name: "levels".to_string(),
        shape: ValueShape::Any,
        condition_property: None,
        docs: "Opaque hierarchy metadata consumable by `time_fill`.".to_string(),
    })
    .output(transform_output(
        "nested",
        "Nested categorical channel carrying hierarchy labels and ordering.",
    ))
    .dynamic_output(DynamicTransformOutputSchema {
        source: DynamicOutputSource::ArrayValueNames {
            property: "levels".to_string(),
        },
        shape: ValueShape::SqlExpression,
        docs: "Each atom level exposes its generated key as a same-named handle.".to_string(),
    })
    .dynamic_output(DynamicTransformOutputSchema {
        source: DynamicOutputSource::ArrayObjectField {
            property: "levels".to_string(),
            field: "level".to_string(),
        },
        shape: ValueShape::SqlExpression,
        docs: "Each configured level exposes its generated key by level name.".to_string(),
    });
    TransformLanguageDefinition {
        schema,
        lowerer: lower_time_levels,
    }
}

fn lower_time_levels(
    declaration: &ResolvedDeclaration,
    context: avenger_chart_core::DataTransformCompileContext,
) -> Result<LoweredTransform, NativeLoweringError> {
    let mut transform = TimeLevels::new(expr_property(declaration, "field")?);
    let ResolvedValue::Array(levels) = declaration.get("levels")? else {
        unreachable!("schema validation checks time levels")
    };
    let mut requested = Vec::new();
    for value in levels {
        let (level, label, output_name) = match value {
            ResolvedValue::String(level) => (parse_time_level(level)?, None, None),
            ResolvedValue::Object(fields) => {
                let level = parse_time_level(&object_string(fields, "level")?)?;
                let label = fields
                    .get("label")
                    .map(|value| match value {
                        ResolvedValue::String(value) => parse_time_level_label(value),
                        _ => Err(NativeLoweringError::InvalidPropertyType {
                            property: "label".to_string(),
                            expected: "time level label".to_string(),
                        }),
                    })
                    .transpose()?;
                let output_name = fields
                    .get("output_name")
                    .map(|value| match value {
                        ResolvedValue::String(value) => Ok(value.clone()),
                        _ => Err(NativeLoweringError::InvalidPropertyType {
                            property: "output_name".to_string(),
                            expected: "string".to_string(),
                        }),
                    })
                    .transpose()?;
                (level, label, output_name)
            }
            _ => unreachable!("schema validation checks time level entries"),
        };
        requested.push(level);
        transform = transform.level_with(level, |mut config| {
            if let Some(label) = label {
                config = config.label(label);
            }
            if let Some(output_name) = output_name {
                config = config.output_name(output_name);
            }
            config
        });
    }
    if let Some(value) = declaration.properties.get("time_context") {
        transform = transform.time_context(time_context(value)?);
    }
    if let Some(name) = optional_string(declaration, "name")? {
        transform = transform.name(name);
    }
    let (transform, output) = transform.into_compiled_and_output(context)?;
    let mut outputs = BTreeMap::from([
        (
            "levels".to_string(),
            NativeOutputValue::opaque(output.levels()),
        ),
        ("nested".to_string(), output.try_nested()?.into()),
    ]);
    for level in requested {
        outputs.insert(time_level_name(level).to_string(), output.key(level).into());
    }
    Ok(LoweredTransform { transform, outputs })
}

fn time_fill_definition() -> TransformLanguageDefinition {
    let extent = ValueShape::Object(
        [
            (
                "start".to_string(),
                PropertySchema::required(
                    ValueShape::Array(Box::new(ValueShape::SqlExpression)),
                    "Hierarchy-aligned lower extent components.",
                ),
            ),
            (
                "end".to_string(),
                PropertySchema::required(
                    ValueShape::Array(Box::new(ValueShape::SqlExpression)),
                    "Hierarchy-aligned upper extent components.",
                ),
            ),
        ]
        .into_iter()
        .collect(),
    );
    let schema = KindSchema::new(
        NativeKindKey::new(NativeKindNamespace::Transform, "time_fill"),
        "Complete missing calendar hierarchy rows and fill their values.",
    )
    .property(
        "field",
        PropertySchema::required(ValueShape::SqlExpression, "The value expression to fill."),
    )
    .property(
        "levels",
        PropertySchema::required(ValueShape::Any, "A `time_levels.levels` metadata handle."),
    )
    .property(
        "group_by",
        PropertySchema::optional(
            ValueShape::OneOrMany(Box::new(ValueShape::SqlExpression)),
            "Expressions defining independent completion groups.",
        ),
    )
    .property(
        "fill_value",
        PropertySchema::required(ValueShape::SqlExpression, "Value used for generated rows."),
    )
    .property(
        "extent",
        PropertySchema::optional(extent, "Optional explicit hierarchy-aligned extent."),
    )
    .property(
        "as_value",
        PropertySchema::optional(ValueShape::String, "Generated value column name."),
    )
    .property(
        "flag",
        PropertySchema::optional(ValueShape::String, "Optional generated-row flag column."),
    )
    .output(transform_output(
        "value",
        "The completed and filled value column.",
    ))
    .output(conditional_transform_output(
        "flag",
        "flag",
        "The generated-row flag when `flag` is configured.",
    ));
    TransformLanguageDefinition {
        schema,
        lowerer: lower_time_fill,
    }
}

fn lower_time_fill(
    declaration: &ResolvedDeclaration,
    context: avenger_chart_core::DataTransformCompileContext,
) -> Result<LoweredTransform, NativeLoweringError> {
    let levels = match declaration.get("levels")? {
        ResolvedValue::Output(output) => output
            .downcast_ref::<TimeLevelKeys>()
            .cloned()
            .ok_or_else(|| NativeLoweringError::InvalidPropertyType {
                property: "levels".to_string(),
                expected: "time_levels.levels output".to_string(),
            })?,
        _ => {
            return Err(NativeLoweringError::InvalidPropertyType {
                property: "levels".to_string(),
                expected: "time_levels.levels output".to_string(),
            });
        }
    };
    let mut transform = TimeFill::new(expr_property(declaration, "field")?)
        .levels(levels)
        .group_by(resolved_exprs(declaration.properties.get("group_by"))?)
        .fill_value(expr_property(declaration, "fill_value")?);
    if let Some(ResolvedValue::Object(extent)) = declaration.properties.get("extent") {
        let start = resolved_exprs(extent.get("start"))?;
        let end = resolved_exprs(extent.get("end"))?;
        transform = transform.extent(start, end);
    }
    if let Some(name) = optional_string(declaration, "as_value")? {
        transform = transform.as_value(name);
    }
    let has_flag = declaration.properties.contains_key("flag");
    if let Some(name) = optional_string(declaration, "flag")? {
        transform = transform.flag(name);
    }
    let (transform, output) = transform.into_compiled_and_output(context)?;
    let mut outputs = BTreeMap::from([("value".to_string(), output.value().into())]);
    if has_flag {
        outputs.insert("flag".to_string(), output.flag().into());
    }
    Ok(LoweredTransform { transform, outputs })
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

fn time_context_shape() -> ValueShape {
    ValueShape::Object(
        [
            (
                "timezone".to_string(),
                PropertySchema::optional(ValueShape::String, "IANA timezone name."),
            ),
            (
                "week_start".to_string(),
                PropertySchema::optional(
                    atom(&[
                        "sunday",
                        "monday",
                        "tuesday",
                        "wednesday",
                        "thursday",
                        "friday",
                        "saturday",
                    ]),
                    "First weekday used by calendar operations.",
                ),
            ),
        ]
        .into_iter()
        .collect(),
    )
}

fn time_context(value: &ResolvedValue) -> Result<TimeContext, NativeLoweringError> {
    let ResolvedValue::Object(fields) = value else {
        return Err(NativeLoweringError::InvalidPropertyType {
            property: "time_context".to_string(),
            expected: "time context object".to_string(),
        });
    };
    let mut context = TimeContext::new();
    if let Some(ResolvedValue::String(timezone)) = fields.get("timezone") {
        context = context.timezone(timezone);
    }
    if let Some(ResolvedValue::String(week_start)) = fields.get("week_start") {
        context = context.week_start(match week_start.as_str() {
            "sunday" => WeekStart::Sunday,
            "monday" => WeekStart::Monday,
            "tuesday" => WeekStart::Tuesday,
            "wednesday" => WeekStart::Wednesday,
            "thursday" => WeekStart::Thursday,
            "friday" => WeekStart::Friday,
            "saturday" => WeekStart::Saturday,
            _ => unreachable!("schema validation checks week start"),
        });
    }
    Ok(context)
}

fn time_unit_shape() -> ValueShape {
    atom(&[
        "year", "quarter", "month", "week", "day", "hour", "minute", "second",
    ])
}

fn time_unit_parts(value: &ResolvedValue) -> Result<Vec<TimeUnitPart>, NativeLoweringError> {
    let values = match value {
        ResolvedValue::Array(values) => values.iter().collect::<Vec<_>>(),
        value => vec![value],
    };
    values
        .into_iter()
        .map(|value| {
            let ResolvedValue::String(value) = value else {
                return Err(NativeLoweringError::InvalidPropertyType {
                    property: "units".to_string(),
                    expected: "time unit atom or array".to_string(),
                });
            };
            Ok(match value.as_str() {
                "year" => TimeUnitPart::Year,
                "quarter" => TimeUnitPart::Quarter,
                "month" => TimeUnitPart::Month,
                "week" => TimeUnitPart::Week,
                "day" => TimeUnitPart::Day,
                "hour" => TimeUnitPart::Hour,
                "minute" => TimeUnitPart::Minute,
                "second" => TimeUnitPart::Second,
                _ => unreachable!("schema validation checks time unit"),
            })
        })
        .collect()
}

fn time_level_shape() -> ValueShape {
    atom(&[
        "year",
        "quarter",
        "month",
        "day_of_month",
        "day_of_year",
        "hour",
        "minute",
    ])
}

fn time_level_label_shape() -> ValueShape {
    atom(&[
        "key",
        "year4",
        "quarter_short",
        "month_name",
        "month_abbrev",
    ])
}

fn parse_time_level(value: &str) -> Result<TimeLevel, NativeLoweringError> {
    Ok(match value {
        "year" => TimeLevel::Year,
        "quarter" => TimeLevel::Quarter,
        "month" => TimeLevel::Month,
        "day_of_month" => TimeLevel::DayOfMonth,
        "day_of_year" => TimeLevel::DayOfYear,
        "hour" => TimeLevel::Hour,
        "minute" => TimeLevel::Minute,
        _ => {
            return Err(NativeLoweringError::Lowering {
                kind: "time_levels".to_string(),
                message: format!("unsupported time level '{value}'"),
            });
        }
    })
}

fn time_level_name(value: TimeLevel) -> &'static str {
    match value {
        TimeLevel::Year => "year",
        TimeLevel::Quarter => "quarter",
        TimeLevel::Month => "month",
        TimeLevel::DayOfMonth => "day_of_month",
        TimeLevel::DayOfYear => "day_of_year",
        TimeLevel::Hour => "hour",
        TimeLevel::Minute => "minute",
        TimeLevel::Week => "week",
        TimeLevel::DayOfWeek => "day_of_week",
    }
}

fn parse_time_level_label(value: &str) -> Result<TimeLevelLabel, NativeLoweringError> {
    Ok(match value {
        "key" => TimeLevelLabel::Key,
        "year4" => TimeLevelLabel::Year4,
        "quarter_short" => TimeLevelLabel::QuarterShort,
        "month_name" => TimeLevelLabel::MonthName,
        "month_abbrev" => TimeLevelLabel::MonthAbbrev,
        _ => {
            return Err(NativeLoweringError::Lowering {
                kind: "time_levels".to_string(),
                message: format!("unsupported time level label '{value}'"),
            });
        }
    })
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
                "time_unit",
                "time_levels",
                "time_fill",
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

    #[test]
    fn temporal_metadata_flows_between_native_lowerers() {
        let context = DataTransformCompileContext::new(CoordinationScope::Free);
        let definitions = definitions();
        let time_levels = definitions
            .iter()
            .find(|definition| definition.schema.key.kind == "time_levels")
            .unwrap();
        let lowered_levels = (time_levels.lowerer)(
            &ResolvedDeclaration::new("time_levels")
                .property("field", ResolvedValue::Expr(col("date")))
                .property(
                    "levels",
                    ResolvedValue::Array(vec![
                        ResolvedValue::String("year".to_string()),
                        ResolvedValue::String("month".to_string()),
                    ]),
                ),
            context,
        )
        .unwrap();
        assert!(matches!(
            lowered_levels.outputs.get("nested"),
            Some(NativeOutputValue::Channel(_))
        ));
        let levels = lowered_levels.outputs.get("levels").unwrap().clone();

        let time_fill = definitions
            .iter()
            .find(|definition| definition.schema.key.kind == "time_fill")
            .unwrap();
        let lowered_fill = (time_fill.lowerer)(
            &ResolvedDeclaration::new("time_fill")
                .property("field", ResolvedValue::Expr(col("value")))
                .property("levels", ResolvedValue::Output(levels))
                .property("fill_value", ResolvedValue::Integer(0)),
            context,
        )
        .unwrap();
        assert!(matches!(
            lowered_fill.outputs.get("value"),
            Some(NativeOutputValue::Expr(_))
        ));
    }
}
