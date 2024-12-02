pub mod linear;
pub mod log;
pub mod opts;
pub mod pow;
pub mod symlog;

use avenger_common::value::{ScalarOrArray, ScalarOrArrayRef};
use linear::LinearNumericScale;
use log::LogNumericScale;
use opts::NumericScaleOptions;
use pow::PowNumericScale;
use symlog::SymlogNumericScale;

use crate::error::AvengerScaleError;

#[derive(Clone, Debug)]
pub enum NumericScale {
    Linear(LinearNumericScale),
    Log(LogNumericScale),
    Pow(PowNumericScale),
    Symlog(SymlogNumericScale),
}

impl NumericScale {
    pub fn domain(self, (start, end): (f32, f32)) -> Self {
        match self {
            NumericScale::Linear(scale) => NumericScale::Linear(scale.domain((start, end))),
            NumericScale::Log(scale) => NumericScale::Log(scale.domain((start, end))),
            NumericScale::Pow(scale) => NumericScale::Pow(scale.domain((start, end))),
            NumericScale::Symlog(scale) => NumericScale::Symlog(scale.domain((start, end))),
        }
    }

    pub fn get_domain(&self) -> (f32, f32) {
        match self {
            NumericScale::Linear(scale) => scale.get_domain(),
            NumericScale::Log(scale) => scale.get_domain(),
            NumericScale::Pow(scale) => scale.get_domain(),
            NumericScale::Symlog(scale) => scale.get_domain(),
        }
    }

    pub fn range(self, (start, end): (f32, f32)) -> Self {
        match self {
            NumericScale::Linear(scale) => NumericScale::Linear(scale.range((start, end))),
            NumericScale::Log(scale) => NumericScale::Log(scale.range((start, end))),
            NumericScale::Pow(scale) => NumericScale::Pow(scale.range((start, end))),
            NumericScale::Symlog(scale) => NumericScale::Symlog(scale.range((start, end))),
        }
    }

    pub fn get_range(&self) -> (f32, f32) {
        match self {
            NumericScale::Linear(scale) => scale.get_range(),
            NumericScale::Log(scale) => scale.get_range(),
            NumericScale::Pow(scale) => scale.get_range(),
            NumericScale::Symlog(scale) => scale.get_range(),
        }
    }

    pub fn clamp(self, clamp: bool) -> Self {
        match self {
            NumericScale::Linear(scale) => NumericScale::Linear(scale.clamp(clamp)),
            NumericScale::Log(scale) => NumericScale::Log(scale.clamp(clamp)),
            NumericScale::Pow(scale) => NumericScale::Pow(scale.clamp(clamp)),
            NumericScale::Symlog(scale) => NumericScale::Symlog(scale.clamp(clamp)),
        }
    }

    pub fn get_clamp(&self) -> bool {
        match self {
            NumericScale::Linear(scale) => scale.get_clamp(),
            NumericScale::Log(scale) => scale.get_clamp(),
            NumericScale::Pow(scale) => scale.get_clamp(),
            NumericScale::Symlog(scale) => scale.get_clamp(),
        }
    }

    pub fn nice(self, count: Option<usize>) -> Self {
        match self {
            NumericScale::Linear(scale) => NumericScale::Linear(scale.nice(count)),
            NumericScale::Log(scale) => NumericScale::Log(scale.nice()),
            NumericScale::Pow(scale) => NumericScale::Pow(scale.nice(count)),
            NumericScale::Symlog(scale) => NumericScale::Symlog(scale.nice(count)),
        }
    }

    pub fn scale<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, f32>>,
        opts: &NumericScaleOptions,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        match self {
            NumericScale::Linear(scale) => scale.scale(values, opts),
            NumericScale::Log(scale) => scale.scale(values, opts),
            NumericScale::Pow(scale) => scale.scale(values, opts),
            NumericScale::Symlog(scale) => scale.scale(values, opts),
        }
    }

    pub fn invert<'a>(
        &self,
        values: impl Into<ScalarOrArrayRef<'a, f32>>,
        opts: &NumericScaleOptions,
    ) -> Result<ScalarOrArray<f32>, AvengerScaleError> {
        match self {
            NumericScale::Linear(scale) => scale.invert(values, opts),
            NumericScale::Log(scale) => scale.invert(values, opts),
            NumericScale::Pow(scale) => scale.invert(values, opts),
            NumericScale::Symlog(scale) => scale.invert(values, opts),
        }
    }

    pub fn ticks(&self, count: Option<f32>) -> Vec<f32> {
        match self {
            NumericScale::Linear(scale) => scale.ticks(count),
            NumericScale::Log(scale) => scale.ticks(count),
            NumericScale::Pow(scale) => scale.ticks(count),
            NumericScale::Symlog(scale) => scale.ticks(count),
        }
    }
}

impl From<LinearNumericScale> for NumericScale {
    fn from(scale: LinearNumericScale) -> Self {
        NumericScale::Linear(scale)
    }
}

impl From<LogNumericScale> for NumericScale {
    fn from(scale: LogNumericScale) -> Self {
        NumericScale::Log(scale)
    }
}

impl From<PowNumericScale> for NumericScale {
    fn from(scale: PowNumericScale) -> Self {
        NumericScale::Pow(scale)
    }
}

impl From<SymlogNumericScale> for NumericScale {
    fn from(scale: SymlogNumericScale) -> Self {
        NumericScale::Symlog(scale)
    }
}
