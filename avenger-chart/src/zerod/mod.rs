//! Zero-dimensional coordinate system module
//!
//! This module provides a 0D coordinate system for contexts where marks need
//! to be displayed without spatial positioning, such as legends.

mod coord;
mod marks;

pub use coord::ZeroDCoord;
