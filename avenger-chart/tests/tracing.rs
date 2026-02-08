use std::sync::Once;

use tracing_subscriber::EnvFilter;

static INIT_TRACING: Once = Once::new();

pub fn try_init_tracing() {
    INIT_TRACING.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(EnvFilter::from_default_env())
            .with_test_writer()
            .try_init();
    });
}
