use std::ops::Range;

use avenger_format_datetime::{
    DateTimeFormatContext, DateTimeFormatOverrides, DateTimeLocaleRegistry, DateTimeStyleLength,
    NaiveDateTimeInput, format_naive_datetime, format_zoned_datetime, parse_datetime_timezone,
};
use avenger_format_number::{
    Align, CurrencyDisplay, DigitSpec, FormatType, NumberFormatContext, NumberFormatOverrides,
    NumberLocaleRegistry, NumberTypesetting, SignPolicy, Symbol, format_number,
};

use crate::label::LabelError;
use crate::typst_eval::delimiter::{DelimiterDisplayHint, DelimiterInfo};
use crate::typst_library::foundations::{Scope, Value};
use crate::typst_library::symbols::{named_emoji, named_symbol};
use crate::typst_library::text::call::{parse_text_markup_option, text_span_kind};
use crate::typst_library::text::content::{
    EmojiAlias, LabelContent, LabelParamRef, LineNode, MathSpan, PlainTextNode, SmartQuoteNode,
    SymbolAlias, TextMarkupKind, TextMarkupOptions, TextMarkupSpan,
};
use crate::typst_library::text::smartquote::SmartQuote;

use crate::typst_syntax::ast::{self as typst_ast, AstNode};
use crate::typst_syntax::{
    RangeMapper, RootedPath, SpanKind, SyntaxKind, SyntaxNode, VirtualPath, VirtualRoot,
};

#[cfg(test)]
pub(crate) fn parse_line(source: &str) -> Result<LabelContent, LabelError> {
    parse_line_with_params(source, &Scope::default())
}

#[cfg(test)]
pub(crate) fn parse_line_with_params(
    source: &str,
    params: &Scope,
) -> Result<LabelContent, LabelError> {
    parse_line_with_format_context(source, params, MarkupFormatContext::default())
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NumberFormatMarkupContext<'a> {
    pub(crate) locale_id: Option<&'a str>,
    pub(crate) registry: Option<&'a NumberLocaleRegistry>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DateTimeFormatMarkupContext<'a> {
    pub(crate) locale_id: Option<&'a str>,
    pub(crate) timezone: Option<&'a str>,
    pub(crate) registry: Option<&'a DateTimeLocaleRegistry>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MarkupFormatContext<'a> {
    pub(crate) number: NumberFormatMarkupContext<'a>,
    pub(crate) datetime: DateTimeFormatMarkupContext<'a>,
}

#[cfg(test)]
pub(crate) fn parse_line_with_number_format_context(
    source: &str,
    params: &Scope,
    number_format: NumberFormatMarkupContext<'_>,
) -> Result<LabelContent, LabelError> {
    parse_line_with_format_context(
        source,
        params,
        MarkupFormatContext {
            number: number_format,
            datetime: DateTimeFormatMarkupContext::default(),
        },
    )
}

pub(crate) fn parse_line_with_format_context(
    source: &str,
    params: &Scope,
    format_context: MarkupFormatContext<'_>,
) -> Result<LabelContent, LabelError> {
    let mut root = crate::typst_syntax::parse(source);
    synthesize_ranges(&mut root, source.len())?;
    reject_syntax_errors(&root)?;
    let markup = root
        .cast::<typst_ast::Markup>()
        .ok_or_else(|| LabelError::Engine {
            start: 0,
            end: source.len(),
            message: "Typst parser did not return a markup root".to_string(),
        })?;

    let mut nodes = Vec::new();
    lower_markup(markup, source, params, format_context, &mut nodes)?;
    Ok(LabelContent {
        source: source.to_string(),
        nodes,
    })
}

fn lower_markup(
    markup: typst_ast::Markup<'_>,
    source: &str,
    params: &Scope,
    format_context: MarkupFormatContext<'_>,
    nodes: &mut Vec<LineNode>,
) -> Result<(), LabelError> {
    for expr in markup.exprs() {
        lower_markup_expr(expr, source, params, format_context, nodes)?;
    }
    Ok(())
}

fn lower_markup_expr(
    expr: typst_ast::Expr<'_>,
    source: &str,
    params: &Scope,
    format_context: MarkupFormatContext<'_>,
    nodes: &mut Vec<LineNode>,
) -> Result<(), LabelError> {
    match expr {
        typst_ast::Expr::Text(text) => {
            push_plain(nodes, text.get().as_str(), text.to_untyped().range());
        }
        typst_ast::Expr::Space(space) => {
            let node = space.to_untyped();
            push_plain(nodes, node.full_text().as_str(), node.range());
        }
        typst_ast::Expr::Escape(escape) => {
            let text = escape.get().to_string();
            push_plain(nodes, &text, escape.to_untyped().range());
        }
        typst_ast::Expr::Shorthand(shorthand) => {
            let text = shorthand.get().to_string();
            push_plain(nodes, &text, shorthand.to_untyped().range());
        }
        typst_ast::Expr::SmartQuote(quote) => {
            let node = quote.to_untyped();
            nodes.push(LineNode::SmartQuote(SmartQuoteNode {
                quote: SmartQuote {
                    double: quote.double(),
                },
                byte_range: node.range(),
            }));
        }
        typst_ast::Expr::Equation(equation) => {
            if equation.block() {
                return Err(unsupported(
                    equation.to_untyped().range().start,
                    "display math is not supported in Avenger text lines",
                ));
            }
            let body = equation.body();
            let source_range = body.to_untyped().range();
            let full_range = equation.to_untyped().range();
            let source_text = source[source_range.clone()].to_string();
            nodes.push(LineNode::Math(MathSpan {
                source: source_text,
                source_range,
                delimiter: math_delimiter_info(full_range),
            }));
        }
        typst_ast::Expr::FuncCall(call) => {
            lower_static_call(call, source, params, format_context, nodes)?;
        }
        typst_ast::Expr::Ident(ident) => {
            let range = expand_hash_range(source, ident.to_untyped().range());
            nodes.push(LineNode::Param(LabelParamRef {
                name: ident.as_str().to_string(),
                byte_range: range,
            }));
        }
        typst_ast::Expr::FieldAccess(access) => {
            lower_static_field_access(access, source, nodes)?;
        }
        typst_ast::Expr::Strong(strong) => {
            lower_markup_span(
                TextMarkupKind::Strong,
                TextMarkupOptions::default(),
                strong.body(),
                strong.to_untyped().range(),
                source,
                params,
                format_context,
                nodes,
            )?;
        }
        typst_ast::Expr::Emph(emph) => {
            lower_markup_span(
                TextMarkupKind::Emph,
                TextMarkupOptions::default(),
                emph.body(),
                emph.to_untyped().range(),
                source,
                params,
                format_context,
                nodes,
            )?;
        }
        typst_ast::Expr::Raw(raw) => {
            lower_raw_markup(raw, nodes)?;
        }
        other => {
            return Err(unsupported_markup_expr(source, other));
        }
    }
    Ok(())
}

fn lower_static_call(
    call: typst_ast::FuncCall<'_>,
    source: &str,
    params: &Scope,
    format_context: MarkupFormatContext<'_>,
    nodes: &mut Vec<LineNode>,
) -> Result<(), LabelError> {
    let range = expand_hash_range(source, call.to_untyped().range());
    let Some(name) = code_expr_name(call.callee()) else {
        return Err(unsupported(range.start, "unsupported static text command"));
    };
    if name == "numfmt" {
        return lower_numfmt_call(call, source, params, format_context.number, nodes, range);
    }
    if name == "datefmt" {
        return lower_datefmt_call(call, params, format_context.datetime, nodes, range);
    }
    let Some(kind) = text_span_kind(&name) else {
        return Err(unsupported(range.start, "unsupported static text command"));
    };

    let mut body = None;
    let mut options = TextMarkupOptions::default();
    for arg in call.args().items() {
        match arg {
            typst_ast::Arg::Pos(typst_ast::Expr::ContentBlock(block)) => {
                if kind == TextMarkupKind::Raw {
                    return Err(unsupported(range.start, "raw expects a string literal"));
                }
                let body_markup = block.body();
                let body_range = body_markup.to_untyped().range();
                let mut body_nodes = Vec::new();
                lower_markup(body_markup, source, params, format_context, &mut body_nodes)?;
                if body.replace((body_nodes, body_range)).is_some() {
                    return Err(unsupported(
                        range.start,
                        "static text command expects one bracketed content block",
                    ));
                }
            }
            typst_ast::Arg::Pos(typst_ast::Expr::Str(string))
                if kind.is_case_transform() || kind == TextMarkupKind::Raw =>
            {
                let string_range = string.to_untyped().range();
                let body_nodes = vec![LineNode::Plain(PlainTextNode {
                    text: string.get().to_string(),
                    byte_range: string_range.clone(),
                })];
                if body.replace((body_nodes, string_range)).is_some() {
                    return Err(unsupported(
                        range.start,
                        "static text command expects one bracketed content block",
                    ));
                }
            }
            typst_ast::Arg::Named(named) => {
                parse_text_markup_option(kind, named, params, &mut options)?;
            }
            typst_ast::Arg::Spread(_) => {
                return Err(unsupported(
                    range.start,
                    "static text commands do not support spread arguments",
                ));
            }
            typst_ast::Arg::Pos(_) => {
                return Err(unsupported(
                    range.start,
                    "static text command expects bracketed content",
                ));
            }
        }
    }

    let Some(body) = body else {
        return Err(unsupported(
            range.start,
            "static text command expects bracketed content",
        ));
    };

    let (body_nodes, body_range) = body;
    nodes.push(LineNode::TextSpan(TextMarkupSpan {
        kind,
        options,
        body: body_nodes,
        byte_range: range,
        body_range,
    }));
    Ok(())
}

fn lower_numfmt_call(
    call: typst_ast::FuncCall<'_>,
    _source: &str,
    params: &Scope,
    number_format: NumberFormatMarkupContext<'_>,
    nodes: &mut Vec<LineNode>,
    range: Range<usize>,
) -> Result<(), LabelError> {
    let mut value = None;
    let mut spec = None;
    let mut overrides = NumberFormatOverrides::default();

    for arg in call.args().items() {
        match arg {
            typst_ast::Arg::Pos(expr) => {
                if value.is_none() {
                    value = Some(parse_numfmt_number(expr, params, range.start)?);
                } else if spec.is_none() {
                    spec = Some(parse_numfmt_string(expr, params, range.start)?);
                } else {
                    return Err(unsupported(
                        range.start,
                        "numfmt expects value and format string",
                    ));
                }
            }
            typst_ast::Arg::Named(named) => {
                parse_numfmt_named_arg(named, params, &mut overrides)?;
            }
            typst_ast::Arg::Spread(_) => {
                return Err(unsupported(
                    range.start,
                    "numfmt does not support spread arguments",
                ));
            }
        }
    }

    let Some(value) = value else {
        return Err(unsupported(range.start, "numfmt expects a value argument"));
    };
    let spec = spec.unwrap_or_default();
    let builtin_registry;
    let registry = if let Some(registry) = number_format.registry {
        registry
    } else {
        builtin_registry = NumberLocaleRegistry::with_builtins();
        &builtin_registry
    };
    let locale_id = number_format.locale_id.unwrap_or("en-US");
    let locale = registry
        .resolve(locale_id)
        .map_err(|err| numfmt_engine_error(range.clone(), err.to_string()))?;
    let formatted = format_number(
        value,
        Some(&spec),
        overrides,
        NumberFormatContext::new(&locale).with_registry(registry),
    )
    .map_err(|err| numfmt_engine_error(range.clone(), err.to_string()))?;

    match formatted.typesetting {
        NumberTypesetting::Plain => push_plain(nodes, &formatted.text, range),
        NumberTypesetting::Exponent {
            mantissa, exponent, ..
        } => {
            let source = format!("{mantissa} times 10^({exponent})");
            nodes.push(LineNode::Math(MathSpan {
                source,
                source_range: range.clone(),
                delimiter: DelimiterInfo {
                    opening_range: range.start..range.start,
                    closing_range: range.end..range.end,
                    full_range: range,
                    display_hint: DelimiterDisplayHint::Inline,
                },
            }));
        }
    }
    Ok(())
}

enum DatefmtValue {
    Date(chrono::NaiveDate),
    DateTime(chrono::NaiveDateTime),
    UtcDateTime(chrono::DateTime<chrono::Utc>),
}

fn lower_datefmt_call(
    call: typst_ast::FuncCall<'_>,
    params: &Scope,
    datetime_format: DateTimeFormatMarkupContext<'_>,
    nodes: &mut Vec<LineNode>,
    range: Range<usize>,
) -> Result<(), LabelError> {
    let mut value = None;
    let mut spec = None;
    let mut overrides = DateTimeFormatOverrides::default();
    let mut locale_override = None;

    for arg in call.args().items() {
        match arg {
            typst_ast::Arg::Pos(expr) => {
                if value.is_none() {
                    value = Some(parse_datefmt_value(expr, params, range.start)?);
                } else if spec.is_none() {
                    spec = Some(parse_datefmt_string(expr, params, range.start)?);
                } else {
                    return Err(unsupported(
                        range.start,
                        "datefmt expects value and format string",
                    ));
                }
            }
            typst_ast::Arg::Named(named) => {
                parse_datefmt_named_arg(named, params, &mut overrides, &mut locale_override)?;
            }
            typst_ast::Arg::Spread(_) => {
                return Err(unsupported(
                    range.start,
                    "datefmt does not support spread arguments",
                ));
            }
        }
    }

    let Some(value) = value else {
        return Err(unsupported(range.start, "datefmt expects a value argument"));
    };
    let Some(spec) = spec else {
        return Err(unsupported(range.start, "datefmt expects a format string"));
    };

    let builtin_registry;
    let registry = if let Some(registry) = datetime_format.registry {
        registry
    } else {
        builtin_registry = DateTimeLocaleRegistry::with_builtins();
        &builtin_registry
    };
    let locale_id = locale_override
        .as_deref()
        .or(datetime_format.locale_id)
        .unwrap_or("en-US");
    let locale = registry
        .resolve(locale_id)
        .map_err(|err| datefmt_engine_error(range.clone(), err.to_string()))?;
    let timezone = parse_datetime_timezone(datetime_format.timezone.unwrap_or("UTC"))
        .map_err(|err| datefmt_engine_error(range.clone(), err.to_string()))?;
    let context = DateTimeFormatContext::new(&locale, timezone).with_registry(registry);

    let formatted = match value {
        DatefmtValue::Date(value) => format_naive_datetime(
            NaiveDateTimeInput::Date(value),
            Some(&spec),
            overrides,
            context,
        ),
        DatefmtValue::DateTime(value) => format_naive_datetime(
            NaiveDateTimeInput::DateTime(value),
            Some(&spec),
            overrides,
            context,
        ),
        DatefmtValue::UtcDateTime(value) => {
            format_zoned_datetime(value, Some(&spec), overrides, context)
        }
    }
    .map_err(|err| datefmt_engine_error(range.clone(), err.to_string()))?;

    push_plain(nodes, &formatted.text, range);
    Ok(())
}

fn parse_numfmt_named_arg(
    named: typst_ast::Named<'_>,
    params: &Scope,
    overrides: &mut NumberFormatOverrides,
) -> Result<(), LabelError> {
    let position = named.name().to_untyped().range().start;
    match named.name().as_str() {
        "style" | "type" => {
            let value = parse_numfmt_string(named.expr(), params, position)?;
            let mut chars = value.chars();
            let Some(ch) = chars.next() else {
                return Err(unsupported(position, "unsupported numfmt style"));
            };
            if chars.next().is_some() {
                return Err(unsupported(position, "unsupported numfmt style"));
            }
            overrides.format_type = FormatType::from_char(ch);
            if overrides.format_type.is_none() {
                return Err(unsupported(position, "unsupported numfmt style"));
            }
        }
        "precision" => {
            set_numfmt_digit_spec(
                overrides,
                DigitSpec::Precision(parse_numfmt_u8(
                    named.expr(),
                    params,
                    position,
                    "unsupported numfmt precision",
                )?),
                position,
            )?;
        }
        "fraction_digits" => {
            set_numfmt_digit_spec(
                overrides,
                DigitSpec::Fraction(parse_numfmt_u8(
                    named.expr(),
                    params,
                    position,
                    "unsupported numfmt fraction_digits",
                )?),
                position,
            )?;
        }
        "significant_digits" => {
            set_numfmt_digit_spec(
                overrides,
                DigitSpec::Significant(parse_numfmt_u8(
                    named.expr(),
                    params,
                    position,
                    "unsupported numfmt significant_digits",
                )?),
                position,
            )?;
        }
        "width" => {
            overrides.width = Some(parse_numfmt_optional_usize(
                named.expr(),
                params,
                position,
                "unsupported numfmt width",
            )?);
        }
        "fill" => {
            overrides.fill = Some(parse_numfmt_optional_char(
                named.expr(),
                params,
                position,
                "unsupported numfmt fill",
            )?);
        }
        "align" => {
            overrides.align = Some(parse_numfmt_optional_align(named.expr(), params, position)?);
        }
        "group" => {
            overrides.group = Some(parse_numfmt_bool(named.expr(), params, position)?);
        }
        "trim" => {
            overrides.trim = Some(parse_numfmt_bool(named.expr(), params, position)?);
        }
        "zero" => {
            overrides.zero = Some(parse_numfmt_bool(named.expr(), params, position)?);
        }
        "currency" => {
            overrides.currency = Some(parse_numfmt_string(named.expr(), params, position)?);
        }
        "currency_display" => {
            let display = parse_numfmt_string(named.expr(), params, position)?;
            overrides.currency_display = Some(match display.as_str() {
                "symbol" => CurrencyDisplay::Symbol,
                "code" => CurrencyDisplay::Code,
                "name" => CurrencyDisplay::Name,
                "narrow-symbol" | "narrow_symbol" => CurrencyDisplay::NarrowSymbol,
                _ => return Err(unsupported(position, "unsupported numfmt currency_display")),
            });
        }
        "sign" => {
            let value = parse_numfmt_string(named.expr(), params, position)?;
            let mut chars = value.chars();
            let Some(ch) = chars.next() else {
                return Err(unsupported(position, "unsupported numfmt sign"));
            };
            if chars.next().is_some() {
                return Err(unsupported(position, "unsupported numfmt sign"));
            }
            overrides.sign = SignPolicy::from_char(ch);
            if overrides.sign.is_none() {
                return Err(unsupported(position, "unsupported numfmt sign"));
            }
        }
        "symbol" => {
            let value = parse_numfmt_string(named.expr(), params, position)?;
            overrides.symbol = Some(match value.as_str() {
                "$" => Some(Symbol::CurrencyCompat),
                "#" => Some(Symbol::Alternate),
                "none" => None,
                _ => return Err(unsupported(position, "unsupported numfmt symbol")),
            });
        }
        _ => return Err(unsupported(position, "unsupported numfmt option")),
    }
    Ok(())
}

fn set_numfmt_digit_spec(
    overrides: &mut NumberFormatOverrides,
    digit_spec: DigitSpec,
    position: usize,
) -> Result<(), LabelError> {
    if overrides.digit_spec.is_some() {
        return Err(unsupported(
            position,
            "numfmt accepts only one digit-control option",
        ));
    }
    overrides.digit_spec = Some(digit_spec);
    Ok(())
}

fn parse_datefmt_named_arg(
    named: typst_ast::Named<'_>,
    params: &Scope,
    overrides: &mut DateTimeFormatOverrides,
    locale_override: &mut Option<String>,
) -> Result<(), LabelError> {
    let position = named.name().to_untyped().range().start;
    match named.name().as_str() {
        "locale" => {
            *locale_override = Some(parse_datefmt_string(named.expr(), params, position)?);
        }
        "timezone" | "tz" => {
            overrides.timezone = Some(parse_datefmt_string(named.expr(), params, position)?);
        }
        "date_style" => {
            overrides.date_style =
                Some(parse_datefmt_style_length(named.expr(), params, position)?);
        }
        "time_style" => {
            overrides.time_style =
                Some(parse_datefmt_style_length(named.expr(), params, position)?);
        }
        "datetime_style" => {
            overrides.datetime_style =
                Some(parse_datefmt_style_length(named.expr(), params, position)?);
        }
        _ => return Err(unsupported(position, "unsupported datefmt option")),
    }
    Ok(())
}

fn parse_datefmt_value(
    expr: typst_ast::Expr<'_>,
    params: &Scope,
    position: usize,
) -> Result<DatefmtValue, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return match value {
            Value::Date(value) => Ok(DatefmtValue::Date(*value)),
            Value::DateTime(value) => Ok(DatefmtValue::DateTime(*value)),
            Value::UtcDateTime(value) => Ok(DatefmtValue::UtcDateTime(*value)),
            _ => Err(unsupported(position, "datefmt value must be temporal")),
        };
    }
    Err(unsupported(position, "datefmt value must be temporal"))
}

fn parse_datefmt_string(
    expr: typst_ast::Expr<'_>,
    params: &Scope,
    position: usize,
) -> Result<String, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return match value {
            Value::Str(value) => Ok(value.clone()),
            _ => Err(unsupported(position, "datefmt argument must be a string")),
        };
    }
    match expr {
        typst_ast::Expr::Str(value) => Ok(value.get().to_string()),
        _ => Err(unsupported(position, "datefmt argument must be a string")),
    }
}

fn parse_datefmt_style_length(
    expr: typst_ast::Expr<'_>,
    params: &Scope,
    position: usize,
) -> Result<DateTimeStyleLength, LabelError> {
    let value = parse_datefmt_string(expr, params, position)?;
    DateTimeStyleLength::from_str(&value)
        .ok_or_else(|| unsupported(position, "unsupported datefmt style length"))
}

fn parse_numfmt_number(
    expr: typst_ast::Expr<'_>,
    params: &Scope,
    position: usize,
) -> Result<f64, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return match value {
            Value::Int(value) => Ok(*value as f64),
            Value::Float(value) if value.is_finite() => Ok(*value),
            _ => Err(unsupported(position, "numfmt value must be numeric")),
        };
    }
    match expr {
        typst_ast::Expr::Int(value) => Ok(value.get() as f64),
        typst_ast::Expr::Float(value) => Ok(value.get()),
        typst_ast::Expr::Unary(unary) => {
            let sign = match unary.op() {
                typst_ast::UnOp::Pos => 1.0,
                typst_ast::UnOp::Neg => -1.0,
                typst_ast::UnOp::Not => {
                    return Err(unsupported(position, "numfmt value must be numeric"));
                }
            };
            parse_numfmt_number(unary.expr(), params, position).map(|value| sign * value)
        }
        _ => Err(unsupported(position, "numfmt value must be numeric")),
    }
}

fn parse_numfmt_string(
    expr: typst_ast::Expr<'_>,
    params: &Scope,
    position: usize,
) -> Result<String, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return match value {
            Value::Str(value) => Ok(value.clone()),
            _ => Err(unsupported(position, "numfmt argument must be a string")),
        };
    }
    match expr {
        typst_ast::Expr::Str(value) => Ok(value.get().to_string()),
        _ => Err(unsupported(position, "numfmt argument must be a string")),
    }
}

fn parse_numfmt_bool(
    expr: typst_ast::Expr<'_>,
    params: &Scope,
    position: usize,
) -> Result<bool, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return match value {
            Value::Bool(value) => Ok(*value),
            _ => Err(unsupported(position, "numfmt argument must be a boolean")),
        };
    }
    match expr {
        typst_ast::Expr::Bool(value) => Ok(value.get()),
        _ => Err(unsupported(position, "numfmt argument must be a boolean")),
    }
}

fn parse_numfmt_u8(
    expr: typst_ast::Expr<'_>,
    params: &Scope,
    position: usize,
    message: &'static str,
) -> Result<u8, LabelError> {
    let value = if let Some(value) = param_value_for_ident(expr, params) {
        match value {
            Value::Int(value) => *value,
            _ => return Err(unsupported(position, message)),
        }
    } else {
        match expr {
            typst_ast::Expr::Int(value) => value.get(),
            _ => return Err(unsupported(position, message)),
        }
    };
    u8::try_from(value).map_err(|_| unsupported(position, message))
}

fn parse_numfmt_optional_usize(
    expr: typst_ast::Expr<'_>,
    params: &Scope,
    position: usize,
    message: &'static str,
) -> Result<Option<usize>, LabelError> {
    let value = if let Some(value) = param_value_for_ident(expr, params) {
        match value {
            Value::None => return Ok(None),
            Value::Int(value) => *value,
            _ => return Err(unsupported(position, message)),
        }
    } else {
        match expr {
            typst_ast::Expr::None(_) => return Ok(None),
            typst_ast::Expr::Int(value) => value.get(),
            _ => return Err(unsupported(position, message)),
        }
    };
    usize::try_from(value)
        .map(Some)
        .map_err(|_| unsupported(position, message))
}

fn parse_numfmt_optional_char(
    expr: typst_ast::Expr<'_>,
    params: &Scope,
    position: usize,
    message: &'static str,
) -> Result<Option<char>, LabelError> {
    let value = parse_numfmt_optional_string(expr, params, position, message)?;
    let Some(value) = value else {
        return Ok(None);
    };
    let mut chars = value.chars();
    let Some(ch) = chars.next() else {
        return Err(unsupported(position, message));
    };
    if chars.next().is_some() {
        return Err(unsupported(position, message));
    }
    Ok(Some(ch))
}

fn parse_numfmt_optional_align(
    expr: typst_ast::Expr<'_>,
    params: &Scope,
    position: usize,
) -> Result<Option<Align>, LabelError> {
    let value = parse_numfmt_optional_string(expr, params, position, "unsupported numfmt align")?;
    let Some(value) = value else {
        return Ok(None);
    };
    let mut chars = value.chars();
    let Some(ch) = chars.next() else {
        return Err(unsupported(position, "unsupported numfmt align"));
    };
    if chars.next().is_some() {
        return Err(unsupported(position, "unsupported numfmt align"));
    }
    let Some(align) = Align::from_char(ch) else {
        return Err(unsupported(position, "unsupported numfmt align"));
    };
    Ok(Some(align))
}

fn parse_numfmt_optional_string(
    expr: typst_ast::Expr<'_>,
    params: &Scope,
    position: usize,
    message: &'static str,
) -> Result<Option<String>, LabelError> {
    if let Some(value) = param_value_for_ident(expr, params) {
        return match value {
            Value::None => Ok(None),
            Value::Str(value) => Ok(Some(value.clone())),
            _ => Err(unsupported(position, message)),
        };
    }
    match expr {
        typst_ast::Expr::None(_) => Ok(None),
        typst_ast::Expr::Str(value) => Ok(Some(value.get().to_string())),
        _ => Err(unsupported(position, message)),
    }
}

fn param_value_for_ident<'a>(expr: typst_ast::Expr<'_>, params: &'a Scope) -> Option<&'a Value> {
    let typst_ast::Expr::Ident(ident) = expr else {
        return None;
    };
    params.get(ident.as_str())
}

fn numfmt_engine_error(range: Range<usize>, message: String) -> LabelError {
    LabelError::Engine {
        start: range.start,
        end: range.end,
        message,
    }
}

fn datefmt_engine_error(range: Range<usize>, message: String) -> LabelError {
    LabelError::Engine {
        start: range.start,
        end: range.end,
        message,
    }
}

fn lower_raw_markup(raw: typst_ast::Raw<'_>, nodes: &mut Vec<LineNode>) -> Result<(), LabelError> {
    let range = raw.to_untyped().range();
    if raw.block() {
        return Err(unsupported(
            range.start,
            "raw block labels are not supported",
        ));
    }
    if raw.lang().is_some() {
        return Err(unsupported(
            range.start,
            "raw syntax highlighting is not supported",
        ));
    }

    let lines = raw.lines().collect::<Vec<_>>();
    let body_range = lines
        .first()
        .zip(lines.last())
        .map(|(first, last)| first.to_untyped().range().start..last.to_untyped().range().end)
        .unwrap_or_else(|| range.clone());
    let text = lines
        .iter()
        .map(|line| line.get().as_str())
        .collect::<Vec<_>>()
        .join("\n");
    nodes.push(LineNode::TextSpan(TextMarkupSpan {
        kind: TextMarkupKind::Raw,
        options: TextMarkupOptions::default(),
        body: vec![LineNode::Plain(PlainTextNode {
            text,
            byte_range: body_range.clone(),
        })],
        byte_range: range,
        body_range,
    }));
    Ok(())
}

#[allow(
    clippy::too_many_arguments,
    reason = "Keep the explicit inputs of the existing layout and rendering pipeline."
)]
fn lower_markup_span(
    kind: TextMarkupKind,
    options: TextMarkupOptions,
    body_markup: typst_ast::Markup<'_>,
    byte_range: Range<usize>,
    source: &str,
    params: &Scope,
    format_context: MarkupFormatContext<'_>,
    nodes: &mut Vec<LineNode>,
) -> Result<(), LabelError> {
    let body_range = body_markup.to_untyped().range();
    let mut body = Vec::new();
    lower_markup(body_markup, source, params, format_context, &mut body)?;
    nodes.push(LineNode::TextSpan(TextMarkupSpan {
        kind,
        options,
        body,
        byte_range,
        body_range,
    }));
    Ok(())
}

fn lower_static_field_access(
    access: typst_ast::FieldAccess<'_>,
    source: &str,
    nodes: &mut Vec<LineNode>,
) -> Result<(), LabelError> {
    let range = expand_hash_range(source, access.to_untyped().range());
    let Some(name) = code_field_access_name(access) else {
        return Err(unsupported(range.start, "unsupported static text command"));
    };
    if let Some(alias) = name.strip_prefix("emoji.") {
        let Some(emoji) = named_emoji(alias) else {
            return Err(unsupported(range.start, "unknown emoji alias"));
        };
        nodes.push(LineNode::Emoji(EmojiAlias {
            name: alias.to_string(),
            emoji,
            byte_range: range,
        }));
        return Ok(());
    }

    if let Some(alias) = name.strip_prefix("sym.") {
        let Some(text) = named_symbol(alias) else {
            return Err(unsupported(range.start, "unknown symbol alias"));
        };
        nodes.push(LineNode::Symbol(SymbolAlias {
            name: alias.to_string(),
            text,
            byte_range: range,
        }));
        return Ok(());
    }

    Err(unsupported(range.start, "unsupported static text command"))
}

fn code_expr_name(expr: typst_ast::Expr<'_>) -> Option<String> {
    match expr {
        typst_ast::Expr::Ident(ident) => Some(ident.as_str().to_string()),
        typst_ast::Expr::FieldAccess(access) => code_field_access_name(access),
        _ => None,
    }
}

fn code_field_access_name(access: typst_ast::FieldAccess<'_>) -> Option<String> {
    let mut name = code_expr_name(access.target())?;
    name.push('.');
    name.push_str(access.field().as_str());
    Some(name)
}

fn push_plain(nodes: &mut Vec<LineNode>, text: &str, range: Range<usize>) {
    if text.is_empty() {
        return;
    }
    if let Some(LineNode::Plain(previous)) = nodes.last_mut() {
        previous.text.push_str(text);
        previous.byte_range.end = range.end;
        return;
    }
    nodes.push(LineNode::Plain(PlainTextNode {
        text: text.to_string(),
        byte_range: range,
    }));
}

fn math_delimiter_info(full_range: Range<usize>) -> DelimiterInfo {
    DelimiterInfo {
        opening_range: full_range.start..full_range.start + 1,
        closing_range: full_range.end.saturating_sub(1)..full_range.end,
        full_range,
        display_hint: DelimiterDisplayHint::Inline,
    }
}

fn expand_hash_range(source: &str, range: Range<usize>) -> Range<usize> {
    if range.start > 0 && source.as_bytes().get(range.start - 1) == Some(&b'#') {
        range.start - 1..range.end
    } else {
        range
    }
}

fn reject_syntax_errors(root: &SyntaxNode) -> Result<(), LabelError> {
    if !root.diagnosis().errors {
        return Ok(());
    }
    let (errors, _) = root.errors_and_warnings();
    let message = errors
        .first()
        .map(|error| error.message.to_string())
        .unwrap_or_else(|| "invalid Typst syntax".to_string());
    let position = first_error_range(root)
        .map(|range| range.start)
        .unwrap_or_default();
    Err(LabelError::Syntax { position, message })
}

fn first_error_range(node: &SyntaxNode) -> Option<Range<usize>> {
    if node.kind() == SyntaxKind::Error {
        return Some(node.range());
    }
    node.children().find_map(first_error_range)
}

fn unsupported_markup_expr(source: &str, expr: typst_ast::Expr<'_>) -> LabelError {
    let range = expr.to_untyped().range();
    let expanded = expand_hash_range(source, range.clone());
    if expanded.start != range.start {
        unsupported(expanded.start, "unsupported static text command")
    } else {
        unsupported(
            range.start,
            "this Typst markup construct is not supported in Avenger text lines",
        )
    }
}

fn unsupported(position: usize, message: &'static str) -> LabelError {
    LabelError::UnsupportedSyntax { position, message }
}

trait SyntaxNodeRange {
    fn range(&self) -> Range<usize>;
}

impl SyntaxNodeRange for SyntaxNode {
    fn range(&self) -> Range<usize> {
        match self.span().get() {
            SpanKind::Range { range, .. } => range,
            _ => 0..self.len(),
        }
    }
}

fn synthesize_ranges(root: &mut SyntaxNode, source_len: usize) -> Result<(), LabelError> {
    let mapper =
        RangeMapper::new(std::iter::once(0..source_len)).map_err(|message| LabelError::Engine {
            start: 0,
            end: source_len,
            message: message.to_string(),
        })?;
    root.synthesize_mapped(scratch_file_id(), &mapper)
        .map_err(|message| LabelError::Engine {
            start: 0,
            end: source_len,
            message: message.to_string(),
        })
}

fn scratch_file_id() -> crate::typst_syntax::FileId {
    RootedPath::new(
        VirtualRoot::Project,
        VirtualPath::new("avenger-typst-label-line.typ").expect("static virtual path is valid"),
    )
    .intern()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typst_library::Color;
    use crate::typst_library::foundations::Value;
    use crate::typst_library::text::content::DecorationLength;
    use crate::typst_svg::{LineCap, LineJoin};

    fn parse(source: &str) -> LabelContent {
        parse_line(source).unwrap()
    }

    fn parse_with_params(source: &str, params: &Scope) -> LabelContent {
        parse_line_with_params(source, params).unwrap()
    }

    fn scope(values: impl IntoIterator<Item = (&'static str, Value)>) -> Scope {
        Scope::new(
            values
                .into_iter()
                .map(|(name, value)| (name.to_string(), value))
                .collect(),
        )
    }

    #[test]
    fn parses_plain_and_math_spans() {
        let line = parse("Price \\$7, score $R^2$ = 0.94");

        assert_eq!(line.nodes.len(), 3);
        assert!(
            matches!(&line.nodes[0], LineNode::Plain(plain) if plain.text == "Price $7, score ")
        );
        assert!(matches!(&line.nodes[1], LineNode::Math(math) if math.source == "R^2"));
        assert!(matches!(&line.nodes[2], LineNode::Plain(plain) if plain.text == " = 0.94"));
    }

    #[test]
    fn canonical_typst_unmatched_dollar_errors() {
        let err = parse_line("cost $5").unwrap_err();
        assert!(matches!(err, LabelError::Syntax { .. }));
    }

    #[test]
    fn parses_static_text_spans() {
        let line = parse("This is #underline[important] and #strike[old]");

        assert_eq!(line.nodes.len(), 4);
        assert!(matches!(
            &line.nodes[1],
            LineNode::TextSpan(span)
                if span.kind == TextMarkupKind::Underline
                    && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "important")
        ));
        assert!(matches!(
            &line.nodes[3],
            LineNode::TextSpan(span)
                if span.kind == TextMarkupKind::Strike
                    && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "old")
        ));
    }

    #[test]
    fn parses_embedded_code_identifier_as_param_ref() {
        let line = parse("Series #series_name");

        assert_eq!(line.nodes.len(), 2);
        assert!(matches!(&line.nodes[0], LineNode::Plain(plain) if plain.text == "Series "));
        assert!(matches!(
            &line.nodes[1],
            LineNode::Param(param)
                if param.name == "series_name"
                    && param.byte_range == (7..19)
        ));
    }

    #[test]
    fn parses_nested_static_text_spans() {
        let line = parse("#underline[important #super[2]]");

        let LineNode::TextSpan(span) = &line.nodes[0] else {
            panic!("expected outer span");
        };
        assert_eq!(span.kind, TextMarkupKind::Underline);
        assert_eq!(span.body.len(), 2);
        assert!(
            matches!(&span.body[1], LineNode::TextSpan(inner) if inner.kind == TextMarkupKind::Superscript)
        );
    }

    #[test]
    fn parses_script_text_options() {
        let line = parse(
            "#super(typographic: false, baseline: -0.25em, size: 0.7em)[N] \
             #sub(typographic: false, baseline: 2pt, size: 8pt)[2]",
        );

        assert_eq!(line.nodes.len(), 3);
        assert!(matches!(&line.nodes[0], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Superscript
                && !span.options.script.typographic
                && span.options.script.baseline == Some(DecorationLength::Em(-0.25))
                && span.options.script.size == Some(DecorationLength::Em(0.7))
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "N")
        ));
        assert!(matches!(&line.nodes[2], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Subscript
                && !span.options.script.typographic
                && span.options.script.baseline == Some(DecorationLength::Pt(2.0))
                && span.options.script.size == Some(DecorationLength::Pt(8.0))
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "2")
        ));
    }

    #[test]
    fn rejects_unknown_script_option() {
        let err = parse_line("#super(foo: true)[x]").unwrap_err();
        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 7,
                message: "unsupported script option"
            }
        );
    }

    #[test]
    fn parses_case_transform_static_text() {
        let line = parse("#lower[MiXeD #sym.arrow.r] #upper(\"loud\")");

        assert_eq!(line.nodes.len(), 3);
        assert!(matches!(&line.nodes[0], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Lower
                && matches!(&span.body[..], [LineNode::Plain(_), LineNode::Symbol(_)])
        ));
        assert!(matches!(&line.nodes[2], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Upper
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "loud")
        ));
    }

    #[test]
    fn parses_smallcaps_static_text() {
        let line = parse("#smallcaps[Smallcaps] #smallcaps(all: true)[UNICEF]");

        assert_eq!(line.nodes.len(), 3);
        assert!(matches!(&line.nodes[0], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Smallcaps
                && !span.options.smallcaps.all
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "Smallcaps")
        ));
        assert!(matches!(&line.nodes[2], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Smallcaps
                && span.options.smallcaps.all
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "UNICEF")
        ));
    }

    #[test]
    fn parses_emph_and_strong_static_text() {
        let line = parse("_Emph_ *Strong* #emph[call] #strong(delta: 150)[mild]");

        assert_eq!(line.nodes.len(), 7);
        assert!(matches!(&line.nodes[0], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Emph
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "Emph")
        ));
        assert!(matches!(&line.nodes[2], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Strong
                && span.options.strong.delta == 300
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "Strong")
        ));
        assert!(matches!(&line.nodes[4], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Emph
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "call")
        ));
        assert!(matches!(&line.nodes[6], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Strong
                && span.options.strong.delta == 150
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "mild")
        ));
    }

    #[test]
    fn parses_inline_raw_static_text() {
        let line = parse("Use `x # y` and #raw(\"z * w\")");

        assert_eq!(line.nodes.len(), 4);
        let LineNode::TextSpan(span) = &line.nodes[1] else {
            panic!("expected backtick raw span");
        };
        assert_eq!(span.kind, TextMarkupKind::Raw);
        assert_eq!(span.byte_range, 4..11);
        assert_eq!(span.body_range, 5..10);
        assert!(matches!(&span.body[..], [LineNode::Plain(plain)]
            if plain.text == "x # y" && plain.byte_range == (5..10)
        ));
        assert!(matches!(&line.nodes[3], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Raw
                && matches!(&span.body[..], [LineNode::Plain(plain)] if plain.text == "z * w")
        ));
    }

    #[test]
    fn rejects_raw_block_and_highlighting() {
        let block = parse_line("```typ\nlet x = 1\n```").unwrap_err();
        assert_eq!(
            block,
            LabelError::UnsupportedSyntax {
                position: 0,
                message: "raw block labels are not supported"
            }
        );

        let highlighted = parse_line("```typ let x = 1```").unwrap_err();
        assert_eq!(
            highlighted,
            LabelError::UnsupportedSyntax {
                position: 0,
                message: "raw syntax highlighting is not supported"
            }
        );
    }

    #[test]
    fn parses_named_emoji_aliases() {
        let line = parse("Revenue #emoji.rocket #emoji.chart.up #emoji.face.halo");

        assert_eq!(line.nodes.len(), 6);
        assert!(
            matches!(&line.nodes[1], LineNode::Emoji(alias) if alias.name == "rocket" && alias.emoji == "🚀")
        );
        assert!(
            matches!(&line.nodes[3], LineNode::Emoji(alias) if alias.name == "chart.up" && alias.emoji == "📈")
        );
        assert!(
            matches!(&line.nodes[5], LineNode::Emoji(alias) if alias.name == "face.halo" && alias.emoji == "😇")
        );
    }

    #[test]
    fn parses_named_symbol_aliases() {
        let line = parse(
            "Flow #sym.arrow.r target #sym.gt.eq.not #sym.arrow.double.r #sym.forces.not #sym.gender.male.stroke.t",
        );

        assert_eq!(line.nodes.len(), 10);
        assert!(
            matches!(&line.nodes[1], LineNode::Symbol(alias) if alias.name == "arrow.r" && alias.text == "→")
        );
        assert!(
            matches!(&line.nodes[3], LineNode::Symbol(alias) if alias.name == "gt.eq.not" && alias.text == "≱")
        );
        assert!(
            matches!(&line.nodes[5], LineNode::Symbol(alias) if alias.name == "arrow.double.r" && alias.text == "⇒")
        );
        assert!(
            matches!(&line.nodes[7], LineNode::Symbol(alias) if alias.name == "forces.not" && alias.text == "⊮")
        );
        assert!(
            matches!(&line.nodes[9], LineNode::Symbol(alias) if alias.name == "gender.male.stroke.t" && alias.text == "⚨")
        );
    }

    #[test]
    fn parses_decoration_options() {
        let line = parse(
            "#underline(stroke: 1.5pt + red, offset: 2pt, extent: 3pt, background: true, evade: false)[important]",
        );

        let LineNode::TextSpan(span) = &line.nodes[0] else {
            panic!("expected text span");
        };
        assert_eq!(span.kind, TextMarkupKind::Underline);
        assert_eq!(
            span.options.decoration.stroke.paint,
            Some(Color::rgba(1.0, 65.0 / 255.0, 54.0 / 255.0, 1.0))
        );
        assert_eq!(
            span.options.decoration.stroke.thickness,
            Some(DecorationLength::Pt(1.5))
        );
        assert_eq!(span.options.decoration.stroke.line_cap, None);
        assert_eq!(
            span.options.decoration.offset,
            Some(DecorationLength::Pt(2.0))
        );
        assert_eq!(span.options.decoration.extent, DecorationLength::Pt(3.0));
        assert!(span.options.decoration.background);
        assert_eq!(span.options.decoration.evade, Some(false));
    }

    #[test]
    fn parses_non_stroke_decoration_option_params() {
        let params = scope([
            ("underline_offset", Value::Str("2pt".to_string())),
            ("underline_extent", Value::Str("-0.5em".to_string())),
            ("underline_background", Value::Bool(true)),
            ("underline_evade", Value::Bool(false)),
        ]);

        let line = parse_with_params(
            "#underline(offset: underline_offset, extent: underline_extent, background: underline_background, evade: underline_evade)[care]",
            &params,
        );

        let LineNode::TextSpan(span) = &line.nodes[0] else {
            panic!("expected text span");
        };
        assert_eq!(span.kind, TextMarkupKind::Underline);
        assert_eq!(
            span.options.decoration.offset,
            Some(DecorationLength::Pt(2.0))
        );
        assert_eq!(span.options.decoration.extent, DecorationLength::Em(-0.5));
        assert!(span.options.decoration.background);
        assert_eq!(span.options.decoration.evade, Some(false));
    }

    #[test]
    fn parses_script_smallcaps_and_strong_option_params() {
        let params = scope([
            ("use_typographic", Value::Bool(false)),
            ("script_baseline", Value::Str("-0.25em".to_string())),
            ("script_size", Value::Str("8pt".to_string())),
            ("all_caps", Value::Bool(true)),
            ("weight_delta", Value::Int(150)),
        ]);

        let line = parse_with_params(
            "#super(typographic: use_typographic, baseline: script_baseline, size: script_size)[N] \
             #smallcaps(all: all_caps)[UNICEF] \
             #strong(delta: weight_delta)[bold]",
            &params,
        );

        assert_eq!(line.nodes.len(), 5);
        assert!(matches!(&line.nodes[0], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Superscript
                && !span.options.script.typographic
                && span.options.script.baseline == Some(DecorationLength::Em(-0.25))
                && span.options.script.size == Some(DecorationLength::Pt(8.0))
        ));
        assert!(matches!(&line.nodes[2], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Smallcaps
                && span.options.smallcaps.all
        ));
        assert!(matches!(&line.nodes[4], LineNode::TextSpan(span)
            if span.kind == TextMarkupKind::Strong
                && span.options.strong.delta == 150
        ));
    }

    #[test]
    fn parses_numfmt_plain_output() {
        let params = scope([("value", Value::Float(1234.5))]);
        let line = parse_with_params("Peak #numfmt(value, \",.1f\") N", &params);

        assert_eq!(line.nodes.len(), 1);
        assert!(matches!(&line.nodes[0], LineNode::Plain(plain) if plain.text == "Peak 1,234.5 N"));
    }

    #[test]
    fn parses_numfmt_with_custom_locale_registry() {
        let mut registry = NumberLocaleRegistry::with_builtins();
        registry
            .register_custom_locale_json(
                "label-test",
                r#"{ "base": "en-US", "decimal": "~", "group": "_" }"#,
            )
            .expect("custom locale");
        let params = scope([("value", Value::Float(1234.5))]);
        let line = parse_line_with_number_format_context(
            "#numfmt(value, \",.1f\")",
            &params,
            NumberFormatMarkupContext {
                locale_id: Some("label-test"),
                registry: Some(&registry),
            },
        )
        .expect("line");

        assert_eq!(line.nodes.len(), 1);
        assert!(matches!(&line.nodes[0], LineNode::Plain(plain) if plain.text == "1_234~5"));
    }

    #[test]
    fn parses_datefmt_naive_date_output() {
        let params = scope([(
            "value",
            Value::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 5).unwrap()),
        )]);
        let line = parse_with_params("#datefmt(value, \"MMM d, y\")", &params);

        assert_eq!(line.nodes.len(), 1);
        assert!(matches!(&line.nodes[0], LineNode::Plain(plain) if plain.text == "Jan 5, 2024"));
    }

    #[test]
    fn parses_datefmt_style_override_params() {
        let params = scope([
            (
                "value",
                Value::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 5).unwrap()),
            ),
            ("style", Value::Str("long".to_string())),
        ]);
        let line = parse_with_params("#datefmt(value, \"{date}\", date_style: style)", &params);

        assert_eq!(line.nodes.len(), 1);
        assert!(
            matches!(&line.nodes[0], LineNode::Plain(plain) if plain.text == "January 5, 2024")
        );
    }

    #[test]
    fn parses_datefmt_zoned_with_context_timezone() {
        let params = scope([(
            "value",
            Value::UtcDateTime(
                chrono::DateTime::from_timestamp(1_704_067_200, 0).expect("UTC datetime"),
            ),
        )]);
        let line = parse_line_with_format_context(
            "#datefmt(value, \"y-MM-dd HH:mm\")",
            &params,
            MarkupFormatContext {
                datetime: DateTimeFormatMarkupContext {
                    timezone: Some("America/New_York"),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .expect("line");

        assert_eq!(line.nodes.len(), 1);
        assert!(
            matches!(&line.nodes[0], LineNode::Plain(plain) if plain.text == "2023-12-31 19:00")
        );
    }

    #[test]
    fn parses_datefmt_with_call_locale_override() {
        let mut registry = DateTimeLocaleRegistry::with_builtins();
        registry
            .register_custom_locale_json(
                "label-date",
                r#"{ "base": "en-US", "date_patterns": { "long": "y'~'MM'~'dd" } }"#,
            )
            .expect("custom datetime locale");
        let params = scope([(
            "value",
            Value::Date(chrono::NaiveDate::from_ymd_opt(2024, 1, 5).unwrap()),
        )]);
        let line = parse_line_with_format_context(
            "#datefmt(value, \"{date:long}\", locale: \"label-date\")",
            &params,
            MarkupFormatContext {
                datetime: DateTimeFormatMarkupContext {
                    registry: Some(&registry),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .expect("line");

        assert_eq!(line.nodes.len(), 1);
        assert!(matches!(&line.nodes[0], LineNode::Plain(plain) if plain.text == "2024~01~05"));
    }

    #[test]
    fn parses_datefmt_zoned_with_tz_alias_override() {
        let params = scope([(
            "value",
            Value::UtcDateTime(
                chrono::DateTime::from_timestamp(1_704_067_200, 0).expect("UTC datetime"),
            ),
        )]);
        let line = parse_with_params(
            "#datefmt(value, \"y-MM-dd HH:mm\", tz: \"America/New_York\")",
            &params,
        );

        assert_eq!(line.nodes.len(), 1);
        assert!(
            matches!(&line.nodes[0], LineNode::Plain(plain) if plain.text == "2023-12-31 19:00")
        );
    }

    #[test]
    fn parses_numfmt_exponent_as_math() {
        let params = scope([("value", Value::Float(1200.0))]);
        let line = parse_with_params("#numfmt(value, \".1e\")", &params);

        assert_eq!(line.nodes.len(), 1);
        assert!(
            matches!(&line.nodes[0], LineNode::Math(math) if math.source == "1.2 times 10^(3)")
        );
    }

    #[test]
    fn parses_numfmt_named_override_params() {
        let params = scope([("value", Value::Float(1.234)), ("precision", Value::Int(1))]);
        let line = parse_with_params("#numfmt(value, \".3f\", precision: precision)", &params);

        assert_eq!(line.nodes.len(), 1);
        assert!(matches!(&line.nodes[0], LineNode::Plain(plain) if plain.text == "1.2"));
    }

    #[test]
    fn parses_numfmt_width_fill_align_overrides() {
        let params = scope([("value", Value::Float(42.0))]);
        let line = parse_with_params(
            "#numfmt(value, \".0f\", width: 5, fill: \".\", align: \"<\")",
            &params,
        );

        assert_eq!(line.nodes.len(), 1);
        assert!(matches!(&line.nodes[0], LineNode::Plain(plain) if plain.text == "42..."));
    }

    #[test]
    fn parses_numfmt_width_none_override() {
        let params = scope([("value", Value::Float(42.0))]);
        let line = parse_with_params("#numfmt(value, \"08.0f\", width: none)", &params);

        assert_eq!(line.nodes.len(), 1);
        assert!(matches!(&line.nodes[0], LineNode::Plain(plain) if plain.text == "42"));
    }

    #[test]
    fn rejects_duplicate_numfmt_digit_options() {
        let params = scope([("value", Value::Float(1.234))]);
        let err = parse_line_with_params(
            "#numfmt(value, \".3f\", precision: 1, fraction_digits: 2)",
            &params,
        )
        .unwrap_err();

        assert!(matches!(
            err,
            LabelError::UnsupportedSyntax { message, .. }
                if message == "numfmt accepts only one digit-control option"
        ));
    }

    #[test]
    fn rejects_invalid_non_stroke_option_param_casts() {
        let params = scope([("badlength", Value::Bool(true))]);

        let err =
            parse_line_with_params("#underline(offset: badlength)[care]", &params).unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 11,
                message: "label parameter cannot be cast to decoration length"
            }
        );
    }

    #[test]
    fn rejects_unsupported_decoration_options() {
        let err = parse_line("#strike(evade: false)[old]").unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 8,
                message: "strike does not support evade"
            }
        );

        let err = parse_line("#underline(stroke: (miter-limit: 2pt))[group]").unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 20,
                message: "unsupported stroke miter limit"
            }
        );

        let err =
            parse_line("#underline(stroke: 1pt + gradient.linear(red, blue))[group]").unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 11,
                message: "unsupported decoration paint"
            }
        );

        let err = parse_line("#underline(stroke: pattern())[group]").unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 11,
                message: "unsupported decoration paint"
            }
        );
    }

    #[test]
    fn parses_stroke_cap_join_and_dash() {
        let line = parse(
            "#underline(stroke: (cap: \"round\", join: \"bevel\", dash: (array: (2pt, \"dot\"), phase: 0.5pt), miter-limit: 2))[x]",
        );

        let LineNode::TextSpan(span) = &line.nodes[0] else {
            panic!("expected text span");
        };
        assert_eq!(
            span.options.decoration.stroke.line_cap,
            Some(LineCap::Round)
        );
        assert_eq!(
            span.options.decoration.stroke.line_join,
            Some(LineJoin::Bevel)
        );
        assert_eq!(
            span.options
                .decoration
                .stroke
                .dash
                .as_ref()
                .map(|dash| dash.array.len()),
            Some(2)
        );
        assert_eq!(
            span.options
                .decoration
                .stroke
                .dash
                .as_ref()
                .map(|dash| dash.phase),
            Some(DecorationLength::Pt(0.5))
        );
        assert_eq!(span.options.decoration.stroke.miter_limit, Some(2.0));
    }

    #[test]
    fn rejects_unknown_hash_commands() {
        let err = parse_line("#let x = 1").unwrap_err();

        assert!(matches!(
            err,
            LabelError::UnsupportedSyntax {
                position: 0,
                message: "unsupported static text command"
            }
        ));
    }

    #[test]
    fn rejects_unknown_emoji_aliases() {
        let err = parse_line("#emoji.not.real").unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 0,
                message: "unknown emoji alias"
            }
        );
    }

    #[test]
    fn rejects_unknown_symbol_aliases() {
        let err = parse_line("#sym.not.real").unwrap_err();

        assert_eq!(
            err,
            LabelError::UnsupportedSyntax {
                position: 0,
                message: "unknown symbol alias"
            }
        );
    }
}
