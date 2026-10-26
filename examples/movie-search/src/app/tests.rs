//! Matching titles, and typing a query into the search box.

use super::*;
use avenger_common::time::Instant;
use avenger_eventstream::window::{
    ElementState, Key, MouseButton, WindowCursorMoved, WindowEvent, WindowKeyboardInput,
    WindowMouseInput,
};

/// The titles of the movies that match the query.
fn matching(state: &SearchState) -> Vec<String> {
    let titles = state
        .movies
        .column_by_name(TITLE)
        .unwrap()
        .as_string::<i32>();
    (titles.iter().zip(state.matches.iter()))
        .filter(|(_, matches)| *matches == Some(true))
        .map(|(title, _)| title.unwrap().to_string())
        .collect()
}

#[test]
fn matching_ignores_case_and_finds_the_query_anywhere_in_a_title() {
    let mut state = SearchState::new();
    assert_eq!(state.matches.true_count(), state.movies.num_rows());
    assert_eq!(state.heading(), "2,259 movies");

    state.search("ALIEN".to_string());
    assert_eq!(
        matching(&state),
        [
            "Aliens",
            "Alien³",
            "My Stepmother Is an Alien",
            "Alien",
            "How to Lose Friends & Alienate People",
            "Monsters vs. Aliens",
        ]
    );
    assert_eq!(state.heading(), "6 of 2,259 movies match");
    state.search("godfather".to_string());
    assert_eq!(state.heading(), "1 of 2,259 movies matches");
}

/// The number of points that the mark named `name` draws.
fn points(app: &AvengerApp<SearchState>, name: &str) -> u32 {
    fn find(marks: &[SceneMark], name: &str) -> Option<u32> {
        marks.iter().find_map(|mark| match mark {
            SceneMark::Symbol(symbol) if symbol.name == name => Some(symbol.len),
            SceneMark::Group(group) => find(&group.marks, name),
            _ => None,
        })
    }
    find(&app.scenes()[0].marks, name).unwrap()
}

#[test]
fn typing_in_the_search_box_highlights_the_matches() {
    pollster::block_on(async {
        let mut app = builder().build().await.unwrap();
        let search = app.state().controls[SEARCH];
        let field = [search.x + 20.0, search.y + search.height - 8.0];
        let mouse = |state| {
            WindowEvent::MouseInput(WindowMouseInput {
                state,
                button: MouseButton::Left,
            })
        };
        let mut events = vec![
            WindowEvent::CursorMoved(WindowCursorMoved { position: field }),
            mouse(ElementState::Pressed),
            mouse(ElementState::Released),
        ];
        for c in "Alien".chars() {
            events.extend(
                [
                    (ElementState::Pressed, Some(c.to_string())),
                    (ElementState::Released, None),
                ]
                .map(|(state, text)| {
                    WindowEvent::KeyboardInput(WindowKeyboardInput {
                        key: Key::Character(c),
                        state,
                        text,
                    })
                }),
            );
        }
        for event in events {
            app.update(&event, Instant::now()).await;
        }
        assert_eq!(app.state().query, "Alien");
        assert_eq!((points(&app, MATCHES), points(&app, OTHERS)), (6, 2253));
    });
}
