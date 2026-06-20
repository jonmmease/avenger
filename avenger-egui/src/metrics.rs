use std::{sync::Mutex as StdMutex, time::Duration};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CanvasMetrics {
    pub routed_event_batches: u64,
    pub routed_events: u64,
    pub last_routed_event_count: u64,
    pub render_pending_events_coalesced: u64,
    pub render_pending_events_replayed: u64,
    pub scene_rebuild_requests: u64,
    pub event_dispatch_requests: u64,
    pub scene_frames_published: u64,
    pub stale_scene_frames_dropped: u64,
    pub background_render_requests: u64,
    pub background_render_requests_coalesced: u64,
    pub background_render_frames_submitted: u64,
    pub background_render_frames_published: u64,
    pub background_render_frames_consumed: u64,
    pub stale_background_render_frames_dropped: u64,
    pub offscreen_texture_renders: u64,
    pub texture_registrations: u64,
    pub texture_updates: u64,
    pub frames_painted: u64,
    pub reused_latest_frame_paints: u64,
    pub frames_painted_while_pending: u64,
    pub last_requested_generation: Option<u64>,
    pub last_published_scene_generation: Option<u64>,
    pub last_painted_texture_generation: Option<u64>,
    pub last_scene_evaluation_us: u64,
    pub last_set_scene_us: u64,
    pub last_command_encode_us: u64,
    pub last_submit_us: u64,
    pub last_texture_publish_us: u64,
    pub last_scene_to_texture_publish_us: u64,
    pub last_background_queue_wait_us: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasLatencyBottleneck {
    NoSamples,
    SceneEvaluation,
    BackgroundQueueWait,
    GpuRender,
    EguiTextureRegistration,
}

impl CanvasLatencyBottleneck {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NoSamples => "no samples",
            Self::SceneEvaluation => "scene evaluation",
            Self::BackgroundQueueWait => "background queue wait",
            Self::GpuRender => "gpu render",
            Self::EguiTextureRegistration => "egui texture registration",
        }
    }
}

impl CanvasMetrics {
    pub fn last_gpu_render_us(&self) -> u64 {
        self.last_set_scene_us
            .saturating_add(self.last_command_encode_us)
            .saturating_add(self.last_submit_us)
    }

    pub fn latency_bottleneck(&self) -> CanvasLatencyBottleneck {
        let candidates = [
            (
                CanvasLatencyBottleneck::SceneEvaluation,
                self.last_scene_evaluation_us,
            ),
            (
                CanvasLatencyBottleneck::BackgroundQueueWait,
                self.last_background_queue_wait_us,
            ),
            (
                CanvasLatencyBottleneck::GpuRender,
                self.last_gpu_render_us(),
            ),
            (
                CanvasLatencyBottleneck::EguiTextureRegistration,
                self.last_texture_publish_us,
            ),
        ];
        let Some((bottleneck, elapsed_us)) = candidates
            .into_iter()
            .max_by_key(|(_, elapsed_us)| *elapsed_us)
        else {
            return CanvasLatencyBottleneck::NoSamples;
        };
        if elapsed_us == 0 {
            CanvasLatencyBottleneck::NoSamples
        } else {
            bottleneck
        }
    }

    pub fn latency_bottleneck_us(&self) -> u64 {
        match self.latency_bottleneck() {
            CanvasLatencyBottleneck::NoSamples => 0,
            CanvasLatencyBottleneck::SceneEvaluation => self.last_scene_evaluation_us,
            CanvasLatencyBottleneck::BackgroundQueueWait => self.last_background_queue_wait_us,
            CanvasLatencyBottleneck::GpuRender => self.last_gpu_render_us(),
            CanvasLatencyBottleneck::EguiTextureRegistration => self.last_texture_publish_us,
        }
    }
}

pub(crate) fn update_metrics(
    metrics: &StdMutex<CanvasMetrics>,
    update: impl FnOnce(&mut CanvasMetrics),
) {
    update(&mut metrics.lock().expect("avenger egui metrics lock poisoned"));
}

pub(crate) fn duration_us(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}
