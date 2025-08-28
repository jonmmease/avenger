use crate::legend_builder::{
    AngleLegendBuilder, ColorLegendBuilder, LegendBuilder, OpacityLegendBuilder,
    ShapeLegendBuilder, SizeLegendBuilder, StrokeDashLegendBuilder, StrokeWidthLegendBuilder,
};
use crate::marks::channel::{ChannelValue, LegendConfig, ScaleConfig};
use crate::scales::{Auto, Scale, ScaleSpec};
use std::sync::Arc;

// Channel config for color channels (fill, stroke, color)
pub struct ColorChannelConfig {
    value: ChannelValue,
}

impl ColorChannelConfig {
    pub fn new(value: ChannelValue) -> Self {
        Self { value }
    }

    /// Configure the legend for this color channel
    pub fn legend<F>(mut self, f: F) -> Self
    where
        F: FnOnce(ColorLegendBuilder) -> ColorLegendBuilder,
    {
        let builder = ColorLegendBuilder::new();
        let configured = f(builder);
        let legend = configured.build();

        // Convert to the closure type expected by ChannelValue
        let legend_config: LegendConfig = Arc::new(move |_| legend.clone());

        // Update the channel value with legend config
        self.value = match self.value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                legend_config: Some(legend_config),
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config: Some(legend_config),
            },
            other => other,
        };

        self
    }

    /// Configure the scale for this channel
    pub fn scale<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        let scale_config: ScaleConfig = Arc::new(f);

        self.value = match self.value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                legend_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config: Some(scale_config),
                legend_config,
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                legend_config,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config: Some(scale_config),
                legend_config,
            },
            other => other,
        };

        self
    }

    /// Configure the scale with a specific type
    pub fn scale_with<S: ScaleSpec>(
        self,
        f: impl FnOnce(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        // Use Fn instead of FnOnce for the inner closure
        let f = Arc::new(std::sync::Mutex::new(Some(f)));
        self.scale(move |default_scale| {
            let typed_scale = default_scale.into_type::<S>();
            if let Some(f) = f.lock().unwrap().take() {
                f(typed_scale).into_auto()
            } else {
                typed_scale.into_auto()
            }
        })
    }

    /// Disable legend for this channel
    pub fn no_legend(self) -> Self {
        // Create a legend but mark it as not visible
        self.legend(|l| l.visible(false))
    }

    /// Disable scale for this channel
    pub fn no_scale(mut self) -> Self {
        self.value = self.value.no_scale();
        self
    }

    /// Get the configured channel value
    pub fn into_inner(self) -> ChannelValue {
        self.value
    }
}

// Channel config for size channels
pub struct SizeChannelConfig {
    value: ChannelValue,
}

impl SizeChannelConfig {
    pub fn new(value: ChannelValue) -> Self {
        Self { value }
    }

    /// Configure the legend for this size channel
    pub fn legend<F>(mut self, f: F) -> Self
    where
        F: FnOnce(SizeLegendBuilder) -> SizeLegendBuilder,
    {
        let builder = SizeLegendBuilder::new();
        let configured = f(builder);
        let legend = configured.build();

        let legend_config: LegendConfig = Arc::new(move |_| legend.clone());

        self.value = match self.value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                legend_config: Some(legend_config),
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config: Some(legend_config),
            },
            other => other,
        };

        self
    }

    pub fn scale<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        let scale_config: ScaleConfig = Arc::new(f);

        self.value = match self.value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                legend_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config: Some(scale_config),
                legend_config,
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                legend_config,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config: Some(scale_config),
                legend_config,
            },
            other => other,
        };

        self
    }

    pub fn scale_with<S: ScaleSpec>(
        self,
        f: impl FnOnce(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        // Use Fn instead of FnOnce for the inner closure
        let f = Arc::new(std::sync::Mutex::new(Some(f)));
        self.scale(move |default_scale| {
            let typed_scale = default_scale.into_type::<S>();
            if let Some(f) = f.lock().unwrap().take() {
                f(typed_scale).into_auto()
            } else {
                typed_scale.into_auto()
            }
        })
    }

    /// Disable legend for this channel  
    pub fn no_legend(self) -> Self {
        // Create a legend but mark it as not visible
        self.legend(|l| l.visible(false))
    }

    /// Disable scale for this channel
    pub fn no_scale(mut self) -> Self {
        self.value = self.value.no_scale();
        self
    }

    pub fn into_inner(self) -> ChannelValue {
        self.value
    }
}

// Channel config for shape channels
pub struct ShapeChannelConfig {
    value: ChannelValue,
}

impl ShapeChannelConfig {
    pub fn new(value: ChannelValue) -> Self {
        Self { value }
    }

    /// Configure the legend for this shape channel
    pub fn legend<F>(mut self, f: F) -> Self
    where
        F: FnOnce(ShapeLegendBuilder) -> ShapeLegendBuilder,
    {
        let builder = ShapeLegendBuilder::new();
        let configured = f(builder);
        let legend = configured.build();

        let legend_config: LegendConfig = Arc::new(move |_| legend.clone());

        self.value = match self.value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                legend_config: Some(legend_config),
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config: Some(legend_config),
            },
            other => other,
        };

        self
    }

    pub fn scale<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        let scale_config: ScaleConfig = Arc::new(f);

        self.value = match self.value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                legend_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config: Some(scale_config),
                legend_config,
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                legend_config,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config: Some(scale_config),
                legend_config,
            },
            other => other,
        };

        self
    }

    pub fn scale_with<S: ScaleSpec>(
        self,
        f: impl FnOnce(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        // Use Fn instead of FnOnce for the inner closure
        let f = Arc::new(std::sync::Mutex::new(Some(f)));
        self.scale(move |default_scale| {
            let typed_scale = default_scale.into_type::<S>();
            if let Some(f) = f.lock().unwrap().take() {
                f(typed_scale).into_auto()
            } else {
                typed_scale.into_auto()
            }
        })
    }

    /// Disable legend for this channel
    pub fn no_legend(self) -> Self {
        // Create a legend but mark it as not visible
        self.legend(|l| l.visible(false))
    }

    /// Disable scale for this channel
    pub fn no_scale(mut self) -> Self {
        self.value = self.value.no_scale();
        self
    }

    pub fn into_inner(self) -> ChannelValue {
        self.value
    }
}

// Channel config for opacity channels
pub struct OpacityChannelConfig {
    value: ChannelValue,
}

impl OpacityChannelConfig {
    pub fn new(value: ChannelValue) -> Self {
        Self { value }
    }

    /// Configure the legend for this opacity channel
    pub fn legend<F>(mut self, f: F) -> Self
    where
        F: FnOnce(OpacityLegendBuilder) -> OpacityLegendBuilder,
    {
        let builder = OpacityLegendBuilder::new();
        let configured = f(builder);
        let legend = configured.build();

        let legend_config: LegendConfig = Arc::new(move |_| legend.clone());

        self.value = match self.value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                legend_config: Some(legend_config),
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config: Some(legend_config),
            },
            other => other,
        };

        self
    }

    pub fn scale<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        let scale_config: ScaleConfig = Arc::new(f);

        self.value = match self.value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                legend_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config: Some(scale_config),
                legend_config,
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                legend_config,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config: Some(scale_config),
                legend_config,
            },
            other => other,
        };

        self
    }

    /// Disable legend for this channel
    pub fn no_legend(self) -> Self {
        // Create a legend but mark it as not visible
        self.legend(|l| l.visible(false))
    }

    /// Disable scale for this channel  
    pub fn no_scale(self) -> Self {
        self
    }

    pub fn into_inner(self) -> ChannelValue {
        self.value
    }
}

// Channel config for angle channels
pub struct AngleChannelConfig {
    value: ChannelValue,
}

impl AngleChannelConfig {
    pub fn new(value: ChannelValue) -> Self {
        Self { value }
    }

    /// Configure the legend for this angle channel  
    pub fn legend<F>(mut self, f: F) -> Self
    where
        F: FnOnce(AngleLegendBuilder) -> AngleLegendBuilder,
    {
        let builder = AngleLegendBuilder::new();
        let configured = f(builder);
        let legend = configured.build();

        let legend_config: LegendConfig = Arc::new(move |_| legend.clone());

        self.value = match self.value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                legend_config: Some(legend_config),
            },
            other => other,
        };

        self
    }

    pub fn scale<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        let scale_config: ScaleConfig = Arc::new(f);

        self.value = match self.value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                legend_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config: Some(scale_config),
                legend_config,
            },
            other => other,
        };

        self
    }

    /// Disable legend for this channel
    pub fn no_legend(self) -> Self {
        // Create a legend but mark it as not visible
        self.legend(|l| l.visible(false))
    }

    /// Disable scale for this channel
    pub fn no_scale(mut self) -> Self {
        self.value = self.value.no_scale();
        self
    }

    pub fn into_inner(self) -> ChannelValue {
        self.value
    }
}

// Channel config for stroke width channels
pub struct StrokeWidthChannelConfig {
    value: ChannelValue,
}

impl StrokeWidthChannelConfig {
    pub fn new(value: ChannelValue) -> Self {
        Self { value }
    }

    /// Configure the legend for this stroke width channel
    pub fn legend<F>(mut self, f: F) -> Self
    where
        F: FnOnce(StrokeWidthLegendBuilder) -> StrokeWidthLegendBuilder,
    {
        let builder = StrokeWidthLegendBuilder::new();
        let configured = f(builder);
        let legend = configured.build();

        let legend_config: LegendConfig = Arc::new(move |_| legend.clone());

        self.value = match self.value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                legend_config: Some(legend_config),
            },
            other => other,
        };

        self
    }

    pub fn scale<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        let scale_config: ScaleConfig = Arc::new(f);

        self.value = match self.value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                legend_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config: Some(scale_config),
                legend_config,
            },
            other => other,
        };

        self
    }

    pub fn scale_with<S: ScaleSpec>(
        self,
        f: impl FnOnce(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        // Use Fn instead of FnOnce for the inner closure
        let f = Arc::new(std::sync::Mutex::new(Some(f)));
        self.scale(move |default_scale| {
            let typed_scale = default_scale.into_type::<S>();
            let f = f
                .lock()
                .unwrap()
                .take()
                .expect("scale_with closure already called");
            let configured = f(typed_scale);
            configured.into_type::<Auto>()
        })
    }

    /// Disable legend for this channel
    pub fn no_legend(self) -> Self {
        // Create a legend but mark it as not visible
        self.legend(|l| l.visible(false))
    }

    /// Disable scale for this channel
    pub fn no_scale(mut self) -> Self {
        self.value = self.value.no_scale();
        self
    }

    pub fn into_inner(self) -> ChannelValue {
        self.value
    }
}
// Channel config for stroke dash channels
pub struct StrokeDashChannelConfig {
    value: ChannelValue,
}

impl StrokeDashChannelConfig {
    pub fn new(value: ChannelValue) -> Self {
        Self { value }
    }

    /// Configure the legend for this stroke dash channel
    pub fn legend<F>(mut self, f: F) -> Self
    where
        F: FnOnce(StrokeDashLegendBuilder) -> StrokeDashLegendBuilder,
    {
        let builder = StrokeDashLegendBuilder::new();
        let configured = f(builder);
        let legend = configured.build();

        let legend_config: LegendConfig = Arc::new(move |_| legend.clone());

        self.value = match self.value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config,
                legend_config: Some(legend_config),
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config,
                legend_config: Some(legend_config),
            },
            other => other,
        };

        self
    }

    pub fn scale<F>(mut self, f: F) -> Self
    where
        F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static,
    {
        let scale_config: ScaleConfig = Arc::new(f);

        self.value = match self.value {
            ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                legend_config,
                ..
            } => ChannelValue::Scaled {
                expr,
                scale_name,
                band,
                scale_config: Some(scale_config),
                legend_config,
            },
            ChannelValue::Conditional {
                conditions,
                otherwise,
                legend_config,
                ..
            } => ChannelValue::Conditional {
                conditions,
                otherwise,
                scale_config: Some(scale_config),
                legend_config,
            },
            other => other,
        };

        self
    }

    pub fn scale_with<S: ScaleSpec>(
        self,
        f: impl FnOnce(Scale<S>) -> Scale<S> + Send + Sync + 'static,
    ) -> Self {
        // Use Fn instead of FnOnce for the inner closure
        let f = Arc::new(std::sync::Mutex::new(Some(f)));
        self.scale(move |default_scale| {
            let typed_scale = default_scale.into_type::<S>();
            let f = f
                .lock()
                .unwrap()
                .take()
                .expect("scale_with closure already called");
            let configured = f(typed_scale);
            configured.into_type::<Auto>()
        })
    }

    /// Disable legend for this channel
    pub fn no_legend(self) -> Self {
        // Create a legend but mark it as not visible
        self.legend(|l| l.visible(false))
    }

    /// Disable scale for this channel
    pub fn no_scale(mut self) -> Self {
        self.value = self.value.no_scale();
        self
    }

    pub fn into_inner(self) -> ChannelValue {
        self.value
    }
}
