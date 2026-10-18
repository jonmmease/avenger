pub mod axis;
pub mod error;
mod guide;
pub mod legend;

pub use guide::{BackgroundStyle, GuideSide, PartExtent};

/// The bundled fonts with D3 number and datetime formatting, in UTC.
#[cfg(test)]
pub(crate) fn test_engine() -> avenger_typst_label::LabelEngine {
    use std::sync::Arc;
    avenger_typst_label::bundled_label_engine()
        .with_number_formatting(Arc::new(
            avenger_format_number_d3::D3NumberFormatProvider::new(),
        ))
        .with_datetime_formatting(Arc::new(
            avenger_format_datetime_d3::D3DateTimeFormatProvider::new(),
        ))
}
