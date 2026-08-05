//! Avenger-language schemas and erased lowerers for built-in scales.

use avenger_chart_core::{Auto, Scale};
use avenger_chart_lang_types::{
    NativeLoweringError, ObjectLanguageDefinition, ResolvedDeclaration, ResolvedValue,
    resolved_expr,
};
use avenger_chart_schema::{
    EnumValueSchema, KindSchema, NativeKindKey, NativeKindNamespace, PropertySchema, ValueShape,
};
use datafusion::common::ScalarValue;
use palette::Srgba;

use crate::{
    Band, Linear, Log, NestedBand, Ordinal, Point, Pow, Quantile, Quantize, Sqrt, Symlog,
    Threshold, Time,
};

/// Complete stock scale inventory.
pub fn definitions() -> Vec<ObjectLanguageDefinition> {
    vec![
        definition(
            "linear",
            "A continuous linear scale.",
            linear_schema,
            lower_linear,
        ),
        definition(
            "log",
            "A logarithmic continuous scale.",
            log_schema,
            lower_log,
        ),
        definition(
            "pow",
            "A power-transformed continuous scale.",
            pow_schema,
            lower_pow,
        ),
        definition(
            "sqrt",
            "A square-root continuous scale.",
            sqrt_schema,
            lower_sqrt,
        ),
        definition(
            "symlog",
            "A symmetric-log continuous scale.",
            symlog_schema,
            lower_symlog,
        ),
        definition(
            "time",
            "A temporal continuous scale.",
            time_schema,
            lower_time,
        ),
        definition(
            "band",
            "A discrete band-position scale.",
            band_schema,
            lower_band,
        ),
        definition(
            "nested_band",
            "A hierarchical discrete band-position scale.",
            identity_schema,
            lower_nested_band,
        ),
        definition(
            "point",
            "A discrete point-position scale.",
            point_schema,
            lower_point,
        ),
        definition(
            "ordinal",
            "A discrete ordinal scale.",
            ordinal_schema,
            lower_ordinal,
        ),
        definition(
            "threshold",
            "A discrete threshold scale.",
            identity_schema,
            lower_threshold,
        ),
        definition(
            "quantile",
            "A quantile scale derived from a sample domain.",
            identity_schema,
            lower_quantile,
        ),
        definition(
            "quantize",
            "A uniformly quantized continuous-domain scale.",
            identity_schema,
            lower_quantize,
        ),
    ]
}

fn definition(
    kind: &'static str,
    docs: &'static str,
    extend: fn(KindSchema) -> KindSchema,
    lowerer: avenger_chart_lang_types::ObjectLowerer,
) -> ObjectLanguageDefinition {
    ObjectLanguageDefinition {
        schema: extend(common_schema(kind, docs)),
        lowerer,
    }
}

fn common_schema(kind: &str, docs: &str) -> KindSchema {
    KindSchema::new(NativeKindKey::new(NativeKindNamespace::Scale, kind), docs)
        .property(
            "domain",
            PropertySchema::optional(
                ValueShape::Array(Box::new(ValueShape::Any)),
                "Explicit scale domain values.",
            ),
        )
        .property(
            "range",
            PropertySchema::optional(
                ValueShape::Array(Box::new(ValueShape::Any)),
                "Explicit scalar scale range values.",
            ),
        )
        .property(
            "raw_domain",
            PropertySchema::optional(
                ValueShape::SqlExpression,
                "Runtime interval-domain override, with the regular domain retained as fallback.",
            ),
        )
        .property(
            "order_by",
            PropertySchema::optional(
                ValueShape::SqlExpression,
                "Expression used to order an inferred categorical domain.",
            ),
        )
        .property(
            "order",
            PropertySchema::optional(
                ValueShape::Atom {
                    values: [
                        (
                            "ascending",
                            "Sort the inferred categorical domain ascending.",
                        ),
                        (
                            "descending",
                            "Sort the inferred categorical domain descending.",
                        ),
                    ]
                    .into_iter()
                    .map(|(value, docs)| EnumValueSchema {
                        value: value.to_string(),
                        docs: docs.to_string(),
                    })
                    .collect(),
                },
                "Direction for inferred categorical-domain ordering.",
            ),
        )
}

fn identity_schema(schema: KindSchema) -> KindSchema {
    schema
}

fn boolean(schema: KindSchema, name: &str, docs: &str) -> KindSchema {
    schema.property(name, PropertySchema::optional(ValueShape::Boolean, docs))
}

fn number(schema: KindSchema, name: &str, docs: &str) -> KindSchema {
    schema.property(name, PropertySchema::optional(ValueShape::Number, docs))
}

fn linear_schema(schema: KindSchema) -> KindSchema {
    let schema = boolean(schema, "zero", "Include zero in the inferred domain.");
    let schema = boolean(schema, "nice", "Round the domain to pleasant values.");
    let schema = boolean(schema, "clamp", "Clamp outputs to the configured range.");
    number(
        schema,
        "padding",
        "Add pixel padding around the inferred domain.",
    )
}

fn log_schema(schema: KindSchema) -> KindSchema {
    let schema = number(schema, "base", "Positive logarithm base.");
    let schema = boolean(schema, "nice", "Round the domain to pleasant powers.");
    boolean(schema, "clamp", "Clamp outputs to the configured range.")
}

fn pow_schema(schema: KindSchema) -> KindSchema {
    let schema = number(schema, "exponent", "Power exponent.");
    let schema = boolean(schema, "zero", "Include zero in the inferred domain.");
    let schema = boolean(schema, "nice", "Round the domain to pleasant values.");
    boolean(schema, "clamp", "Clamp outputs to the configured range.")
}

fn sqrt_schema(schema: KindSchema) -> KindSchema {
    let schema = boolean(schema, "zero", "Include zero in the inferred domain.");
    let schema = boolean(schema, "nice", "Round the domain to pleasant values.");
    boolean(schema, "clamp", "Clamp outputs to the configured range.")
}

fn symlog_schema(schema: KindSchema) -> KindSchema {
    let schema = number(
        schema,
        "constant",
        "Positive linear-region constant around zero.",
    );
    let schema = boolean(schema, "nice", "Round the domain to pleasant values.");
    boolean(schema, "clamp", "Clamp outputs to the configured range.")
}

fn time_schema(schema: KindSchema) -> KindSchema {
    let schema = boolean(
        schema,
        "nice",
        "Round the temporal domain to pleasant boundaries.",
    );
    boolean(schema, "clamp", "Clamp outputs to the configured range.")
}

fn band_schema(schema: KindSchema) -> KindSchema {
    let schema = number(
        schema,
        "padding_inner",
        "Fractional padding between adjacent bands.",
    );
    let schema = number(
        schema,
        "padding_outer",
        "Fractional padding outside the first and last bands.",
    );
    let schema = number(
        schema,
        "align",
        "Band alignment within the range, from zero through one.",
    );
    boolean(
        schema,
        "round",
        "Round band positions and widths to whole pixels.",
    )
}

fn point_schema(schema: KindSchema) -> KindSchema {
    let schema = number(schema, "padding", "Fractional outer padding.");
    let schema = number(
        schema,
        "align",
        "Point alignment within the range, from zero through one.",
    );
    boolean(schema, "round", "Round point positions to whole pixels.")
}

fn ordinal_schema(schema: KindSchema) -> KindSchema {
    schema.property(
        "unknown",
        PropertySchema::optional(
            ValueShape::SqlExpression,
            "Output value used for inputs absent from the domain.",
        ),
    )
}

macro_rules! scale_lowerer {
    ($name:ident, $marker:ty, $domain:expr) => {
        fn $name(
            declaration: &ResolvedDeclaration,
        ) -> Result<Box<dyn std::any::Any + Send + Sync>, NativeLoweringError> {
            lower_scale(
                Scale::<$marker>::new().into_type::<Auto>(),
                declaration,
                $domain,
            )
            .map(|scale| Box::new(scale) as Box<dyn std::any::Any + Send + Sync>)
        }
    };
}

#[derive(Clone, Copy)]
enum DomainForm {
    Interval,
    Discrete,
}

scale_lowerer!(lower_linear, Linear, DomainForm::Interval);
scale_lowerer!(lower_log, Log, DomainForm::Interval);
scale_lowerer!(lower_pow, Pow, DomainForm::Interval);
scale_lowerer!(lower_sqrt, Sqrt, DomainForm::Interval);
scale_lowerer!(lower_symlog, Symlog, DomainForm::Interval);
scale_lowerer!(lower_time, Time, DomainForm::Interval);
scale_lowerer!(lower_band, Band, DomainForm::Discrete);
scale_lowerer!(lower_nested_band, NestedBand, DomainForm::Discrete);
scale_lowerer!(lower_point, Point, DomainForm::Discrete);
scale_lowerer!(lower_ordinal, Ordinal, DomainForm::Discrete);
scale_lowerer!(lower_threshold, Threshold, DomainForm::Discrete);
scale_lowerer!(lower_quantile, Quantile, DomainForm::Discrete);
scale_lowerer!(lower_quantize, Quantize, DomainForm::Interval);

fn lower_scale(
    mut scale: Scale<Auto>,
    declaration: &ResolvedDeclaration,
    domain_form: DomainForm,
) -> Result<Scale<Auto>, NativeLoweringError> {
    if let Some(ResolvedValue::Array(domain)) = declaration.properties.get("domain") {
        let expressions = domain
            .iter()
            .enumerate()
            .map(|(index, value)| expression(value, &format!("domain[{index}]")))
            .collect::<Result<Vec<_>, _>>()?;
        scale = match domain_form {
            DomainForm::Interval => {
                let [start, stop] = expressions.as_slice() else {
                    return Err(invalid("domain", "exactly two values"));
                };
                scale.domain_interval(start.clone(), stop.clone())
            }
            DomainForm::Discrete => scale.domain_discrete(expressions),
        };
    }
    if let Some(ResolvedValue::Array(range)) = declaration.properties.get("range") {
        if matches!(domain_form, DomainForm::Interval)
            && let Some(colors) = literal_color_range(range)
        {
            scale = scale.range_colors(colors);
        } else {
            scale = scale.range_discrete(
                range
                    .iter()
                    .enumerate()
                    .map(|(index, value)| scalar(value, &format!("range[{index}]")))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
    }
    for (name, value) in &declaration.properties {
        match name.as_str() {
            "domain" | "range" => {}
            "raw_domain" => scale = scale.raw_domain(expression(value, name)?),
            "order_by" => scale = scale.order_by(expression(value, name)?),
            "order" => match value {
                ResolvedValue::String(value) if value == "ascending" => scale = scale.order_asc(),
                ResolvedValue::String(value) if value == "descending" => scale = scale.order_desc(),
                _ => return Err(invalid(name, "ascending or descending")),
            },
            option => scale = scale._option(option, expression(value, name)?),
        }
    }
    Ok(scale)
}

fn literal_color_range(range: &[ResolvedValue]) -> Option<Vec<Srgba>> {
    if range.is_empty() {
        return None;
    }
    range
        .iter()
        .map(|value| {
            let ResolvedValue::String(value) = value else {
                return None;
            };
            avenger_color::parse_color_string(value)
                .map(|[red, green, blue, alpha]| Srgba::new(red, green, blue, alpha))
        })
        .collect()
}

fn expression(
    value: &ResolvedValue,
    property: &str,
) -> Result<datafusion::logical_expr::Expr, NativeLoweringError> {
    resolved_expr(value).ok_or_else(|| invalid(property, "scalar SQL expression"))
}

fn scalar(value: &ResolvedValue, property: &str) -> Result<ScalarValue, NativeLoweringError> {
    match value {
        ResolvedValue::Boolean(value) => Ok(ScalarValue::Boolean(Some(*value))),
        ResolvedValue::Integer(value) => Ok(ScalarValue::Int64(Some(*value))),
        ResolvedValue::Number(value) => Ok(ScalarValue::Float64(Some(*value))),
        ResolvedValue::String(value) => Ok(ScalarValue::Utf8(Some(value.clone()))),
        ResolvedValue::Scalar(value) => Ok(value.clone()),
        ResolvedValue::Expr(datafusion::logical_expr::Expr::Literal(value, _)) => Ok(value.clone()),
        _ => Err(invalid(property, "scalar literal")),
    }
}

fn invalid(property: &str, expected: &str) -> NativeLoweringError {
    NativeLoweringError::InvalidPropertyType {
        property: property.to_string(),
        expected: expected.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use avenger_chart_core::ScaleRange;

    use super::*;

    #[test]
    fn continuous_literal_css_color_range_lowers_as_colors() {
        let declaration = ResolvedDeclaration::new("linear").property(
            "range",
            ResolvedValue::Array(vec![
                ResolvedValue::String("#b35806".to_string()),
                ResolvedValue::String("white".to_string()),
                ResolvedValue::String("#4393c3".to_string()),
            ]),
        );

        let scale = lower_scale(
            Scale::<Linear>::new().into_type::<Auto>(),
            &declaration,
            DomainForm::Interval,
        )
        .unwrap();

        let Some(ScaleRange::Color(colors)) = scale.get_range() else {
            panic!("expected a color range");
        };
        assert_eq!(colors.len(), 3);
        assert_eq!(colors[1], [1.0, 1.0, 1.0, 1.0]);
    }
}
