//! A server that builds a chart on each request and answers with its bytes, which a page draws
//! with avenger-chart-web. DataFusion and the data stay on the server.
//!
//! `POST /chart` takes JSON of a genre and a range of years, such as
//! `{"genre": "Drama", "from": 1990, "to": 2000}`, selects the movies of `movie-search` with a
//! DataFusion query, and answers with the bytes of a scatter chart of their ratings, which a page
//! draws with navigation and a tooltip of each movie. A `null` genre selects every genre.

use std::sync::Arc;

use arrow::{array::AsArray, compute::concat_batches};
use avenger_chart::prelude::*;
use datafusion::{
    datasource::MemTable,
    prelude::{col, lit, SessionContext},
};
use serde::Deserialize;

/// The choices of a request for a chart.
#[derive(Debug, Deserialize)]
pub struct Choices {
    pub genre: Option<String>,
    pub from: i64,
    pub to: i64,
}

/// An answer that is not a chart: its HTTP status and its message.
#[derive(Debug, PartialEq)]
pub struct Rejection {
    pub status: u16,
    pub message: String,
}

impl Rejection {
    fn bad(message: impl Into<String>) -> Self {
        Self {
            status: 400,
            message: message.into(),
        }
    }

    fn internal(error: impl std::fmt::Display) -> Self {
        Self {
            status: 500,
            message: error.to_string(),
        }
    }
}

/// The movies, in a DataFusion session.
pub struct Movies {
    ctx: SessionContext,
}

impl Movies {
    pub fn new() -> Self {
        let ctx = SessionContext::new();
        let movies = chart_web::movies();
        let table = MemTable::try_new(movies.schema(), vec![vec![movies]])
            .expect("the movies make a table");
        ctx.register_table("movies", Arc::new(table))
            .expect("the table registers");
        Self { ctx }
    }

    /// The genres of the movies, in the order of their names.
    pub async fn genres(&self) -> Result<Vec<String>, Rejection> {
        let frame = (self
            .ctx
            .table("movies")
            .await
            .map_err(Rejection::internal)?)
        .filter(col("genre").is_not_null())
        .and_then(|frame| frame.aggregate(vec![col("genre")], vec![]))
        .and_then(|frame| frame.sort(vec![col("genre").sort(true, false)]))
        .map_err(Rejection::internal)?;
        let batches = frame.collect().await.map_err(Rejection::internal)?;
        Ok(batches
            .iter()
            .flat_map(|batch| {
                let genres = batch.column(0).as_string::<i32>();
                genres
                    .iter()
                    .flatten()
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .collect())
    }

    /// The bytes of the chart of `body`, JSON of [`Choices`].
    pub async fn chart(&self, body: &[u8]) -> Result<Vec<u8>, Rejection> {
        let choices: Choices = serde_json::from_slice(body).map_err(|error| {
            Rejection::bad(format!("the request is not JSON of choices: {error}"))
        })?;
        if choices.from > choices.to {
            return Err(Rejection::bad(format!(
                "the range of years from {} to {} is empty",
                choices.from, choices.to
            )));
        }
        let mut selected = col("year")
            .gt_eq(lit(choices.from))
            .and(col("year").lt_eq(lit(choices.to)));
        if let Some(genre) = &choices.genre {
            selected = selected.and(col("genre").eq(lit(genre.as_str())));
        }
        let frame = (self
            .ctx
            .table("movies")
            .await
            .map_err(Rejection::internal)?)
        .filter(selected)
        .map_err(Rejection::internal)?;
        let schema = Arc::new(frame.schema().as_arrow().clone());
        let batches = frame.collect().await.map_err(Rejection::internal)?;
        let movies = concat_batches(&schema, &batches).map_err(Rejection::internal)?;
        if movies.num_rows() == 0 {
            return Err(Rejection::bad("no movies have these choices"));
        }
        let title = match &choices.genre {
            Some(genre) => format!("{genre} movies, {} to {}", choices.from, choices.to),
            None => format!("Movies, {} to {}", choices.from, choices.to),
        };
        scatter(movies, title)
            .to_bytes()
            .map(|bytes| bytes.to_vec())
            .map_err(Rejection::internal)
    }
}

impl Default for Movies {
    fn default() -> Self {
        Self::new()
    }
}

/// A scatter chart of the ratings of `movies`, colored by year, with a navigation and a tooltip
/// of each movie.
fn scatter(movies: arrow::array::RecordBatch, title: String) -> Chart {
    let axis = |field: &str, title: &str| {
        Encoding::scaled_field(field).scale(Axis::builder().title(title))
    };
    let symbols = CartesianLayer::symbol(
        movies,
        SymbolMark::builder()
            .x(axis("rotten_tomatoes_rating", "Rotten Tomatoes rating"))
            .y(axis("imdb_rating", "IMDB rating"))
            .color(
                Encoding::scaled_field("year").scale(
                    Legend::builder()
                        .title("Year")
                        .labels(Labels::builder().format("d")),
                ),
            )
            .opacity(0.7),
    );
    let field = |column: &str, title: &str| TooltipField::builder().column(column).title(title);
    let tooltip = Tooltip::nearest(NearestFind::builder().max_radius(12.0))
        .title(TooltipField::builder().column("title"))
        .fields([
            field("genre", "Genre"),
            field("year", "Year").labels(Labels::builder().format("d")),
            field("imdb_rating", "IMDB"),
            field("rotten_tomatoes_rating", "Rotten Tomatoes"),
        ]);
    let plot = Plot::cartesian([symbols])
        .id("movies")
        .title(title)
        .tools([ToolBuilder::from(Navigate::builder()), tooltip.into()]);
    Chart::builder(plot)
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .build()
        .expect("the chart builds")
}
