//! The charts of the gallery of avenger-chart-web, built with the builders of avenger-chart over
//! the movies, the penguins, and the iris flowers of the repository. Each plot declares a
//! navigation and a tooltip, which the library runs in the browser.

use std::{collections::BTreeMap, io::Cursor, sync::Arc};

use arrow::{
    array::{ArrayRef, AsArray, Float64Array, Int64Array, RecordBatch, StringArray},
    compute::{concat_batches, filter_record_batch, is_not_null},
    csv::ReaderBuilder,
    datatypes::{DataType, Field, Float64Type, Int64Type, Schema},
};
use avenger_chart::prelude::*;

/// A chart of the gallery, with the name of its file, its title, and its description.
pub struct Entry {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub chart: Chart,
}

/// The charts of the gallery, in its order.
pub fn gallery() -> Vec<Entry> {
    let movies = movies();
    let penguins = penguins();
    vec![
        Entry {
            name: "movie-ratings",
            title: "Movie ratings",
            description: "The ratings of 2,259 movies, colored by year. Drag to pan, scroll to zoom, and hover for a movie.",
            chart: build(movie_ratings(&movies)),
        },
        Entry {
            name: "movie-genres",
            title: "Movies by genre",
            description: "The number of movies of each genre. Hover a bar for its count.",
            chart: build(movie_genres(&movies)),
        },
        Entry {
            name: "ratings-by-year",
            title: "Ratings by year",
            description: "The mean IMDB rating of each year's movies. Drag and scroll to move through the years.",
            chart: build(ratings_by_year(&movies)),
        },
        Entry {
            name: "penguins",
            title: "Penguin sizes",
            description: "The flipper lengths and body masses of three species of penguins.",
            chart: build(penguin_sizes(&penguins)),
        },
        Entry {
            name: "penguin-islands",
            title: "Penguins by island",
            description: "The bills of the penguins of each island, in panels that pan and zoom together.",
            chart: build(penguin_islands(&penguins)),
        },
        Entry {
            name: "iris",
            title: "Iris petals",
            description: "The petals of three species of iris flowers.",
            chart: build(iris()),
        },
    ]
}

fn build(root: impl Into<NodeBuilder>) -> Chart {
    Chart::builder(root)
        .number_format_syntax("d3")
        .datetime_format_syntax("d3")
        .build()
        .expect("the gallery's charts build")
}

/// The rows of a CSV file with `fields`, without the rows that lack a value of `required`.
fn read(csv: &str, fields: Vec<Field>, required: &[&str]) -> RecordBatch {
    let schema = Arc::new(Schema::new(fields));
    let reader = (ReaderBuilder::new(schema.clone()).with_header(true))
        .build(Cursor::new(csv))
        .expect("the CSV file reads");
    let batches = reader
        .collect::<Result<Vec<_>, _>>()
        .expect("the CSV rows parse");
    let mut rows = concat_batches(&schema, &batches).expect("the batches concatenate");
    for column in required {
        let present = is_not_null(rows.column_by_name(column).expect("the column")).unwrap();
        rows = filter_record_batch(&rows, &present).unwrap();
    }
    rows
}

/// The movies of `movie-search`.
pub fn movies() -> RecordBatch {
    read(
        include_str!("../../movie-search/data/movies.csv"),
        vec![
            Field::new("title", DataType::Utf8, false),
            Field::new("imdb_rating", DataType::Float64, false),
            Field::new("rotten_tomatoes_rating", DataType::Float64, false),
            Field::new("genre", DataType::Utf8, true),
            Field::new("year", DataType::Int64, false),
        ],
        &[],
    )
}

/// The penguins of avenger-sql's tests that have their measurements.
fn penguins() -> RecordBatch {
    read(
        include_str!("../../../avenger-sql/tests/data/penguins.csv"),
        vec![
            Field::new("species", DataType::Utf8, false),
            Field::new("island", DataType::Utf8, false),
            Field::new("bill_length_mm", DataType::Float64, true),
            Field::new("bill_depth_mm", DataType::Float64, true),
            Field::new("flipper_length_mm", DataType::Float64, true),
            Field::new("body_mass_g", DataType::Float64, true),
            Field::new("sex", DataType::Utf8, true),
        ],
        &[
            "bill_length_mm",
            "bill_depth_mm",
            "flipper_length_mm",
            "body_mass_g",
        ],
    )
}

fn axis(field: &str, title: &str) -> EncodingBuilder {
    Encoding::scaled_field(field).scale(Axis::builder().title(title))
}

/// The axis of a measurement whose domain fits its values without zero, which lies far from
/// them.
fn measure(field: &str, title: &str) -> EncodingBuilder {
    let fit = DataDomain::builder().fit(IntervalFit::builder().zero(false));
    Encoding::scaled_field(field).scale(
        Scale::builder()
            .domain(Domain::data(fit))
            .axis(Axis::builder().title(title)),
    )
}

fn legend(field: &str, title: &str) -> EncodingBuilder {
    Encoding::scaled_field(field).scale(Legend::builder().title(title))
}

fn field(column: &str, title: &str) -> TooltipFieldBuilder {
    TooltipField::builder().column(column).title(title)
}

/// A navigation and a tooltip of the nearest point within `radius` pixels.
fn tools(tooltip: TooltipBuilder) -> [ToolBuilder; 2] {
    [ToolBuilder::from(Navigate::builder()), tooltip.into()]
}

fn nearest(radius: f32) -> TooltipBuilder {
    Tooltip::nearest(NearestFind::builder().max_radius(radius))
}

fn movie_ratings(movies: &RecordBatch) -> PlotBuilder {
    let symbols = CartesianLayer::symbol(
        movies.clone(),
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
            .opacity(0.6),
    );
    let tooltip = nearest(12.0)
        .title(TooltipField::builder().column("title"))
        .fields([
            field("genre", "Genre"),
            field("year", "Year").labels(Labels::builder().format("d")),
            field("imdb_rating", "IMDB"),
            field("rotten_tomatoes_rating", "Rotten Tomatoes"),
        ]);
    Plot::cartesian([symbols])
        .id("ratings")
        .title("Movie ratings")
        .tools(tools(tooltip))
}

/// The number of movies of each genre, most first.
fn genre_counts(movies: &RecordBatch) -> RecordBatch {
    let genres = movies.column_by_name("genre").unwrap().as_string::<i32>();
    let mut counts: BTreeMap<&str, i64> = BTreeMap::new();
    for genre in genres.iter().flatten() {
        *counts.entry(genre).or_default() += 1;
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by_key(|(genre, count)| (-count, *genre));
    let schema = Schema::new(vec![
        Field::new("genre", DataType::Utf8, false),
        Field::new("movies", DataType::Int64, false),
    ]);
    let genre: StringArray = counts.iter().map(|(genre, _)| Some(*genre)).collect();
    let count: Int64Array = counts.iter().map(|(_, count)| Some(*count)).collect();
    RecordBatch::try_new(
        Arc::new(schema),
        vec![Arc::new(genre) as ArrayRef, Arc::new(count)],
    )
    .unwrap()
}

fn movie_genres(movies: &RecordBatch) -> PlotBuilder {
    let bars = CartesianLayer::bar(
        genre_counts(movies),
        BarMark::builder()
            .x(axis("movies", "Movies"))
            .y(axis("genre", "Genre")),
    );
    let tooltip = Tooltip::exact()
        .title(TooltipField::builder().column("genre"))
        .fields([field("movies", "Movies")]);
    Plot::cartesian([bars])
        .id("genres")
        .title("Movies by genre")
        .tools([ToolBuilder::from(tooltip)])
}

/// The mean IMDB rating and the number of movies of each year.
fn yearly_ratings(movies: &RecordBatch) -> RecordBatch {
    let years = movies
        .column_by_name("year")
        .unwrap()
        .as_primitive::<Int64Type>();
    let ratings = movies
        .column_by_name("imdb_rating")
        .unwrap()
        .as_primitive::<Float64Type>();
    let mut sums: BTreeMap<i64, (f64, i64)> = BTreeMap::new();
    for (year, rating) in years.values().iter().zip(ratings.values()) {
        let sum = sums.entry(*year).or_default();
        sum.0 += rating;
        sum.1 += 1;
    }
    let schema = Schema::new(vec![
        Field::new("year", DataType::Int64, false),
        Field::new("mean_rating", DataType::Float64, false),
        Field::new("movies", DataType::Int64, false),
    ]);
    let year: Int64Array = sums.keys().map(|year| Some(*year)).collect();
    let mean: Float64Array = sums
        .values()
        .map(|(sum, n)| Some(sum / *n as f64))
        .collect();
    let count: Int64Array = sums.values().map(|(_, n)| Some(*n)).collect();
    RecordBatch::try_new(
        Arc::new(schema),
        vec![Arc::new(year) as ArrayRef, Arc::new(mean), Arc::new(count)],
    )
    .unwrap()
}

fn ratings_by_year(movies: &RecordBatch) -> PlotBuilder {
    let yearly = yearly_ratings(movies);
    let year = || {
        let fit = DataDomain::builder().fit(IntervalFit::builder().zero(false));
        Encoding::scaled_field("year").scale(
            Scale::builder().domain(Domain::data(fit)).axis(
                Axis::builder()
                    .title("Year")
                    .labels(Labels::builder().format("d")),
            ),
        )
    };
    let line = CartesianLayer::line(
        yearly.clone(),
        LineMark::builder()
            .x(year())
            .y(measure("mean_rating", "Mean IMDB rating")),
    );
    let points = CartesianLayer::symbol(
        yearly,
        SymbolMark::builder()
            .x(year())
            .y(measure("mean_rating", "Mean IMDB rating")),
    );
    let tooltip = nearest(16.0)
        .title(
            TooltipField::builder()
                .column("year")
                .labels(Labels::builder().format("d")),
        )
        .fields([
            field("mean_rating", "Mean rating").labels(Labels::builder().format(".2f")),
            field("movies", "Movies"),
        ]);
    Plot::cartesian([line, points])
        .id("years")
        .title("Ratings by year")
        .tools(tools(tooltip))
}

fn penguin_tooltip() -> TooltipBuilder {
    nearest(12.0)
        .title(TooltipField::builder().column("species"))
        .fields([
            field("island", "Island"),
            field("sex", "Sex"),
            field("flipper_length_mm", "Flipper (mm)"),
            field("body_mass_g", "Mass (g)"),
            field("bill_length_mm", "Bill length (mm)"),
            field("bill_depth_mm", "Bill depth (mm)"),
        ])
}

fn penguin_sizes(penguins: &RecordBatch) -> PlotBuilder {
    let symbols = CartesianLayer::symbol(
        penguins.clone(),
        SymbolMark::builder()
            .x(measure("flipper_length_mm", "Flipper length (mm)"))
            .y(measure("body_mass_g", "Body mass (g)"))
            .color(legend("species", "Species"))
            .opacity(0.8),
    );
    Plot::cartesian([symbols])
        .id("penguins")
        .title("Penguin sizes")
        .tools(tools(penguin_tooltip()))
}

/// A panel of the bills of each island's penguins. The panels share their scales, so a pan or a
/// zoom of one moves them all.
fn penguin_islands(penguins: &RecordBatch) -> ConcatBuilder {
    let panels = Node::partitions(penguins, "island", PartitionOrder::Ascending, |island| {
        let symbols = CartesianLayer::symbol(
            island,
            SymbolMark::builder()
                .x(measure("bill_length_mm", "Bill length (mm)"))
                .y(measure("bill_depth_mm", "Bill depth (mm)"))
                .color(legend("species", "Species"))
                .opacity(0.8),
        );
        Ok(Plot::cartesian([symbols])
            .id("island")
            .tools(tools(penguin_tooltip())))
    })
    .expect("the islands partition");
    Concat::new(Direction::Row).facet("island").children(panels)
}

fn iris() -> PlotBuilder {
    let flowers = read(
        include_str!("../../iris-pan-zoom/data/Iris.csv"),
        vec![
            Field::new("Id", DataType::Int64, false),
            Field::new("SepalLengthCm", DataType::Float64, false),
            Field::new("SepalWidthCm", DataType::Float64, false),
            Field::new("PetalLengthCm", DataType::Float64, false),
            Field::new("PetalWidthCm", DataType::Float64, false),
            Field::new("Species", DataType::Utf8, false),
        ],
        &[],
    );
    let symbols = CartesianLayer::symbol(
        flowers,
        SymbolMark::builder()
            .x(axis("PetalLengthCm", "Petal length (cm)"))
            .y(axis("PetalWidthCm", "Petal width (cm)"))
            .color(legend("Species", "Species"))
            .opacity(0.8),
    );
    let tooltip = nearest(12.0)
        .title(TooltipField::builder().column("Species"))
        .fields([
            field("PetalLengthCm", "Petal length (cm)"),
            field("PetalWidthCm", "Petal width (cm)"),
            field("SepalLengthCm", "Sepal length (cm)"),
            field("SepalWidthCm", "Sepal width (cm)"),
        ]);
    Plot::cartesian([symbols])
        .id("iris")
        .title("Iris petals")
        .tools(tools(tooltip))
}
