//! Primitive mark implementations and builders for [`PixelFrame`].

use std::sync::Arc;

use avenger_chart_core::{
    AvengerChartError, ChannelConfig, ChannelValue, CompiledMark, CompiledMarkState, Mark,
    PixelFrame, impl_mark_trait_common,
};

use crate::{CompiledRect, CompiledRule, CompiledSymbol, CompiledText, Rect, Rule, Symbol, Text};

pub type CompiledPixelFrameRect = CompiledRect;
pub type CompiledPixelFrameRule = CompiledRule;
pub type CompiledPixelFrameSymbol = CompiledSymbol;
pub type CompiledPixelFrameText = CompiledText;

/// Axis-free configuration for a pixel position channel.
#[derive(Clone)]
pub struct PixelFramePositionConfig {
    value: ChannelValue,
}

impl PixelFramePositionConfig {
    pub fn new(value: ChannelValue) -> Self {
        Self { value }
    }
}

impl ChannelConfig for PixelFramePositionConfig {
    fn get_value(&self) -> &ChannelValue {
        &self.value
    }

    fn set_value(&mut self, value: ChannelValue) {
        self.value = value;
    }

    fn into_inner(self) -> ChannelValue {
        self.value
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<PixelFrame> for Rect<PixelFrame> {
    impl_mark_trait_common!(Rect);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledRect::new(
            compiled_state,
            self.mark_effects().clone(),
        )))
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<PixelFrame> for Rule<PixelFrame> {
    impl_mark_trait_common!(Rule);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledRule::new(
            compiled_state,
            self.mark_effects().clone(),
        )))
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<PixelFrame> for Symbol<PixelFrame> {
    impl_mark_trait_common!(Symbol);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledSymbol::new(
            compiled_state,
            self.mark_effects().clone(),
        )))
    }
}

#[cfg_attr(target_arch = "wasm32", async_trait::async_trait(?Send))]
#[cfg_attr(not(target_arch = "wasm32"), async_trait::async_trait)]
impl Mark<PixelFrame> for Text<PixelFrame> {
    impl_mark_trait_common!(Text);

    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        _session_context: &datafusion::prelude::SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
        Ok(Arc::new(CompiledText::new(
            compiled_state,
            self.mark_effects().clone(),
            self.text_syntax_mode(),
        )))
    }
}

/// Pixel position builders for rectangles.
pub trait PixelFrameRectPositionChannels: Sized {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn x_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig;
    fn x2<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn x2_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig;
    fn y<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn y_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig;
    fn y2<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn y2_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig;
}

impl PixelFrameRectPositionChannels for Rect<PixelFrame> {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    fn x_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig,
    {
        self.with_channel_value(
            "x",
            configure(PixelFramePositionConfig::new(value.into())).into_inner(),
        )
    }

    fn x2<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x2", value.into())
    }

    fn x2_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig,
    {
        self.with_channel_value(
            "x2",
            configure(PixelFramePositionConfig::new(value.into())).into_inner(),
        )
    }

    fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    fn y_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig,
    {
        self.with_channel_value(
            "y",
            configure(PixelFramePositionConfig::new(value.into())).into_inner(),
        )
    }

    fn y2<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y2", value.into())
    }

    fn y2_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig,
    {
        self.with_channel_value(
            "y2",
            configure(PixelFramePositionConfig::new(value.into())).into_inner(),
        )
    }
}

/// Pixel position builders for rules.
pub trait PixelFrameRulePositionChannels: Sized {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn x_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig;
    fn x2<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn x2_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig;
    fn y<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn y_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig;
    fn y2<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn y2_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig;
}

impl PixelFrameRulePositionChannels for Rule<PixelFrame> {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    fn x_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig,
    {
        self.with_channel_value(
            "x",
            configure(PixelFramePositionConfig::new(value.into())).into_inner(),
        )
    }

    fn x2<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x2", value.into())
    }

    fn x2_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig,
    {
        self.with_channel_value(
            "x2",
            configure(PixelFramePositionConfig::new(value.into())).into_inner(),
        )
    }

    fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    fn y_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig,
    {
        self.with_channel_value(
            "y",
            configure(PixelFramePositionConfig::new(value.into())).into_inner(),
        )
    }

    fn y2<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y2", value.into())
    }

    fn y2_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig,
    {
        self.with_channel_value(
            "y2",
            configure(PixelFramePositionConfig::new(value.into())).into_inner(),
        )
    }
}

/// Pixel position builders for symbols.
pub trait PixelFrameSymbolPositionChannels: Sized {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn x_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig;
    fn y<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn y_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig;
}

impl PixelFrameSymbolPositionChannels for Symbol<PixelFrame> {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    fn x_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig,
    {
        self.with_channel_value(
            "x",
            configure(PixelFramePositionConfig::new(value.into())).into_inner(),
        )
    }

    fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    fn y_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig,
    {
        self.with_channel_value(
            "y",
            configure(PixelFramePositionConfig::new(value.into())).into_inner(),
        )
    }
}

/// Pixel position builders for text.
pub trait PixelFrameTextPositionChannels: Sized {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn x_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig;
    fn y<V: Into<ChannelValue>>(self, value: V) -> Self;
    fn y_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig;
}

impl PixelFrameTextPositionChannels for Text<PixelFrame> {
    fn x<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("x", value.into())
    }

    fn x_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig,
    {
        self.with_channel_value(
            "x",
            configure(PixelFramePositionConfig::new(value.into())).into_inner(),
        )
    }

    fn y<V: Into<ChannelValue>>(self, value: V) -> Self {
        self.with_channel_value("y", value.into())
    }

    fn y_with<V, F>(self, value: V, configure: F) -> Self
    where
        V: Into<ChannelValue>,
        F: FnOnce(PixelFramePositionConfig) -> PixelFramePositionConfig,
    {
        self.with_channel_value(
            "y",
            configure(PixelFramePositionConfig::new(value.into())).into_inner(),
        )
    }
}
