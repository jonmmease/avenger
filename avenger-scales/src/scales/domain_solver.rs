/*!
# Scatter Plot Domain Optimization Solver

This module solves the optimal domain problem for scatter plots with marker size constraints.

## Problem Statement

Given a set of data points with associated marker sizes, find the minimal domain [d_min, d_max]
such that when the points are mapped to screen coordinates using a linear scale, all markers
fit within the screen range without clipping.

## Mathematical Formulation

### Input:
- Points: d₁, d₂, ..., dₙ (data values)
- Marker sizes: s₁, s₂, ..., sₙ (screen units)
- Screen width: R (screen units)

### Variables:
- d_min: minimum of the data domain
- d_max: maximum of the data domain

### Objective:
Minimize (d_max - d_min)

### Constraints:

1. **Domain validity**: d_max > d_min

2. **Data containment**: ∀i: d_min ≤ d_i ≤ d_max

3. **Marker fitting**: Each marker must fit within the screen when mapped

   The linear scale maps [d_min, d_max] → [0, R]

   For point d_i with size s_i:
   - Left edge of marker: position - s_i/2 ≥ 0
   - Right edge of marker: position + s_i/2 ≤ R

   Where position = (d_i - d_min) / (d_max - d_min) × R

   This gives us the bilinear constraints:
   - Left: (d_i - d_min) / (d_max - d_min) ≥ s_i / R
   - Right: (d_max - d_i) / (d_max - d_min) ≥ s_i / R

## Solution Approach

### Key Insight: Active Constraints

At the optimal solution, at least one marker will have its edge exactly at a screen boundary.
This means we can find the optimal solution by enumerating combinations of "active" constraints.

### Dominance-Based Filtering

Before enumeration, we can dramatically reduce the search space using dominance relationships:

**Left Support Dominance**: Point j is dominated by point i as a left support if:
- i is to the left of j (d_i < d_j)
- i has smaller or equal radius (s_i ≤ s_j)

**Right Support Dominance**: Point j is dominated by point i as a right support if:
- i is to the right of j (d_i > d_j)
- i has smaller or equal radius (s_i ≤ s_j)

Dominated points can be filtered out as they cannot be optimal support points.

### Algorithm:

1. **Filter candidates**: Identify non-dominated points for left and right supports
2. **Enumerate**: Try all combinations of (left_support, right_support) from filtered sets
3. **Solve**: For each pair, solve the 2×2 linear system
4. **Validate**: Check feasibility and track the minimum domain width

### Complexity:

- Without filtering: O(n²) combinations to check
- With filtering: O(k₁ × k₂) where k₁, k₂ << n are the filtered set sizes
- For many real-world cases (e.g., uniform sizes), this reduces to O(1)

## Example Usage

```rust
use avenger_scales::scales::domain_solver::compute_domain_from_data_with_padding_linear;

let points = vec![5.0, 20.0, 40.0, 50.0, 60.0, 80.0, 95.0];
let sizes = vec![3.0, 50.0, 70.0, 80.0, 70.0, 50.0, 3.0];
let screen_width = 200.0;

let solution = compute_domain_from_data_with_padding_linear(&points, &sizes, screen_width)?;
println!("Optimal domain: [{}, {}]", solution.0, solution.1);
```
*/

use crate::error::AvengerScaleError;
use std::fmt;

/// Solution to the scatter plot domain optimization problem
#[derive(Debug, Clone, Copy)]
pub struct DomainSolution {
    /// Minimum value of the domain
    pub d_min: f64,
    /// Maximum value of the domain
    pub d_max: f64,
}

impl DomainSolution {
    /// Get the width of the domain
    pub fn width(&self) -> f64 {
        self.d_max - self.d_min
    }
}

impl fmt::Display for DomainSolution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{:.6}, {:.6}]", self.d_min, self.d_max)
    }
}

/// Error type for domain solver
#[derive(Debug, Clone)]
pub enum DomainError {
    /// Problem is infeasible (e.g., marker too large for screen)
    Infeasible(String),
    /// Invalid input data
    InvalidInput(String),
    /// No solution found
    NoSolution,
}

impl fmt::Display for DomainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DomainError::Infeasible(msg) => write!(f, "Infeasible: {}", msg),
            DomainError::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            DomainError::NoSolution => write!(f, "No solution found"),
        }
    }
}

impl std::error::Error for DomainError {}

/// Solve the scatter plot domain optimization problem
///
/// # Arguments
/// * `domain_points` - Data values for each point
/// * `range_sizes` - Marker sizes in screen units for each point
/// * `range` - Width of the screen in screen units
///
/// # Returns
/// * `Ok(DomainSolution)` - The optimal domain [d_min, d_max]
/// * `Err(DomainError)` - If the problem is infeasible or invalid
pub fn compute_domain_from_data_with_padding_linear(
    domain_points: &[f64],
    range_sizes: &[f64],
    range: f64,
) -> Result<(f64, f64), AvengerScaleError> {
    // Validate inputs
    if domain_points.len() != range_sizes.len() {
        return Err(DomainError::InvalidInput(
            "Points and sizes must have the same length".to_string(),
        )
        .into());
    }

    if range <= 0.0 {
        return Err(DomainError::InvalidInput("Screen width must be positive".to_string()).into());
    }

    // Handle empty case
    if domain_points.is_empty() {
        return Ok((0.0, 1.0));
    }

    // Handle single point case
    if domain_points.len() == 1 {
        return solve_single_point(domain_points[0], range_sizes[0], range);
    }

    // Get filtered candidates
    let left_candidates = filter_left_supports(domain_points, range_sizes);
    let right_candidates = filter_right_supports(domain_points, range_sizes);

    // Try all combinations of filtered candidates
    let mut best_solution = None;
    let mut best_width = f64::INFINITY;

    let mut _attempted = 0;
    let mut _solutions_found = 0;
    let mut _feasible_found = 0;

    for &left_idx in &left_candidates {
        for &right_idx in &right_candidates {
            // Skip invalid combinations
            if domain_points[left_idx] >= domain_points[right_idx] {
                continue;
            }

            _attempted += 1;

            // Solve for this pair of active constraints
            if let Some(solution) =
                solve_for_active_pair(domain_points, range_sizes, range, left_idx, right_idx)
            {
                _solutions_found += 1;
                let width = solution.1 - solution.0;
                if is_feasible(domain_points, range_sizes, range, solution) {
                    _feasible_found += 1;
                    if width < best_width {
                        best_width = width;
                        best_solution = Some(solution);
                    }
                }
            }
        }
    }

    best_solution.ok_or(AvengerScaleError::DomainFromPaddingError(
        DomainError::NoSolution,
    ))
}

/// Solve for a single point
fn solve_single_point(d: f64, s: f64, r: f64) -> Result<(f64, f64), AvengerScaleError> {
    // For a single point, we need:
    // s/2 ≤ position ≤ R - s/2
    // where position = (d - d_min) / (d_max - d_min) × R

    // This requires: s ≤ R - s, or s ≤ R/2
    if s > r {
        return Err(DomainError::Infeasible(format!(
            "Marker size {} exceeds screen width {}",
            s, r
        ))
        .into());
    }

    // The optimal domain has the point centered
    let factor = s / (r - 2.0 * s);
    if factor < 0.0 || !factor.is_finite() {
        return Err(DomainError::Infeasible("Marker too large for screen".to_string()).into());
    }

    let half_width = d * factor;

    Ok((d - half_width, d + half_width))
}

/// Filter points to find potential left support points
/// A point can be a left support only if it has larger radius than all points to its left
fn filter_left_supports(points: &[f64], sizes: &[f64]) -> Vec<usize> {
    let n = points.len();
    if n == 0 {
        return vec![];
    }

    // Create sorted indices by position
    let mut indices: Vec<usize> = (0..n).collect();
    indices.sort_by(|&i, &j| points[i].partial_cmp(&points[j]).unwrap());

    let mut candidates = Vec::new();
    let mut max_radius_so_far = 0.0;

    // Scan from left to right
    for &idx in &indices {
        let radius = sizes[idx];
        // Keep point if it has larger radius than all points to its left
        if radius > max_radius_so_far || candidates.is_empty() {
            candidates.push(idx);
            max_radius_so_far = radius;
        }
    }

    candidates
}

/// Filter points to find potential right support points
/// A point can be a right support only if it has larger radius than all points to its right
fn filter_right_supports(points: &[f64], sizes: &[f64]) -> Vec<usize> {
    let n = points.len();
    if n == 0 {
        return vec![];
    }

    // Create sorted indices by position
    let mut indices: Vec<usize> = (0..n).collect();
    indices.sort_by(|&i, &j| points[i].partial_cmp(&points[j]).unwrap());

    let mut candidates = Vec::new();
    let mut max_radius_so_far = 0.0;

    // Scan from right to left
    for &idx in indices.iter().rev() {
        let radius = sizes[idx];
        // Keep point if it has larger radius than all points to its right
        if radius > max_radius_so_far || candidates.is_empty() {
            candidates.push(idx);
            max_radius_so_far = radius;
        }
    }

    candidates.reverse(); // Maintain left-to-right order
    candidates
}

/// Solve assuming specific left and right constraints are active
fn solve_for_active_pair(
    points: &[f64],
    sizes: &[f64],
    screen_width: f64,
    left_idx: usize,
    right_idx: usize,
) -> Option<(f64, f64)> {
    let d_left = points[left_idx];
    let s_left = sizes[left_idx];
    let d_right = points[right_idx];
    let s_right = sizes[right_idx];
    let r = screen_width;

    // System of equations:
    // (d_left - d_min) / (d_max - d_min) = s_left / r
    // (d_max - d_right) / (d_max - d_min) = s_right / r

    // Let w = d_max - d_min (domain width)
    // Then: d_left - d_min = s_left * w / r
    //       d_max - d_right = s_right * w / r

    // From first equation: d_min = d_left - s_left * w / r
    // From second equation: d_max = d_right + s_right * w / r

    // Since w = d_max - d_min:
    // w = (d_right + s_right * w / r) - (d_left - s_left * w / r)
    // w = d_right - d_left + (s_right + s_left) * w / r
    // w * (1 - (s_right + s_left) / r) = d_right - d_left

    let sum_sizes = s_left + s_right;
    if sum_sizes >= r {
        return None; // Infeasible
    }

    let w = (d_right - d_left) * r / (r - sum_sizes);
    let d_min = d_left - s_left * w / r;
    let d_max = d_right + s_right * w / r;

    Some((d_min, d_max))
}

/// Check if a solution satisfies all constraints
fn is_feasible(points: &[f64], sizes: &[f64], screen_width: f64, solution: (f64, f64)) -> bool {
    if solution.1 <= solution.0 {
        return false;
    }

    let (d_min, d_max) = solution;
    let domain_width = d_max - d_min;

    for i in 0..points.len() {
        let d_i = points[i];
        let radius = sizes[i]; // This is already the radius

        // Check data containment
        if d_i < d_min || d_i > d_max {
            return false;
        }

        // Calculate actual screen position and edges
        let screen_pos = (d_i - d_min) / domain_width * screen_width;
        let left_edge = screen_pos - radius;
        let right_edge = screen_pos + radius;

        // Check if the point fits within screen bounds
        if left_edge < -1e-10 || right_edge > screen_width + 1e-10 {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_point() {
        let points = vec![50.0];
        let sizes = vec![20.0];
        let screen_width = 100.0;

        let result =
            compute_domain_from_data_with_padding_linear(&points, &sizes, screen_width).unwrap();
        assert!((result.0 - 33.3333).abs() < 0.01);
        assert!((result.1 - 66.6667).abs() < 0.01);
    }

    #[test]
    fn test_standard_case() {
        let points = vec![5.0, 20.0, 40.0, 50.0, 60.0, 80.0, 95.0];
        let sizes = vec![3.0, 50.0, 70.0, 80.0, 70.0, 50.0, 3.0];
        let screen_width = 200.0;

        let result =
            compute_domain_from_data_with_padding_linear(&points, &sizes, screen_width).unwrap();

        // The algorithm found a valid solution - test that it's feasible
        let is_valid = is_feasible(&points, &sizes, screen_width, result);
        assert!(is_valid, "Solution should be feasible");
    }

    #[test]
    fn test_uniform_sizes() {
        let points: Vec<f64> = (0..20).map(|i| i as f64 * 5.0).collect();
        let sizes = vec![5.0; 20];
        let screen_width = 200.0;

        // With uniform sizes, only endpoints matter
        let result =
            compute_domain_from_data_with_padding_linear(&points, &sizes, screen_width).unwrap();

        // Verify the solution is valid
        assert!(is_feasible(&points, &sizes, screen_width, result));
    }

    #[test]
    fn test_infeasible() {
        let points = vec![50.0];
        let sizes = vec![150.0]; // Marker larger than screen
        let screen_width = 100.0;

        let result = compute_domain_from_data_with_padding_linear(&points, &sizes, screen_width);
        assert!(matches!(
            result,
            Err(AvengerScaleError::DomainFromPaddingError(
                DomainError::Infeasible(_)
            ))
        ));
    }

    #[test]
    fn test_decreasing_sizes() {
        let points: Vec<f64> = (0..10).map(|i| i as f64 * 10.0).collect();
        let sizes: Vec<f64> = (0..10).map(|i| 20.0 - i as f64 * 2.0).collect();
        let screen_width = 200.0;

        let result =
            compute_domain_from_data_with_padding_linear(&points, &sizes, screen_width).unwrap();

        // With decreasing sizes, leftmost point dominates for left support
        // and rightmost point dominates for right support
        assert!(is_feasible(&points, &sizes, screen_width, result));
    }

    #[test]
    fn test_large_radius_second_to_last_visual_test_data() {
        // Exact data from the visual test
        let y_values = vec![2.0, 3.5, 2.8, 4.2, 5.1, 4.8, 6.2, 5.5];
        let size_values: [f64; 8] = [50.0, 100.0, 75.0, 150.0, 200.0, 125.0, 175.0, 30625.0];

        // Convert area to radius: radius = sqrt(area) * 0.5
        let radii: Vec<f64> = size_values.iter().map(|&area| area.sqrt() * 0.5).collect();

        // Visual test uses 600x400 preferred size, but we need the actual range width
        // For a 600x400 plot with default margins, the actual plotting area is smaller
        // Default margins in avenger: left=60, right=20, top=20, bottom=60
        // So actual width = 600 - 60 - 20 = 520
        let screen_width = 520.0;

        let result = compute_domain_from_data_with_padding_linear(&y_values, &radii, screen_width);

        match result {
            Ok((d_min, d_max)) => {
                // Verify all points fit within screen bounds
                for (i, (y, r)) in y_values.iter().zip(radii.iter()).enumerate() {
                    let screen_pos = (y - d_min) / (d_max - d_min) * screen_width;
                    let left_edge = screen_pos - r;
                    let right_edge = screen_pos + r;
                    assert!(
                        left_edge >= -0.01 && right_edge <= screen_width + 0.01,
                        "Point {} should fit within screen bounds",
                        i
                    );
                }
            }
            Err(e) => {
                panic!("Expected a solution but got error: {:?}", e);
            }
        }
    }

    #[test]
    fn test_large_radius_second_to_last() {
        // Test case where second-to-last point has very large radius
        let points = vec![2.0, 3.5, 2.8, 4.2, 5.1, 4.8, 6.2, 5.5];
        let sizes = vec![3.54, 7.07, 6.12, 12.25, 14.14, 11.18, 13.23, 87.5]; // radii
        let screen_width = 300.0; // Increased to accommodate large radius

        let result = compute_domain_from_data_with_padding_linear(&points, &sizes, screen_width);

        match result {
            Ok((d_min, d_max)) => {
                // Just verify the solution is feasible
                assert!(
                    is_feasible(&points, &sizes, screen_width, (d_min, d_max)),
                    "Solution should be feasible"
                );
            }
            Err(_) => panic!("Should find a solution"),
        }
    }
}
