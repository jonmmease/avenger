use std::sync::{Arc, Mutex};

use avenger_resource::{PrefetchRetargetPlanner, ResourceRequest, ResourceRequestPurpose};
use datafusion::{common::ScalarValue, prelude::SessionContext};
use indexmap::IndexMap;

use crate::{
    FormattingContext, MaterializationRequest, Theme, ThemeContext, ThemeValue, TimeContext,
};

/// Diagnostics hook used by higher-level runtime crates to observe expensive
/// evaluation work without making lower-level crates depend on the facade.
#[doc(hidden)]
pub trait EvaluationDiagnostics: Send + Sync {
    fn record_scale_domain_collect(&self) {}
}

/// Public/base evaluation context for chart evaluation.
///
/// This owns the stable inputs that coordinate systems, marks, scales, legends,
/// and theme resolution can share without depending on the top-level layout
/// runtime.
#[derive(Clone)]
pub struct EvaluationContext {
    /// The theme to use for rendering.
    pub theme: Arc<Theme>,
    /// The DataFusion session context for DataFrame operations.
    pub session_context: Arc<SessionContext>,
    /// Parameter values for prepared statements and theme/media evaluation.
    pub params: IndexMap<String, ScalarValue>,
    /// Time handling defaults used by temporal transforms, scales, and guides.
    pub time_context: TimeContext,
    /// Formatting defaults used by scales, guides, and label adapters.
    pub formatting_context: FormattingContext,
    #[doc(hidden)]
    pub diagnostics: Option<Arc<dyn EvaluationDiagnostics>>,
    #[doc(hidden)]
    pub resource_request_sink: Option<Arc<Mutex<Vec<ResourceRequest>>>>,
    #[doc(hidden)]
    pub materialization_request_sink: Option<Arc<Mutex<Vec<MaterializationRequest>>>>,
    #[doc(hidden)]
    pub prefetch_planner_sink: Option<Arc<Mutex<Vec<Arc<dyn PrefetchRetargetPlanner>>>>>,
}

impl EvaluationContext {
    pub fn new(
        theme: Arc<Theme>,
        session_context: Arc<SessionContext>,
        params: IndexMap<String, ScalarValue>,
    ) -> Self {
        Self {
            theme,
            session_context,
            params,
            time_context: TimeContext::default(),
            formatting_context: FormattingContext::default(),
            diagnostics: None,
            resource_request_sink: None,
            materialization_request_sink: None,
            prefetch_planner_sink: None,
        }
    }

    /// Get the theme.
    pub fn theme(&self) -> &Arc<Theme> {
        &self.theme
    }

    /// Get the DataFusion session context.
    pub fn session_context(&self) -> &Arc<SessionContext> {
        &self.session_context
    }

    /// Get runtime parameter values.
    pub fn params(&self) -> &IndexMap<String, ScalarValue> {
        &self.params
    }

    /// Get time handling defaults.
    pub fn time_context(&self) -> &TimeContext {
        &self.time_context
    }

    /// Get formatting defaults.
    pub fn formatting_context(&self) -> &FormattingContext {
        &self.formatting_context
    }

    /// Create a new context with different params, reusing other fields.
    pub fn with_params(&self, params: IndexMap<String, ScalarValue>) -> Self {
        Self {
            theme: self.theme.clone(),
            session_context: self.session_context.clone(),
            params,
            time_context: self.time_context.clone(),
            formatting_context: self.formatting_context.clone(),
            diagnostics: self.diagnostics.clone(),
            resource_request_sink: self.resource_request_sink.clone(),
            materialization_request_sink: self.materialization_request_sink.clone(),
            prefetch_planner_sink: self.prefetch_planner_sink.clone(),
        }
    }

    pub fn with_time_context(&self, time_context: TimeContext) -> Self {
        Self {
            theme: self.theme.clone(),
            session_context: self.session_context.clone(),
            params: self.params.clone(),
            time_context,
            formatting_context: self.formatting_context.clone(),
            diagnostics: self.diagnostics.clone(),
            resource_request_sink: self.resource_request_sink.clone(),
            materialization_request_sink: self.materialization_request_sink.clone(),
            prefetch_planner_sink: self.prefetch_planner_sink.clone(),
        }
    }

    pub fn with_formatting_context(&self, formatting_context: FormattingContext) -> Self {
        Self {
            theme: self.theme.clone(),
            session_context: self.session_context.clone(),
            params: self.params.clone(),
            time_context: self.time_context.clone(),
            formatting_context,
            diagnostics: self.diagnostics.clone(),
            resource_request_sink: self.resource_request_sink.clone(),
            materialization_request_sink: self.materialization_request_sink.clone(),
            prefetch_planner_sink: self.prefetch_planner_sink.clone(),
        }
    }

    #[doc(hidden)]
    pub fn with_diagnostics(&self, diagnostics: Arc<dyn EvaluationDiagnostics>) -> Self {
        Self {
            theme: self.theme.clone(),
            session_context: self.session_context.clone(),
            params: self.params.clone(),
            time_context: self.time_context.clone(),
            formatting_context: self.formatting_context.clone(),
            diagnostics: Some(diagnostics),
            resource_request_sink: self.resource_request_sink.clone(),
            materialization_request_sink: self.materialization_request_sink.clone(),
            prefetch_planner_sink: self.prefetch_planner_sink.clone(),
        }
    }

    #[doc(hidden)]
    pub fn with_resource_request_sink(&self, sink: Arc<Mutex<Vec<ResourceRequest>>>) -> Self {
        Self {
            theme: self.theme.clone(),
            session_context: self.session_context.clone(),
            params: self.params.clone(),
            time_context: self.time_context.clone(),
            formatting_context: self.formatting_context.clone(),
            diagnostics: self.diagnostics.clone(),
            resource_request_sink: Some(sink),
            materialization_request_sink: self.materialization_request_sink.clone(),
            prefetch_planner_sink: self.prefetch_planner_sink.clone(),
        }
    }

    #[doc(hidden)]
    pub fn with_materialization_request_sink(
        &self,
        sink: Arc<Mutex<Vec<MaterializationRequest>>>,
    ) -> Self {
        Self {
            theme: self.theme.clone(),
            session_context: self.session_context.clone(),
            params: self.params.clone(),
            time_context: self.time_context.clone(),
            formatting_context: self.formatting_context.clone(),
            diagnostics: self.diagnostics.clone(),
            resource_request_sink: self.resource_request_sink.clone(),
            materialization_request_sink: Some(sink),
            prefetch_planner_sink: self.prefetch_planner_sink.clone(),
        }
    }

    #[doc(hidden)]
    pub fn record_scale_domain_collect(&self) {
        if let Some(diagnostics) = &self.diagnostics {
            diagnostics.record_scale_domain_collect();
        }
    }

    /// Record a resource that an evaluated scene needs an embedding host to load.
    pub fn request_resource(&self, request: ResourceRequest) {
        if let Some(sink) = &self.resource_request_sink {
            let mut guard = sink.lock().expect("resource request sink lock poisoned");
            if let Some(existing) = guard
                .iter_mut()
                .find(|existing| existing.key == request.key)
            {
                let priority = existing.priority.max(request.priority);
                if existing.purpose == ResourceRequestPurpose::Prefetch
                    && request.purpose == ResourceRequestPurpose::Required
                {
                    *existing = request;
                } else if existing.purpose == request.purpose
                    && request.priority > existing.priority
                {
                    *existing = request;
                }
                existing.priority = priority;
            } else {
                guard.push(request);
            }
        }
    }

    /// Record an async materialization that an evaluated chart would like the
    /// embedding runtime or session executor to produce.
    pub fn request_materialization(&self, request: MaterializationRequest) {
        if let Some(sink) = &self.materialization_request_sink {
            let mut guard = sink
                .lock()
                .expect("materialization request sink lock poisoned");
            if let Some(existing) = guard
                .iter_mut()
                .find(|existing| existing.key == request.key)
            {
                if request.priority > existing.priority {
                    *existing = request;
                }
            } else {
                guard.push(request);
            }
        }
    }

    #[doc(hidden)]
    pub fn with_prefetch_planner_sink(
        &self,
        sink: Arc<Mutex<Vec<Arc<dyn PrefetchRetargetPlanner>>>>,
    ) -> Self {
        let mut ctx = self.clone();
        ctx.prefetch_planner_sink = Some(sink);
        ctx
    }

    /// Record a prefetch-retarget planner published by a coordinate-system
    /// guide for this evaluation. One planner per retargetable scope;
    /// later planners for the same scope replace earlier ones.
    pub fn publish_prefetch_planner(&self, planner: Arc<dyn PrefetchRetargetPlanner>) {
        if let Some(sink) = &self.prefetch_planner_sink {
            let mut guard = sink.lock().expect("prefetch planner sink lock poisoned");
            if let Some(existing) = guard
                .iter_mut()
                .find(|existing| existing.scope() == planner.scope())
            {
                *existing = planner;
            } else {
                guard.push(planner);
            }
        }
    }

    #[doc(hidden)]
    pub fn prefetch_planners_snapshot(&self) -> Vec<Arc<dyn PrefetchRetargetPlanner>> {
        self.prefetch_planner_sink
            .as_ref()
            .map(|sink| {
                sink.lock()
                    .expect("prefetch planner sink lock poisoned")
                    .clone()
            })
            .unwrap_or_default()
    }

    #[doc(hidden)]
    pub fn resource_requests_snapshot(&self) -> Vec<ResourceRequest> {
        self.resource_request_sink
            .as_ref()
            .map(|sink| {
                sink.lock()
                    .expect("resource request sink lock poisoned")
                    .clone()
            })
            .unwrap_or_default()
    }

    #[doc(hidden)]
    pub fn materialization_requests_snapshot(&self) -> Vec<MaterializationRequest> {
        self.materialization_request_sink
            .as_ref()
            .map(|sink| {
                sink.lock()
                    .expect("materialization request sink lock poisoned")
                    .clone()
            })
            .unwrap_or_default()
    }

    /// Create a new context with canvas dimensions added to params.
    pub fn with_dimension_params(&self, width: f32, height: f32) -> Self {
        let mut params = self.params.clone();
        params.insert("width".to_string(), ScalarValue::Float32(Some(width)));
        params.insert("height".to_string(), ScalarValue::Float32(Some(height)));
        self.with_params(params)
    }

    /// Query a theme property with the context's runtime params applied.
    pub fn query_theme(&self, context: &ThemeContext, property: &str) -> Option<ThemeValue> {
        let mut context_with_params = context.clone();
        context_with_params.params.extend(self.params.clone());
        self.theme.query(&context_with_params, property)
    }

    /// Get font size with the context's runtime params applied.
    pub fn font_size(&self, context: &ThemeContext) -> Option<f32> {
        let mut context_with_params = context.clone();
        context_with_params.params.extend(self.params.clone());
        self.theme.font_size(&context_with_params)
    }

    /// Resolve a mark default value with the context's runtime params applied.
    pub fn mark_default(&self, mark_type: &str, channel: &str) -> Option<ScalarValue> {
        self.theme.mark_default(mark_type, channel, &self.params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avenger_resource::{ResourceKey, ResourceKind, ResourceSource};

    use crate::MaterializationOutputKind;

    #[test]
    fn resource_request_sink_survives_context_clones() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let ctx = EvaluationContext::new(
            Arc::new(Theme::light()),
            Arc::new(SessionContext::new()),
            IndexMap::new(),
        )
        .with_resource_request_sink(sink)
        .with_time_context(TimeContext::default())
        .with_params(IndexMap::new());

        ctx.request_resource(ResourceRequest {
            priority: 1.0,
            ..ResourceRequest::new(
                ResourceKey::new("tile/0/0/0"),
                ResourceKind::new("image"),
                ResourceSource::Url {
                    url: "https://tiles.example/0/0/0.png".to_string(),
                },
            )
        });

        let requests = ctx.resource_requests_snapshot();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].key, ResourceKey::new("tile/0/0/0"));
    }

    #[test]
    fn resource_request_sink_dedupes_by_key_and_keeps_highest_priority() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let ctx = EvaluationContext::new(
            Arc::new(Theme::light()),
            Arc::new(SessionContext::new()),
            IndexMap::new(),
        )
        .with_resource_request_sink(sink);

        let mut request = ResourceRequest {
            priority: 1.0,
            ..ResourceRequest::new(
                ResourceKey::new("tile/0/0/0"),
                ResourceKind::new("image"),
                ResourceSource::Url {
                    url: "https://tiles.example/0/0/0.png".to_string(),
                },
            )
        };
        ctx.request_resource(request.clone());
        request.priority = 0.25;
        ctx.request_resource(request.clone());
        request.priority = 2.0;
        ctx.request_resource(request);

        let requests = ctx.resource_requests_snapshot();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].priority, 2.0);
    }

    #[test]
    fn resource_request_sink_promotes_required_over_prefetch() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let ctx = EvaluationContext::new(
            Arc::new(Theme::light()),
            Arc::new(SessionContext::new()),
            IndexMap::new(),
        )
        .with_resource_request_sink(sink);

        let mut request = ResourceRequest {
            priority: -1.0,
            purpose: ResourceRequestPurpose::Prefetch,
            ..ResourceRequest::new(
                ResourceKey::new("tile/0/0/0"),
                ResourceKind::new("image"),
                ResourceSource::Url {
                    url: "https://tiles.example/0/0/0.png".to_string(),
                },
            )
        };
        ctx.request_resource(request.clone());
        request.priority = 0.0;
        request.purpose = ResourceRequestPurpose::Required;
        ctx.request_resource(request);

        let requests = ctx.resource_requests_snapshot();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].purpose, ResourceRequestPurpose::Required);
        assert_eq!(requests[0].priority, 0.0);
    }

    #[test]
    fn materialization_request_sink_survives_context_clones() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let ctx = EvaluationContext::new(
            Arc::new(Theme::light()),
            Arc::new(SessionContext::new()),
            IndexMap::new(),
        )
        .with_materialization_request_sink(sink)
        .with_time_context(TimeContext::default())
        .with_params(IndexMap::new());

        ctx.request_materialization(MaterializationRequest::new(
            "view/raster/1",
            "rasterize-2d",
            MaterializationOutputKind::RecordBatch,
        ));

        let requests = ctx.materialization_requests_snapshot();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].key, "view/raster/1".into());
    }

    #[test]
    fn materialization_request_sink_dedupes_by_key_and_keeps_highest_priority() {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let ctx = EvaluationContext::new(
            Arc::new(Theme::light()),
            Arc::new(SessionContext::new()),
            IndexMap::new(),
        )
        .with_materialization_request_sink(sink);

        ctx.request_materialization(
            MaterializationRequest::new(
                "view/raster/1",
                "rasterize-2d",
                MaterializationOutputKind::RecordBatch,
            )
            .priority(0.25),
        );
        ctx.request_materialization(
            MaterializationRequest::new(
                "view/raster/1",
                "rasterize-2d",
                MaterializationOutputKind::RecordBatch,
            )
            .priority(2.0)
            .spec(serde_json::json!({"winner": true})),
        );
        ctx.request_materialization(
            MaterializationRequest::new(
                "view/raster/1",
                "rasterize-2d",
                MaterializationOutputKind::RecordBatch,
            )
            .priority(1.0),
        );

        let requests = ctx.materialization_requests_snapshot();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].priority, 2.0);
        assert_eq!(requests[0].spec, serde_json::json!({"winner": true}));
    }
}
