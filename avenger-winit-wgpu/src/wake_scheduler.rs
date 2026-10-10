use crate::WinitWgpuEvent;
use avenger_common::time::Instant;
use avenger_eventstream::runtime::{RuntimeWakeEvent, RuntimeWakeKey};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use winit::event_loop::EventLoopProxy;

#[derive(Default)]
struct PendingWakes {
    next_ticket: u64,
    tickets: HashMap<RuntimeWakeKey, (u64, u64)>,
}

impl PendingWakes {
    fn request(&mut self, key: RuntimeWakeKey, generation: u64) -> u64 {
        self.next_ticket += 1;
        self.tickets.insert(key, (self.next_ticket, generation));
        self.next_ticket
    }

    fn clear(&mut self) {
        self.tickets.clear();
    }

    fn claim(&mut self, event: &RuntimeWakeEvent, ticket: u64) -> bool {
        if self.tickets.get(&event.key) == Some(&(ticket, event.generation)) {
            self.tickets.remove(&event.key);
            true
        } else {
            false
        }
    }
}

/// Owns keyed timers independently of canvas and text-input state.
/// Tickets stay pending until dispatch so cancellation also rejects queued events.
pub(crate) struct RuntimeWakeScheduler {
    pending: Arc<Mutex<PendingWakes>>,
    proxy: EventLoopProxy<WinitWgpuEvent>,
}

impl RuntimeWakeScheduler {
    pub(crate) fn new(proxy: EventLoopProxy<WinitWgpuEvent>) -> Self {
        Self {
            pending: Default::default(),
            proxy,
        }
    }

    pub(crate) fn request(&self, key: RuntimeWakeKey, deadline: Instant, generation: u64) {
        let ticket = self
            .pending
            .lock()
            .unwrap()
            .request(key.clone(), generation);
        let event = RuntimeWakeEvent { key, generation };
        let pending = Arc::downgrade(&self.pending);
        let proxy = self.proxy.clone();
        let delay = deadline.saturating_duration_since(Instant::now());
        let deliver = move || {
            let Some(pending) = pending.upgrade() else {
                return;
            };
            if pending.lock().unwrap().tickets.get(&event.key) == Some(&(ticket, event.generation))
            {
                let _ = proxy.send_event(WinitWgpuEvent::RuntimeWake { event, ticket });
            }
        };
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            deliver();
        });
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::{closure::Closure, JsCast};
            let callback = Closure::once_into_js(deliver);
            web_sys::window()
                .expect("browser window")
                .set_timeout_with_callback_and_timeout_and_arguments_0(
                    callback.unchecked_ref(),
                    delay.as_millis().min(i32::MAX as u128) as i32,
                )
                .expect("schedule runtime wake-up");
        }
    }

    pub(crate) fn cancel(&self, key: &RuntimeWakeKey) {
        self.pending.lock().unwrap().tickets.remove(key);
    }

    pub(crate) fn clear(&self) {
        self.pending.lock().unwrap().clear();
    }

    pub(crate) fn claim(&self, event: &RuntimeWakeEvent, ticket: u64) -> bool {
        self.pending.lock().unwrap().claim(event, ticket)
    }
}

/// Delivers a host event after a delay on either platform.
pub(super) fn send_event_after(
    proxy: EventLoopProxy<WinitWgpuEvent>,
    event: WinitWgpuEvent,
    delay: std::time::Duration,
) {
    #[cfg(not(target_arch = "wasm32"))]
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        let _ = proxy.send_event(event);
    });
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::{closure::Closure, JsCast};
        let callback = Closure::once_into_js(move || {
            let _ = proxy.send_event(event);
        });
        web_sys::window()
            .expect("browser window")
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.unchecked_ref(),
                delay.as_millis().min(i32::MAX as u128) as i32,
            )
            .expect("schedule host event");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_and_replacement_reject_already_queued_wakes() {
        let mut pending = PendingWakes::default();
        let event = RuntimeWakeEvent {
            key: RuntimeWakeKey::new("chart", 0, "readout"),
            generation: 1,
        };
        let first = pending.request(event.key.clone(), event.generation);
        pending.tickets.remove(&event.key);
        assert!(!pending.claim(&event, first));
        let replacement = pending.request(event.key.clone(), event.generation);
        assert!(!pending.claim(&event, first));
        assert!(pending.claim(&event, replacement));
        assert!(!pending.claim(&event, replacement));
    }
    #[test]
    fn replacement_rejects_an_old_queued_wake_when_the_new_manager_reuses_its_key() {
        let mut pending = PendingWakes::default();
        let event = RuntimeWakeEvent {
            key: RuntimeWakeKey::new("manager", 0, "debounce"),
            generation: 1,
        };
        let old = pending.request(event.key.clone(), event.generation);
        pending.clear();
        let new = pending.request(event.key.clone(), event.generation);
        assert!(!pending.claim(&event, old));
        assert!(pending.claim(&event, new));
    }
}
