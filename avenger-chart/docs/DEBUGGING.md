# Debugging avenger-chart

## Using Tracing for Debug Output

The avenger-chart crate uses the `tracing` crate for structured logging and debugging output. This replaces the previous environment variable approach and provides more flexibility.

### Enabling Debug Output

To see debug output from avenger-chart, you need to set up a tracing subscriber in your application and configure the appropriate log levels.

#### Basic Setup

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

#### Environment Variable Configuration

You can control the log level via the `RUST_LOG` environment variable:

```bash
# Show debug output from avenger-chart
RUST_LOG=avenger_chart=debug cargo run

# Show trace-level output (more verbose)
RUST_LOG=avenger_chart=trace cargo run

# Show only warnings and errors
RUST_LOG=avenger_chart=warn cargo run

# Show debug for specific modules
RUST_LOG=avenger_chart::chart_layout=debug,avenger_chart::render=trace cargo run
```

### Debug Output Categories

The crate provides structured logging for various operations:

#### Layout Debugging
- Legend container positioning
- Plot area bounds
- Axis bounds
- Grid configuration

#### Legend Debugging
- Legend measurement and sizing
- Symbol/line/colorbar legend configuration
- Padding and background settings

#### Scale Debugging
- Domain inference
- Scale transformations
- Size scale mappings

### Example: Debugging Layout Issues

```rust
use tracing_subscriber;
use avenger_chart::plot::Plot;

fn main() {
    // Enable debug logging for layout module
    tracing_subscriber::fmt()
        .with_env_filter("avenger_chart::chart_layout=debug")
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

### Debug Layout Rectangles

In debug builds, you can enable visualization of layout bounds by using trace-level logging:

```rust
// This requires debug_assertions to be enabled (debug builds)
tracing_subscriber::fmt()
    .with_env_filter("avenger_chart::render=trace")
    .init();
```

This will add visual rectangles showing the bounds of different layout components.

### Performance Considerations

- Debug and trace logging can impact performance
- Use more specific filters to reduce overhead
- Disable debug output in production builds

### Integration with Other Tools

The tracing output can be consumed by various tools:
- `tracing-subscriber` for console output
- `tracing-chrome` for Chrome tracing format
- `tracing-opentelemetry` for OpenTelemetry integration
- Custom subscribers for application-specific logging

## Legacy Environment Variables

The following environment variables have been replaced with tracing:
- `AVENGER_DEBUG_LAYOUT` → Use `RUST_LOG=avenger_chart=debug`
- `AVENGER_DEBUG_LAYOUT_RECTS` → Use `RUST_LOG=avenger_chart::render=trace` (debug builds only)