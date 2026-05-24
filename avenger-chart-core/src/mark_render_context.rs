use std::sync::Arc;

use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;

use crate::{EvaluationContext, Theme, ThemeContext, ThemeValue};

/// Core render view available to custom mark implementations.
///
/// This intentionally contains only stable mark-facing inputs. Top-level
/// layout/runtime state such as facet trees, child-frame paths, and refinement
/// state remains outside core.
#[derive(Clone, Copy)]
pub struct MarkRenderContext<'a> {
    eval: &'a EvaluationContext,
    plot_width: f32,
    plot_height: f32,
}

impl<'a> MarkRenderContext<'a> {
    pub fn new(eval: &'a EvaluationContext, plot_width: f32, plot_height: f32) -> Self {
        Self {
            eval,
            plot_width,
            plot_height,
        }
    }

    pub fn eval(&self) -> &'a EvaluationContext {
        self.eval
    }

    pub fn plot_width(&self) -> f32 {
        self.plot_width
    }

    pub fn plot_height(&self) -> f32 {
        self.plot_height
    }

    pub fn theme(&self) -> &Arc<Theme> {
        self.eval.theme()
    }

    pub fn session_context(&self) -> &Arc<SessionContext> {
        self.eval.session_context()
    }

    pub fn params(&self) -> &IndexMap<String, ScalarValue> {
        self.eval.params()
    }

    pub fn query_theme(&self, context: &ThemeContext, property: &str) -> Option<ThemeValue> {
        self.eval.query_theme(context, property)
    }

    pub fn font_size(&self, context: &ThemeContext) -> Option<f32> {
        self.eval.font_size(context)
    }
}
