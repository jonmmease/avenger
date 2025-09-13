//! Flexible guide system for coordinate systems
//!
//! This module provides a trait-based abstraction for visual guides in different
//! coordinate systems. Guides combine axes (configured at the channel level) with
//! coordinate-system-specific options (configured at the plot level).

use crate::error::AvengerChartError;
use crate::render::Padding;
use crate::theme::Theme;
use avenger_scenegraph::marks::mark::SceneMark;
use std::collections::HashMap;

/// Trait for visual guides in coordinate systems
///
/// A Guide represents the visual reference elements for a coordinate system.
/// This includes both axes (configured at the channel level) and coordinate-specific
/// options (configured at the plot level).
#[async_trait::async_trait]
pub trait Guide: Clone + Send + Sync + 'static {
    /// The axis type used by this guide (if any)
    type Axis: Clone + Send + Sync + 'static;

    /// Set axes that were configured at the channel level
    ///
    /// This is called during guide creation to apply all axis configurations
    /// from both plot-level and mark-level specifications.
    fn set_axes(&mut self, axes: HashMap<String, Self::Axis>);

    /// Get the configured axes
    ///
    /// Returns a reference to the map of channel names to axis configurations.
    fn axes(&self) -> &HashMap<String, Self::Axis>;

    /// Measure how much space this guide needs outside the plot area
    async fn measure_overflow(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        theme: &Theme,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError>;

    /// Render this guide to scene marks
    async fn render(
        &self,
        scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        plot_width: f32,
        plot_height: f32,
        padding: &Padding,
        theme: &Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;
}

/// Space requirements for guide overflow beyond plot area
#[derive(Debug, Clone, Default)]
pub struct OverflowSpaceRequirement {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}

/// Empty guide for coordinate systems without visual guides
#[derive(Clone, Debug, Default)]
pub struct NoGuide {
    // Store an empty map directly in the struct
    axes: HashMap<String, ()>,
}

#[async_trait::async_trait]
impl Guide for NoGuide {
    type Axis = ();

    fn set_axes(&mut self, _axes: HashMap<String, Self::Axis>) {
        // No-op for systems without axes
    }

    fn axes(&self) -> &HashMap<String, Self::Axis> {
        // Return reference to our empty map
        &self.axes
    }

    async fn measure_overflow(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _theme: &Theme,
    ) -> Result<OverflowSpaceRequirement, AvengerChartError> {
        Ok(OverflowSpaceRequirement::default())
    }

    async fn render(
        &self,
        _scales: &HashMap<String, avenger_scales::scales::ConfiguredScale>,
        _plot_width: f32,
        _plot_height: f32,
        _padding: &Padding,
        _theme: &Theme,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        Ok(Vec::new())
    }
}
