#[cfg(test)]
mod tests {
    use super::*;
    use crate::typst_eval::math::parse_math;

    type LineSegment = ((f32, f32), (f32, f32));

    fn path_y_extent(item: &PathItem) -> f32 {
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for command in &item.path.commands {
            let mut include = |y: f32| {
                let y = y + item.transform.ty;
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            };
            match *command {
                PathCommand::MoveTo { y, .. } | PathCommand::LineTo { y, .. } => include(y),
                PathCommand::QuadTo { y1, y, .. } => {
                    include(y1);
                    include(y);
                }
                PathCommand::CubicTo { y1, y2, y, .. } => {
                    include(y1);
                    include(y2);
                    include(y);
                }
                PathCommand::Close => {}
            }
        }
        if min_y.is_finite() && max_y.is_finite() {
            max_y - min_y
        } else {
            0.0
        }
    }

    fn cancel_shape_lines(source: &str, options: &MathLayoutOptions) -> Vec<LineSegment> {
        let math = parse_math(source, 0).unwrap();
        let artifact = try_typeset_simple_row_fragment(&math, options, &EngineOptions::default())
            .unwrap()
            .unwrap_or_else(|| panic!("cancel call should be handled: {source}"));
        artifact
            .paths
            .items
            .into_iter()
            .filter(|item| matches!(item.kind, PathKind::MathShape))
            .filter_map(|item| match &item.path.commands[..] {
                [
                    PathCommand::MoveTo { x: x0, y: y0 },
                    PathCommand::LineTo { x: x1, y: y1 },
                ] => Some((
                    (x0 + item.transform.tx, y0 + item.transform.ty),
                    (x1 + item.transform.tx, y1 + item.transform.ty),
                )),
                _ => None,
            })
            .collect()
    }

    fn line_delta(line: LineSegment) -> (f32, f32) {
        (line.1.0 - line.0.0, line.1.1 - line.0.1)
    }

    fn line_length(line: LineSegment) -> f32 {
        let (dx, dy) = line_delta(line);
        dx.hypot(dy)
    }

    #[test]
    #[cfg(not(feature = "raster"))]
    fn atom_fragment_layout_is_available_without_raster_feature() {
        let math = parse_math("1", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple atom should be handled without raster support");
        assert_eq!(artifact.pdf_text.glyph_runs.len(), 1);
        assert!(!artifact.paths.items.is_empty());
    }

    #[test]
    fn atom_fragment_can_emit_pdf_glyph_metadata() {
        let math = parse_math("1", 0).unwrap();
        let options = MathLayoutOptions::default();

        assert!(
            try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
                .unwrap()
                .is_some_and(|artifact| artifact.pdf_text.glyph_runs.len() == 1
                    && artifact.font_resources.len() == 1)
        );
    }

    #[cfg(feature = "raster")]
    #[test]
    fn atom_fragment_emits_frame_ready_paths_for_rasterization() {
        let math = parse_math("alpha + beta -> gamma", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple row should be handled by Typst paths");

        assert!(!artifact.paths.items.is_empty());
    }

    #[test]
    fn simple_row_can_emit_script_glyph_metadata() {
        let math = parse_math("x^2", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple superscript should be handled by Typst row path");
        let pdf = artifact.pdf_text;
        assert_eq!(pdf.glyph_runs.len(), 2);
        assert!(pdf.glyph_runs[1].font_size < pdf.glyph_runs[0].font_size);
    }

    #[test]
    fn simple_row_can_force_limits_and_scripts() {
        let config = EngineOptions::default();
        let font =
            load_default_math_font(&config, &MathFontSpec::LeteSansMath, &FontWeight::Normal)
                .expect("default math font should load");
        let font_size = 20.0;
        let limits = parse_math("limits(A)_1^2", 0).unwrap();
        let scripts = parse_math("scripts(A)_1^2", 0).unwrap();
        let inline_display_only = parse_math("limits(A, inline: #false)_1^2", 0).unwrap();
        let display_limits = parse_math("display(limits(A, inline: #false))_1^2", 0).unwrap();

        let limits_metrics = layout_simple_row(&font, &limits, font_size)
            .unwrap()
            .expect("limits row should layout")
            .metrics;
        let scripts_metrics = layout_simple_row(&font, &scripts, font_size)
            .unwrap()
            .expect("scripts row should layout")
            .metrics;
        let inline_display_only_metrics = layout_simple_row(&font, &inline_display_only, font_size)
            .unwrap()
            .expect("inline display-only limits row should layout")
            .metrics;
        let display_limits_metrics = layout_simple_row(&font, &display_limits, font_size)
            .unwrap()
            .expect("explicit display-only limits row should layout")
            .metrics;

        assert!(
            limits_metrics.height > scripts_metrics.height + font_size * 0.4,
            "forced limits should create a taller inline label box"
        );
        assert!(
            limits_metrics.width < scripts_metrics.width,
            "forced limits should center top/bottom attachments instead of widening side scripts"
        );
        assert!(
            (inline_display_only_metrics.width - scripts_metrics.width).abs() < font_size * 0.05,
            "display-only limits should use side scripts in inline math context"
        );
        assert!(
            display_limits_metrics.height > scripts_metrics.height + font_size * 0.4,
            "display-only limits should stack in explicit display math context"
        );
        assert!(
            display_limits_metrics.width < scripts_metrics.width,
            "display-only limits should center attachments in explicit display math context"
        );
    }

    #[test]
    fn simple_row_display_large_operator_defaults_to_limits() {
        let config = EngineOptions::default();
        let font =
            load_default_math_font(&config, &MathFontSpec::LeteSansMath, &FontWeight::Normal)
                .expect("default math font should load");
        let font_size = 20.0;
        let metrics = |source: &str| {
            let math = parse_math(source, 0).unwrap();
            layout_simple_row(&font, &math, font_size)
                .unwrap()
                .unwrap_or_else(|| panic!("{source} should layout"))
                .metrics
        };

        let inline_sum = metrics("sum_(i=0)^n");
        let display_sum = metrics("display(sum_(i=0)^n)");
        let forced_sum_scripts = metrics("scripts(display(sum))_(i=0)^n");
        let inline_integral = metrics("integral_a^b");
        let display_integral = metrics("display(integral_a^b)");
        let forced_integral_limits = metrics("display(limits(integral))_a^b");

        assert!(
            display_sum.height > inline_sum.height + font_size * 0.35,
            "display-style sum should stack limits by default"
        );
        assert!(
            display_sum.width < inline_sum.width,
            "display-style sum limits should avoid side-script widening"
        );
        assert!(
            forced_sum_scripts.width > display_sum.width + font_size * 0.2,
            "scripts(...) should force side scripts even around display-style sum"
        );
        assert!(
            display_integral.width > forced_integral_limits.width + font_size * 0.1,
            "display-style integral should keep side scripts by default"
        );
        assert!(
            display_integral.height > inline_integral.height,
            "display-style integral should still use a display-sized glyph"
        );
    }

    #[test]
    fn simple_row_relation_class_defaults_to_limits() {
        let config = EngineOptions::default();
        let font =
            load_default_math_font(&config, &MathFontSpec::LeteSansMath, &FontWeight::Normal)
                .expect("default math font should load");
        let font_size = 20.0;
        let relation_limits = parse_math("class(\"relation\", x)_a^b", 0).unwrap();
        let forced_scripts = parse_math("scripts(class(\"relation\", x))_a^b", 0).unwrap();
        let normal_scripts = parse_math("class(\"normal\", x)_a^b", 0).unwrap();

        let relation_metrics = layout_simple_row(&font, &relation_limits, font_size)
            .unwrap()
            .expect("relation class row should layout")
            .metrics;
        let forced_scripts_metrics = layout_simple_row(&font, &forced_scripts, font_size)
            .unwrap()
            .expect("forced scripts row should layout")
            .metrics;
        let normal_metrics = layout_simple_row(&font, &normal_scripts, font_size)
            .unwrap()
            .expect("normal class row should layout")
            .metrics;

        assert!(
            relation_metrics.height > forced_scripts_metrics.height + font_size * 0.25,
            "relation class should place top/bottom attachments as centered limits by default"
        );
        assert!(
            relation_metrics.width < forced_scripts_metrics.width,
            "relation class limits should avoid widening the row with a side script"
        );
        assert!(
            (normal_metrics.width - forced_scripts_metrics.width).abs() < font_size * 0.05,
            "normal class should keep ordinary side script placement"
        );
    }

    #[test]
    fn simple_row_can_emit_prime_glyphs_as_scripts() {
        let math = parse_math("a'''_b", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("prime attachment should be handled by Typst row path");
        let pdf = artifact.pdf_text;
        let text = pdf
            .glyph_runs
            .iter()
            .map(|run| run.text.as_str())
            .collect::<String>();
        assert_eq!(text.matches(PRIME_CHAR).count(), 3);
        let max_font_size = pdf
            .glyph_runs
            .iter()
            .map(|run| run.font_size)
            .fold(0.0, f32::max);
        assert!(
            pdf.glyph_runs
                .iter()
                .any(|run| run.text.contains(PRIME_CHAR) && run.font_size < max_font_size)
        );
        let paths = artifact.paths;
        assert!(paths.items.len() >= 5);
    }

    #[test]
    fn simple_row_can_emit_attach_call_with_corner_slots() {
        let base_math = parse_math("Pi", 0).unwrap();
        let attach_math = parse_math(
            "attach(Pi, t: alpha, b: beta, tl: 1, tr: 2+3, bl: 4+5, br: 6)",
            0,
        )
        .unwrap();
        let options = MathLayoutOptions::default();

        let base = try_typeset_simple_row_fragment(&base_math, &options, &EngineOptions::default())
            .unwrap()
            .expect("base should be handled by Typst row path");
        let artifact =
            try_typeset_simple_row_fragment(&attach_math, &options, &EngineOptions::default())
                .unwrap()
                .expect("attach call should be handled by Typst row path");

        assert!(artifact.metrics.width > base.metrics.width);
        let paths = artifact.paths;
        assert!(paths.items.len() > base.paths.items.len());
        let pdf = artifact.pdf_text;
        let glyph_count = pdf
            .glyph_runs
            .iter()
            .map(|run| run.glyphs.len())
            .sum::<usize>();
        assert!(glyph_count >= 10);
    }

    #[test]
    fn simple_row_can_emit_fraction_rule_paths() {
        let math = parse_math("a / (b + c)", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple fraction should be handled by Typst row path");
        let paths = artifact.paths;
        assert!(
            paths
                .items
                .iter()
                .any(|item| matches!(item.kind, PathKind::MathShape) && item.stroke.is_some())
        );
        let pdf = artifact.pdf_text;
        assert_eq!(pdf.glyph_runs.len(), 4);
    }

    #[test]
    fn simple_row_can_emit_frac_call_rule_paths() {
        let math = parse_math("frac(x + y, z)", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple frac call should be handled by Typst row path");
        let paths = artifact.paths;
        assert!(
            paths
                .items
                .iter()
                .any(|item| matches!(item.kind, PathKind::MathShape) && item.stroke.is_some())
        );
        let pdf = artifact.pdf_text;
        assert_eq!(pdf.glyph_runs.len(), 4);
    }

    #[test]
    fn simple_row_can_emit_frac_style_variants() {
        let options = MathLayoutOptions::default();

        for source in [
            "frac(x + y, z, style: \"skewed\")",
            "frac(x + y, z, style: \"horizontal\")",
        ] {
            let math = parse_math(source, 0).unwrap();
            let artifact =
                try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
                    .unwrap()
                    .unwrap_or_else(|| panic!("{source} should be handled by Typst row path"));
            let paths = artifact.paths;
            assert!(
                paths
                    .items
                    .iter()
                    .all(|item| !matches!(item.kind, PathKind::MathShape)),
                "{source} should not emit a vertical fraction rule"
            );
            let pdf = artifact.pdf_text;
            assert!(
                pdf.glyph_runs.len() >= 3,
                "{source} should emit numerator, slash, and denominator glyphs"
            );
        }
    }

    #[test]
    fn simple_row_can_emit_binom_paths_without_fraction_rule() {
        let math = parse_math("binom(n, k)", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple binom call should be handled by Typst row path");
        let paths = artifact.paths;
        assert_eq!(paths.items.len(), 4);
        assert!(
            paths
                .items
                .iter()
                .all(|item| !matches!(item.kind, PathKind::MathShape))
        );
        let pdf = artifact.pdf_text;
        let text: String = pdf
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert_eq!(text, "(𝑛𝑘)");
    }

    #[test]
    fn simple_row_can_emit_variadic_binom_lower_terms() {
        let math = parse_math("binom(n, k_1, k_2, k_3)", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("variadic binom call should be handled by Typst row path");
        let paths = artifact.paths;
        assert!(
            paths
                .items
                .iter()
                .all(|item| !matches!(item.kind, PathKind::MathShape))
        );
        let pdf = artifact.pdf_text;
        let text: String = pdf
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert!(text.contains("𝑘"));
        assert_eq!(text.matches(',').count(), 2);
    }

    #[test]
    fn simple_row_can_emit_cancel_overlay_path() {
        let math = parse_math("cancel(x)", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple cancel call should be handled by Typst row path");
        let paths = artifact.paths;
        assert_eq!(paths.items.len(), 2);
        assert!(matches!(paths.items[0].kind, PathKind::GlyphOutline { .. }));
        assert!(matches!(paths.items[1].kind, PathKind::MathShape));
        assert!(paths.items[1].stroke.is_some());
        let pdf = artifact.pdf_text;
        assert_eq!(pdf.glyph_runs.len(), 1);
    }

    #[test]
    fn simple_row_cancel_honors_literal_geometry_options() {
        let options = MathLayoutOptions::default();

        let default_line = cancel_shape_lines("cancel(x)", &options);
        let long_line = cancel_shape_lines("cancel(x, length: #200%)", &options);
        let horizontal_line = cancel_shape_lines("cancel(x, angle: #90deg)", &options);
        let vertical_line = cancel_shape_lines("cancel(x, angle: #0deg)", &options);
        let inverted_line = cancel_shape_lines("cancel(x, inverted: #true)", &options);
        let cross_lines = cancel_shape_lines("cancel(x, cross: #true)", &options);

        assert_eq!(default_line.len(), 1);
        assert_eq!(long_line.len(), 1);
        assert_eq!(horizontal_line.len(), 1);
        assert_eq!(vertical_line.len(), 1);
        assert_eq!(inverted_line.len(), 1);
        assert_eq!(cross_lines.len(), 2);

        assert!(
            line_length(long_line[0]) > line_length(default_line[0]) * 1.4,
            "length option should lengthen the cancel line"
        );
        assert!(
            line_delta(horizontal_line[0]).1.abs() < line_delta(horizontal_line[0]).0.abs() * 0.05,
            "90deg should make the cancel line nearly horizontal"
        );
        assert!(
            line_delta(vertical_line[0]).0.abs() < line_delta(vertical_line[0]).1.abs() * 0.05,
            "0deg should make the cancel line nearly vertical"
        );
        assert!(
            line_delta(default_line[0]).0.signum() != line_delta(inverted_line[0]).0.signum(),
            "inverted cancel should flip the horizontal direction"
        );
        assert!(
            line_delta(cross_lines[0]).0.signum() != line_delta(cross_lines[1]).0.signum(),
            "cross cancel should draw opposing lines"
        );
    }

    #[test]
    fn simple_row_cancel_honors_literal_stroke_options() {
        let math = parse_math(
            "cancel(x, stroke: #(thickness: 0.25em, paint: maroon, cap: \"round\", dash: \"dotted\"))",
            0,
        )
        .unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple cancel call should be handled by Typst row path");
        let paths = artifact.paths;
        let stroke = paths
            .items
            .iter()
            .find(|item| matches!(item.kind, PathKind::MathShape))
            .and_then(|item| item.stroke.as_ref())
            .expect("cancel stroke should exist");

        assert_eq!(stroke.color, Color::rgba(0.5, 0.0, 0.0, 1.0));
        assert!((stroke.width - options.style.font_size * 0.25).abs() < 1e-4);
        assert_eq!(stroke.line_cap, crate::typst_svg::LineCap::Round);
        assert_eq!(stroke.line_join, crate::typst_svg::LineJoin::Miter);
        let dash = stroke.dash.as_ref().expect("dash should be resolved");
        assert_eq!(dash.array.len(), 2);
        assert!((dash.array[0] - stroke.width).abs() < 1e-4);
        assert!((dash.array[1] - 2.0).abs() < 1e-4);
        assert_eq!(dash.phase, 0.0);
        assert_eq!(stroke.miter_limit, 4.0);
    }

    #[test]
    fn simple_row_can_emit_sqrt_overbar_paths() {
        let math = parse_math("sqrt(x)", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple sqrt should be handled by Typst row path");
        let paths = artifact.paths;
        assert_eq!(paths.items.len(), 3);
        assert!(matches!(paths.items[1].kind, PathKind::MathShape));
        let pdf = artifact.pdf_text;
        assert_eq!(pdf.glyph_runs.len(), 2);
    }

    #[test]
    fn simple_row_can_emit_math_underline_overline_paths() {
        let math = parse_math("overline(underline(x + y))", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple math underline/overline should be handled by Typst row path");
        let paths = artifact.paths;
        let shape_count = paths
            .items
            .iter()
            .filter(|item| matches!(item.kind, PathKind::MathShape))
            .count();
        assert_eq!(shape_count, 2);
        assert!(paths.items.iter().any(|item| item.stroke.is_some()));
        let pdf = artifact.pdf_text;
        assert_eq!(pdf.glyph_runs.len(), 3);
    }

    #[test]
    fn simple_row_can_emit_math_under_over_constructs() {
        let options = MathLayoutOptions::default();
        let plain = parse_math("x + y", 0).unwrap();
        let plain_artifact =
            try_typeset_simple_row_fragment(&plain, &options, &EngineOptions::default())
                .unwrap()
                .expect("plain body should be handled");
        for source in [
            "overbrace(x + y)",
            "underbrace(x + y)",
            "overbracket(x + y)",
            "underbracket(x + y)",
            "overparen(x + y)",
            "underparen(x + y)",
            "overshell(x + y)",
            "undershell(x + y)",
        ] {
            let math = parse_math(source, 0).unwrap();
            let artifact =
                try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
                    .unwrap()
                    .unwrap_or_else(|| panic!("{source} should be handled by Typst row path"));

            assert!(
                artifact.metrics.height > plain_artifact.metrics.height,
                "{source} should add vertical extent over the plain body"
            );
            assert!(
                artifact.paths.items.len() >= 4,
                "{source} should emit body glyphs plus the under/over construct"
            );
            assert!(
                artifact
                    .paths
                    .items
                    .iter()
                    .any(|item| matches!(item.kind, PathKind::MathShape)),
                "{source} should emit the under/over construct as a math shape"
            );
            assert!(
                artifact.pdf_text.glyph_runs.len() >= plain_artifact.pdf_text.glyph_runs.len(),
                "{source} should retain PDF glyph metadata for the body"
            );
        }
    }

    #[test]
    fn simple_row_can_emit_annotated_math_under_over_constructs() {
        let options = MathLayoutOptions::default();
        let unannotated = parse_math("overbrace(x + y)", 0).unwrap();
        let unannotated_artifact =
            try_typeset_simple_row_fragment(&unannotated, &options, &EngineOptions::default())
                .unwrap()
                .expect("unannotated under/over should be handled");

        for source in [
            "overbrace(x + y, \"sum\")",
            "underbrace(x + y, alpha)",
            "overbracket(x + y, n)",
            "underparen(x + y, \"note\")",
        ] {
            let math = parse_math(source, 0).unwrap();
            let artifact =
                try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
                    .unwrap()
                    .unwrap_or_else(|| panic!("{source} should be handled by Typst row path"));

            assert!(
                artifact.metrics.height > unannotated_artifact.metrics.height,
                "{source} should add annotation vertical extent"
            );
            assert!(
                artifact
                    .paths
                    .items
                    .iter()
                    .any(|item| matches!(item.kind, PathKind::MathShape)),
                "{source} should keep the under/over ornament shape"
            );
            assert!(
                artifact.pdf_text.glyph_runs.len() > unannotated_artifact.pdf_text.glyph_runs.len(),
                "{source} should retain PDF glyph metadata for the annotation"
            );
        }
    }

    #[test]
    fn simple_row_can_emit_indexed_root_paths() {
        let math = parse_math("root(3, x)", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple indexed root should be handled by Typst row path");
        let paths = artifact.paths;
        assert_eq!(paths.items.len(), 4);
        assert!(matches!(paths.items[2].kind, PathKind::MathShape));
        let pdf = artifact.pdf_text;
        assert_eq!(pdf.glyph_runs.len(), 3);
        assert!(pdf.glyph_runs[0].font_size < pdf.glyph_runs[1].font_size);
    }

    #[test]
    fn simple_row_can_emit_visible_group_paths() {
        let math = parse_math("x(t)", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple visible group should be handled by Typst row path");
        let paths = artifact.paths;
        assert_eq!(paths.items.len(), 4);
        let pdf = artifact.pdf_text;
        let text: String = pdf
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert_eq!(text, "𝑥(𝑡)");
    }

    #[cfg(feature = "raster")]
    #[test]
    fn simple_row_visible_group_emits_frame_ready_paths_for_rasterization() {
        let math = parse_math("x(t)", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple visible group should emit Typst paths");
        assert!(!artifact.paths.items.is_empty());
    }

    #[test]
    fn simple_row_extends_identifier_subscript_with_adjacent_group() {
        let math = parse_math("J_n(x)", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("identifier subscript group should be handled by Typst row path");
        let paths = artifact.paths;
        assert_eq!(paths.items.len(), 5);
        let pdf = artifact.pdf_text;
        assert_eq!(pdf.glyph_runs.len(), 5);
        assert!(
            pdf.glyph_runs[1..]
                .iter()
                .all(|run| run.font_size < pdf.glyph_runs[0].font_size)
        );
    }

    #[test]
    fn simple_row_can_emit_delimiter_helper_calls() {
        let options = MathLayoutOptions::default();

        for (source, expected) in [
            ("abs(x)", "|𝑥|"),
            ("norm(v)", "‖𝑣‖"),
            ("floor(x)", "⌊𝑥⌋"),
            ("ceil(x)", "⌈𝑥⌉"),
            ("round(x)", "⌊𝑥⌉"),
            ("ceil.l(x)", "⌈𝑥⌉"),
            ("floor.l(x)", "⌊𝑥⌋"),
            ("paren.l(x)", "(𝑥)"),
            ("brace.l(x)", "{𝑥}"),
            ("bracket.l(x)", "[𝑥]"),
            ("chevron.l(x)", "⟨𝑥⟩"),
            ("bar(x)", "|𝑥|"),
            ("bar.double(x)", "‖𝑥‖"),
        ] {
            let math = parse_math(source, 0).unwrap();
            let artifact =
                try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
                    .unwrap()
                    .unwrap_or_else(|| panic!("simple delimiter call should be handled: {source}"));
            let paths = artifact.paths;
            assert_eq!(paths.items.len(), 3, "{source}");
            let pdf = artifact.pdf_text;
            let text: String = pdf
                .glyph_runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .map(|glyph| glyph.unicode.as_str())
                .collect();
            assert_eq!(text, expected, "{source}");
        }
    }

    #[test]
    fn simple_row_can_emit_lr_delimited_call() {
        let math = parse_math("lr(|x + y|)", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple lr call should be handled by Typst row path");
        let paths = artifact.paths;
        assert_eq!(paths.items.len(), 5);
        let pdf = artifact.pdf_text;
        let text: String = pdf
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert_eq!(text, "|𝑥+𝑦|");
    }

    #[test]
    fn simple_row_stretches_mid_delimiter_inside_lr() {
        let options = MathLayoutOptions::default();

        let raw = parse_math("lr(| A | frac(frac(1, 2), frac(1, 2)) |)", 0).unwrap();
        let mid = parse_math("lr(| A mid(|) frac(frac(1, 2), frac(1, 2)) |)", 0).unwrap();
        let raw_artifact =
            try_typeset_simple_row_fragment(&raw, &options, &EngineOptions::default())
                .unwrap()
                .expect("raw middle delimiter row should be handled");
        let mid_artifact =
            try_typeset_simple_row_fragment(&mid, &options, &EngineOptions::default())
                .unwrap()
                .expect("mid delimiter row should be handled");

        let raw_paths = raw_artifact.paths;
        let mid_paths = mid_artifact.paths;
        assert!(raw_paths.items.len() >= 5);
        assert!(mid_paths.items.len() >= 5);

        let raw_middle_height = path_y_extent(&raw_paths.items[2]);
        let mid_middle_height = path_y_extent(&mid_paths.items[2]);
        assert!(
            mid_middle_height > raw_middle_height + 3.0,
            "mid delimiter should stretch to surrounding lr height: raw={raw_middle_height}, mid={mid_middle_height}"
        );

        let pdf = mid_artifact.pdf_text;
        let text: String = pdf
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert!(text.starts_with("|𝐴|1"));
        assert!(text.ends_with("|"));
    }

    #[test]
    fn simple_row_stretches_named_mid_delimiters_inside_lr() {
        let options = MathLayoutOptions::default();

        for (raw_source, mid_source, expected) in [
            (
                "lr(| A slash frac(frac(1, 2), frac(1, 2)) |)",
                "lr(| A mid(slash) frac(frac(1, 2), frac(1, 2)) |)",
                "/",
            ),
            (
                "lr(| A bar.v.double frac(frac(1, 2), frac(1, 2)) |)",
                "lr(| A mid(bar.v.double) frac(frac(1, 2), frac(1, 2)) |)",
                "‖",
            ),
        ] {
            let raw = parse_math(raw_source, 0).unwrap();
            let mid = parse_math(mid_source, 0).unwrap();
            let raw_artifact =
                try_typeset_simple_row_fragment(&raw, &options, &EngineOptions::default())
                    .unwrap()
                    .unwrap_or_else(|| {
                        panic!("raw named delimiter row should be handled: {raw_source}")
                    });
            let mid_artifact =
                try_typeset_simple_row_fragment(&mid, &options, &EngineOptions::default())
                    .unwrap()
                    .unwrap_or_else(|| {
                        panic!("mid named delimiter row should be handled: {mid_source}")
                    });

            let raw_middle_height = path_y_extent(&raw_artifact.paths.items[2]);
            let mid_middle_height = path_y_extent(&mid_artifact.paths.items[2]);
            assert!(
                mid_middle_height > raw_middle_height + 3.0,
                "{mid_source} should stretch to surrounding lr height: raw={raw_middle_height}, mid={mid_middle_height}"
            );

            let text: String = mid_artifact
                .pdf_text
                .glyph_runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .map(|glyph| glyph.unicode.as_str())
                .collect();
            assert!(
                text.contains(expected),
                "{mid_source} should preserve named delimiter text in PDF glyph metadata: {text}"
            );
        }
    }

    #[test]
    fn simple_row_applies_lr_delimiter_size_option() {
        let options = MathLayoutOptions::default();

        let plain = parse_math("lr(|x|)", 0).unwrap();
        let sized = parse_math("lr(size: #240%, |x|)", 0).unwrap();
        let plain_artifact =
            try_typeset_simple_row_fragment(&plain, &options, &EngineOptions::default())
                .unwrap()
                .expect("plain lr call should be handled by Typst row path");
        let sized_artifact =
            try_typeset_simple_row_fragment(&sized, &options, &EngineOptions::default())
                .unwrap()
                .expect("sized lr call should be handled by Typst row path");

        assert!(
            sized_artifact.metrics.height > plain_artifact.metrics.height + 3.0,
            "explicit delimiter size should increase line height: plain={:?}, sized={:?}",
            plain_artifact.metrics,
            sized_artifact.metrics
        );
        assert_eq!(sized_artifact.paths.items.len(), 3);
    }

    #[test]
    fn simple_row_applies_delimiter_helper_size_option() {
        let options = MathLayoutOptions::default();

        let plain = parse_math("abs(x)", 0).unwrap();
        let sized = parse_math("abs(x, size: #2em)", 0).unwrap();
        let plain_artifact =
            try_typeset_simple_row_fragment(&plain, &options, &EngineOptions::default())
                .unwrap()
                .expect("plain abs call should be handled by Typst row path");
        let sized_artifact =
            try_typeset_simple_row_fragment(&sized, &options, &EngineOptions::default())
                .unwrap()
                .expect("sized abs call should be handled by Typst row path");

        assert!(
            sized_artifact.metrics.height > plain_artifact.metrics.height + 5.0,
            "explicit delimiter size should increase helper height: plain={:?}, sized={:?}",
            plain_artifact.metrics,
            sized_artifact.metrics
        );
        assert_eq!(sized_artifact.paths.items.len(), 3);
    }

    #[test]
    fn simple_row_applies_callable_delimiter_symbol_size_option() {
        let options = MathLayoutOptions::default();

        let plain = parse_math("bracket.l(x)", 0).unwrap();
        let sized = parse_math("bracket.l(x, size: #240%)", 0).unwrap();
        let plain_artifact =
            try_typeset_simple_row_fragment(&plain, &options, &EngineOptions::default())
                .unwrap()
                .expect("plain bracket.l call should be handled by Typst row path");
        let sized_artifact =
            try_typeset_simple_row_fragment(&sized, &options, &EngineOptions::default())
                .unwrap()
                .expect("sized bracket.l call should be handled by Typst row path");

        assert!(
            sized_artifact.metrics.height > plain_artifact.metrics.height + 3.0,
            "explicit delimiter size should increase callable symbol height: plain={:?}, sized={:?}",
            plain_artifact.metrics,
            sized_artifact.metrics
        );
        assert_eq!(sized_artifact.paths.items.len(), 3);
    }

    #[test]
    fn simple_row_can_emit_operator_calls() {
        let options = MathLayoutOptions::default();

        for (source, expected) in [
            ("sin(x)", "sin(𝑥)"),
            ("cos(theta)", "cos(𝜃)"),
            ("sech(x)", "sech(𝑥)"),
            ("liminf_(n -> oo)", "lim inf𝑛→∞"),
            ("Pr(X)", "Pr(𝑋)"),
            ("op(\"custom\")", "custom"),
            ("op(\"myop\", limits: #true)_n", "myop𝑛"),
            ("op(lt, limits: #false)", "<"),
        ] {
            let math = parse_math(source, 0).unwrap();
            let artifact =
                try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
                    .unwrap()
                    .unwrap_or_else(|| panic!("operator call should be handled: {source}"));
            let pdf = artifact.pdf_text;
            let text: String = pdf
                .glyph_runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .map(|glyph| glyph.unicode.as_str())
                .collect();
            assert_eq!(text, expected, "{source}");
        }
    }

    #[test]
    fn simple_row_can_emit_operator_identifier_with_script() {
        let math = parse_math("lim_(x -> oo) f(x)", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("operator identifier with script should be handled by Typst row path");
        let pdf = artifact.pdf_text;
        let text: String = pdf
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert!(text.starts_with("lim"));
        assert!(text.contains("∞"));
    }

    #[test]
    fn simple_row_can_emit_math_variant_calls() {
        let options = MathLayoutOptions::default();

        for (source, expected) in [
            ("bb(R)", "ℝ"),
            ("cal(P)", "𝒫"),
            ("scr(L)", "ℒ"),
            ("frak(g)", "𝔤"),
            ("sans(x)", "𝘹"),
            ("mono(123)", "𝟷𝟸𝟹"),
            ("bold(alpha + 2)", "𝜶+𝟐"),
            ("upright(R)", "R"),
            ("italic(R)", "𝑅"),
        ] {
            let math = parse_math(source, 0).unwrap();
            let artifact =
                try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
                    .unwrap()
                    .unwrap_or_else(|| panic!("math variant call should be handled: {source}"));
            let pdf = artifact.pdf_text;
            let text: String = pdf
                .glyph_runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .map(|glyph| glyph.unicode.as_str())
                .collect();
            assert_eq!(text, expected, "{source}");
        }
    }

    #[test]
    fn simple_row_can_emit_vertical_stretch_call() {
        let options = MathLayoutOptions::default();

        let plain = parse_math("stretch(|)", 0).unwrap();
        let stretched = parse_math("stretch(|, size: #2em)", 0).unwrap();
        let plain_artifact =
            try_typeset_simple_row_fragment(&plain, &options, &EngineOptions::default())
                .unwrap()
                .expect("plain vertical stretch call should be handled by Typst row path");
        let stretched_artifact =
            try_typeset_simple_row_fragment(&stretched, &options, &EngineOptions::default())
                .unwrap()
                .expect("stretched bar call should be handled by Typst row path");

        assert!(
            stretched_artifact.metrics.height > plain_artifact.metrics.height + 5.0,
            "vertical stretch should increase height: plain={:?}, stretched={:?}",
            plain_artifact.metrics,
            stretched_artifact.metrics
        );
        let paths = stretched_artifact.paths;
        assert_eq!(paths.items.len(), 1);
    }

    #[test]
    fn simple_row_can_emit_horizontal_stretch_call() {
        let options = MathLayoutOptions::default();

        let plain = parse_math("stretch(->)", 0).unwrap();
        let stretched = parse_math("stretch(->, size: #200%)", 0).unwrap();
        let plain_artifact =
            try_typeset_simple_row_fragment(&plain, &options, &EngineOptions::default())
                .unwrap()
                .expect("plain horizontal stretch call should be handled by Typst row path");
        let stretched_artifact =
            try_typeset_simple_row_fragment(&stretched, &options, &EngineOptions::default())
                .unwrap()
                .expect("stretched arrow call should be handled by Typst row path");

        assert!(
            stretched_artifact.metrics.width > plain_artifact.metrics.width + 5.0,
            "horizontal stretch should increase width: plain={:?}, stretched={:?}",
            plain_artifact.metrics,
            stretched_artifact.metrics
        );
        assert!(
            stretched_artifact.paths.items.len() > 1,
            "horizontal assembly should emit multiple glyph outlines"
        );
        let glyph_text: String = stretched_artifact
            .pdf_text
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert_eq!(glyph_text, "→");
    }

    #[test]
    fn simple_row_can_emit_math_accent_calls() {
        let options = MathLayoutOptions::default();

        for (source, expected) in [
            ("grave(a)", "𝑎\u{0300}"),
            ("acute(b)", "𝑏\u{0301}"),
            ("hat(x)", "𝑥\u{0302}"),
            ("hat(i)", "𝚤\u{0302}"),
            ("hat(dotless: #false, i)", "𝑖\u{0302}"),
            ("tilde(x)", "𝑥\u{0303}"),
            ("macron(x)", "𝑥\u{0304}"),
            ("dash(x)", "𝑥\u{0305}"),
            ("breve(x)", "𝑥\u{0306}"),
            ("dot(x)", "𝑥\u{0307}"),
            ("dot.double(x)", "𝑥\u{0308}"),
            ("ddot(x)", "𝑥\u{0308}"),
            ("dot.triple(x)", "𝑥\u{20db}"),
            ("dot.quad(x)", "𝑥\u{20dc}"),
            ("circle(x)", "𝑥\u{030a}"),
            ("acute.double(x)", "𝑥\u{030b}"),
            ("caron(x)", "𝑥\u{030c}"),
            ("arrow(v)", "𝑣\u{20d7}"),
            ("arrow.l(v)", "𝑣\u{20d6}"),
            ("arrow.l.r(v)", "𝑣\u{20e1}"),
            ("harpoon(v)", "𝑣\u{20d1}"),
            ("harpoon.lt(v)", "𝑣\u{20d0}"),
            ("accent(v, <-)", "𝑣\u{20d6}"),
            ("accent(v, \".\")", "𝑣\u{0307}"),
            ("accent(v, arrow.l.r)", "𝑣\u{20e1}"),
        ] {
            let math = parse_math(source, 0).unwrap();
            let artifact =
                try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
                    .unwrap()
                    .unwrap_or_else(|| panic!("math accent call should be handled: {source}"));
            let paths = artifact.paths;
            assert_eq!(paths.items.len(), 2, "{source}");
            let pdf = artifact.pdf_text;
            let text: String = pdf
                .glyph_runs
                .iter()
                .flat_map(|run| &run.glyphs)
                .map(|glyph| glyph.unicode.as_str())
                .collect();
            assert_eq!(text, expected, "{source}");
        }
    }

    #[test]
    fn simple_row_can_emit_sized_math_accent_call() {
        let options = MathLayoutOptions::default();

        let sized = parse_math("arrow.l.r(A B C D, size: #200%)", 0).unwrap();
        let sized_artifact =
            try_typeset_simple_row_fragment(&sized, &options, &EngineOptions::default())
                .unwrap()
                .expect("sized accent call should be handled by Typst row path");

        let glyph_unicodes: Vec<&str> = sized_artifact
            .pdf_text
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert_eq!(
            glyph_unicodes
                .iter()
                .filter(|unicode| **unicode == "\u{20e1}")
                .count(),
            1,
            "assembled accent should expose one semantic arrow"
        );
        assert!(
            glyph_unicodes.iter().any(|unicode| unicode.is_empty()),
            "assembled accent should use non-semantic extender glyphs"
        );
    }

    #[test]
    fn simple_row_can_emit_bottom_math_accent_call() {
        let options = MathLayoutOptions::default();

        let base = parse_math("x", 0).unwrap();
        let bottom = parse_math("accent(x, \"\u{0330}\")", 0).unwrap();
        let base_artifact =
            try_typeset_simple_row_fragment(&base, &options, &EngineOptions::default())
                .unwrap()
                .expect("base row should be handled by Typst row path");
        let bottom_artifact =
            try_typeset_simple_row_fragment(&bottom, &options, &EngineOptions::default())
                .unwrap()
                .expect("bottom accent call should be handled by Typst row path");

        assert!(
            bottom_artifact.metrics.descent > base_artifact.metrics.descent,
            "bottom accent should contribute descent: base={:?}, bottom={:?}",
            base_artifact.metrics,
            bottom_artifact.metrics
        );
        let text: String = bottom_artifact
            .pdf_text
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert_eq!(text, "𝑥\u{0330}");
    }

    #[test]
    fn simple_row_omits_script_group_delimiters() {
        let math = parse_math("sum_(i=0)^n i", 0).unwrap();
        let options = MathLayoutOptions::default();

        let artifact = try_typeset_simple_row_fragment(&math, &options, &EngineOptions::default())
            .unwrap()
            .expect("simple grouped script should be handled by Typst row path");
        let pdf = artifact.pdf_text;
        let text: String = pdf
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();
        assert_eq!(text, "∑𝑛𝑖=0𝑖");
    }

    #[test]
    fn atom_fragment_styles_latin_and_greek_as_math_italic() {
        let x = parse_math("x", 0).unwrap();
        let alpha = parse_math("alpha", 0).unwrap();

        assert_eq!(single_atom_text(&x).as_deref(), Some("𝑥"));
        assert_eq!(single_atom_text(&alpha).as_deref(), Some("𝛼"));
    }

    #[test]
    fn atom_fragment_keeps_numbers_and_operators_plain() {
        let number = parse_math("0.94", 0).unwrap();
        let plus = parse_math("+", 0).unwrap();
        let arrow = parse_math("->", 0).unwrap();

        assert_eq!(single_atom_text(&number).as_deref(), Some("0.94"));
        assert_eq!(single_atom_text(&plus).as_deref(), Some("+"));
        assert_eq!(single_atom_text(&arrow).as_deref(), Some("→"));
    }

    #[test]
    fn simple_row_inserts_binary_and_relation_spacing() {
        let font_size = 12.0;

        assert_eq!(
            math_spacing(
                SimpleMathClass::Alphabetic,
                SimpleMathClass::Binary,
                font_size
            ),
            MEDIUM_EM * font_size
        );
        assert_eq!(
            math_spacing(
                SimpleMathClass::Relation,
                SimpleMathClass::Alphabetic,
                font_size
            ),
            THICK_EM * font_size
        );
    }

    #[test]
    fn simple_row_honors_explicit_math_spacing_widths() {
        fn width(source: &str) -> f32 {
            let math = parse_math(source, 0).unwrap();
            try_typeset_simple_row_fragment(
                &math,
                &MathLayoutOptions::default(),
                &EngineOptions::default(),
            )
            .unwrap()
            .unwrap_or_else(|| panic!("math spacing source should be handled: {source}"))
            .metrics
            .width
        }

        let base = width("a b");
        let thin = width("a thin b");
        let med = width("a med b");
        let thick = width("a thick b");
        let quad = width("a quad b");
        let wide = width("a wide b");

        assert!(thin > base, "thin should add explicit width");
        assert!(med > thin, "med should be wider than thin");
        assert!(thick > med, "thick should be wider than med");
        assert!(quad > thick, "quad should be wider than thick");
        assert!(wide > quad, "wide should be wider than quad");
    }

    #[test]
    fn simple_row_differentials_emit_upright_pdf_glyphs() {
        let math = parse_math("x dif y + Dif z", 0).unwrap();
        let artifact = try_typeset_simple_row_fragment(
            &math,
            &MathLayoutOptions::default(),
            &EngineOptions::default(),
        )
        .unwrap()
        .expect("differential row should be handled by Typst row path");
        let text: String = artifact
            .pdf_text
            .glyph_runs
            .iter()
            .flat_map(|run| &run.glyphs)
            .map(|glyph| glyph.unicode.as_str())
            .collect();

        assert!(
            text.contains('d') && text.contains('D'),
            "differentials should be embedded as PDF glyph text: {text}"
        );
        assert!(
            !text.contains("𝑑") && !text.contains("𝐷"),
            "differentials should remain upright by default: {text}"
        );
    }

    #[test]
    fn simple_row_class_call_overrides_spacing_class() {
        let config = EngineOptions::default();
        let font =
            load_default_math_font(&config, &MathFontSpec::LeteSansMath, &FontWeight::Normal)
                .expect("default math font should load");
        let font_size = 20.0;
        let normal = parse_math("a ! b", 0).unwrap();
        let relation = parse_math("a class(\"relation\", !) b", 0).unwrap();

        let normal_width = layout_simple_row(&font, &normal, font_size)
            .unwrap()
            .expect("normal row should layout")
            .metrics
            .width;
        let relation_width = layout_simple_row(&font, &relation, font_size)
            .unwrap()
            .expect("class row should layout")
            .metrics
            .width;

        assert!(
            relation_width > normal_width + font_size * 0.1,
            "relation class should add more spacing than the default binary-vary operator"
        );
    }

    #[test]
    fn simple_row_math_size_calls_scale_body() {
        let config = EngineOptions::default();
        let font =
            load_default_math_font(&config, &MathFontSpec::LeteSansMath, &FontWeight::Normal)
                .expect("default math font should load");
        let font_size = 20.0;
        let inline = parse_math("inline(x)", 0).unwrap();
        let script = parse_math("script(x)", 0).unwrap();
        let sscript = parse_math("sscript(x)", 0).unwrap();

        let inline_width = layout_simple_row(&font, &inline, font_size)
            .unwrap()
            .expect("inline row should layout")
            .metrics
            .width;
        let script_width = layout_simple_row(&font, &script, font_size)
            .unwrap()
            .expect("script row should layout")
            .metrics
            .width;
        let sscript_width = layout_simple_row(&font, &sscript, font_size)
            .unwrap()
            .expect("sscript row should layout")
            .metrics
            .width;

        assert!(script_width < inline_width);
        assert!(sscript_width < script_width);
    }

    #[test]
    fn simple_row_display_math_size_uses_display_fraction_metrics() {
        let config = EngineOptions::default();
        let font =
            load_default_math_font(&config, &MathFontSpec::LeteSansMath, &FontWeight::Normal)
                .expect("default math font should load");
        let font_size = 20.0;
        let inline = parse_math("inline(frac(1, 2))", 0).unwrap();
        let display = parse_math("display(frac(1, 2))", 0).unwrap();

        let inline_metrics = layout_simple_row(&font, &inline, font_size)
            .unwrap()
            .expect("inline fraction row should layout")
            .metrics;
        let display_metrics = layout_simple_row(&font, &display, font_size)
            .unwrap()
            .expect("display fraction row should layout")
            .metrics;

        assert!(
            display_metrics.height > inline_metrics.height + font_size * 0.3,
            "display fraction should use taller display-style stack metrics: inline={inline_metrics:?}, display={display_metrics:?}"
        );
        assert!(
            display_metrics.width > inline_metrics.width,
            "display fraction should keep text-style numerator and denominator instead of script-style children"
        );
    }
}
