/// Plot-area dimension used by a coordinate-owned scale range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlotAreaDimension {
    Width,
    Height,
    MinWidthHeight,
}

impl PlotAreaDimension {
    fn resolve(self, plot_area_width: f64, plot_area_height: f64) -> f64 {
        match self {
            PlotAreaDimension::Width => plot_area_width,
            PlotAreaDimension::Height => plot_area_height,
            PlotAreaDimension::MinWidthHeight => plot_area_width.min(plot_area_height),
        }
    }
}

/// One endpoint of a coordinate-owned scale range.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PlotAreaRangeEndpoint {
    Constant(f64),
    Dimension {
        dimension: PlotAreaDimension,
        factor: f64,
    },
}

impl PlotAreaRangeEndpoint {
    pub const ZERO: Self = Self::Constant(0.0);
    pub const WIDTH: Self = Self::Dimension {
        dimension: PlotAreaDimension::Width,
        factor: 1.0,
    };
    pub const HEIGHT: Self = Self::Dimension {
        dimension: PlotAreaDimension::Height,
        factor: 1.0,
    };
    pub const HALF_MIN_DIMENSION: Self = Self::Dimension {
        dimension: PlotAreaDimension::MinWidthHeight,
        factor: 0.5,
    };

    fn resolve(self, plot_area_width: f64, plot_area_height: f64) -> f64 {
        match self {
            PlotAreaRangeEndpoint::Constant(value) => value,
            PlotAreaRangeEndpoint::Dimension { dimension, factor } => {
                dimension.resolve(plot_area_width, plot_area_height) * factor
            }
        }
    }
}

/// Coordinate-owned expression for a scale range as a function of plot dimensions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlotAreaRangeExpr {
    pub start: PlotAreaRangeEndpoint,
    pub end: PlotAreaRangeEndpoint,
}

impl PlotAreaRangeExpr {
    pub const fn new(start: PlotAreaRangeEndpoint, end: PlotAreaRangeEndpoint) -> Self {
        Self { start, end }
    }

    pub fn resolve(self, plot_area_width: f64, plot_area_height: f64) -> (f64, f64) {
        (
            self.start.resolve(plot_area_width, plot_area_height),
            self.end.resolve(plot_area_width, plot_area_height),
        )
    }
}

/// Describes how a configured scale range should respond to plot-area resizing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScaleRangeBinding {
    /// The coordinate system owns this range, and it should be recomputed after
    /// plot-area dimensions change.
    PlotArea(PlotAreaRangeExpr),
    /// The coordinate system supplied this fixed range, but it is not dimension-dependent.
    FixedInterval(f64, f64),
    /// The range comes from a mark, theme, user configuration, or another non-layout source.
    Independent,
}

impl ScaleRangeBinding {
    pub const fn plot_area(start: PlotAreaRangeEndpoint, end: PlotAreaRangeEndpoint) -> Self {
        Self::PlotArea(PlotAreaRangeExpr::new(start, end))
    }

    pub const fn fixed_interval(start: f64, end: f64) -> Self {
        Self::FixedInterval(start, end)
    }

    pub fn resolve(self, plot_area_width: f64, plot_area_height: f64) -> Option<(f64, f64)> {
        match self {
            ScaleRangeBinding::PlotArea(expr) => {
                Some(expr.resolve(plot_area_width, plot_area_height))
            }
            ScaleRangeBinding::FixedInterval(start, end) => Some((start, end)),
            ScaleRangeBinding::Independent => None,
        }
    }

    pub fn resolve_for_retarget(
        self,
        plot_area_width: f64,
        plot_area_height: f64,
    ) -> Option<(f64, f64)> {
        match self {
            ScaleRangeBinding::PlotArea(expr) => {
                Some(expr.resolve(plot_area_width, plot_area_height))
            }
            ScaleRangeBinding::FixedInterval(_, _) | ScaleRangeBinding::Independent => None,
        }
    }
}
