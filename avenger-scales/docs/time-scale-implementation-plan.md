# Time Scale Implementation Plan

## Overview

This plan outlines the implementation of comprehensive time scale support for avenger-scales, addressing limitations in existing visualization libraries (Vega, D3) by providing configurable timezone support and unified handling of all Arrow temporal types.

## Key Goals

- [ ] **Unified Time Scale**: One scale type that works across Date, Timestamp, and Timestamptz Arrow types
- [ ] **Configurable Timezone Support**: Beyond Vega's local/UTC limitation - support arbitrary IANA timezones  
- [ ] **Arrow-First Architecture**: Leverage Arrow kernels where possible, fallback to chrono-tz when needed
- [ ] **Calendar-Aware Operations**: Smart tick generation, nice intervals, and DST handling
- [ ] **Performance**: Efficient temporal computations using Arrow's columnar approach

## Research Findings

### Arrow Temporal Capabilities
- **Available**: Basic temporal conversions (timestamp_*_to_datetime functions)
- **Available**: Temporal array types (Date32/64, Timestamp*, Time*)  
- **Available**: Limited timezone module (arrow::array::timezone with Tz struct)
- **Missing**: Calendar arithmetic, nice interval generation, timezone-aware operations
- **Missing**: Comprehensive temporal compute kernels

### Required External Dependencies
- **chrono-tz**: IANA timezone database, DST handling, timezone conversions
- **chrono**: Core temporal arithmetic and calendar operations

### D3/Vega Time Scale Algorithm Analysis

#### D3 Time Interval Hierarchy
D3 provides a comprehensive hierarchy of time intervals for tick generation and nice domain calculations:

**Interval Progression**: millisecond → second → minute → hour → day → week → month → year

**Key Intervals with Step Values**:
- **Milliseconds**: 1, 5, 15, 25, 50, 100, 250, 500ms
- **Seconds**: 1, 5, 15, 30s  
- **Minutes**: 1, 5, 15, 30min
- **Hours**: 1, 3, 6, 12h
- **Days**: 1, 2d
- **Weeks**: 1w (7 days)
- **Months**: 1, 3m (quarters)
- **Years**: 1y

#### D3 Nice Algorithm
```javascript
scale.nice = function(interval) {
  var d = domain();
  if (!interval || typeof interval.range !== "function") {
    interval = tickInterval(d[0], d[d.length - 1], interval == null ? 10 : interval);
  }
  return interval ? domain(nice(d, interval)) : scale;
};
```

**Algorithm Steps**:
1. **Auto-interval selection**: If no interval specified, `tickInterval()` selects appropriate interval based on domain span and target tick count (default 10)
2. **Interval validation**: Ensures interval has required methods (`range()` function)
3. **Domain adjustment**: Applies interval-based nice transformation to extend domain to clean boundaries

#### D3 Tick Generation Algorithm
**Interval Selection Strategy**:
1. Calculate target interval duration: `(domain_end - domain_start) / desired_tick_count`
2. Use binary search (`bisector`) to find closest predefined interval from hierarchy
3. Apply interval's `range()` method to generate tick positions
4. Handle edge cases (ascending/descending domains, timezone boundaries)

**Calendar Boundary Alignment**:
- **Milliseconds/Seconds**: Round to clean decimal boundaries  
- **Minutes/Hours**: Align to hour boundaries (e.g., :00, :15, :30, :45)
- **Days**: Align to midnight boundaries
- **Weeks**: Align to week start (Sunday/Monday depending on locale)
- **Months**: Align to first day of month
- **Years**: Align to January 1st

#### D3 Time Interval Implementation Pattern
Each interval (year, month, day, etc.) implements:
- `floor(date)`: Round down to interval boundary
- `ceil(date)`: Round up to next interval boundary  
- `offset(date, step)`: Add/subtract intervals
- `range(start, stop)`: Generate sequence of interval boundaries
- `every(step)`: Create interval with custom step size

**Calendar Arithmetic Handling**:
- **DST Transitions**: Days can be 23-25 hours; handled by checking timezone offsets
- **Variable Month Lengths**: 28-31 days handled by calendar-aware date arithmetic
- **Leap Years**: Handled by JavaScript Date object's built-in calendar logic
- **Week Boundaries**: Supports different week start days (Sunday through Saturday)

#### Vega Enhancements Over D3
- **UTC vs Local Time**: Explicit `time` vs `utc` scale types
- **TimeUnit Integration**: Connects with Vega-Lite's `timeUnit` encoding
- **Known Limitations**: 
  - "Nice" ticks can behave unexpectedly when domain falls between nice boundaries
  - Limited timezone support (only local and UTC)
  - Tooltip timezone display issues

#### Implementation Insights for Avenger-Scales
1. **Predefined Interval Hierarchy**: Essential for automatic interval selection
2. **Binary Search Selection**: Efficient algorithm for choosing appropriate intervals
3. **Calendar-Aware Boundaries**: Critical for human-readable tick positions
4. **DST Handling**: Must account for variable day lengths in local time
5. **Configurable Week Start**: Important for international applications
6. **UTC/Local Separation**: Clear distinction needed for timezone handling

### Additional Library Research

#### Pandas Time Series Resampling
**Algorithm Strategy**:
- **Frequency Offset System**: Uses string-based frequency specifications ("D", "H", "M", "5Min", etc.)
- **Parameterized Offsets**: Support for custom week starts, month ends, business days
- **Label/Closed Control**: Flexible bin edge labeling and boundary inclusion rules
- **Origin/Offset Parameters**: Fine-grained control over bin alignment and starting points

**Key Features**:
- **DateOffset Classes**: Extensible system for defining custom time intervals
- **Resampling Types**: Both upsampling (interpolation) and downsampling (aggregation)
- **Business Calendar Support**: Built-in handling of business days, holidays
- **Performance**: Highly optimized for large time series datasets

#### Matplotlib Date Locators
**Comprehensive Locator System**:
- **AutoDateLocator**: Intelligent automatic interval selection
- **Specific Locators**: YearLocator, MonthLocator, WeekdayLocator, DayLocator, HourLocator, etc.
- **RRuleLocator**: Complex rule-based patterns ("last Friday of each month")
- **ConciseDateFormatter**: Context-aware date formatting

**Algorithm Features**:
- **Multi-scale Approach**: Different locators for different zoom levels
- **Calendar Intelligence**: Weekday-aware positioning, month-end handling
- **Minor/Major Tick Coordination**: Hierarchical tick systems
- **Date Range Constraints**: Support for years 0001-9999

#### ggplot2 (R) Date/Time Scales
**Break Generation Strategy**:
- **Smart Defaults**: Automatic sensible major/minor tick placement
- **Calendar Units**: "sec", "min", "hour", "day", "week", "month", "year" with multipliers
- **Priority System**: `date_breaks` > `breaks`, `date_labels` > `labels`
- **Offset Support**: Ability to shift break alignment

**Advanced Features**:
- **Multiple Scale Types**: `scale_*_date`, `scale_*_datetime`, `scale_*_time`
- **Flexible Formatting**: Integration with strftime() format codes
- **Break Positioning**: Control over label alignment and tick spacing

#### Plotly Time Axis
**Dynamic Adaptation**:
- **Zoom-aware Formatting**: Different formats for different zoom levels
- **tickformatstops**: Multi-level formatting based on visible range
- **Period vs Instant Mode**: Label positioning at period centers vs boundaries
- **Calendar Interval Support**: dtick with calendar-aware stepping

**Limitations Identified**:
- **Limited Manual Control**: Less granular control compared to matplotlib
- **Automatic Behavior**: Sometimes unpredictable tick placement

#### Observable Plot Time Scales
**D3-Based Foundation**:
- **Inherited D3 Logic**: Builds on D3's proven time scale algorithms
- **Plot-Specific Enhancements**: Integration with Plot's mark system
- **Interval Transform**: Built-in support for time-based grouping
- **UTC/Local Options**: Clear separation of timezone handling

#### Polars Time Series
**Performance-Focused Approach**:
- **Dynamic Grouping**: `group_by_dynamic` for time-based aggregation
- **Interval Syntax**: Composable interval strings ("1h30m")
- **Sorting Optimization**: Fast-path operations for sorted temporal data
- **Upsampling/Downsampling**: Efficient frequency conversion

**Modern Features**:
- **Lazy Evaluation**: Deferred computation for large datasets
- **Native Temporal Types**: Built-in Date/Datetime with multiple precisions
- **Activity-Based Sampling**: Support for non-uniform time intervals

#### DuckDB Temporal Functions
**Comprehensive Calendar System**:
- **ICU Integration**: International calendar and timezone support
- **Window Functions**: Tumbling, hopping, sliding temporal windows
- **Interval Arithmetic**: Three-component intervals (months, days, microseconds)
- **Range Generation**: Built-in functions for creating time sequences

**Enterprise Features**:
- **Non-Gregorian Calendars**: Support for alternative calendar systems
- **DST-Aware Binning**: Proper handling of timezone transitions
- **Temporal Analytics**: Advanced time-based aggregation functions

#### Apache Arrow Temporal Kernels
**Low-Level Foundations**:
- **SIMD Optimization**: 64-byte padding for vectorized operations
- **Timezone Support**: Per-kernel timezone awareness (with known limitations)
- **Type Casting**: Temporal type conversions with resolution handling
- **Performance Focus**: Columnar operations for high-throughput scenarios

**Current Limitations**:
- **Limited High-Level Functions**: Mostly component extraction, less calendar logic
- **Missing Features**: No built-in tick generation or nice interval algorithms
- **Work in Progress**: Active development of temporal arithmetic kernels

### Synthesis for Avenger-Scales

**Best Practices from Research**:
1. **Multi-Library Approach**: Combine strengths from different libraries
2. **Pandas-Style Frequency Strings**: Proven, intuitive interval specification
3. **Matplotlib's Locator Hierarchy**: Comprehensive automatic/manual control
4. **D3's Calendar Intelligence**: Human-readable boundary alignment
5. **DuckDB's ICU Integration**: International calendar and timezone support
6. **Arrow's Performance Foundation**: SIMD-optimized columnar operations

**Competitive Advantages to Implement**:
1. **Configurable Timezone Support**: Beyond Vega's local/UTC limitation
2. **Arrow-Native Performance**: Leverage columnar optimizations
3. **Unified API**: Handle Date/Timestamp/TimestampTz seamlessly
4. **International Support**: ICU-style calendar and timezone awareness
5. **Business Calendar Extensions**: Support for custom calendars and holidays  

## Concrete Specification for Avenger-Scales Time Scale

### Supported Temporal Types
1. **Arrow Date32**: Days since Unix epoch (1970-01-01)
2. **Arrow Date64**: Milliseconds since Unix epoch  
3. **Arrow Timestamp**: With units (s, ms, μs, ns) and optional timezone
4. **Arrow TimestampTz**: Timestamp with timezone metadata

### Core API

#### Scale Construction
```rust
// Unified constructor that auto-detects temporal type
TimeScale::configured(domain: (ArrayRef, ArrayRef), range: (f32, f32)) -> ConfiguredScale

// Examples:
// Date array → numeric range
let scale = TimeScale::configured((date_array_start, date_array_end), (0.0, 100.0));

// Timestamp array → color range  
let scale = TimeScale::configured_color((timestamp_array_start, timestamp_array_end), &["red", "blue"]);
```

#### Configuration Options
```rust
scale
    .with_option("timezone", "America/New_York")  // Display timezone (IANA string or "local"/"utc")
    .with_option("nice", true)                    // Extend domain to calendar boundaries
    .with_option("nice", 10.0)                    // Target ~10 ticks with nice boundaries
    .with_option("interval", "day")               // Force specific tick interval
    .with_option("interval", "3 hours")           // Custom interval with count
    .with_option("week_start", "monday")          // Week boundary configuration
    .with_option("locale", "en-US")               // Formatting locale
```

### Interval Specification Language

#### Basic Units
- `"millisecond"` or `"ms"` 
- `"second"` or `"s"`
- `"minute"` or `"min"`  
- `"hour"` or `"h"`
- `"day"` or `"d"`
- `"week"` or `"w"`
- `"month"` or `"mo"`
- `"year"` or `"y"`

#### Compound Intervals
- `"3 hours"` - Every 3 hours
- `"15 minutes"` - Every 15 minutes
- `"2 weeks"` - Every 2 weeks
- `"quarter"` - Every 3 months

#### Special Intervals
- `"business_day"` - Monday-Friday only
- `"month_start"` - First day of each month
- `"month_end"` - Last day of each month
- `"week_start"` - Configured week start day

### Nice Domain Algorithm

#### Interval Hierarchy (D3-inspired)
```rust
const INTERVAL_HIERARCHY: &[(Duration, &str, Vec<i32>)] = &[
    (Duration::milliseconds(1), "ms", vec![1, 5, 15, 25, 50, 100, 250, 500]),
    (Duration::seconds(1), "s", vec![1, 5, 15, 30]),
    (Duration::minutes(1), "min", vec![1, 5, 15, 30]),
    (Duration::hours(1), "h", vec![1, 3, 6, 12]),
    (Duration::days(1), "d", vec![1, 2, 7]),
    (Duration::days(7), "w", vec![1]),
    (Duration::months(1), "mo", vec![1, 3]),
    (Duration::years(1), "y", vec![1, 2, 5, 10, 20, 50, 100]),
];
```

#### Nice Boundary Rules
1. **Milliseconds/Seconds**: Round to clean decimal multiples
2. **Minutes**: Align to :00, :15, :30, :45
3. **Hours**: Align to hour boundaries
4. **Days**: Align to midnight in display timezone
5. **Weeks**: Align to configured week start
6. **Months**: Align to first of month
7. **Years**: Align to January 1st

### Tick Generation Algorithm

#### Automatic Interval Selection
```rust
fn select_interval(domain_span: Duration, target_count: f32) -> Interval {
    let target_interval = domain_span / target_count;
    // Binary search through INTERVAL_HIERARCHY
    // Return closest interval that produces readable ticks
}
```

#### Tick Position Rules
1. **Always include nice boundaries** when domain spans them
2. **Respect timezone** for day/week/month boundaries  
3. **Handle DST transitions** gracefully (skip/repeat as needed)
4. **Generate uniform spacing** within calendar constraints

### Timezone Handling

#### Timezone Resolution Order
1. Explicit scale configuration: `.with_option("timezone", "...")` 
2. TimestampTz embedded timezone (if present)
3. System local timezone (if "local" specified)
4. UTC fallback

#### Timezone Operations
```rust
// All internal calculations in UTC
// Convert to display timezone only for:
// - Nice domain calculations
// - Tick generation
// - Label formatting
```

### Scale Operations

#### Forward Scaling (temporal → numeric)
```rust
scale(dates: &ArrayRef) -> Result<ArrayRef, AvengerScaleError>
// Convert temporal values to normalized [0, 1] range
// Then map to configured output range
```

#### Inverse Scaling (numeric → temporal)
```rust
invert(values: &ArrayRef) -> Result<ArrayRef, AvengerScaleError>
// Map from output range to [0, 1]
// Convert to temporal values preserving original type
```

#### Tick Generation
```rust
ticks(count: Option<f32>) -> Result<ArrayRef, AvengerScaleError>
// Generate ~count tick positions
// Return array of same temporal type as domain
// Positions align with calendar boundaries
```

#### Tick Formatting
```rust
tick_format(count: Option<f32>) -> Result<Formatter, AvengerScaleError>
// Return timezone-aware formatter
// Adapts format to tick interval:
//   - Years: "2024"
//   - Months: "Jan 2024"  
//   - Days: "Jan 15"
//   - Hours: "3:00 PM"
//   - Minutes: "3:45 PM"
//   - Seconds: "3:45:30"
```

### DST and Calendar Edge Cases

#### DST Transitions
- **Spring forward**: Skip non-existent hour (2 AM → 3 AM)
- **Fall back**: Disambiguate repeated hour using offset
- **Tick generation**: Adjust spacing to maintain visual uniformity

#### Variable Length Periods
- **Months**: 28-31 days handled by calendar arithmetic
- **Years**: Leap years handled automatically
- **Days**: 23-25 hours during DST handled by timezone library

### Performance Requirements
1. **Scale 1M temporal values** in < 100ms
2. **Generate ticks** for any domain in < 10ms  
3. **Batch timezone conversions** for efficiency
4. **Cache computed tick positions** per scale instance

### Error Handling
```rust
enum TimeScaleError {
    InvalidTimezone(String),
    InvalidInterval(String),
    InvalidTemporalType(DataType),
    TimezoneConversionError(String),
    DomainRangeError(String),
}
```

### Examples

#### Financial Time Series
```rust
// Market hours with minute ticks
let scale = TimeScale::configured((market_open, market_close), (0.0, width))
    .with_option("timezone", "America/New_York")
    .with_option("interval", "30 minutes")
    .with_option("nice", false);  // Exact market hours
```

#### Multi-Year Climate Data  
```rust
// Monthly averages over decades
let scale = TimeScale::configured((start_date, end_date), (0.0, height))
    .with_option("timezone", "UTC")
    .with_option("interval", "year")
    .with_option("nice", true);
```

#### International Event Timeline
```rust
// Events in different timezones displayed in local time
let scale = TimeScale::configured((first_event, last_event), (0.0, width))
    .with_option("timezone", "local")
    .with_option("nice", 10.0);  // ~10 automatic ticks
```

## Architecture Design

### Core Components

#### 1. TimeScale Structure
```rust
pub struct TimeScale {
    // Uses existing ConfiguredScale pattern
}

impl TimeScale {
    pub fn configured(domain: (ArrayRef, ArrayRef), range: (f32, f32)) -> ConfiguredScale
    pub fn configured_color<I>(domain: (ArrayRef, ArrayRef), range: I) -> ConfiguredScale
}
```

#### 2. Temporal Handler Pattern
```rust
enum TemporalHandler {
    Date(DateHandler),
    Timestamp(TimestampHandler), 
    TimestampTz(TimestampTzHandler),
}

trait TemporalOperations {
    fn nice_domain(&self, domain: (i64, i64), count: f32) -> Result<(i64, i64), AvengerScaleError>;
    fn generate_ticks(&self, domain: (i64, i64), count: f32) -> Result<Vec<i64>, AvengerScaleError>;
    fn to_display_timezone(&self, timestamp: i64) -> Result<i64, AvengerScaleError>;
}
```

## Implementation Plan

### Phase 1: Foundation & Core Infrastructure
- [x] **1.1** Create TimeScale module structure
  - [x] Add `avenger-scales/src/scales/time.rs`
  - [x] Add TimeScale struct with ScaleImpl trait
  - [x] Add basic configuration options (timezone, nice, unit preferences)
  
- [x] **1.2** Add temporal dependencies
  - [x] Add chrono-tz to Cargo.toml for timezone support
  - [x] Add chrono for temporal arithmetic
  - [x] Verify Arrow temporal_conversions integration
  
- [x] **1.3** Implement temporal type detection
  - [x] Create TemporalHandler enum and traits
  - [x] Add automatic type detection from ArrayRef
  - [x] Implement handler factory pattern

### Phase 2: Core Temporal Operations
- [x] **2.1** Domain handling for each temporal type
  - [x] DateHandler: Date32/Date64 → chrono::NaiveDate
  - [x] TimestampHandler: Timestamp(unit, None) → chrono::NaiveDateTime 
  - [x] TimestampTzHandler: Timestamp(unit, Some(tz)) → chrono::DateTime<Tz>
  
- [x] **2.2** Arrow integration layer
  - [x] Wrapper functions for arrow::temporal_conversions
  - [x] Efficient array-to-temporal conversions
  - [x] Handle different temporal units (s, ms, μs, ns)
  
- [x] **2.3** Timezone configuration system
  - [x] Parse IANA timezone strings with chrono-tz
  - [x] Default timezone handling (defaults to UTC)
  - [x] Timezone validation and error handling

### Phase 3: Calendar-Aware Algorithms  
- [x] **3.1** Nice interval generation (inspired by D3)
  - [x] Calendar interval hierarchy: ms → s → min → hour → day → week → month → year
  - [x] Smart interval selection based on domain span
  - [x] Handle DST transitions in interval calculations
  - [x] Support for custom interval preferences
  
- [x] **3.2** Tick generation algorithm
  - [x] Implement D3-style tick generation with calendar awareness
  - [x] Generate human-readable tick positions (midnight, first of month, etc.)
  - [x] Support for custom tick counts and intervals
  - [x] Handle edge cases (DST transitions)
  - [ ] Handle edge cases (leap years, different calendar months)

- [x] **3.3** Domain normalization for time
  - [x] Extend domain to nice round temporal values
  - [ ] Zero option interpretation for temporal (not applicable - will document)
  - [x] Calendar-boundary alignment

### Phase 4: Scale Operations Implementation
- [x] **4.1** Core scaling operations
  - [x] Implement scale() method for temporal → numeric mapping
  - [x] Implement invert() method for numeric → temporal mapping
  - [x] Handle timezone conversions during scaling
  
- [x] **4.2** Temporal arithmetic
  - [x] Calendar-aware domain calculations
  - [x] Handle different temporal units consistently
  - [ ] Support for relative temporal operations (future enhancement)
  
- [x] **4.3** Error handling and edge cases
  - [x] Invalid timezone specifications
  - [x] Malformed temporal data
  - [x] DST transition edge cases
  - [ ] Out-of-range temporal values

### Phase 5: Advanced Features
- [ ] **5.1** Formatting and display
  - [ ] Timezone-aware formatting for ticks and labels
  - [ ] Locale-aware temporal formatting options
  - [ ] Integration with existing Formatter system
  - [ ] Implement tick_format() method

- [ ] **5.3** Additional temporal types support (future enhancement)
  - [ ] Duration arrays if needed
  - [ ] Time-only arrays (Time32, Time64)
  - [ ] Interval arrays (future Arrow addition)

### Phase 6: Integration & Testing
- [x] **6.1** ConfiguredScale integration
  - [x] Add TimeScale to scale type enumeration
  - [x] Update normalize() method for temporal domains
  - [x] Ensure consistent API across all scale types
  
- [ ] **6.2** Comprehensive testing
  - [x] Unit tests for each temporal handler
  - [x] Integration tests with different Arrow temporal types
  - [x] Timezone conversion accuracy tests
  - [x] DST transition edge case tests (US timezones)
  - [ ] Additional DST tests (European, Southern hemisphere)
  - [ ] Performance benchmarks vs existing solutions
  
- [ ] **6.3** Examples and documentation
  - [ ] Create temporal scale examples
  - [ ] Document timezone configuration options
  - [ ] Compare with Vega/D3 time scale behavior
  - [ ] Migration guide for users

## Configuration API Design

### Scale Configuration Options
```rust
TimeScale::configured(temporal_domain, numeric_range)
    .with_option("timezone", "America/New_York")     // IANA timezone
    .with_option("nice", true)                       // Calendar-aware nice intervals
    .with_option("unit_preference", "auto")          // Preferred time units for ticks
    .with_option("locale", "en-US")                  // Locale for formatting
```

### Supported Configuration Values
- **timezone**: 
  - IANA timezone strings ("America/New_York", "Europe/London", "UTC")
  - "local" for system timezone
  - "utc" for UTC (default for compatibility)
- **nice**: boolean or numeric tick count hint
- **unit_preference**: "auto", "calendar" (prefer month/year), "metric" (prefer powers of 10)
- **zero**: Not applicable for time scales (error or ignore)

## Technical Considerations

### Arrow Integration Strategy
1. **Prioritize Arrow kernels** where available (basic conversions)
2. **Use chrono-tz for gaps** (timezone operations, calendar arithmetic)
3. **Efficient interop** between Arrow arrays and chrono types
4. **Minimize copying** - work with Arrow data directly when possible

### Timezone Handling Philosophy
1. **Always explicit** - no implicit timezone assumptions
2. **UTC internal storage** - convert for display only
3. **Preserve original timezone info** from Timestamptz arrays
4. **Configurable display timezone** - independent of data timezone

### Performance Priorities
1. **Batch operations** on Arrow arrays
2. **Cache timezone computations** where possible  
3. **Lazy evaluation** of expensive operations
4. **Zero-copy** operations where Arrow supports it

## Future Extensions

### Potential Enhancements
- [ ] **Custom calendar systems** (fiscal years, academic calendars)
- [ ] **Business time calculations** (excluding weekends/holidays)
- [ ] **Streaming temporal data** support
- [ ] **Temporal aggregation** operations (daily/monthly rollups)
- [ ] **Time zone-aware data export** for different formats

### Arrow Evolution
- [ ] Monitor Arrow temporal kernel development
- [ ] Migrate to native Arrow operations as they become available
- [ ] Contribute temporal improvements back to Arrow ecosystem

## Success Metrics

### Functionality Goals
- [ ] Support all Arrow temporal types (Date32/64, Timestamp*, TimestampTz)
- [ ] Handle 50+ major IANA timezones correctly
- [ ] Generate human-readable ticks for domains spanning microseconds to decades
- [ ] Handle DST transitions without visual artifacts

### Performance Goals  
- [ ] Scale 1M+ temporal values in <100ms
- [ ] Generate ticks for any reasonable domain in <10ms
- [ ] Memory usage comparable to existing numeric scales

### API Goals
- [ ] Zero-breaking-change integration with existing avenger-scales
- [ ] API simpler than manual chrono + Arrow temporal handling
- [ ] Clear error messages for timezone/temporal issues
- [ ] Compatible with existing scale patterns (normalize, with_option, etc.)

## Dependencies

### Required Crates
```toml
[dependencies]
chrono = { version = "0.4", features = ["serde"] }
chrono-tz = "0.8"
# arrow already included in project
```

### Optional Future Dependencies
```toml
# For advanced calendar systems
icu_calendar = "1.0"  # If custom calendar support needed
# For business time calculations  
chrono-business = "0.1"  # If business time features needed
```

---

## Complete DST Support Implementation Plan

### Overview
Implement comprehensive Daylight Saving Time (DST) handling to ensure the implementation never panics and handles all edge cases correctly.

### Core Principles
1. **Never Panic**: All time operations must be safe and handle edge cases gracefully
2. **Predictable Behavior**: Users should understand how ambiguous times are resolved
3. **Visual Accuracy**: Tick spacing should reflect actual time duration, not nominal
4. **Explicit Choices**: When ambiguity exists, make explicit, documented choices

### DST Phase 1: Safe Time Construction Infrastructure

#### 1.1 Create Safe Time Construction Wrappers
- [x] Create `SafeDateTime` wrapper module with panic-free operations
  ```rust
  // safe_time.rs
  pub fn safe_with_hour(dt: DateTime<Tz>, hour: u32) -> Result<DateTime<Tz>, DstError>
  pub fn safe_with_time(dt: DateTime<Tz>, hour: u32, min: u32, sec: u32) -> Result<DateTime<Tz>, DstError>
  pub fn safe_and_hms(date: Date<Tz>, hour: u32, min: u32, sec: u32) -> Result<DateTime<Tz>, DstError>
  ```

#### 1.2 DST Transition Detection
- [x] Implement DST transition detection utilities
  ```rust
  pub fn is_dst_transition_date(date: Date<Tz>) -> bool
  pub fn find_transition_hours(date: Date<Tz>) -> DstTransition
  pub enum DstTransition {
      None,
      SpringForward { missing_start: u32, missing_end: u32 },
      FallBack { repeated_start: u32, repeated_end: u32 }
  }
  ```

#### 1.3 Ambiguous Time Resolution Strategy
- [x] Implement consistent ambiguous time resolution
  ```rust
  pub enum DstStrategy {
      EarliestOffset,  // For fall-back, use first occurrence
      LatestOffset,    // For fall-back, use second occurrence
      PreferStandard,  // Prefer standard time over DST
      PreferDaylight,  // Prefer DST over standard time
  }
  ```

### DST Phase 2: Update Interval Operations

#### 2.1 Fix TimeInterval::floor() for DST
- [x] Update floor to handle non-existent times
- [x] Handle spring-forward gap safely

#### 2.2 Fix TimeInterval::ceil() for DST
- [x] Ensure ceil handles DST transitions correctly
- [x] Test with domains ending in non-existent hours

#### 2.3 Fix TimeInterval::offset() for DST
- [x] Make offset operations DST-aware
- [x] Add hours in local time, not by duration

#### 2.4 Duration-Based Intervals
- [x] Add actual duration calculation for intervals
- [x] Account for DST changes in duration

### DST Phase 3: Fix Tick Generation

#### 3.1 DST-Aware Tick Generation
- [x] Update `generate_temporal_ticks` to handle transitions
- [x] Handle spring-forward gaps gracefully
- [x] Skip to next valid time when needed

#### 3.2 Tick Spacing Validation
- [x] Add tick spacing validation to ensure visual consistency
- [x] Implement adaptive tick generation for DST transitions

#### 3.3 Sub-hour Interval Handling
- [x] Special handling for minute/second intervals during transitions
- [x] Ensure no duplicate timestamps in fall-back hour

### DST Phase 4: Fix Nice Domain Calculation

#### 4.1 Safe Nice Boundaries
- [x] Update `compute_nice_temporal_bounds` for DST safety
- [x] Handle boundaries in non-existent times

#### 4.2 Domain Validation
- [x] Validate that nice domains don't create impossible ranges
- [x] Handle domains entirely within DST transitions

### DST Phase 5: Update Scale Operations

#### 5.1 Domain Normalization
- [x] Ensure normalized domains are DST-safe
- [x] Handle domains spanning DST transitions

#### 5.2 Scale Interpolation
- [x] Use actual duration for interpolation, not nominal
- [x] Account for DST transitions in the domain
- [x] Implemented compute_actual_duration_millis() for DST-aware duration calculation
- [x] Updated scale() method to use actual duration when DST transitions occur

#### 5.3 Inversion Accuracy
- [x] Ensure invert() handles DST transitions correctly
- [x] Test inversion at DST boundaries
- [x] Implemented iterative refinement for DST-aware inversion
- [x] Added comprehensive tests for spring-forward and fall-back scenarios

### DST Phase 6: Comprehensive Testing

#### 6.1 DST Transition Tests
- [x] Test all US timezone transitions (EST, CST, MST, PST)
- [ ] Test European transitions (CET, BST)
- [ ] Test Southern hemisphere (AEST, NZST)
- [x] Test non-DST timezones (JST, IST, UTC)

#### 6.2 Edge Case Tests
- [x] Domain starting in spring-forward gap
- [x] Domain ending in fall-back overlap
- [ ] Single-hour domain during DST transition
- [x] Tick generation across multiple DST transitions
- [x] Scale/invert operations across DST transitions

#### 6.3 Property-Based Tests
- [x] No panic property: all operations must return Result or safe default
- [x] Monotonicity: ticks must be strictly increasing
- [x] Roundtrip: scale → invert → scale should preserve values (tested in test_dst_scale_operations)
- [x] Duration accuracy: visual spacing matches actual time (verified in DST scale tests)

#### 6.4 Historical DST Tests
- [ ] Test historical DST rule changes
- [ ] Test future DST dates (if rules known)

### DST Phase 7: Performance Optimization

#### 7.1 DST Transition Caching
- [ ] Cache DST transition points per timezone
- [ ] Avoid repeated transition calculations

#### 7.2 Batch Operations
- [ ] Optimize batch tick generation across DST
- [ ] Minimize timezone conversions

### DST Phase 8: Documentation and Examples

#### 8.1 DST Behavior Documentation
- [ ] Document how each ambiguous case is resolved
- [ ] Explain visual spacing during transitions
- [ ] Provide examples for common DST scenarios

#### 8.2 Configuration Options
- [ ] Add DST handling configuration options
  ```rust
  .with_option("dst_strategy", "earliest")  // or "latest", "standard", "daylight"
  .with_option("strict_dst", true)  // fail vs adjust for invalid times
  ```

#### 8.3 Migration Guide
- [ ] Document any breaking changes
- [ ] Provide upgrade path for existing users

### DST Success Criteria

- [ ] Zero panics in all DST scenarios
- [ ] All tests pass with DST-observing timezones
- [ ] Clear documentation of DST behavior
- [ ] Performance within 10% of non-DST operations
- [ ] No breaking changes to existing API (only additions)

---

## Summary of Remaining Work

### High Priority - Core Functionality
1. **Formatting & Display (Phase 5.1)**
   - [ ] Implement tick_format() method for timezone-aware date/time formatting
   - [ ] Integration with existing Formatter system
   - [ ] Locale-aware formatting options

2. **Edge Case Handling**
   - [ ] Out-of-range temporal values
   - [ ] Single-hour domain during DST transition test
   - [ ] Handle leap years in calendar arithmetic
   - [ ] Handle different calendar month lengths

### Medium Priority - Testing & Documentation
3. **Additional Testing (Phase 6)**
   - [ ] European timezone DST tests (CET, BST)
   - [ ] Southern hemisphere DST tests (AEST, NZST)
   - [ ] Performance benchmarks vs existing solutions
   - [ ] Historical DST rule change tests

4. **Documentation (Phase 6.3 & 8)**
   - [ ] Create temporal scale examples
   - [ ] Document timezone configuration options
   - [ ] DST behavior documentation
   - [ ] Migration guide for users
   - [ ] Compare with Vega/D3 time scale behavior

### Low Priority - Optimization & Future Features
5. **Performance Optimization (Phase 7)**
   - [ ] DST transition caching
   - [ ] Batch operation optimization
   - [ ] Minimize timezone conversions

6. **Future Enhancements**
   - [ ] Support for Time32/Time64 arrays
   - [ ] Duration arrays
   - [ ] Interval arrays
   - [ ] Custom calendar systems
   - [ ] Business time calculations
   - [ ] Relative temporal operations

---

**Status**: Implementation In Progress  
**Completed**: Core TimeScale functionality with timezone support, nice intervals, tick generation, invert, and comprehensive DST support (Phases 1-5)
**Remaining**: Formatting/display, additional edge cases, testing, documentation, and optimization
**Risk Level**: Low (core functionality complete, remaining work is enhancement and polish)