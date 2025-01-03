use datafusion::prelude::DataFrame;
use datafusion::{
    prelude::{lit, Expr},
    scalar::ScalarValue,
};
use indexmap::IndexMap;

use avenger_scenegraph::marks::symbol::SymbolShape;

#[derive(Debug, Clone)]
pub struct Mark {
    pub mark_type: String,
    pub name: Option<String>,
    pub from: Option<DataFrame>,
    pub details: Option<Vec<String>>,
    pub encodings: IndexMap<String, Expr>,
    pub zindex: Option<i32>,
    pub shapes: Option<Vec<SymbolShape>>,
}

macro_rules! encoding_fn {
    ($name:ident) => {
        pub fn $name<E: Into<Expr>>(self, value: E) -> Self {
            let mut encodings = self.encodings;
            encodings.insert(stringify!($name).to_string(), value.into());
            Self { encodings, ..self }
        }
    };
}

macro_rules! mark_type_fn {
    ($name:ident) => {
        pub fn $name() -> Self {
            Self::new(stringify!($name).to_string())
        }
    };
}

impl Mark {
    pub fn new<S: Into<String>>(mark_type: S) -> Self {
        Self {
            mark_type: mark_type.into(),
            name: None,
            from: None,
            encodings: Default::default(),
            zindex: None,
            details: None,
            shapes: None,
        }
    }

    /// Set or overwrite the source dataset for the mark.
    pub fn from(self, from: DataFrame) -> Self {
        Self {
            from: Some(from),
            ..self
        }
    }

    pub fn get_from(&self) -> Option<&DataFrame> {
        self.from.as_ref()
    }

    pub fn details<S: Into<String>>(self, details: Vec<S>) -> Self {
        Self {
            details: Some(details.into_iter().map(|s| s.into()).collect()),
            ..self
        }
    }

    pub fn get_details(&self) -> &Option<Vec<String>> {
        &self.details
    }

    pub fn get_mark_type(&self) -> &str {
        &self.mark_type
    }

    pub fn shapes(self, shapes: Vec<SymbolShape>) -> Self {
        Self {
            shapes: Some(shapes),
            ..self
        }
    }

    pub fn get_shapes(&self) -> &Option<Vec<SymbolShape>> {
        &self.shapes
    }

    pub fn zindex(self, zindex: i32) -> Self {
        Self { zindex: Some(zindex), ..self }
    }

    pub fn get_zindex(&self) -> Option<i32> {
        self.zindex
    }

    // Constructor helpers for primitive mark types
    mark_type_fn!(arc);
    mark_type_fn!(area);
    mark_type_fn!(image);
    mark_type_fn!(line);
    mark_type_fn!(path);
    mark_type_fn!(rect);
    mark_type_fn!(rule);
    mark_type_fn!(symbol);
    mark_type_fn!(text);
    mark_type_fn!(trail);

    pub fn name<S: Into<String>>(self, name: S) -> Self {
        Self {
            name: Some(name.into()),
            ..self
        }
    }

    /// Set or overwrite an encoding for the mark.
    pub fn encode<S: Into<String>, E: Into<Expr>>(self, name: S, expr: E) -> Self {
        let mut encodings = self.encodings;
        encodings.insert(name.into(), expr.into());
        Self { encodings, ..self }
    }

    // Helpers for common encodings
    encoding_fn!(x);
    encoding_fn!(x2);
    encoding_fn!(y);
    encoding_fn!(y2);
    encoding_fn!(width);
    encoding_fn!(height);
    encoding_fn!(fill);
    encoding_fn!(shape_index);
    encoding_fn!(stroke);
    encoding_fn!(stroke_width);
    encoding_fn!(stroke_dash);
    encoding_fn!(stroke_dash_offset);
    encoding_fn!(stroke_opacity);
    encoding_fn!(size);
    encoding_fn!(angle);

    // Arc mark encodings
    encoding_fn!(start_angle);
    encoding_fn!(end_angle);
    encoding_fn!(outer_radius);
    encoding_fn!(inner_radius);
    encoding_fn!(pad_angle);
    encoding_fn!(corner_radius);
}

#[derive(Debug, Clone)]
pub enum Encoding {
    Expr(Expr),
    Scaled(ScaledEncoding),
}

impl Encoding {
    pub fn is_scalar(&self) -> bool {
        self.inner_expr().column_refs().is_empty()
    }

    pub fn inner_expr(&self) -> &Expr {
        match self {
            Encoding::Expr(expr) => expr,
            Encoding::Scaled(scaled) => &scaled.expr,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScaledEncoding {
    expr: Expr,
    scale: String,
    offset: Option<f32>,
    band: Option<f32>,
}

impl ScaledEncoding {
    pub fn new(expr: Expr, name: &str) -> Self {
        Self {
            expr,
            scale: name.to_string(),
            offset: None,
            band: None,
        }
    }

    pub fn offset(self, offset: f32) -> Self {
        Self {
            offset: Some(offset),
            ..self
        }
    }

    pub fn get_offset(&self) -> Option<f32> {
        self.offset
    }

    pub fn band(self, band: f32) -> Self {
        Self {
            band: Some(band),
            ..self
        }
    }

    pub fn get_band(&self) -> Option<f32> {
        self.band
    }

    pub fn get_expr(&self) -> &Expr {
        &self.expr
    }

    pub fn get_scale(&self) -> &str {
        &self.scale
    }
}

impl From<Expr> for Encoding {
    fn from(expr: Expr) -> Self {
        Encoding::Expr(expr)
    }
}

impl From<ScaledEncoding> for Encoding {
    fn from(scaled: ScaledEncoding) -> Self {
        Encoding::Scaled(scaled)
    }
}

impl From<ScalarValue> for Encoding {
    fn from(value: ScalarValue) -> Self {
        Encoding::Expr(lit(value))
    }
}

pub trait EncodingUtils {
    fn scale(self, name: &str) -> ScaledEncoding;
}

impl EncodingUtils for Expr {
    fn scale(self, name: &str) -> ScaledEncoding {
        ScaledEncoding::new(self, name)
    }
}
