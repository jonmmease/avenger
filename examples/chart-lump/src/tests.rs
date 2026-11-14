//! The lumped genres from avenger-sql, a click on the Other bar, which selects every genre outside
//! the kept ones and NULL and grays the movies of the kept genres and of no genre, and a click on
//! the NULL bar, which selects the movies without a genre.

use std::time::Duration;

use arrow::{
    array::{Array, ArrayRef, AsArray, Float64Array},
    compute::cast,
    datatypes::Int64Type,
};
use avenger_common::time::Instant;
use avenger_eventstream::window::{
    ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput,
};
use avenger_scenegraph::marks::mark::SceneMark;

use super::*;

/// The tables of the chart, through avenger-sql.
fn lumped() -> Tables {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(tables(movies()))
        .unwrap()
}

/// The row of the Other group among the counts of the genres.
fn other_row(genres: &RecordBatch) -> usize {
    let lumped = genres.column_by_name("lumped").unwrap().as_struct();
    (0..genres.num_rows())
        .find(|&row| lumped.column(1).is_valid(row))
        .expect("an Other group")
}

#[test]
fn the_lump_keeps_the_five_genres_with_the_most_movies() {
    let tables = lumped();
    let lumped = tables.genres.column_by_name("lumped").unwrap();
    assert!(
        avenger_chart::lump::is_lump(lumped.data_type()),
        "{}",
        lumped.data_type()
    );
    // The kept genres, the Other group, and the NULL category of the movies without a genre.
    assert_eq!(tables.genres.num_rows(), KEPT + 2);
    let nulls = avenger_chart::lump::nulls(lumped).unwrap();
    assert_eq!(nulls.true_count(), 1);
    let lumped = lumped.as_struct();
    let other = other_row(&tables.genres);
    assert_eq!(lumped.column(1).as_string::<i32>().value(other), "Other");
    // The Other group holds every movie outside the kept genres with a genre, and names its number
    // of genres.
    let movies = tables
        .genres
        .column_by_name("movies")
        .unwrap()
        .as_primitive::<Int64Type>();
    let total: i64 = movies.values().iter().sum();
    assert_eq!(total as usize, tables.movies.num_rows());
    let genres = lumped.column(3).as_primitive::<Int64Type>();
    assert!(genres.value(other) > 1, "{genres:?}");
    let weights = lumped
        .column(2)
        .as_primitive::<arrow::datatypes::Float64Type>();
    assert_eq!(weights.value(other), movies.value(other) as f64);
}

/// The fills of the symbols of the plot `name` of a scene.
fn fills(marks: &[SceneMark], name: &str) -> Vec<[f32; 4]> {
    marks
        .iter()
        .flat_map(|mark| match mark {
            SceneMark::Group(group) if group.name == name => (group.marks.iter())
                .filter_map(|mark| match mark {
                    SceneMark::Symbol(symbol) => Some(
                        (symbol.fill.as_vec(symbol.len as usize, None).into_iter())
                            .map(|fill| match fill {
                                avenger_color::ColorOrGradient::Color(color) => color,
                                _ => [0.0; 4],
                            })
                            .collect::<Vec<_>>(),
                    ),
                    _ => None,
                })
                .flatten()
                .collect(),
            SceneMark::Group(group) => fills(&group.marks, name),
            _ => Vec::new(),
        })
        .collect()
}

fn light_gray(fill: &[f32; 4]) -> bool {
    fill[..3]
        .iter()
        .all(|channel| (channel - 211.0 / 255.0).abs() < 1e-3)
}

/// The app of the chart of `tables`.
async fn app(tables: &Tables) -> avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState> {
    let app = ChartApp::new(chart(tables), cascade(), engine())
        .size(SIZE)
        .build()
        .await
        .unwrap()
        .with_runtime(tokio::runtime::Handle::current())
        .unwrap();
    assert!(
        app.state().warnings().is_empty(),
        "{:?}",
        app.state().warnings()
    );
    app
}

/// A click on the middle of the bar of `row` of the counts of the genres.
async fn click_bar(
    app: &mut avenger_app::app::AvengerApp<avenger_chart_app::ChartAppState>,
    tables: &Tables,
    row: usize,
) {
    let plot = &app.state().plots()[0];
    let key = avenger_chart::lump::keys(tables.genres.column_by_name("lumped").unwrap())
        .unwrap()
        .slice(row, 1);
    let movies = cast(
        tables.genres.column_by_name("movies").unwrap(),
        &DataType::Float64,
    )
    .unwrap();
    let half: ArrayRef = Arc::new(Float64Array::from(vec![
        movies
            .as_primitive::<arrow::datatypes::Float64Type>()
            .value(row)
            / 2.0,
    ]));
    let [x, y] = plot.coords.pixels([&key, &half]).unwrap();
    let at = [plot.rect[0] + x.value(0), plot.rect[1] + y.value(0)];
    let events = [
        WindowEvent::CursorMoved(WindowCursorMoved { position: at }),
        WindowEvent::MouseInput(WindowMouseInput {
            state: ElementState::Pressed,
            button: MouseButton::Left,
        }),
        WindowEvent::MouseInput(WindowMouseInput {
            state: ElementState::Released,
            button: MouseButton::Left,
        }),
    ];
    for event in events {
        std::thread::sleep(Duration::from_millis(10));
        app.update(&event, Instant::now()).await;
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_click_on_the_other_bar_grays_the_movies_of_the_kept_genres() {
    let tables = tokio::task::spawn_blocking(lumped).await.unwrap();
    let mut app = app(&tables).await;
    let before = fills(&app.scenes()[0].marks, "ratings");
    assert!(!before.iter().any(light_gray));
    click_bar(&mut app, &tables, other_row(&tables.genres)).await;

    // The selection is the complement of the five kept genres and NULL.
    let clauses = &app.state().evaluator().selections()[GENRES].clauses;
    match &clauses[..] {
        [clause] => match &clause.conditions[..] {
            [avenger_selection::Condition::Set {
                columns,
                values,
                complement: true,
                ..
            }] => {
                assert_eq!(columns, &["genre".to_string()]);
                assert_eq!(values[0].len(), KEPT + 1);
                assert_eq!(values[0].null_count(), 1);
            }
            conditions => panic!("a set, and not {conditions:?}"),
        },
        clauses => panic!("one clause, and not {clauses:?}"),
    }

    // The movies of the kept genres and of no genre are gray, and the movies of the Other group
    // keep their color.
    let after = fills(&app.scenes()[0].marks, "ratings");
    let lumped = tables.movies.column_by_name("lumped").unwrap().as_struct();
    let others = lumped.column(1);
    assert_eq!(after.len(), tables.movies.num_rows());
    for (row, fill) in after.iter().enumerate() {
        assert_eq!(light_gray(fill), others.is_null(row), "row {row}: {fill:?}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_click_on_the_null_bar_selects_the_movies_without_a_genre() {
    let tables = tokio::task::spawn_blocking(lumped).await.unwrap();
    let mut app = app(&tables).await;
    let lumped = tables.genres.column_by_name("lumped").unwrap();
    let nulls = avenger_chart::lump::nulls(lumped).unwrap();
    let null = (0..nulls.len()).find(|&row| nulls.value(row)).unwrap();
    click_bar(&mut app, &tables, null).await;

    // Only the movies without a genre keep their color.
    let after = fills(&app.scenes()[0].marks, "ratings");
    let genre = tables.movies.column_by_name("genre").unwrap();
    assert_eq!(after.len(), tables.movies.num_rows());
    for (row, fill) in after.iter().enumerate() {
        assert_eq!(light_gray(fill), genre.is_valid(row), "row {row}: {fill:?}");
    }
}
