//! Avenger-language schemas and lowerers owned by the transform crate.

use std::collections::BTreeMap;

use avenger_chart_core::DataTransform;
use avenger_chart_lang_types::{
    LoweredTransform, NativeLoweringError, ResolvedDeclaration, ResolvedValue,
    TransformLanguageDefinition, expr_property, string_property,
};
use avenger_chart_schema::{
    EnumValueSchema, KindSchema, NativeKindKey, NativeKindNamespace, PropertySchema,
    TransformOutputSchema, ValueShape,
};
use datafusion::logical_expr::col;
use indexmap::IndexMap;

use crate::{Aggregate, Filter, Sql};

/// The transform definitions currently available to the native registry.
///
/// Phase 6 extends this inventory in this owner crate rather than introducing
/// transform-kind branches in the registry or compiler.
pub fn definitions() -> Vec<TransformLanguageDefinition> {
    vec![
        filter_definition(),
        aggregate_definition(),
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
            ValueShape::Array(Box::new(ValueShape::SqlExpression)),
            "Grouping expressions.",
        ),
    )
    .property(
        "measures",
        PropertySchema::required(
            ValueShape::Array(Box::new(measure)),
            "Named aggregate measures.",
        ),
    )
    .output(TransformOutputSchema {
        name: "fields".to_string(),
        shape: ValueShape::Object(BTreeMap::new()),
        docs: "Named output fields declared by group keys and measures.".to_string(),
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
    if let Some(ResolvedValue::Array(groups)) = declaration.properties.get("group_by") {
        for group in groups {
            let ResolvedValue::Expr(expr) = group else {
                unreachable!("schema validation checks group expressions")
            };
            aggregate = aggregate.group_by([expr.clone()]);
        }
    }
    let ResolvedValue::Array(measures) = declaration.get("measures")? else {
        unreachable!("schema validation checks aggregate measures")
    };
    let mut names = Vec::new();
    for measure in measures {
        let ResolvedValue::Object(fields) = measure else {
            unreachable!("schema validation checks aggregate measure objects")
        };
        let name = object_string(fields, "name")?;
        let op = object_string(fields, "op")?;
        let expr = fields
            .get("expr")
            .map(|value| match value {
                ResolvedValue::Expr(expr) => Ok(expr.clone()),
                _ => Err(NativeLoweringError::InvalidPropertyType {
                    property: "expr".to_string(),
                    expected: "SQL expression".to_string(),
                }),
            })
            .transpose()?;
        aggregate = match (op.as_str(), expr) {
            ("count", None) => aggregate.count(&name),
            ("sum", Some(expr)) => aggregate.sum(&name, expr),
            ("mean", Some(expr)) => aggregate.mean(&name, expr),
            ("min", Some(expr)) => aggregate.min(&name, expr),
            ("max", Some(expr)) => aggregate.max(&name, expr),
            ("median", Some(expr)) => aggregate.median(&name, expr),
            _ => {
                return Err(NativeLoweringError::Lowering {
                    kind: "aggregate".to_string(),
                    message: format!("operation '{op}' has an invalid expression shape"),
                });
            }
        };
        names.push(name);
    }
    let (transform, _output) = aggregate.into_compiled_and_output(context)?;
    Ok(LoweredTransform {
        transform,
        outputs: names
            .into_iter()
            .map(|name| (name.clone(), col(name)))
            .collect(),
    })
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

    #[test]
    fn definitions_are_complete_for_the_owner_slice_and_documented() {
        let definitions = definitions();
        assert_eq!(
            definitions
                .iter()
                .map(|definition| definition.schema.key.kind.as_str())
                .collect::<Vec<_>>(),
            ["filter", "aggregate", "sql"]
        );
        for definition in definitions {
            assert!(!definition.schema.docs.trim().is_empty());
        }
    }
}
