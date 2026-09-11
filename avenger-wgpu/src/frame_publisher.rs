use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use arc_swap::ArcSwapOption;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FrameGeneration(u64);

impl FrameGeneration {
    pub const ZERO: Self = Self(0);

    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for FrameGeneration {
    fn from(value: u64) -> Self {
        Self::new(value)
    }
}

impl From<FrameGeneration> for u64 {
    fn from(value: FrameGeneration) -> Self {
        value.get()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameRenderMetrics {
    pub scene_evaluation: Duration,
    pub set_scene: Duration,
    pub prepare: Duration,
    pub command_encode: Duration,
    pub submit: Duration,
    pub texture_publish: Duration,
}

#[derive(Debug)]
pub struct RenderedFrame<T> {
    pub generation: FrameGeneration,
    pub payload: T,
    pub metrics: FrameRenderMetrics,
}

impl<T> RenderedFrame<T> {
    pub fn new(generation: FrameGeneration, payload: T, metrics: FrameRenderMetrics) -> Self {
        Self {
            generation,
            payload,
            metrics,
        }
    }
}

#[derive(Debug)]
pub struct LatestFrame<T> {
    inner: Arc<ArcSwapOption<RenderedFrame<T>>>,
}

impl<T> Clone for LatestFrame<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> Default for LatestFrame<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> LatestFrame<T> {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ArcSwapOption::from(None)),
        }
    }

    pub fn snapshot(&self) -> Option<Arc<RenderedFrame<T>>> {
        self.inner.load_full()
    }

    pub fn generation(&self) -> Option<FrameGeneration> {
        self.snapshot().map(|frame| frame.generation)
    }

    fn publish(&self, frame: RenderedFrame<T>) {
        self.inner.store(Some(Arc::new(frame)));
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FramePublisherStatus {
    pub requested_generation: FrameGeneration,
    pub latest_published_generation: Option<FrameGeneration>,
    pub in_progress_generation: Option<FrameGeneration>,
    pub frames_published: u64,
    pub stale_frames_dropped: u64,
    pub busy_begin_attempts: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BeginFrameError {
    RenderInProgress {
        in_progress: FrameGeneration,
    },
    Stale {
        generation: FrameGeneration,
        requested_generation: FrameGeneration,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublishResult {
    Published {
        generation: FrameGeneration,
    },
    DroppedStale {
        generation: FrameGeneration,
        requested_generation: FrameGeneration,
    },
    Canceled {
        generation: FrameGeneration,
    },
}

#[derive(Clone, Copy, Debug)]
struct FramePublisherState {
    next_generation: u64,
    requested_generation: FrameGeneration,
    latest_published_generation: Option<FrameGeneration>,
    in_progress_generation: Option<FrameGeneration>,
    frames_published: u64,
    stale_frames_dropped: u64,
    busy_begin_attempts: u64,
}

impl Default for FramePublisherState {
    fn default() -> Self {
        Self {
            next_generation: 0,
            requested_generation: FrameGeneration::ZERO,
            latest_published_generation: None,
            in_progress_generation: None,
            frames_published: 0,
            stale_frames_dropped: 0,
            busy_begin_attempts: 0,
        }
    }
}

#[derive(Debug)]
pub struct FramePublisher<T> {
    latest: LatestFrame<T>,
    state: Arc<Mutex<FramePublisherState>>,
}

impl<T> Clone for FramePublisher<T> {
    fn clone(&self) -> Self {
        Self {
            latest: self.latest.clone(),
            state: self.state.clone(),
        }
    }
}

impl<T> Default for FramePublisher<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> FramePublisher<T> {
    pub fn new() -> Self {
        Self {
            latest: LatestFrame::new(),
            state: Arc::new(Mutex::new(FramePublisherState::default())),
        }
    }

    pub fn latest(&self) -> LatestFrame<T> {
        self.latest.clone()
    }

    pub fn latest_snapshot(&self) -> Option<Arc<RenderedFrame<T>>> {
        self.latest.snapshot()
    }

    pub fn request_frame(&self) -> FrameGeneration {
        let mut state = self.state.lock().expect("frame publisher lock poisoned");
        state.next_generation += 1;
        state.requested_generation = FrameGeneration::new(state.next_generation);
        state.requested_generation
    }

    pub fn requested_generation(&self) -> FrameGeneration {
        self.status().requested_generation
    }

    pub fn is_render_in_progress(&self) -> bool {
        self.status().in_progress_generation.is_some()
    }

    pub fn status(&self) -> FramePublisherStatus {
        let state = self.state.lock().expect("frame publisher lock poisoned");
        FramePublisherStatus {
            requested_generation: state.requested_generation,
            latest_published_generation: state.latest_published_generation,
            in_progress_generation: state.in_progress_generation,
            frames_published: state.frames_published,
            stale_frames_dropped: state.stale_frames_dropped,
            busy_begin_attempts: state.busy_begin_attempts,
        }
    }

    pub fn is_stale(&self, generation: FrameGeneration) -> bool {
        generation < self.requested_generation()
    }

    pub fn try_begin_render(
        &self,
        generation: FrameGeneration,
    ) -> Result<FrameRenderTicket<T>, BeginFrameError> {
        let mut state = self.state.lock().expect("frame publisher lock poisoned");
        if generation < state.requested_generation {
            state.stale_frames_dropped += 1;
            return Err(BeginFrameError::Stale {
                generation,
                requested_generation: state.requested_generation,
            });
        }

        if let Some(in_progress) = state.in_progress_generation {
            state.busy_begin_attempts += 1;
            return Err(BeginFrameError::RenderInProgress { in_progress });
        }

        state.in_progress_generation = Some(generation);
        Ok(FrameRenderTicket {
            publisher: self.clone(),
            generation,
            finished: false,
        })
    }

    pub fn try_begin_latest_render(&self) -> Result<FrameRenderTicket<T>, BeginFrameError> {
        let generation = self.requested_generation();
        self.try_begin_render(generation)
    }

    fn publish_ticket(
        &self,
        generation: FrameGeneration,
        payload: T,
        metrics: FrameRenderMetrics,
    ) -> PublishResult {
        let mut state = self.state.lock().expect("frame publisher lock poisoned");
        clear_in_progress(&mut state, generation);

        if generation < state.requested_generation {
            state.stale_frames_dropped += 1;
            return PublishResult::DroppedStale {
                generation,
                requested_generation: state.requested_generation,
            };
        }

        self.latest
            .publish(RenderedFrame::new(generation, payload, metrics));
        state.latest_published_generation = Some(generation);
        state.frames_published += 1;
        PublishResult::Published { generation }
    }

    fn cancel_ticket(&self, generation: FrameGeneration) -> PublishResult {
        let mut state = self.state.lock().expect("frame publisher lock poisoned");
        clear_in_progress(&mut state, generation);
        PublishResult::Canceled { generation }
    }
}

#[derive(Debug)]
pub struct FrameRenderTicket<T> {
    publisher: FramePublisher<T>,
    generation: FrameGeneration,
    finished: bool,
}

impl<T> FrameRenderTicket<T> {
    pub fn generation(&self) -> FrameGeneration {
        self.generation
    }

    pub fn should_continue(&self) -> bool {
        !self.publisher.is_stale(self.generation)
    }

    pub fn publish(mut self, payload: T, metrics: FrameRenderMetrics) -> PublishResult {
        self.finished = true;
        self.publisher
            .publish_ticket(self.generation, payload, metrics)
    }

    pub fn cancel(mut self) -> PublishResult {
        self.finished = true;
        self.publisher.cancel_ticket(self.generation)
    }
}

impl<T> Drop for FrameRenderTicket<T> {
    fn drop(&mut self) {
        if !self.finished {
            self.publisher.cancel_ticket(self.generation);
        }
    }
}

fn clear_in_progress(state: &mut FramePublisherState, generation: FrameGeneration) {
    if state.in_progress_generation == Some(generation) {
        state.in_progress_generation = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_frame_allocates_monotonic_generations() {
        let publisher = FramePublisher::<&'static str>::new();

        assert_eq!(publisher.request_frame().get(), 1);
        assert_eq!(publisher.request_frame().get(), 2);
        assert_eq!(publisher.requested_generation().get(), 2);
    }

    #[test]
    fn publish_updates_latest_snapshot() {
        let publisher = FramePublisher::new();
        let generation = publisher.request_frame();
        let ticket = publisher.try_begin_render(generation).unwrap();

        let result = ticket.publish("frame", FrameRenderMetrics::default());

        assert_eq!(result, PublishResult::Published { generation });
        let latest = publisher.latest_snapshot().unwrap();
        assert_eq!(latest.generation, generation);
        assert_eq!(latest.payload, "frame");
        assert_eq!(publisher.status().frames_published, 1);
    }

    #[test]
    fn stale_publish_is_dropped() {
        let publisher = FramePublisher::new();
        let first = publisher.request_frame();
        let ticket = publisher.try_begin_render(first).unwrap();
        let second = publisher.request_frame();

        assert!(!ticket.should_continue());
        let result = ticket.publish("stale", FrameRenderMetrics::default());

        assert_eq!(
            result,
            PublishResult::DroppedStale {
                generation: first,
                requested_generation: second,
            }
        );
        assert!(publisher.latest_snapshot().is_none());
        assert_eq!(publisher.status().stale_frames_dropped, 1);
    }

    #[test]
    fn stale_generation_cannot_begin_render() {
        let publisher = FramePublisher::<&'static str>::new();
        let first = publisher.request_frame();
        let second = publisher.request_frame();

        let result = publisher.try_begin_render(first);

        assert_eq!(
            result.unwrap_err(),
            BeginFrameError::Stale {
                generation: first,
                requested_generation: second,
            }
        );
        assert_eq!(publisher.status().stale_frames_dropped, 1);
    }

    #[test]
    fn in_progress_render_blocks_second_begin() {
        let publisher = FramePublisher::<&'static str>::new();
        let generation = publisher.request_frame();
        let _ticket = publisher.try_begin_render(generation).unwrap();

        let result = publisher.try_begin_render(generation);

        assert_eq!(
            result.unwrap_err(),
            BeginFrameError::RenderInProgress {
                in_progress: generation,
            }
        );
        assert!(publisher.is_render_in_progress());
        assert_eq!(publisher.status().busy_begin_attempts, 1);
    }

    #[test]
    fn dropping_ticket_clears_in_progress_flag() {
        let publisher = FramePublisher::<&'static str>::new();
        let generation = publisher.request_frame();

        {
            let _ticket = publisher.try_begin_render(generation).unwrap();
            assert!(publisher.is_render_in_progress());
        }

        assert!(!publisher.is_render_in_progress());
    }
}
