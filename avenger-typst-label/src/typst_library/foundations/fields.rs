//! Ported from crates/typst-library/src/foundations/fields.rs @ v0.15.1, modified for Avenger.
//!
//! Fields on values.
//!
//! avenger: no versions; `fields_on`, which only documentation and autocompletion read, is
//! gone.

use ecow::{EcoString, eco_format};

use crate::typst_library::diag::StrResult;
use crate::typst_library::foundations::{IntoValue, Type, Value};
use crate::typst_library::layout::Alignment;
use crate::typst_library::visualize::Stroke;

/// Try to access a field on a value.
///
/// This function is exclusively for types which have predefined fields, such as
/// stroke and length.
pub(crate) fn field(value: &Value, field: &str) -> StrResult<Value> {
    let ty = value.ty();
    let nope = || Err(no_fields(ty));
    let missing = || Err(missing_field(ty, field));

    // Special cases, such as module and dict, are handled by Value itself
    let result = match value {
        Value::Length(length) => match field {
            "em" => length.em.get().into_value(),
            "abs" => length.abs.into_value(),
            _ => return missing(),
        },
        Value::Relative(rel) => match field {
            "ratio" => rel.rel.into_value(),
            "length" => rel.abs.into_value(),
            _ => return missing(),
        },
        Value::Dyn(dynamic) => {
            if let Some(stroke) = dynamic.downcast::<Stroke>() {
                match field {
                    "paint" => stroke.paint.clone().into_value(),
                    "thickness" => stroke.thickness.into_value(),
                    "cap" => stroke.cap.into_value(),
                    "join" => stroke.join.into_value(),
                    "dash" => stroke.dash.clone().into_value(),
                    "miter-limit" => {
                        stroke.miter_limit.map(|limit| limit.get()).into_value()
                    }
                    _ => return missing(),
                }
            } else if let Some(align) = dynamic.downcast::<Alignment>() {
                match field {
                    "x" => align.x().into_value(),
                    "y" => align.y().into_value(),
                    _ => return missing(),
                }
            } else {
                return nope();
            }
        }
        _ => return nope(),
    };

    Ok(result)
}

/// The error message for a type not supporting field access.
#[cold]
fn no_fields(ty: Type) -> EcoString {
    eco_format!("cannot access fields on type {ty}")
}

/// The missing field error message.
#[cold]
fn missing_field(ty: Type, field: &str) -> EcoString {
    eco_format!("{ty} does not contain field \"{field}\"")
}
