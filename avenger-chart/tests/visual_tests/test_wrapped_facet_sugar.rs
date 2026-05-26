use std::sync::Arc;

use super::helpers::assert_visual_match_default;
use avenger_chart::prelude::*;
use datafusion::{
    arrow::{
        array::{Float64Array, StringArray},
        compute::concat_batches,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    functions_aggregate::min_max::min,
    prelude::*,
};

async fn wrapped_facet_data(ctx: &SessionContext) -> DataFrame {
    let facets = [
        ("Atlas", 92.0),
        ("Boreal", 81.0),
        ("Cygnus", 73.0),
        ("Draco", 64.0),
        ("Equinox", 55.0),
        ("Fornax", 48.0),
        ("Gemini", 39.0),
    ];

    let mut facet_values = Vec::new();
    let mut x_values = Vec::new();
    let mut y_values = Vec::new();
    let mut sort_values = Vec::new();

    for (facet_index, (facet, sort_value)) in facets.iter().enumerate() {
        for point_index in 0..8 {
            let phase = point_index as f64 / 7.0;
            facet_values.push(*facet);
            x_values.push(point_index as f64);
            y_values.push(
                18.0 + phase * 52.0 + (*sort_value / 14.0) + ((facet_index % 3) as f64 * 4.0),
            );
            sort_values.push(*sort_value);
        }
    }

    let schema = Arc::new(Schema::new(vec![
        Field::new("facet", DataType::Utf8, false),
        Field::new("x", DataType::Float64, false),
        Field::new("y", DataType::Float64, false),
        Field::new("sort_value", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(facet_values)),
            Arc::new(Float64Array::from(x_values)),
            Arc::new(Float64Array::from(y_values)),
            Arc::new(Float64Array::from(sort_values)),
        ],
    )
    .expect("wrapped facet batch");

    ctx.register_batch("wrapped_facet_source", batch)
        .expect("register wrapped facet data");
    let derived = ctx
        .sql(
        r#"
        WITH facet_scores AS (
            SELECT facet, MAX(sort_value) AS sort_value
            FROM wrapped_facet_source
            GROUP BY facet
        ),
        facet_order AS (
            SELECT
                facet,
                sort_value,
                CAST(ROW_NUMBER() OVER (ORDER BY sort_value DESC, facet ASC) - 1 AS BIGINT) AS facet_index
            FROM facet_scores
        )
        SELECT
            s.facet,
            s.x,
            s.y,
            s.sort_value,
            o.facet_index,
            o.facet_index % 2 AS wrap_row
        FROM wrapped_facet_source AS s
        JOIN facet_order AS o
            ON s.facet = o.facet
        "#,
    )
    .await
    .expect("wrapped facet sql");
    let batches = derived.collect().await.expect("collect wrapped facet sql");
    let schema = batches
        .first()
        .expect("wrapped facet sql should produce a batch")
        .schema();
    let batch = concat_batches(&schema, &batches).expect("materialize wrapped facet sql");
    ctx.read_batch(batch)
        .expect("read materialized wrapped facet data")
}

fn leaf_plot() -> Plot<Cartesian> {
    Plot::<Cartesian>::new().mark(
        Symbol::new()
            .x_with(col("x"), |c| {
                c.scale(|s| s.domain((0.0, 7.0))).axis(|a| a.title("x"))
            })
            .y_with(col("y"), |c| {
                c.scale(|s| s.domain((15.0, 85.0))).axis(|a| a.title("y"))
            })
            .size(72.0)
            .fill("#4c78a8")
            .stroke("#1f3552")
            .stroke_width(1.0),
    )
}

#[tokio::test]
async fn wrapped_facet_via_ranked_rows_and_free_columns() {
    let ctx = SessionContext::new();
    let data = wrapped_facet_data(&ctx).await;

    let column_facets =
        Plot::<FacetColumn>::new().mark(Subplot::new(leaf_plot()).col_with(col("facet"), |c| {
            c.order_by(min(col("facet_index")))
                .order_asc()
                .free_slots()
                .guide(|f| f.title("Facet"))
        }));
    let plot = Plot::<FacetRow>::new()
        .data(data)
        .canvas_size(1120, 680)
        .mark(Subplot::new(column_facets).row_with(col("wrap_row"), |c| {
            c.order_by(min(col("facet_index")))
                .order_asc()
                .guide(|f| f.visible(false))
        }));

    let compiled = plot
        .compile(&ctx)
        .await
        .expect("compile wrapped facet sugar plot");
    assert_visual_match_default(
        &compiled,
        &ctx,
        None,
        "wrapped_facet_sugar",
        "wrapped_facet_via_ranked_rows_and_free_columns",
    )
    .await;
}
