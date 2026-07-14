use std::{
    any::Any,
    collections::HashMap,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use avenger_common::{cursor::CursorStyle, time::Instant};
use avenger_eventstream::runtime::{
    LogicalRect, RuntimeHostCommand, RuntimeWakeEvent, RuntimeWakeKey,
};
use avenger_eventstream::scene::SceneGraphEvent;

static NEXT_DOCUMENT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NativeWidgetDocumentId(u64);

impl NativeWidgetDocumentId {
    pub fn new() -> Self {
        let id = NEXT_DOCUMENT_ID.fetch_add(1, Ordering::Relaxed);
        assert!(id != u64::MAX, "native widget document id space exhausted");
        Self(id)
    }
}

impl Default for NativeWidgetDocumentId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for NativeWidgetDocumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeWidgetDocumentId(..)")
    }
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct NativeWidgetPlotId(Arc<str>);

impl NativeWidgetPlotId {
    pub fn from_member_path(member_path: impl Into<Arc<str>>) -> Self {
        let member_path = member_path.into();
        assert!(
            !member_path.is_empty(),
            "native widget plot member path must not be empty"
        );
        Self(member_path)
    }

    pub fn chart_root() -> Self {
        Self::from_member_path("chart-root")
    }

    pub fn member_path(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for NativeWidgetPlotId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("NativeWidgetPlotId").field(&self.0).finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NativeWidgetNamespace {
    document_id: NativeWidgetDocumentId,
    plot_id: NativeWidgetPlotId,
}

impl NativeWidgetNamespace {
    pub fn new(document_id: NativeWidgetDocumentId, plot_id: NativeWidgetPlotId) -> Self {
        Self {
            document_id,
            plot_id,
        }
    }

    pub fn ephemeral() -> Self {
        Self::new(
            NativeWidgetDocumentId::new(),
            NativeWidgetPlotId::chart_root(),
        )
    }

    pub fn document_id(&self) -> NativeWidgetDocumentId {
        self.document_id
    }

    pub fn plot_id(&self) -> &NativeWidgetPlotId {
        &self.plot_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NativeWidgetInstanceKey {
    namespace: NativeWidgetNamespace,
    widget_id: Arc<str>,
}

impl NativeWidgetInstanceKey {
    pub fn new(namespace: NativeWidgetNamespace, widget_id: impl Into<Arc<str>>) -> Self {
        let widget_id = widget_id.into();
        assert!(!widget_id.is_empty(), "native widget id must not be empty");
        Self {
            namespace,
            widget_id,
        }
    }

    pub fn namespace(&self) -> &NativeWidgetNamespace {
        &self.namespace
    }

    pub fn widget_id(&self) -> &str {
        &self.widget_id
    }

    fn wake_namespace(&self) -> String {
        format!(
            "native-widget:{}:{}:{}",
            self.namespace.document_id.0,
            self.namespace.plot_id.member_path(),
            self.widget_id
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NativeWidgetAttachmentEpoch(u64);

impl NativeWidgetAttachmentEpoch {
    pub fn get(self) -> u64 {
        self.0
    }
}

#[derive(Default)]
struct NativeWidgetInstanceSlotState {
    next_epoch: u64,
    active_epoch: Option<NativeWidgetAttachmentEpoch>,
    instance: Option<Arc<dyn Any + Send + Sync>>,
}

pub struct NativeWidgetInstanceSlot {
    key: NativeWidgetInstanceKey,
    state: Mutex<NativeWidgetInstanceSlotState>,
}

impl fmt::Debug for NativeWidgetInstanceSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeWidgetInstanceSlot")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl NativeWidgetInstanceSlot {
    fn new(key: NativeWidgetInstanceKey) -> Self {
        Self {
            key,
            state: Mutex::new(NativeWidgetInstanceSlotState::default()),
        }
    }

    pub fn key(&self) -> &NativeWidgetInstanceKey {
        &self.key
    }

    pub fn attach(&self) -> NativeWidgetAttachmentEpoch {
        let mut state = self.state.lock().expect("native widget slot lock poisoned");
        state.next_epoch = state
            .next_epoch
            .checked_add(1)
            .expect("native widget attachment epoch exhausted");
        let epoch = NativeWidgetAttachmentEpoch(state.next_epoch);
        state.active_epoch = Some(epoch);
        epoch
    }

    pub fn detach(&self, epoch: NativeWidgetAttachmentEpoch) -> bool {
        let mut state = self.state.lock().expect("native widget slot lock poisoned");
        if state.active_epoch == Some(epoch) {
            state.active_epoch = None;
            true
        } else {
            false
        }
    }

    pub fn is_active(&self, epoch: NativeWidgetAttachmentEpoch) -> bool {
        self.state
            .lock()
            .expect("native widget slot lock poisoned")
            .active_epoch
            == Some(epoch)
    }

    pub fn get_or_init<T>(
        &self,
        init: impl FnOnce() -> T,
    ) -> Result<Arc<T>, NativeWidgetSlotTypeMismatch>
    where
        T: Any + Send + Sync,
    {
        let mut state = self.state.lock().expect("native widget slot lock poisoned");
        if let Some(instance) = &state.instance {
            return instance
                .clone()
                .downcast::<T>()
                .map_err(|_| NativeWidgetSlotTypeMismatch {
                    widget_id: self.key.widget_id().to_string(),
                });
        }
        let instance = Arc::new(init());
        state.instance = Some(instance.clone());
        Ok(instance)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeWidgetSlotTypeMismatch {
    pub widget_id: String,
}

impl fmt::Display for NativeWidgetSlotTypeMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "native widget instance slot '{}' was initialized with another runtime type",
            self.widget_id
        )
    }
}

impl std::error::Error for NativeWidgetSlotTypeMismatch {}

pub trait NativeWidgetInstanceStore: Send + Sync {
    fn slot(&self, key: NativeWidgetInstanceKey) -> Arc<NativeWidgetInstanceSlot>;
    fn remove_namespace(&self, namespace: &NativeWidgetNamespace);
}

#[derive(Default)]
pub struct InMemoryNativeWidgetInstanceStore {
    slots: Mutex<HashMap<NativeWidgetInstanceKey, Arc<NativeWidgetInstanceSlot>>>,
}

impl InMemoryNativeWidgetInstanceStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn slot_count(&self) -> usize {
        self.slots
            .lock()
            .expect("native widget store lock poisoned")
            .len()
    }
}

impl NativeWidgetInstanceStore for InMemoryNativeWidgetInstanceStore {
    fn slot(&self, key: NativeWidgetInstanceKey) -> Arc<NativeWidgetInstanceSlot> {
        self.slots
            .lock()
            .expect("native widget store lock poisoned")
            .entry(key.clone())
            .or_insert_with(|| Arc::new(NativeWidgetInstanceSlot::new(key)))
            .clone()
    }

    fn remove_namespace(&self, namespace: &NativeWidgetNamespace) {
        self.slots
            .lock()
            .expect("native widget store lock poisoned")
            .retain(|key, _| key.namespace() != namespace);
    }
}

#[derive(Default)]
pub struct NativeWidgetRegistry {
    _private: (),
}

impl NativeWidgetRegistry {
    pub fn new() -> Self {
        Self::default()
    }
}

impl fmt::Debug for NativeWidgetRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeWidgetRegistry { .. }")
    }
}

#[derive(Clone)]
pub struct NativeWidgetRuntimeResources {
    pub registry: Arc<NativeWidgetRegistry>,
    pub instance_store: Arc<dyn NativeWidgetInstanceStore>,
    pub document_id: NativeWidgetDocumentId,
}

impl NativeWidgetRuntimeResources {
    pub fn new(
        registry: Arc<NativeWidgetRegistry>,
        instance_store: Arc<dyn NativeWidgetInstanceStore>,
        document_id: NativeWidgetDocumentId,
    ) -> Self {
        Self {
            registry,
            instance_store,
            document_id,
        }
    }

    pub fn in_memory() -> Self {
        Self::new(
            Arc::new(NativeWidgetRegistry::new()),
            Arc::new(InMemoryNativeWidgetInstanceStore::new()),
            NativeWidgetDocumentId::new(),
        )
    }

    pub fn namespace(&self, plot_id: NativeWidgetPlotId) -> NativeWidgetNamespace {
        NativeWidgetNamespace::new(self.document_id, plot_id)
    }
}

pub trait NativeWidgetHostCommandSink: Send + Sync {
    fn push(&self, command: RuntimeHostCommand);
}

impl NativeWidgetHostCommandSink for Mutex<Vec<RuntimeHostCommand>> {
    fn push(&self, command: RuntimeHostCommand) {
        self.lock()
            .expect("native widget command sink lock poisoned")
            .push(command);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NativeWidgetHostTransform {
    offset: [f32; 2],
}

impl NativeWidgetHostTransform {
    pub fn from_offsets(offsets: impl IntoIterator<Item = [f32; 2]>) -> Option<Self> {
        let mut offset = [0.0, 0.0];
        for next in offsets {
            if !next[0].is_finite() || !next[1].is_finite() {
                return None;
            }
            offset[0] += next[0];
            offset[1] += next[1];
            if !offset[0].is_finite() || !offset[1].is_finite() {
                return None;
            }
        }
        Some(Self { offset })
    }

    pub fn map_rect(self, rect: LogicalRect) -> LogicalRect {
        LogicalRect::new(
            rect.x() + self.offset[0],
            rect.y() + self.offset[1],
            rect.width(),
            rect.height(),
        )
        .expect("finite host transform preserves a finite logical rectangle")
    }
}

#[derive(Clone)]
struct FocusedNativeWidget {
    key: NativeWidgetInstanceKey,
    epoch: NativeWidgetAttachmentEpoch,
    caret: Option<LogicalRect>,
    clipboard_payload: String,
}

#[derive(Default)]
struct NativeWidgetHostServiceState {
    focused: Option<FocusedNativeWidget>,
    attachments: HashMap<NativeWidgetInstanceKey, NativeWidgetAttachmentEpoch>,
}

#[derive(Clone, Default)]
pub struct NativeWidgetHostServices {
    state: Arc<Mutex<NativeWidgetHostServiceState>>,
}

/// The live native-widget attachment selected for one neutral event.
///
/// Keyboard, IME, and clipboard events route only to the focused attachment.
/// Runtime wakes route by their full instance namespace and attachment epoch,
/// whether or not that attachment currently owns focus.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeWidgetEventRoute {
    pub key: NativeWidgetInstanceKey,
    pub epoch: NativeWidgetAttachmentEpoch,
}

impl NativeWidgetHostServices {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn focused_clipboard_payload(&self) -> Option<String> {
        self.state
            .lock()
            .expect("native widget host service lock poisoned")
            .focused
            .as_ref()
            .map(|focused| focused.clipboard_payload.clone())
    }

    pub fn focused_attachment(
        &self,
    ) -> Option<(NativeWidgetInstanceKey, NativeWidgetAttachmentEpoch)> {
        self.state
            .lock()
            .expect("native widget host service lock poisoned")
            .focused
            .as_ref()
            .map(|focused| (focused.key.clone(), focused.epoch))
    }

    /// Resolve the native attachment eligible to receive this event.
    pub fn route_event(&self, event: &SceneGraphEvent) -> Option<NativeWidgetEventRoute> {
        let state = self
            .state
            .lock()
            .expect("native widget host service lock poisoned");
        match event {
            SceneGraphEvent::KeyPress(_)
            | SceneGraphEvent::KeyRelease(_)
            | SceneGraphEvent::Ime(_)
            | SceneGraphEvent::Clipboard(_) => {
                let focused = state.focused.as_ref()?;
                (state.attachments.get(&focused.key) == Some(&focused.epoch)).then(|| {
                    NativeWidgetEventRoute {
                        key: focused.key.clone(),
                        epoch: focused.epoch,
                    }
                })
            }
            SceneGraphEvent::RuntimeWake(wake) => {
                state.attachments.iter().find_map(|(key, epoch)| {
                    (key.wake_namespace() == wake.key.namespace
                        && epoch.get() == wake.key.attachment_epoch)
                        .then(|| NativeWidgetEventRoute {
                            key: key.clone(),
                            epoch: *epoch,
                        })
                })
            }
            _ => None,
        }
    }

    fn attach(&self, key: &NativeWidgetInstanceKey, epoch: NativeWidgetAttachmentEpoch) {
        let mut state = self
            .state
            .lock()
            .expect("native widget host service lock poisoned");
        state.attachments.insert(key.clone(), epoch);
        if let Some(focused) = state.focused.as_mut()
            && &focused.key == key
        {
            focused.epoch = epoch;
        }
    }

    fn detach(
        &self,
        key: &NativeWidgetInstanceKey,
        epoch: NativeWidgetAttachmentEpoch,
        sink: &dyn NativeWidgetHostCommandSink,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("native widget host service lock poisoned");
        if state.attachments.get(key) != Some(&epoch) {
            return;
        }
        state.attachments.remove(key);
        let owns_focus = state
            .focused
            .as_ref()
            .is_some_and(|focused| &focused.key == key && focused.epoch == epoch);
        if owns_focus {
            state.focused = None;
            sink.push(RuntimeHostCommand::SetImeAllowed { allowed: false });
            sink.push(RuntimeHostCommand::SetImeCursorArea { rect: None });
        }
    }

    fn focus(
        &self,
        key: NativeWidgetInstanceKey,
        epoch: NativeWidgetAttachmentEpoch,
        caret: Option<LogicalRect>,
        clipboard_payload: String,
        sink: &dyn NativeWidgetHostCommandSink,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("native widget host service lock poisoned");
        let same_attachment = state
            .focused
            .as_ref()
            .is_some_and(|focused| focused.key == key && focused.epoch == epoch);
        if same_attachment {
            let focused = state.focused.as_mut().expect("checked above");
            focused.clipboard_payload = clipboard_payload;
            if focused.caret != caret {
                focused.caret = caret;
                sink.push(RuntimeHostCommand::SetImeCursorArea { rect: caret });
            }
            return;
        }

        if state.focused.is_some() {
            sink.push(RuntimeHostCommand::SetImeAllowed { allowed: false });
            sink.push(RuntimeHostCommand::SetImeCursorArea { rect: None });
        }
        state.focused = Some(FocusedNativeWidget {
            key,
            epoch,
            caret,
            clipboard_payload,
        });
        sink.push(RuntimeHostCommand::SetImeAllowed { allowed: true });
        sink.push(RuntimeHostCommand::SetImeCursorArea { rect: caret });
    }

    fn cancel_composition_and_refocus(
        &self,
        key: NativeWidgetInstanceKey,
        epoch: NativeWidgetAttachmentEpoch,
        caret: Option<LogicalRect>,
        clipboard_payload: String,
        sink: &dyn NativeWidgetHostCommandSink,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("native widget host service lock poisoned");
        if state.focused.is_some() {
            sink.push(RuntimeHostCommand::SetImeAllowed { allowed: false });
            sink.push(RuntimeHostCommand::SetImeCursorArea { rect: None });
        }
        state.focused = Some(FocusedNativeWidget {
            key,
            epoch,
            caret,
            clipboard_payload,
        });
        sink.push(RuntimeHostCommand::SetImeAllowed { allowed: true });
        sink.push(RuntimeHostCommand::SetImeCursorArea { rect: caret });
    }

    fn blur(
        &self,
        key: &NativeWidgetInstanceKey,
        epoch: NativeWidgetAttachmentEpoch,
        sink: &dyn NativeWidgetHostCommandSink,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("native widget host service lock poisoned");
        let owns_focus = state
            .focused
            .as_ref()
            .is_some_and(|focused| &focused.key == key && focused.epoch == epoch);
        if owns_focus {
            state.focused = None;
            sink.push(RuntimeHostCommand::SetImeAllowed { allowed: false });
            sink.push(RuntimeHostCommand::SetImeCursorArea { rect: None });
        }
    }

    fn set_clipboard_payload(
        &self,
        key: &NativeWidgetInstanceKey,
        epoch: NativeWidgetAttachmentEpoch,
        payload: String,
    ) {
        let mut state = self
            .state
            .lock()
            .expect("native widget host service lock poisoned");
        if let Some(focused) = state.focused.as_mut()
            && &focused.key == key
            && focused.epoch == epoch
        {
            focused.clipboard_payload = payload;
        }
    }
}

pub struct NativeWidgetCtx {
    slot: Arc<NativeWidgetInstanceSlot>,
    epoch: NativeWidgetAttachmentEpoch,
    services: NativeWidgetHostServices,
    sink: Arc<dyn NativeWidgetHostCommandSink>,
    transform: NativeWidgetHostTransform,
}

impl NativeWidgetCtx {
    pub fn attach(
        slot: Arc<NativeWidgetInstanceSlot>,
        services: NativeWidgetHostServices,
        sink: Arc<dyn NativeWidgetHostCommandSink>,
        transform: NativeWidgetHostTransform,
    ) -> Self {
        let epoch = slot.attach();
        services.attach(slot.key(), epoch);
        Self {
            slot,
            epoch,
            services,
            sink,
            transform,
        }
    }

    pub fn epoch(&self) -> NativeWidgetAttachmentEpoch {
        self.epoch
    }

    pub fn is_active(&self) -> bool {
        self.slot.is_active(self.epoch)
    }

    pub fn text_cursor(&self) -> CursorStyle {
        CursorStyle::Text
    }

    pub fn focus(&self, caret: Option<LogicalRect>, clipboard_payload: impl Into<String>) {
        if !self.is_active() {
            return;
        }
        self.services.focus(
            self.slot.key().clone(),
            self.epoch,
            caret.map(|rect| self.transform.map_rect(rect)),
            clipboard_payload.into(),
            self.sink.as_ref(),
        );
    }

    /// Cancel an active platform composition and restore focus at a new caret.
    ///
    /// Ordinary caret refreshes use [`Self::focus`] and are cached. Pointer
    /// click-away/click-back paths use this explicit off/on sequence so the
    /// platform IME cannot retain a composition at the old caret.
    pub fn cancel_composition_and_refocus(
        &self,
        caret: Option<LogicalRect>,
        clipboard_payload: impl Into<String>,
    ) {
        if !self.is_active() {
            return;
        }
        self.services.cancel_composition_and_refocus(
            self.slot.key().clone(),
            self.epoch,
            caret.map(|rect| self.transform.map_rect(rect)),
            clipboard_payload.into(),
            self.sink.as_ref(),
        );
    }

    pub fn blur(&self) {
        if self.is_active() {
            self.services
                .blur(self.slot.key(), self.epoch, self.sink.as_ref());
        }
    }

    pub fn set_clipboard_payload(&self, payload: impl Into<String>) {
        if self.is_active() {
            self.services
                .set_clipboard_payload(self.slot.key(), self.epoch, payload.into());
        }
    }

    pub fn request_wakeup(&self, purpose: &str, deadline: Instant, generation: u64) {
        if self.is_active() {
            self.sink.push(RuntimeHostCommand::RequestWakeup {
                key: self.runtime_wake_key(purpose),
                deadline,
                generation,
            });
        }
    }

    pub fn cancel_wakeup(&self, purpose: &str) {
        if self.is_active() {
            self.sink.push(RuntimeHostCommand::CancelWakeup {
                key: self.runtime_wake_key(purpose),
            });
        }
    }

    pub fn accepts_wakeup(&self, wake: &RuntimeWakeEvent, purpose: &str) -> bool {
        self.is_active() && wake.key == self.runtime_wake_key(purpose)
    }

    pub fn accepts_wakeup_generation(
        &self,
        wake: &RuntimeWakeEvent,
        purpose: &str,
        expected_generation: u64,
    ) -> bool {
        self.accepts_wakeup(wake, purpose) && wake.generation == expected_generation
    }

    fn runtime_wake_key(&self, purpose: &str) -> RuntimeWakeKey {
        RuntimeWakeKey::new(self.slot.key().wake_namespace(), self.epoch.get(), purpose)
    }
}

impl Drop for NativeWidgetCtx {
    fn drop(&mut self) {
        if self.slot.detach(self.epoch) {
            self.services
                .detach(self.slot.key(), self.epoch, self.sink.as_ref());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        plot::{Chart, PlotSessionOptions},
        zerod::ZeroDCoord,
    };
    use datafusion::prelude::SessionContext;

    fn commands(sink: &Arc<Mutex<Vec<RuntimeHostCommand>>>) -> Vec<RuntimeHostCommand> {
        std::mem::take(&mut *sink.lock().unwrap())
    }

    #[test]
    fn namespace_isolation_and_rebuilds_reuse_instance_identity() {
        let store = InMemoryNativeWidgetInstanceStore::new();
        let document = NativeWidgetDocumentId::new();
        let first_namespace = NativeWidgetNamespace::new(
            document,
            NativeWidgetPlotId::from_member_path("panel/first"),
        );
        let second_namespace = NativeWidgetNamespace::new(
            document,
            NativeWidgetPlotId::from_member_path("panel/second"),
        );

        let first = store.slot(NativeWidgetInstanceKey::new(
            first_namespace.clone(),
            "search",
        ));
        let rebuilt = store.slot(NativeWidgetInstanceKey::new(first_namespace, "search"));
        let isolated = store.slot(NativeWidgetInstanceKey::new(second_namespace, "search"));
        assert!(Arc::ptr_eq(&first, &rebuilt));
        assert!(!Arc::ptr_eq(&first, &isolated));
        assert_eq!(store.slot_count(), 2);
    }

    #[test]
    fn attach_new_then_detach_old_preserves_new_lease_and_instance() {
        let namespace = NativeWidgetNamespace::ephemeral();
        let slot = InMemoryNativeWidgetInstanceStore::new()
            .slot(NativeWidgetInstanceKey::new(namespace, "editor"));
        let editor = slot
            .get_or_init(|| Mutex::new(String::from("draft")))
            .unwrap();
        let old = slot.attach();
        let replacement = slot.attach();

        assert!(!slot.detach(old));
        assert!(slot.is_active(replacement));
        assert_eq!(*editor.lock().unwrap(), "draft");
        assert!(slot.detach(replacement));
    }

    #[test]
    fn headless_namespace_cleanup_drops_only_its_slots() {
        let store = InMemoryNativeWidgetInstanceStore::new();
        let keep = NativeWidgetNamespace::ephemeral();
        let temporary = NativeWidgetNamespace::ephemeral();
        store.slot(NativeWidgetInstanceKey::new(keep, "kept"));
        store.slot(NativeWidgetInstanceKey::new(temporary.clone(), "temporary"));
        store.remove_namespace(&temporary);
        assert_eq!(store.slot_count(), 1);
    }

    #[test]
    fn focus_switch_orders_ime_commands_and_translates_nested_caret() {
        let store = InMemoryNativeWidgetInstanceStore::new();
        let namespace = NativeWidgetNamespace::ephemeral();
        let first = store.slot(NativeWidgetInstanceKey::new(namespace.clone(), "first"));
        let second = store.slot(NativeWidgetInstanceKey::new(namespace, "second"));
        let services = NativeWidgetHostServices::new();
        let sink = Arc::new(Mutex::new(Vec::new()));
        let transform =
            NativeWidgetHostTransform::from_offsets([[10.0, 20.0], [3.0, 4.0], [-1.0, 2.0]])
                .unwrap();
        let first_ctx = NativeWidgetCtx::attach(first, services.clone(), sink.clone(), transform);
        first_ctx.focus(
            Some(LogicalRect::new(5.0, 6.0, 1.0, 12.0).unwrap()),
            "first selection",
        );
        assert_eq!(
            commands(&sink),
            vec![
                RuntimeHostCommand::SetImeAllowed { allowed: true },
                RuntimeHostCommand::SetImeCursorArea {
                    rect: LogicalRect::new(17.0, 32.0, 1.0, 12.0),
                },
            ]
        );
        first_ctx.focus(
            Some(LogicalRect::new(5.0, 6.0, 1.0, 12.0).unwrap()),
            "first selection",
        );
        assert!(commands(&sink).is_empty(), "unchanged IME rect is cached");
        first_ctx.cancel_composition_and_refocus(
            Some(LogicalRect::new(7.0, 8.0, 1.0, 12.0).unwrap()),
            "moved selection",
        );
        assert_eq!(
            commands(&sink),
            vec![
                RuntimeHostCommand::SetImeAllowed { allowed: false },
                RuntimeHostCommand::SetImeCursorArea { rect: None },
                RuntimeHostCommand::SetImeAllowed { allowed: true },
                RuntimeHostCommand::SetImeCursorArea {
                    rect: LogicalRect::new(19.0, 34.0, 1.0, 12.0),
                },
            ]
        );

        let second_ctx = NativeWidgetCtx::attach(
            second,
            services.clone(),
            sink.clone(),
            NativeWidgetHostTransform::default(),
        );
        second_ctx.focus(None, "second selection");
        assert_eq!(
            commands(&sink),
            vec![
                RuntimeHostCommand::SetImeAllowed { allowed: false },
                RuntimeHostCommand::SetImeCursorArea { rect: None },
                RuntimeHostCommand::SetImeAllowed { allowed: true },
                RuntimeHostCommand::SetImeCursorArea { rect: None },
            ]
        );
        assert_eq!(
            services.focused_clipboard_payload().as_deref(),
            Some("second selection")
        );
        let routed = services
            .route_event(&SceneGraphEvent::Ime(
                avenger_eventstream::window::ImeEvent::Preedit {
                    text: "draft".into(),
                    cursor: Some((0, 5)),
                },
            ))
            .expect("focused IME route");
        assert_eq!(routed.key.widget_id(), "second");
        assert_eq!(routed.epoch, second_ctx.epoch());
    }

    #[test]
    fn replacement_lease_survives_old_drop_and_unfocused_wakes_route() {
        let store = InMemoryNativeWidgetInstanceStore::new();
        let slot = store.slot(NativeWidgetInstanceKey::new(
            NativeWidgetNamespace::ephemeral(),
            "editor",
        ));
        let services = NativeWidgetHostServices::new();
        let sink = Arc::new(Mutex::new(Vec::new()));
        let old = NativeWidgetCtx::attach(
            slot.clone(),
            services.clone(),
            sink.clone(),
            NativeWidgetHostTransform::default(),
        );
        old.focus(None, "draft");
        commands(&sink);

        let replacement = NativeWidgetCtx::attach(
            slot,
            services.clone(),
            sink.clone(),
            NativeWidgetHostTransform::default(),
        );
        let replacement_epoch = replacement.epoch();
        drop(old);
        assert!(replacement.is_active());
        assert_eq!(
            services.focused_attachment().map(|(_, epoch)| epoch),
            Some(replacement_epoch)
        );
        assert!(commands(&sink).is_empty());

        replacement.blur();
        commands(&sink);
        let deadline = Instant::now();
        replacement.request_wakeup("commit", deadline, 9);
        let queued = commands(&sink);
        let [
            RuntimeHostCommand::RequestWakeup {
                key, generation, ..
            },
        ] = queued.as_slice()
        else {
            panic!("expected one wake command")
        };
        let wake = RuntimeWakeEvent {
            key: key.clone(),
            generation: *generation,
        };
        assert!(replacement.accepts_wakeup(&wake, "commit"));
        assert!(replacement.accepts_wakeup_generation(&wake, "commit", 9));
        assert!(!replacement.accepts_wakeup_generation(&wake, "commit", 8));
        assert_eq!(
            services
                .route_event(&SceneGraphEvent::RuntimeWake(wake.clone()))
                .map(|route| route.epoch),
            Some(replacement_epoch)
        );

        let stale = RuntimeWakeEvent {
            key: RuntimeWakeKey::new(
                key.namespace.clone(),
                replacement_epoch.get().saturating_sub(1),
                key.purpose.clone(),
            ),
            generation: 9,
        };
        assert!(
            services
                .route_event(&SceneGraphEvent::RuntimeWake(stale))
                .is_none()
        );
    }

    #[test]
    fn plot_session_option_equality_uses_resource_identity_and_namespace_value() {
        let resources = NativeWidgetRuntimeResources::in_memory();
        let options = PlotSessionOptions::from_native_widget_resources(
            &resources,
            NativeWidgetPlotId::chart_root(),
        );
        assert_eq!(options, options.clone());

        let mut changed_registry = options.clone();
        changed_registry.native_widget_registry = Arc::new(NativeWidgetRegistry::new());
        assert_ne!(options, changed_registry);

        let mut changed_store = options.clone();
        changed_store.native_widget_instance_store =
            Arc::new(InMemoryNativeWidgetInstanceStore::new());
        assert_ne!(options, changed_store);

        let mut changed_namespace = options.clone();
        changed_namespace.native_widget_namespace =
            resources.namespace(NativeWidgetPlotId::from_member_path("another-member"));
        assert_ne!(options, changed_namespace);

        let debug = format!("{options:?}");
        assert!(debug.contains("opaque@"));
        assert!(!debug.contains("slots"));

        let temporary_a = PlotSessionOptions::default();
        let temporary_b = PlotSessionOptions::default();
        assert_ne!(
            temporary_a.native_widget_namespace, temporary_b.native_widget_namespace,
            "headless/default sessions require temporary document ownership"
        );
    }

    #[tokio::test]
    async fn plot_session_equal_options_are_noop_and_each_identity_change_revises_attachment() {
        let ctx = Arc::new(SessionContext::new());
        let compiled = Chart::<ZeroDCoord>::new()
            .compile(ctx.as_ref())
            .await
            .unwrap();
        let mut session = Arc::new(compiled).instantiate(ctx);
        let resources = NativeWidgetRuntimeResources::in_memory();
        let base = PlotSessionOptions::from_native_widget_resources(
            &resources,
            NativeWidgetPlotId::chart_root(),
        );

        session.set_options(base.clone());
        let revision = session.native_widget_attachment_revision();
        assert_eq!(session.options(), &base);
        session.set_options(base.clone());
        assert_eq!(session.native_widget_attachment_revision(), revision);

        let mut registry = base.clone();
        registry.native_widget_registry = Arc::new(NativeWidgetRegistry::new());
        session.set_options(registry);
        assert_eq!(session.native_widget_attachment_revision(), revision + 1);

        let mut store = base.clone();
        store.native_widget_instance_store = Arc::new(InMemoryNativeWidgetInstanceStore::new());
        session.set_options(store);
        assert_eq!(session.native_widget_attachment_revision(), revision + 2);

        let mut namespace = base;
        namespace.native_widget_namespace = resources.namespace(
            NativeWidgetPlotId::from_member_path("panel/nested-widget-cell"),
        );
        session.set_options(namespace);
        assert_eq!(session.native_widget_attachment_revision(), revision + 3);
    }
}
