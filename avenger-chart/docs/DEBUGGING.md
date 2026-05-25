# Debugging avenger-chart

The avenger-chart crate provides two complementary debugging approaches:

1. **Tracing Logs** - Structured textual logging for understanding internal operations
2. **Visual Debug Rectangles** - Visual overlays showing layout bounds and component positions

## Quick Start

```bash
# Enable debug logs only
RUST_LOG=avenger_chart=debug cargo test -- --nocapture

# Enable visual debug rectangles only
AVENGER_CHART_DEBUG_LAYOUT=1 cargo test

# Enable both (recommended for layout debugging)
AVENGER_CHART_DEBUG_LAYOUT=1 RUST_LOG=avenger_chart::layout=debug cargo test -- --nocapture
```

## Tracing for Debug Logging

The avenger-chart crate uses the `tracing` crate for structured logging. This provides textual output to help understand internal operations like layout calculations, scale transformations, and legend rendering.

`avenger-chart` is a library crate and does **not** initialize a tracing subscriber in production code. Applications, examples, and tests must initialize a subscriber.

### Basic Setup

To see debug output from avenger-chart, set up a tracing subscriber in your application:

```rust
use tracing_subscriber;

fn main() {
    // Initialize tracing with environment filter
    tracing_subscriber::fmt()
        .with_env_filter("avenger_chart=debug")
        .init();

    // Your chart code here
}
```

For visual regression tests in this repository, tracing initialization is handled by `avenger-chart/tests/tracing.rs` via `try_init_tracing()`.

### Environment Variable Configuration

Control log level via the `RUST_LOG` environment variable:

```bash
# Show debug output from all avenger-chart modules
RUST_LOG=avenger_chart=debug cargo run

# Show trace-level output (more verbose)
RUST_LOG=avenger_chart=trace cargo run

# Show only warnings and errors
RUST_LOG=avenger_chart=warn cargo run

# Show debug for specific crates/modules
RUST_LOG=avenger_chart::layout=debug,avenger_chart_legend=trace cargo run
```

**Important**: When running tests, use `-- --nocapture` to see log output:

```bash
RUST_LOG=avenger_chart=debug cargo test -- --nocapture
```

### Available Log Categories

The crate provides structured logging for various operations:

#### Layout And Facet Runtime (`avenger_chart::layout`, `avenger_chart::facet`)
- Layout container positioning
- Plot area bounds calculation
- Axis and guide placement
- Grid configuration
- Taffy layout tree operations
- Facet measurement and coordination

#### Legend Crate (`avenger_chart_legend`)
- Legend measurement and sizing
- Symbol/line/colorbar legend configuration
- Domain value extraction
- Text label formatting
- Padding and background settings

#### Core Utilities (`avenger_chart_core`)
- Expression simplification
- Scalar value conversions
- Color parsing operations

### Example: Debugging Layout Issues

```rust
use tracing_subscriber;
use avenger_chart::plot::Plot;

fn main() {
    // Enable debug logging for layout module
    tracing_subscriber::fmt()
        .with_env_filter("avenger_chart::layout=debug")
        .init();

    // Create a plot - debug output will show layout calculations
    let plot = Plot::new(Cartesian)
        .title("My Chart")
        .legend_fill(|l| l.position(LegendPosition::Right))
        .mark(/* ... */);

    // Render will produce debug output about layout
    plot.render(/* ... */);
}
```

## Visual Debug Layout Rectangles

For visual debugging of layout calculations, avenger-chart can render magenta-colored rectangles showing the bounds of various layout components.

### Enabling Visual Debug

Set the `AVENGER_CHART_DEBUG_LAYOUT` environment variable (to any value):

```bash
# Enable visual debug rectangles
AVENGER_CHART_DEBUG_LAYOUT=1 cargo test

# Works in both debug and release builds
AVENGER_CHART_DEBUG_LAYOUT=1 cargo run --release

# Combine with specific test
AVENGER_CHART_DEBUG_LAYOUT=1 cargo test -p avenger-chart --test visual_regression test_name -- --nocapture
```

### What Gets Visualized

When enabled, magenta rectangles and labels are added for:

- **plot-area**: The main plotting region (center of the chart)
- **of-left, of-right, of-top, of-bottom**: Guide overflow regions for axes
- **Legend channels**: Individual legend containers (labeled by channel name)
- **title**: Chart title area
- **subtitle**: Chart subtitle area

### Implementation Details

The visual debug feature lives in:
- `avenger-chart/src/render/debug.rs` - Rectangle generation logic
- `avenger-chart/src/facet/debug.rs` - Environment and option resolution
- `avenger-chart/src/plot/compiled/rendering.rs` - Overlay mode wiring during evaluation/rendering

## Combining Both Approaches

For comprehensive debugging, combine tracing logs with visual rectangles:

```bash
# Layout debugging (comprehensive)
AVENGER_CHART_DEBUG_LAYOUT=1 RUST_LOG=avenger_chart::layout=debug cargo test -- --nocapture

# Legend debugging
AVENGER_CHART_DEBUG_LAYOUT=1 RUST_LOG=avenger_chart_legend=trace cargo test -- --nocapture

# Everything (very verbose)
AVENGER_CHART_DEBUG_LAYOUT=1 RUST_LOG=avenger_chart=trace cargo test -- --nocapture
```

This provides:
- **Visual feedback**: See exactly where components are positioned
- **Numerical data**: See the calculated dimensions and coordinates in logs
- **Operation flow**: Understand the sequence of layout decisions

## Performance Considerations

- Debug and trace logging can impact performance significantly
- Use more specific module filters to reduce overhead (e.g., `avenger_chart::layout=debug` instead of `avenger_chart=trace`)
- Visual debug rectangles have minimal performance impact
- Disable debug output in production builds by not setting the environment variables

## Integration with Other Tools

The tracing output can be consumed by various tools:

- **tracing-subscriber** - Console output (default)
- **tracing-chrome** - Chrome tracing format for visualization
- **tracing-opentelemetry** - OpenTelemetry integration for distributed tracing
- **Custom subscribers** - Application-specific logging backends

### Example: Chrome Tracing

```rust
use tracing_chrome::ChromeLayerBuilder;
use tracing_subscriber::prelude::*;

fn main() {
    let (chrome_layer, _guard) = ChromeLayerBuilder::new().build();
    tracing_subscriber::registry().with(chrome_layer).init();

    // Your chart code - will generate chrome://tracing compatible output
}
```

## Troubleshooting

**Q: I set `RUST_LOG` but see no output**
- Make sure you're using `-- --nocapture` with cargo test
- Verify you've initialized a tracing subscriber in your application/test
- Check that the module or crate path is correct (for example,
  `avenger_chart::layout` for top-level layout code or `avenger_chart_legend`
  for legend renderer code)

**Q: Visual debug rectangles don't appear**
- Verify `AVENGER_CHART_DEBUG_LAYOUT` is set to `1`

**Q: Too much log output**
- Use more specific module filters: `RUST_LOG=avenger_chart::layout=debug` instead of `avenger_chart=trace`
- Lower the log level to `info` or `warn`
- Focus on the specific component you're debugging

## Guardrails

To verify repository logging conventions:

```bash
avenger-chart/scripts/check_logging_guardrails.sh
```

This enforces:
- no new non-test `eprintln!` callsites in `avenger-chart/src`
- `AVENGER_CHART_DEBUG_LAYOUT` usage restricted to overlay-control paths
