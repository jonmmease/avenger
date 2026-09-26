use crate::{
    DateTimeFormatError, DateTimeFormatProvider, NumberFormatError, NumberFormatProvider,
    PreparedCivilDateTimeFormatter, PreparedInstantFormatter, PreparedNumberFormatter,
};
use std::{
    fmt::{self, Debug},
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

fn next_id() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(1);
    NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
        .expect("formatter binding identity exhausted")
}

struct Bound<P, C> {
    provider: P,
    config: C,
}

impl<P: Debug, C> Debug for Bound<P, C> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Bound")
            .field(&self.provider)
            .finish_non_exhaustive()
    }
}

impl<P: NumberFormatProvider> NumberFormatProvider for Bound<P, P::Config>
where
    P::Config: Send + Sync + 'static,
{
    type Config = ();

    fn prepare(
        &self,
        _: &(),
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
        self.provider.prepare(&self.config, pattern)
    }
}

impl<P: DateTimeFormatProvider> DateTimeFormatProvider for Bound<P, P::Config>
where
    P::Config: Send + Sync + 'static,
{
    type Config = ();

    fn prepare_naive(
        &self,
        _: &(),
        pattern: &str,
    ) -> Result<Arc<dyn PreparedCivilDateTimeFormatter>, DateTimeFormatError> {
        self.provider.prepare_naive(&self.config, pattern)
    }

    fn prepare_zoned(
        &self,
        _: &(),
        pattern: &str,
    ) -> Result<Arc<dyn PreparedInstantFormatter>, DateTimeFormatError> {
        self.provider.prepare_zoned(&self.config, pattern)
    }
}

/// A provider and immutable configuration for preparing runtime number patterns.
/// Clones share the configuration and cache identity.
#[derive(Debug, Clone)]
pub struct NumberFormatBinding {
    id: usize,
    provider: Arc<dyn NumberFormatProvider<Config = ()>>,
}

impl NumberFormatBinding {
    /// Capture typed settings without requiring configuration serialization or cloning.
    pub fn new<P: NumberFormatProvider>(provider: P, config: P::Config) -> Self
    where
        P::Config: Send + Sync + 'static,
    {
        Self {
            id: next_id(),
            provider: Arc::new(Bound { provider, config }),
        }
    }

    /// Prepare a pattern with the captured settings.
    pub fn prepare(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedNumberFormatter>, NumberFormatError> {
        self.provider.prepare(&(), pattern)
    }

    /// Process-local identity for caches. A newly constructed binding has a new identity.
    pub fn cache_id(&self) -> usize {
        self.id
    }
}

/// A provider and immutable configuration for preparing runtime datetime patterns.
/// Clones share the configuration and cache identity.
#[derive(Debug, Clone)]
pub struct DateTimeFormatBinding {
    id: usize,
    provider: Arc<dyn DateTimeFormatProvider<Config = ()>>,
}

impl DateTimeFormatBinding {
    /// Capture typed settings without requiring configuration serialization or cloning.
    pub fn new<P: DateTimeFormatProvider>(provider: P, config: P::Config) -> Self
    where
        P::Config: Send + Sync + 'static,
    {
        Self {
            id: next_id(),
            provider: Arc::new(Bound { provider, config }),
        }
    }

    /// Prepare a civil pattern, ignoring timezone settings.
    pub fn prepare_naive(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedCivilDateTimeFormatter>, DateTimeFormatError> {
        self.provider.prepare_naive(&(), pattern)
    }

    /// Prepare an instant pattern in the configured display timezone.
    pub fn prepare_zoned(
        &self,
        pattern: &str,
    ) -> Result<Arc<dyn PreparedInstantFormatter>, DateTimeFormatError> {
        self.provider.prepare_zoned(&(), pattern)
    }

    /// Process-local identity for caches. A newly constructed binding has a new identity.
    pub fn cache_id(&self) -> usize {
        self.id
    }
}
