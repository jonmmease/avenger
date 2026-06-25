# Debugging Strategy Guide

This guide helps you choose the right debugging approach for different situations. It covers when to use interactive debugging tools (LLDB via MCP) vs. logging/tracing approaches.

## Quick Decision Matrix

| Situation | Recommended Approach |
|-----------|---------------------|
| "What's in this complex data structure?" | Interactive debugger |
| "Where does this value get corrupted?" | Watchpoint (debugger) |
| "Is this code path even reached?" | Logging/println |
| "What happens across 1000 loop iterations?" | Logging |
| "I need to step through unfamiliar code" | Interactive debugger |
| "Bug only reproduces in CI" | Logging |
| "I want to test a quick fix hypothesis" | `set_variable` (debugger) |
| "Async/concurrent timing issue" | Logging with timestamps |

## Interactive Debugging (LLDB/MCP Tools)

### When to Use

- **Inspecting complex nested data structures** at a specific point in execution
- **Finding where a value changes** using watchpoints
- **Stepping through unfamiliar code** to understand control flow
- **Testing hypotheses** by modifying variables without recompiling
- **Examining the call stack** to understand how you got somewhere
- **Reproducible bugs** in tests or binaries you can run locally

### Available MCP Debug Tools

| Tool | Purpose |
|------|---------|
| `start_debug` | Start debugging a test, binary, or example |
| `set_breakpoint` | Pause execution at a specific line or function |
| `set_watchpoint` | Break when a memory location changes |
| `run`, `next`, `step`, `finish` | Control execution flow |
| `continue_to_line` | Run to a specific line without permanent breakpoint |
| `print_variable` | Inspect a single variable |
| `print_array` | Inspect multiple array/slice elements at once |
| `print_slice` | Inspect Rust slices, Vecs, or Box<[T]> |
| `list_locals` | See all local variables in current scope |
| `set_variable` | Modify a variable's value at runtime |
| `backtrace` | View the call stack |

### Example Workflow: Debugging a Test

```bash
# 1. Start a debug session for a specific test
start_debug(
    target_type="test",
    target="module::tests::test_name",
    package="avenger-geometry",
    args=["module::tests::test_name", "--exact"],
    working_directory="/path/to/avenger"
)

# 2. Set breakpoint at the interesting line
set_breakpoint(session_id, file="my_file.rs", line=42)

# 3. Run to the breakpoint
run(session_id)

# 4. Inspect variables
print_variable(session_id, "my_struct")
print_array(session_id, "buffer.data_ptr", count=10)

# 5. Step through code
next(session_id)  # step over
step(session_id)  # step into

# 6. Continue to another point
continue_to_line(session_id, file="my_file.rs", line=100)
```

### Example: Finding Where a Value Changes

```bash
# Set a watchpoint to break when `counter` is modified
set_watchpoint(session_id, expression="counter", watch_type="write")

# Run - will stop whenever counter changes
run(session_id)

# Check backtrace to see what code modified it
backtrace(session_id)
```

## Logging and Tracing

### When to Use

- **Tracing execution across many iterations** (loops, recursive calls)
- **Non-deterministic or timing-sensitive bugs** where stepping would change behavior
- **CI/remote environments** without interactive access
- **Async/concurrent code** where stepping is awkward
- **Quick sanity checks** ("is this branch taken?")
- **Capturing a persistent record** of execution for later analysis

### Approaches

#### 1. Temporary `println!` / `dbg!`

Quick and dirty - good for one-off checks:

```rust
println!(">>> reached here, value = {:?}", my_value);
dbg!(&my_struct);  // prints file:line and value
```

#### 2. Tracing Crate (Structured Logging)

Better for persistent debugging infrastructure:

```rust
use tracing::{debug, trace, info};

debug!(value = ?my_value, "processing item");
trace!(x = point.x, y = point.y, "coordinate");
```

Run with:
```bash
RUST_LOG=my_crate::module=debug cargo test --release -- --nocapture
```

#### 3. Visual Debug (avenger-chart specific)

For layout issues, enable visual debug rectangles:

```bash
AVENGER_CHART_DEBUG_LAYOUT=1 cargo test --release -p avenger-chart
```

See `avenger-chart/docs/DEBUGGING.md` for details.

### Logging Best Practices

1. **Include context**: Variable values, iteration counts, identifiers
2. **Use structured logging**: `debug!(x = val, "message")` over `debug!("x = {}", val)`
3. **Add timestamps** for timing issues: `RUST_LOG_TIMESTAMP=1`
4. **Filter aggressively**: `module::submodule=debug` not `crate=trace`
5. **Clean up before committing**: Remove temporary `println!` statements

## Hybrid Approach

Often the best strategy combines both:

1. **Add logging** to narrow down the problem area
2. **Set breakpoints** once you've identified the suspicious code
3. **Use watchpoints** to find the exact modification point
4. **Inspect and modify variables** to test fixes
5. **Remove logging** once the bug is found and fixed

## Performance Considerations

| Approach | Performance Impact |
|----------|-------------------|
| Debug build + breakpoints | Significant (debug builds are slower) |
| Watchpoints | Minimal (hardware-assisted) |
| `trace!` logging | Moderate (especially with many calls) |
| `debug!` logging | Low-moderate |
| `println!` | Low |
| Visual debug rectangles | Minimal |

For performance-sensitive debugging:
- Use release builds with debug symbols: `cargo build --release`
- Set breakpoints sparingly
- Prefer watchpoints over stepping through loops
- Use specific log filters to reduce output

## Related Documentation

- `avenger-chart/docs/DEBUGGING.md` - Tracing and visual debug setup for avenger-chart
- `guides/suggested-commands.md` - Common cargo test --release commands with debug flags