//! The answers of the server: a chart's bytes for valid choices, and status 400 otherwise.

use avenger_chart::Chart;
use chart_server::Movies;

fn answer(body: &str) -> Result<Vec<u8>, chart_server::Rejection> {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(Movies::new().chart(body.as_bytes()))
}

#[test]
fn valid_choices_answer_with_a_chart() {
    let bytes = answer(r#"{"genre": "Drama", "from": 1990, "to": 2000}"#).unwrap();
    let chart = Chart::from_bytes(bytes).unwrap();
    assert!(chart.to_bytes().is_ok());
    assert!(answer(r#"{"genre": null, "from": 1900, "to": 2020}"#).is_ok());
}

#[test]
fn invalid_choices_answer_with_status_400() {
    for body in [
        "not json",
        r#"{"genre": "Drama", "from": 2000, "to": 1990}"#,
        r#"{"genre": "No such genre", "from": 1900, "to": 2020}"#,
    ] {
        let rejection = answer(body).unwrap_err();
        assert_eq!(rejection.status, 400, "{body}: {}", rejection.message);
    }
}

#[test]
fn the_genres_are_in_the_order_of_their_names() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let genres = runtime.block_on(Movies::new().genres()).unwrap();
    assert!(genres.contains(&"Drama".to_string()));
    assert!(genres.windows(2).all(|pair| pair[0] < pair[1]));
}
