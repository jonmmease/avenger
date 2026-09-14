use super::*;

pub(super) fn send_render_invalidation_event(
    event_proxy: EventLoopProxy<WinitWgpuEvent>,
    invalidation: RenderInvalidation,
) {
    match invalidation.schedule {
        RenderInvalidationSchedule::Now => {
            let _ = event_proxy.send_event(WinitWgpuEvent::RenderInvalidated { invalidation });
        }
        RenderInvalidationSchedule::After(delay) => send_event_after(
            event_proxy,
            WinitWgpuEvent::RenderInvalidated { invalidation },
            delay,
        ),
    }
}

fn requires_refresh(
    event: &RenderInvalidation,
    evaluated: u64,
    rendered: u64,
    requested: u64,
    pending: bool,
) -> bool {
    // A resource redraw cannot satisfy an evaluation request or cancel a delayed wake.
    if matches!(event.schedule, RenderInvalidationSchedule::After(_))
        || matches!(
            event.reason,
            RenderInvalidationReason::EvaluationChanged { .. }
        )
    {
        event.epoch > evaluated
    } else {
        event.epoch > rendered && (event.epoch > requested || !pending)
    }
}

impl<State: Clone + Send + Sync + 'static> WinitWgpuAvengerApp<State> {
    pub(super) fn handle_render_invalidation(&mut self, invalidation: RenderInvalidation) {
        if !requires_refresh(
            &invalidation,
            self.hub_epoch_at_last_evaluation_start.get(),
            self.last_rendered_render_invalidation_epoch,
            self.last_requested_render_invalidation_epoch,
            self.render_invalidation_pending,
        ) {
            return;
        }
        let evaluation_changed = matches!(
            invalidation.reason,
            RenderInvalidationReason::EvaluationChanged { .. }
        );
        // Rebuilding before canvas creation would be overwritten by its initial scene.
        if self.canvas.borrow().is_none() {
            tracing::debug!(
                target: "avenger_winit_wgpu::resize",
                epoch = invalidation.epoch,
                reason = ?invalidation.reason,
                "winit render invalidation deferred until canvas creation"
            );
            self.pending_startup_render_invalidation = Some(invalidation);
            return;
        }

        if evaluation_changed && !self.rebuild_scene_graph_for_render_invalidation(&invalidation) {
            return;
        }

        // `.max()` so a late-delivered delayed event never regresses the
        // monotonic trackers below an epoch that already rendered.
        let canvas = self.canvas.borrow();
        let Some(canvas) = canvas.as_ref() else {
            self.last_requested_render_invalidation_epoch = self
                .last_requested_render_invalidation_epoch
                .max(invalidation.epoch);
            return;
        };
        self.last_requested_render_invalidation_epoch = self
            .last_requested_render_invalidation_epoch
            .max(invalidation.epoch);
        self.render_invalidation_pending = true;
        canvas.window().request_redraw();
        tracing::debug!(
            target: "avenger_winit_wgpu::resize",
            epoch = invalidation.epoch,
            schedule = ?invalidation.schedule,
            "winit render invalidated"
        );
    }
    pub(super) fn rebuild_scene_graph_for_render_invalidation(
        &mut self,
        invalidation: &RenderInvalidation,
    ) -> bool {
        let window_scene_sizing = self.window_scene_sizing;
        let scale = self.scale;

        cfg_if::cfg_if! {
            if #[cfg(target_arch = "wasm32")] {
                let app_clone = self.avenger_app.clone();
                let canvas_shared = self.canvas.clone();
                let invalidation_epoch = invalidation.epoch;
                let hub_epoch_before = self
                    .render_invalidation_hub
                    .as_ref()
                    .map(|hub| hub.epoch());
                if let Some(epoch) = hub_epoch_before {
                    self.hub_epoch_at_last_evaluation_start.set(self.hub_epoch_at_last_evaluation_start.get().max(epoch));
                }
                #[allow(clippy::await_holding_refcell_ref)]
                spawn_local(async move {
                    let scene_graph = match app_clone.borrow_mut().rebuild_scene_graph(true).await {
                        Ok(scene_graph) => scene_graph,
                        Err(err) => {
                            log::error!("Failed to rebuild scene graph after render invalidation: {err:?}");
                            return;
                        }
                    };
                    let mut canvas_borrowed = canvas_shared.borrow_mut();
                    let Some(canvas) = canvas_borrowed.as_mut() else {
                        return;
                    };
                    if let Err(err) = install_scene_graph(
                        canvas,
                        &scene_graph,
                        window_scene_sizing,
                        scale,
                        None,
                    ) {
                        log::error!("Failed to set invalidated scene graph: {err:?}");
                        return;
                    }
                    tracing::debug!(
                        target: "avenger_winit_wgpu::resize",
                        epoch = invalidation_epoch,
                        "winit render invalidation rebuilt scene"
                    );
                });
                true
            } else {
                let rebuild_start = Instant::now();
                // Snapshot BEFORE evaluating: wake-ups the evaluation itself
                // parks get later epochs and must survive the delayed-event
                // redundancy test.
                let hub_epoch_before = self
                    .render_invalidation_hub
                    .as_ref()
                    .map(|hub| hub.epoch());
                let scene_graph = {
                    let mut app = self.avenger_app.borrow_mut();
                    match self.tokio_runtime.block_on(app.rebuild_scene_graph(true)) {
                        Ok(scene_graph) => scene_graph,
                        Err(err) => {
                            log::error!("Failed to rebuild scene graph after render invalidation: {err:?}");
                            return false;
                        }
                    }
                };
                if let Some(epoch) = hub_epoch_before {
                    self.hub_epoch_at_last_evaluation_start.set(self.hub_epoch_at_last_evaluation_start.get().max(epoch));
                }

                if let Some(canvas) = self.canvas.borrow_mut().as_mut() {
                    let install_start = Instant::now();
                    if let Err(err) = install_scene_graph(
                        canvas,
                        &scene_graph,
                        window_scene_sizing,
                        scale,
                        self.canvas_frame.as_mut(),
                    ) {
                        log::error!("Failed to set invalidated scene graph: {err:?}");
                        return false;
                    }
                    tracing::debug!(
                        target: "avenger_winit_wgpu::resize",
                        epoch = invalidation.epoch,
                        rebuild_ms = rebuild_start.elapsed().as_secs_f64() * 1000.0,
                        set_scene_ms = install_start.elapsed().as_secs_f64() * 1000.0,
                        "winit render invalidation rebuilt scene"
                    );
                    self.render_pending = true;
                }
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resource_redraws_preserve_required_evaluations_and_delayed_wakes() {
        let mut event = RenderInvalidation {
            epoch: 4,
            reason: RenderInvalidationReason::ResourceChanged { kind: "image" },
            schedule: RenderInvalidationSchedule::Now,
        };
        assert!(requires_refresh(&event, 2, 3, 3, false));
        assert!(!requires_refresh(&event, 2, 4, 4, false));
        assert!(!requires_refresh(&event, 2, 3, 4, true));
        event.reason = RenderInvalidationReason::EvaluationChanged {
            kind: "data".into(),
        };
        assert!(requires_refresh(&event, 2, 5, 5, false));
        assert!(!requires_refresh(&event, 4, 5, 5, false));
        event.schedule = RenderInvalidationSchedule::After(std::time::Duration::from_millis(10));
        assert!(requires_refresh(&event, 2, 5, 5, false));
        assert!(!requires_refresh(&event, 4, 5, 5, false));
        // Startup replay with no previous evaluation must admit this event.
        assert!(requires_refresh(&event, 0, 0, 0, false));
    }
}
