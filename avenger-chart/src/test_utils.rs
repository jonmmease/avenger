/// Test utilities for debugging and visualization
#[cfg(test)]
pub mod layout_debug {
    use crate::chart_layout::LayoutResult;
    use std::fmt::Write;

    /// Generate an SVG visualization of the layout result
    pub fn layout_to_svg(layout: &LayoutResult, width: f32, height: f32) -> String {
        let mut svg = String::new();

        // SVG header
        writeln!(&mut svg, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">", 
                 width, height, width, height).unwrap();

        // Add a white background
        writeln!(&mut svg, "  <rect x=\"0\" y=\"0\" width=\"{}\" height=\"{}\" fill=\"white\" stroke=\"black\" stroke-width=\"1\"/>",
                 width, height).unwrap();

        // Draw plot area in light gray
        writeln!(&mut svg, "  <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#f0f0f0\" stroke=\"blue\" stroke-width=\"2\"/>",
                 layout.plot_area.x, layout.plot_area.y,
                 layout.plot_area.width, layout.plot_area.height).unwrap();
        writeln!(
            &mut svg,
            "  <text x=\"{}\" y=\"{}\" font-size=\"12\" fill=\"blue\">Plot Area</text>",
            layout.plot_area.x + 5.0,
            layout.plot_area.y + 15.0
        )
        .unwrap();

        // Draw axes in light green
        for (position, bounds) in &layout.axes {
            writeln!(&mut svg, "  <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#e0ffe0\" stroke=\"green\" stroke-width=\"1\" opacity=\"0.7\"/>",
                     bounds.x, bounds.y, bounds.width, bounds.height).unwrap();
            writeln!(
                &mut svg,
                "  <text x=\"{}\" y=\"{}\" font-size=\"10\" fill=\"green\">Axis {:?}</text>",
                bounds.x + 2.0,
                bounds.y + 10.0,
                position
            )
            .unwrap();
        }

        // Draw legends in light red with dimensions
        for (channel, bounds) in &layout.legends {
            writeln!(&mut svg, "  <rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"#ffe0e0\" stroke=\"red\" stroke-width=\"1\" opacity=\"0.7\"/>",
                     bounds.x, bounds.y, bounds.width, bounds.height).unwrap();
            writeln!(
                &mut svg,
                "  <text x=\"{}\" y=\"{}\" font-size=\"10\" fill=\"red\">Legend: {}</text>",
                bounds.x + 2.0,
                bounds.y + 10.0,
                channel
            )
            .unwrap();
            writeln!(
                &mut svg,
                "  <text x=\"{}\" y=\"{}\" font-size=\"8\" fill=\"darkred\">{}x{}</text>",
                bounds.x + 2.0,
                bounds.y + 20.0,
                bounds.width as i32,
                bounds.height as i32
            )
            .unwrap();
        }

        // Add grid lines to show alignment
        // Vertical lines at key x positions
        let mut x_positions = vec![0.0, width];
        x_positions.push(layout.plot_area.x);
        x_positions.push(layout.plot_area.x + layout.plot_area.width);
        for bounds in layout.axes.values() {
            x_positions.push(bounds.x);
            x_positions.push(bounds.x + bounds.width);
        }
        for bounds in layout.legends.values() {
            x_positions.push(bounds.x);
            x_positions.push(bounds.x + bounds.width);
        }
        x_positions.sort_by(|a, b| a.partial_cmp(b).unwrap());
        x_positions.dedup();

        for x in x_positions {
            writeln!(&mut svg, "  <line x1=\"{}\" y1=\"0\" x2=\"{}\" y2=\"{}\" stroke=\"gray\" stroke-width=\"0.5\" opacity=\"0.3\"/>",
                     x, x, height).unwrap();
            writeln!(
                &mut svg,
                "  <text x=\"{}\" y=\"{}\" font-size=\"8\" fill=\"gray\">{}</text>",
                x + 1.0,
                height - 2.0,
                x as i32
            )
            .unwrap();
        }

        // Horizontal lines at key y positions
        let mut y_positions = vec![0.0, height];
        y_positions.push(layout.plot_area.y);
        y_positions.push(layout.plot_area.y + layout.plot_area.height);
        for bounds in layout.axes.values() {
            y_positions.push(bounds.y);
            y_positions.push(bounds.y + bounds.height);
        }
        for bounds in layout.legends.values() {
            y_positions.push(bounds.y);
            y_positions.push(bounds.y + bounds.height);
        }
        y_positions.sort_by(|a, b| a.partial_cmp(b).unwrap());
        y_positions.dedup();

        for y in y_positions {
            writeln!(&mut svg, "  <line x1=\"0\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"gray\" stroke-width=\"0.5\" opacity=\"0.3\"/>",
                     y, width, y).unwrap();
            writeln!(
                &mut svg,
                "  <text x=\"2\" y=\"{}\" font-size=\"8\" fill=\"gray\">{}</text>",
                y - 2.0,
                y as i32
            )
            .unwrap();
        }

        // SVG footer
        writeln!(&mut svg, "</svg>").unwrap();

        svg
    }

    /// Save layout as SVG file
    pub fn save_layout_svg(
        layout: &LayoutResult,
        width: f32,
        height: f32,
        path: &str,
    ) -> std::io::Result<()> {
        use std::fs;
        let svg = layout_to_svg(layout, width, height);
        fs::write(path, svg)?;
        eprintln!("Layout debug SVG saved to: {}", path);
        Ok(())
    }
}
