//! The chart of the cross filter, its app at the window's size, and a box that fades the movies
//! of the other plots outside it.

use std::{sync::Mutex, time::Duration};

use avenger_common::time::Instant;
use avenger_eventstream::window::{
    ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput,
};
use avenger_scenegraph::marks::mark::SceneMark;

use super::*;

/// The opacities of the symbols of the plot `name` of a scene.
fn opacities(marks: &[SceneMark], name: &str) -> Vec<f32> {
    marks
        .iter()
        .flat_map(|mark| match mark {
            SceneMark::Group(group) if group.name == name => (group.marks.iter())
                .filter_map(|mark| match mark {
                    SceneMark::Symbol(symbol) => Some(
                        (symbol.fill.as_vec(symbol.len as usize, None).into_iter())
                            .map(|fill| match fill {
                                avenger_color::ColorOrGradient::Color(color) => color[3],
                                _ => 1.0,
                            })
                            .collect::<Vec<_>>(),
                    ),
                    _ => None,
                })
                .flatten()
                .collect(),
            SceneMark::Group(group) => opacities(&group.marks, name),
            _ => Vec::new(),
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_box_on_the_ratings_fades_the_movies_of_the_other_plots_outside_it() {
    let settled = Arc::new(Mutex::new(Vec::new()));
    let heard = settled.clone();
    let mut app = ChartApp::new(chart(&movies()), cascade(), engine())
        .size(SIZE)
        .reports(Reports::Settled)
        .on_change(move |change| heard.lock().unwrap().push(owners(change)))
        .build()
        .await
        .unwrap()
        .with_runtime(tokio::runtime::Handle::current())
        .unwrap();
    let scene = &app.scenes()[0];
    assert_eq!([scene.width, scene.height], SIZE);
    let before = opacities(&app.scenes()[0].marks, "imdb_by_year");
    assert!(before.iter().all(|opacity| *opacity > 0.5), "{before:?}");

    // A box over the middle of the plot of the ratings.
    let [x, y, width, height] = app.state().plots()[0].rect;
    let from = [x + width * 0.4, y + height * 0.3];
    let to = [x + width * 0.7, y + height * 0.6];
    let events = [
        WindowEvent::CursorMoved(WindowCursorMoved { position: from }),
        WindowEvent::MouseInput(WindowMouseInput {
            state: ElementState::Pressed,
            button: MouseButton::Left,
        }),
        WindowEvent::CursorMoved(WindowCursorMoved { position: to }),
        WindowEvent::MouseInput(WindowMouseInput {
            state: ElementState::Released,
            button: MouseButton::Left,
        }),
    ];
    for event in events {
        std::thread::sleep(Duration::from_millis(10));
        app.update(&event, Instant::now()).await;
    }
    let after = opacities(&app.scenes()[0].marks, "imdb_by_year");
    assert!(after.iter().any(|opacity| *opacity < 0.1), "{after:?}");
    assert!(after.iter().any(|opacity| *opacity > 0.5), "{after:?}");
    // Its own plot fades nothing.
    let own = opacities(&app.scenes()[0].marks, "ratings");
    assert!(own.iter().all(|opacity| *opacity > 0.5));
    // The app reports the settled box once.
    let settled = settled.lock().unwrap();
    assert_eq!(
        settled.as_slice(),
        [vec![(FILTER.to_string(), vec!["ratings".to_string()])]]
    );
}
