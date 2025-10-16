use crate::visual_tests::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::prelude::*;

#[tokio::test]
async fn test_line_legend_background_from_css() {
    let ctx = SessionContext::new();

    // Simple inline data
    let sql = "SELECT * FROM (VALUES
        (1.0, 2.0, 'Series A'), (2.0, 4.0, 'Series A'), (3.0, 3.0, 'Series A'), (4.0, 5.0, 'Series A'),
        (1.0, 3.0, 'Series B'), (2.0, 2.0, 'Series B'), (3.0, 4.0, 'Series B'), (4.0, 3.0, 'Series B')
    ) AS t(x, y, series)";
    let df = ctx.sql(sql).await.expect("Failed to create dataframe");

    // CSS with bright, highly visible colors for background
    let css = r#"
        legend background {
            fill: #ff0000;
            stroke: #00ff00;
            stroke-width: 5.0;
            corner-radius: 10;
            padding: 20;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    let plot = Plot::<Cartesian>::new()
        .theme(theme)
        .data(df)
        .mark(
            Line::new()
                .x(col("x"))
                .y(col("y"))
                .stroke_with(col("series"), |c| {
                    c.scale_with::<Ordinal>(|s| s)
                        .legend(|l| l.title("Series"))
                })
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "css", "line_legend_background_css").await;
}

#[tokio::test]
async fn test_symbol_legend_background_from_css() {
    let ctx = SessionContext::new();

    // Simple inline data for symbol plot
    let sql = "SELECT * FROM (VALUES
        (1.0, 2.0, 'Category A'), (2.0, 4.0, 'Category A'), (3.0, 3.0, 'Category A'),
        (1.0, 3.0, 'Category B'), (2.0, 2.0, 'Category B'), (3.0, 4.0, 'Category B')
    ) AS t(x, y, category)";
    let df = ctx.sql(sql).await.expect("Failed to create dataframe");

    // CSS with bright colors
    let css = r#"
        legend background {
            fill: #ff0000;
            stroke: #00ff00;
            stroke-width: 5.0;
            corner-radius: 10;
            padding: 20;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    let plot = Plot::<Cartesian>::new()
        .theme(theme)
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .fill_with(col("category"), |c| {
                    c.scale_with::<Ordinal>(|s| s)
                        .legend(|l| l.title("Category"))
                })
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "css", "symbol_legend_background_css").await;
}

#[tokio::test]
async fn test_rect_legend_background_from_css() {
    let ctx = SessionContext::new();

    // Simple inline data for rect plot
    let sql = "SELECT * FROM (VALUES
        ('A', 10.0), ('B', 20.0), ('C', 15.0)
    ) AS t(category, value)";
    let df = ctx.sql(sql).await.expect("Failed to create dataframe");

    // CSS with bright colors
    let css = r#"
        legend background {
            fill: #ff0000;
            stroke: #00ff00;
            stroke-width: 5.0;
            corner-radius: 10;
            padding: 20;
        }
    "#;

    let theme = Theme::from_css(css).expect("Failed to parse CSS");

    let plot = Plot::<Cartesian>::new()
        .theme(theme)
        .data(df)
        .mark(
            Rect::new()
                .x(col("category"))
                .x2_with(col(":x"), |c| c.band(1.0))
                .y(lit(0.0))
                .y2(col("value"))
                .fill_with(col("category"), |c| {
                    c.scale_with::<Ordinal>(|s| s)
                        .legend(|l| l.title("Category"))
                })
        );

    let compiled = plot.compile(&ctx).await.expect("Failed to compile plot");
    assert_visual_match_default(&compiled, &ctx, None, "css", "rect_legend_background_css").await;
}
