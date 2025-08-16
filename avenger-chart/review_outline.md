# avenger-chart Code Review Outline

## Overview
This document provides a structured outline for reviewing the avenger-chart codebase, organized by module with file sizes and key responsibilities.

## Core Modules

### 1. lib.rs (24 lines)
- **Purpose**: Library entry point and module declarations
- **Review Focus**: Public API exports, module organization

### 2. error.rs (89 lines)
- **Purpose**: Error types and handling
- **Key Types**: `AvengerChartError` with various error variants
- **Recent Changes**: Added `ChannelResolutionError` variant

### 3. constants.rs (5 lines)
- **Purpose**: Global constants
- **Review Focus**: Naming conventions, documentation

## Chart Components

### 4. plot.rs (1039 lines)
- **Purpose**: Main Plot struct and builders
- **Key Types**: `Plot<C>`, `PlotTitle`, `PlotSubtitle`, `FacetSpec`
- **Recent Changes**: Added `ScaleDomainWithRadius` type alias
- **Review Focus**: 
  - Builder pattern consistency
  - Faceting implementation
  - Scale domain gathering methods

### 5. render.rs (2977 lines) ⚠️ **LARGE FILE**
- **Purpose**: Rendering pipeline from Plot to SceneGraph
- **Key Types**: `PlotRenderer`, `RenderResult`, `LegendCache`
- **Recent Changes**: Fixed clippy warnings for format! macros
- **Review Focus**:
  - File size - consider splitting
  - Legend creation methods (very long)
  - Taffy layout integration
  - Scale processing pipeline

### 6. chart_layout.rs (774 lines)
- **Purpose**: Layout computation using Taffy
- **Key Types**: `ChartLayout`, `LayoutResult`, `LayoutBounds`
- **Review Focus**: 
  - Taffy integration
  - Layout calculation logic
  - Legend positioning

## Coordinate Systems

### 7. coords.rs (149 lines)
- **Purpose**: Coordinate system traits and implementations
- **Key Types**: `CoordinateSystem` trait, `Cartesian`, `Polar`
- **Review Focus**: Trait design, extensibility

## Scales Module (Reorganized)

### 8. scales/mod.rs (20 lines)
- **Purpose**: Module declarations and public API exports
- **Exports**: `Scale`, `ScaleDomain`, `ScaleRange`, `ScaleRegistry`

### 9. scales/scale.rs (749 lines)
- **Purpose**: Core Scale struct and methods
- **Key Methods**: 
  - Domain/range builders
  - `infer_domain_from_data` (226 lines - needs refactoring)
  - `create_configured_scale` (138 lines - needs refactoring)
- **Review Focus**:
  - Long methods need breaking up
  - Builder pattern consistency
  - Separation of concerns

### 10. scales/domain.rs (195 lines)
- **Purpose**: Domain types and compilation
- **Key Types**: `ScaleDomain`, `ScaleDefaultDomain`, `DomainExpr`
- **Review Focus**: 
  - Circular dependency with marks::RadiusExpression
  - Domain inference logic

### 11. scales/range.rs (62 lines)
- **Purpose**: Range types and compilation
- **Key Types**: `ScaleRange` enum
- **Review Focus**: Type safety, color handling

### 12. scales/registry.rs (44 lines)
- **Purpose**: Scale collection management
- **Key Types**: `ScaleRegistry`
- **Review Focus**: API completeness

### 13. scales/factory.rs (77 lines)
- **Purpose**: Scale creation and defaults
- **Functions**: `create_scale_impl`, `apply_scale_defaults`
- **Review Focus**: Default values, scale type handling

### 14. scales/inference.rs (296 lines)
- **Purpose**: Scale type inference from data
- **Functions**: `infer_scale_type`, `infer_scale_type_with_mark`
- **Review Focus**: Inference rules, data type handling

### 15. scales/validation.rs (95 lines)
- **Purpose**: Scale validation utilities
- **Functions**: `expr_references_columns`
- **Clippy Issues**: Has unfixed warnings (not modified in session)

### 16. scales/udf.rs (149 lines)
- **Purpose**: User-defined functions for scale transformations
- **Review Focus**: DataFusion integration

### 17. scales/color_defaults.rs (231 lines)
- **Purpose**: Default color schemes
- **Review Focus**: Color palette choices

### 18. scales/shape_defaults.rs (8 lines)
- **Purpose**: Default shape definitions
- **Review Focus**: Shape variety

### 19. scales/dash_defaults.rs (15 lines)
- **Purpose**: Default dash patterns
- **Review Focus**: Pattern definitions

## Marks Module

### 20. marks/mod.rs (348 lines)
- **Purpose**: Mark trait and common types
- **Key Types**: `Mark` trait, `DataContext`, `ChannelValue`
- **Review Focus**: Trait design, channel handling

### 21. marks/channel.rs (218 lines)
- **Purpose**: Channel value handling
- **Key Types**: `ChannelValue`, `ChannelExpr` trait
- **Review Focus**: Expression handling, scaling

### 22. marks/symbol.rs (507 lines)
- **Purpose**: Symbol/scatter plot marks
- **Key Types**: `Symbol<C>`
- **Review Focus**: Rendering logic, channel support

### 23. marks/line.rs (471 lines)
- **Purpose**: Line chart marks
- **Key Types**: `Line<C>`
- **Review Focus**: Path generation, multi-series support

### 24. marks/rect.rs (379 lines)
- **Purpose**: Rectangle/bar marks
- **Key Types**: `Rect<C>`
- **Review Focus**: Bar chart logic, stacking

### 25. marks/area.rs (5 lines)
- **Purpose**: Area chart marks (stub)
- **Status**: Not implemented

### 26. marks/text.rs (5 lines)
- **Purpose**: Text marks (stub)
- **Status**: Not implemented

## Axis Module

### 27. axis.rs (491 lines)
- **Purpose**: Axis configuration and rendering
- **Key Types**: `CartesianAxis`, `AxisPosition`
- **Review Focus**: Configuration options, label formatting

## Legend Module

### 28. legend.rs (263 lines)
- **Purpose**: Legend configuration
- **Key Types**: `Legend`, `LegendPosition`
- **Review Focus**: Customization options

## Utilities

### 29. utils.rs (307 lines)
- **Purpose**: Helper functions and traits
- **Key Traits**: `DataFrameChartHelpers`, `ScalarValueHelpers`
- **Review Focus**: Utility organization, test coverage

### 30. channel_resolution.rs (820 lines)
- **Purpose**: Channel reference resolution system
- **Key Functions**: `resolve_all_channel_refs`
- **Recent Changes**: 
  - Removed fallback method
  - Added proper error handling
  - Optimized topological sort (O(V²) → O(V+E))
  - Integrated strsim for better error messages
- **Review Focus**: Algorithm correctness, error messages

## Statistics

- **Total Files**: 30 main source files
- **Large Files** (>500 lines):
  - render.rs (2977) ⚠️ Needs splitting
  - plot.rs (1039)
  - channel_resolution.rs (820)
  - chart_layout.rs (774)
  - scales/scale.rs (749)
  - symbol.rs (507)
  
- **Stub Files** (incomplete):
  - area.rs (5 lines)
  - text.rs (5 lines)

## Recommended Review Order

1. **Start with core abstractions**:
   - coords.rs (coordinate systems)
   - marks/mod.rs (Mark trait)
   - scales/mod.rs (Scale API)

2. **Then review implementations**:
   - marks/symbol.rs, line.rs, rect.rs
   - scales/scale.rs (but needs refactoring)
   - axis.rs, legend.rs

3. **Review integration layers**:
   - plot.rs (Plot builder)
   - channel_resolution.rs (channel system)
   - render.rs (rendering pipeline - needs splitting)

4. **Finally, review support code**:
   - utils.rs
   - error.rs
   - chart_layout.rs

## Key Issues to Address

1. **render.rs is too large** (2977 lines)
   - Should be split into multiple files
   - Legend creation methods are particularly long

2. **scales/scale.rs has overly complex methods**
   - `infer_domain_from_data` (226 lines)
   - `create_configured_scale` (138 lines)

3. **Incomplete implementations**
   - area.rs and text.rs are stubs

4. **Clippy warnings** in scales/validation.rs
   - Not fixed as file wasn't modified in session

5. **Circular dependencies**
   - Scale depends on marks::RadiusExpression

## Recent Improvements

1. **Channel resolution system** (channel_resolution.rs)
   - Proper error propagation with Result types
   - Performance optimization from O(V²) to O(V+E)
   - Better error messages with edit distance suggestions
   - Fixed cycle detection algorithm

2. **Scales module reorganization**
   - Split from single 1118-line file into 6 focused modules
   - Better separation of concerns
   - Clearer public API

3. **Code quality**
   - Fixed clippy warnings in modified files
   - Added type aliases for complex types
   - Improved documentation