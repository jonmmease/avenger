use datafusion::{
    prelude::{lit, Expr},
    scalar::ScalarValue,
};
use indexmap::IndexMap;

#[derive(Debug, Clone)]
pub struct Mark {
    pub mark_type: String,
    pub from: Option<String>,
    pub encodings: IndexMap<String, Encoding>,
}

macro_rules! encoding_fn {
    ($name:ident) => {
        pub fn $name<E: Into<Encoding>>(self, value: E) -> Self {
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
            from: None,
            encodings: Default::default(),
        }
    }

    /// Set or overwrite the source dataset for the mark.
    pub fn from<S: Into<String>>(self, from: S) -> Self {
        Self {
            from: Some(from.into()),
            ..self
        }
    }

    pub fn get_from(&self) -> Option<&str> {
        self.from.as_deref()
    }

    pub fn get_mark_type(&self) -> &str {
        &self.mark_type
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

    /// Set or overwrite an encoding for the mark.
    pub fn encode<S: Into<String>, E: Into<Encoding>>(self, name: S, expr: E) -> Self {
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
    encoding_fn!(stroke);
    encoding_fn!(stroke_width);
    encoding_fn!(stroke_dash);
    encoding_fn!(stroke_dash_offset);
    encoding_fn!(stroke_opacity);

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

#[cfg(test)]
mod tests {
    use datafusion::prelude::{col, lit};

    use super::*;

    #[test]
    fn test_mark() {
        let calc = (lit(3.0) / col("foo")).scale("xscale").offset(1.0);
        println!("{:#?}", calc);

        let mark = Mark::line()
            .from("data_0")
            .x(col("sepal_width").scale("xscale"))
            .y(col("sepal_length").scale("yscale"))
            .width(lit(3.0) / col("foo"))
            .height(lit(4.0))
            .fill(lit("red"))
            .stroke(lit("blue"))
            .stroke_width(lit(1.0))
            .stroke_dash_offset(lit(3.0));

        println!("{:#?}", mark);
    }
}
