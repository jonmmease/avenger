use std::collections::HashMap;

use datafusion::{prelude::Expr, scalar::ScalarValue};
use palette::{Hsla, Laba, Srgba};

#[derive(Debug, Clone)]
pub struct Scale {
    pub name: String,
    pub kind: Option<String>,
    pub domain: Option<ScaleDomain>,
    pub range: Option<ScaleRange>,
    pub options: HashMap<String, Expr>,
}

impl Scale {
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            kind: None,
            domain: None,
            range: None,
            options: HashMap::new(),
        }
    }

    pub fn kind<S: Into<String>>(self, kind: S) -> Self {
        Self {
            kind: Some(kind.into()),
            ..self
        }
    }

    pub fn get_kind(&self) -> Option<&String> {
        self.kind.as_ref()
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

    pub fn range_color(self, colors: Vec<Srgba>) -> Self {
        Self {
            range: Some(ScaleRange::Color(colors)),
            ..self
        }
    }

    // Other builder methods
    pub fn option(mut self, key: String, value: Expr) -> Self {
        self.options.insert(key, value);
        self
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
    Enum(Vec<String>),
    Color(Vec<Srgba>),
}

impl ScaleRange {
    pub fn new_numeric<E: Into<Expr>>(start: E, end: E) -> Self {
        Self::Numeric(start.into(), end.into())
    }

    pub fn new_color(colors: Vec<Srgba>) -> Self {
        Self::Color(colors)
    }
}
