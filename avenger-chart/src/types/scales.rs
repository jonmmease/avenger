use datafusion::{prelude::Expr, scalar::ScalarValue};
use palette::{Hsla, Laba, Srgba};

#[derive(Debug, Clone)]
pub struct Scale {
    pub scale_type: Option<String>,
    pub domain: Option<ScaleDomain>,
    pub domain_raw: Option<ScaleDomain>,
    pub range: Option<ScaleRange>,
    pub round: Option<bool>,
}

impl Scale {
    pub fn new() -> Self {
        Self {
            scale_type: None,
            domain: None,
            domain_raw: None,
            range: None,
            round: None,
        }
    }

    pub fn scale_type<S: Into<String>>(self, scale_type: S) -> Self {
        Self {
            scale_type: Some(scale_type.into()),
            ..self
        }
    }

    pub fn get_scale_type(&self) -> Option<&String> {
        self.scale_type.as_ref()
    }

    // Domain builders
    pub fn domain(self, domain: ScaleDomain) -> Self {
        Self {
            domain: Some(domain),
            ..self
        }
    }

    pub fn get_domain(&self) -> Option<&ScaleDomain> {
        self.domain.as_ref()
    }

    pub fn domain_interval<T: Into<Expr>>(self, start: T, end: T) -> Self {
        Self {
            domain: Some(ScaleDomain::Interval(start.into(), end.into())),
            ..self
        }
    }

    pub fn domain_discrete<T: Into<Expr>>(self, values: Vec<T>) -> Self {
        Self {
            domain: Some(ScaleDomain::Discrete(
                values.into_iter().map(|v| v.into()).collect(),
            )),
            ..self
        }
    }

    pub fn domain_data_field(self, dataset: String, field: String) -> Self {
        Self {
            domain: Some(ScaleDomain::DataField(DataField { dataset, field })),
            ..self
        }
    }

    pub fn domain_data_fields<S: Into<String>>(self, fields: Vec<(S, S)>) -> Self {
        Self {
            domain: Some(ScaleDomain::DataFields(
                fields
                    .into_iter()
                    .map(|(dataset, field)| DataField {
                        dataset: dataset.into(),
                        field: field.into(),
                    })
                    .collect(),
            )),
            ..self
        }
    }

    // Range builders
    pub fn range(self, range: ScaleRange) -> Self {
        Self {
            range: Some(range),
            ..self
        }
    }

    pub fn get_range(&self) -> Option<&ScaleRange> {
        self.range.as_ref()
    }

    pub fn range_numeric<F: Into<Expr>>(self, start: F, end: F) -> Self {
        Self {
            range: Some(ScaleRange::Numeric(start.into(), end.into())),
            ..self
        }
    }

    pub fn range_rgb(self, colors: Vec<Srgba>) -> Self {
        Self {
            range: Some(ScaleRange::Rgb(colors)),
            ..self
        }
    }

    pub fn range_hsl(self, colors: Vec<Hsla>) -> Self {
        Self {
            range: Some(ScaleRange::Hsl(colors)),
            ..self
        }
    }

    pub fn range_lab(self, colors: Vec<Laba>) -> Self {
        Self {
            range: Some(ScaleRange::Lab(colors)),
            ..self
        }
    }

    // Other builder methods
    pub fn round(self, round: bool) -> Self {
        Self {
            round: Some(round),
            ..self
        }
    }

    pub fn get_round(&self) -> Option<bool> {
        self.round
    }
}

#[derive(Debug, Clone)]
pub enum ScaleDomain {
    // Intervals
    Interval(Expr, Expr),
    // Discrete values
    Discrete(Vec<Expr>),

    // Domain derived from data
    DataField(DataField),
    DataFields(Vec<DataField>),
}

impl ScaleDomain {
    pub fn new_interval<E: Into<Expr>>(start: E, end: E) -> Self {
        Self::Interval(start.into(), end.into())
    }

    pub fn new_discrete(values: Vec<Expr>) -> Self {
        Self::Discrete(values)
    }

    pub fn new_data_field<S: Into<String>>(self, dataset: S, field: S) -> Self {
        Self::DataField(DataField {
            dataset: dataset.into(),
            field: field.into(),
        })
    }

    pub fn new_data_fields<S: Into<String>>(self, fields: Vec<(S, S)>) -> Self {
        Self::DataFields(
            fields
                .into_iter()
                .map(|(dataset, field)| DataField {
                    dataset: dataset.into(),
                    field: field.into(),
                })
                .collect(),
        )
    }
}

#[derive(Debug, Clone)]
pub struct DataField {
    pub dataset: String,
    pub field: String,
}

#[derive(Debug, Clone)]
pub enum ScaleRange {
    Numeric(Expr, Expr),
    Rgb(Vec<Srgba>),
    Hsl(Vec<Hsla>),
    Lab(Vec<Laba>),
}

impl ScaleRange {
    pub fn new_numeric<E: Into<Expr>>(start: E, end: E) -> Self {
        Self::Numeric(start.into(), end.into())
    }

    pub fn new_rgb(colors: Vec<Srgba>) -> Self {
        Self::Rgb(colors)
    }

    pub fn new_hsl(colors: Vec<Hsla>) -> Self {
        Self::Hsl(colors)
    }

    pub fn new_lab(colors: Vec<Laba>) -> Self {
        Self::Lab(colors)
    }
}
