# Avenger Project Overview

## Purpose
Avenger is a Rust-based visualization engine and renderer designed for information visualization (InfoVis) systems. It provides:
- GPU-accelerated rendering via WebGPU/WebGL2
- WebAssembly support for browser deployment
- A 2D scene graph representation tailored for InfoVis systems
- Visualization primitives (scales, guides, legends, axes)
- Interactive event handling inspired by Vega

## Key Goals
- Serve as a foundational rendering library for data visualization
- Enable high-performance GPU rendering
- Support both native and WASM deployment
- Provide compatibility with Vega specifications

## Tech Stack

### Core Technologies
- **Language**: Rust (stable toolchain)
- **GPU Rendering**: wgpu (cross-platform WebGPU implementation)
- **Geometry Processing**: Lyon (tessellation), rstar (spatial indexing)
- **Text Rendering**: cosmic-text (cross-platform text support)
- **Image Processing**: image crate (PNG/JPEG support)
- **Data Handling**: Apache Arrow, DataFusion
- **Build System**: Cargo workspace with pixi for Python integration

### Key Dependencies
- wgpu 25.0.2
- lyon 1.0.1
- cosmic-text 0.14.2
- arrow/datafusion 48.0.1
- winit 0.30.11 (window management)
- palette 0.7.6 (color handling)
- serde/serde_json (serialization)

### Python Integration
- maturin for Python bindings
- pixi for environment management
- Python 3.12

## Repository Type
Cargo workspace with multiple crates organized by functionality