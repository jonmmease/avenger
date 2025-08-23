# avenger-chart Code Review Checklist

## Core Modules
- [ ] **lib.rs** (24 lines) - Library entry point and module declarations
- [ ] **error.rs** (89 lines) - Error types and handling
- [ ] **constants.rs** (5 lines) - Global constants

## Chart Components
- [ ] **plot.rs** (1039 lines) - Main Plot struct and builders
- [ ] **render.rs** (2977 lines) ⚠️ **LARGE FILE - needs splitting**
- [ ] **chart_layout.rs** (774 lines) - Layout computation using Taffy

## Coordinate Systems
- [ ] **coords.rs** (149 lines) - Coordinate system traits and implementations
- [ ] **cartesian.rs** - Cartesian coordinate implementation
- [ ] **polar.rs** - Polar coordinate implementation

## Scales Module
- [ ] **scales/mod.rs** (20 lines) - Module declarations and public API exports
- [ ] **scales/scale.rs** (749 lines) - Core Scale struct and methods
- [ ] **scales/spec.rs** - ScaleSpec trait and marker types
- [ ] **scales/domain.rs** (195 lines) - Domain types and compilation
- [ ] **scales/range.rs** (62 lines) - Range types and compilation
- [ ] **scales/registry.rs** (44 lines) - Scale collection management
- [ ] **scales/factory.rs** (77 lines) - Scale creation and defaults
- [ ] **scales/inference.rs** (296 lines) - Scale type inference from data
- [ ] **scales/validation.rs** (95 lines) - Scale validation utilities ⚠️ **Has clippy warnings**
- [ ] **scales/udf.rs** (149 lines) - User-defined functions for scale transformations
- [ ] **scales/color_defaults.rs** (231 lines) - Default color schemes
- [ ] **scales/shape_defaults.rs** (8 lines) - Default shape definitions
- [ ] **scales/dash_defaults.rs** (15 lines) - Default dash patterns

## Marks Module
- [ ] **marks/mod.rs** (348 lines) - Mark trait and common types
- [ ] **marks/channel.rs** (218 lines) - Channel value handling
- [ ] **marks/symbol.rs** (507 lines) - Symbol/scatter plot marks
- [ ] **marks/line.rs** (471 lines) - Line chart marks
- [ ] **marks/rect.rs** (379 lines) - Rectangle/bar marks
- [ ] **marks/area.rs** (5 lines) ⚠️ **STUB - not implemented**
- [ ] **marks/text.rs** (5 lines) ⚠️ **STUB - not implemented**
- [ ] **marks/macros.rs** - Mark implementation macros

## Axis Module
- [ ] **axis.rs** (491 lines) - Axis configuration and rendering

## Legend Module
- [ ] **legend.rs** (263 lines) - Legend configuration

## Utilities
- [ ] **utils.rs** (307 lines) - Helper functions and traits
- [ ] **channel_resolution.rs** (820 lines) - Channel reference resolution system

## Test Files
- [ ] **tests/test_scales.rs** - Scale tests
- [ ] **tests/test_marks.rs** - Mark tests
- [ ] **tests/test_channel_resolution.rs** - Channel resolution tests
- [ ] **tests/test_plot.rs** - Plot integration tests

## Examples
- [ ] **examples/type_safe_scales.rs** - Type-safe scale API examples
- [ ] **examples/basic_plot.rs** - Basic plotting examples (if exists)

## Issues to Track

### High Priority
- [ ] Split **render.rs** (2977 lines) into multiple files
- [ ] Refactor long methods in **scales/scale.rs**:
  - [ ] `infer_domain_from_data` (226 lines)
  - [ ] `create_configured_scale` (138 lines)
- [ ] Fix clippy warnings in **scales/validation.rs**

### Medium Priority
- [ ] Implement **marks/area.rs** (currently stub)
- [ ] Implement **marks/text.rs** (currently stub)
- [ ] Address circular dependency between Scale and marks::RadiusExpression

### Low Priority
- [ ] Add more documentation to public APIs
- [ ] Increase test coverage for edge cases
- [ ] Consider adding benchmarks for performance-critical paths

## Review Progress

### Completed Reviews
- [ ] Core abstractions (coords, marks trait, scales API)
- [ ] Mark implementations (symbol, line, rect)
- [ ] Scale system and type safety
- [ ] Channel resolution system
- [ ] Plot builder and faceting

### Pending Reviews
- [ ] Rendering pipeline details
- [ ] Layout computation
- [ ] Legend and axis generation
- [ ] Error handling consistency
- [ ] Public API completeness

## Notes
- Total files: ~35 source files
- Lines of code: ~10,000 lines
- Recent refactoring: Scales module split from 1118 lines to 6 focused modules
- External test crate: avenger-chart-external-test demonstrates extension points