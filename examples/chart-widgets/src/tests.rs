//! The widgets of the movies: where they stand at two window sizes, and a click on a genre that
//! fades its movies and reports its clause.

use std::{sync::Mutex, time::Duration};

use avenger_chart_app::ChartAppState;
use avenger_common::time::Instant;
use avenger_eventstream::window::{
    ElementState, MouseButton, WindowCursorMoved, WindowEvent, WindowMouseInput,
};
use avenger_scenegraph::{marks::mark::SceneMark, scene_graph::SceneGraph};

use super::*;

type App = avenger_app::app::AvengerApp<ChartAppState>;

/// The app of the chart at `size`, with each settled change's clauses in `heard`.
async fn app(size: [f32; 2], heard: Arc<Mutex<Vec<Vec<String>>>>) -> App {
    ChartApp::new(chart(&movies()), cascade(), engine())
        .size(size)
        .reports(Reports::Settled)
        .on_change(move |change| heard.lock().unwrap().push(clauses(change)))
        .build()
        .await
        .unwrap()
        .with_runtime(tokio::runtime::Handle::current())
        .unwrap()
}

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

/// The position of the text `text` among `marks`, inside groups at `origin`.
fn text_in(marks: &[SceneMark], origin: [f32; 2], text: &str) -> Option<[f32; 2]> {
    marks.iter().find_map(|mark| match mark {
        SceneMark::Group(group) => text_in(
            &group.marks,
            [origin[0] + group.origin[0], origin[1] + group.origin[1]],
            text,
        ),
        SceneMark::Text(mark) if mark.text.as_vec(1, None)[0] == text => {
            let [x, y] = [mark.x.as_vec(1, None)[0], mark.y.as_vec(1, None)[0]];
            Some([origin[0] + x, origin[1] + y])
        }
        _ => None,
    })
}

/// The position of the text `text` in the scene of the widgets, the app's highest.
fn text_at(app: &App, text: &str) -> Option<[f32; 2]> {
    let scenes = app.scenes();
    let scene: &SceneGraph = &scenes[scenes.len() - 1];
    text_in(&scene.marks, scene.origin, text)
}

/// The rectangle of the widget `key`.
fn rect(app: &App, key: &str) -> [f32; 4] {
    let widget = (app.state().widgets().iter())
        .find(|widget| widget.key == key)
        .unwrap_or_else(|| panic!("the widget {key}"));
    widget.rects[0]
}

#[tokio::test(flavor = "multi_thread")]
async fn the_sliders_share_a_row_above_the_plot_until_the_window_narrows() {
    let wide = app(SIZE, Arc::default()).await;
    let plot = wide.state().plots()[0].rect;
    let (years, rating, genres) = (
        rect(&wide, "years"),
        rect(&wide, "rating"),
        rect(&wide, "genres"),
    );
    assert_eq!(years[1], rating[1], "{years:?} {rating:?}");
    assert!(rating[0] >= years[0] + years[2], "{years:?} {rating:?}");
    assert!(years[1] + years[3] <= plot[1], "{years:?} {plot:?}");
    // The genres stand right of the plot.
    assert!(genres[0] >= plot[0] + plot[2], "{genres:?} {plot:?}");

    // A narrow window puts the rating on a row above the years.
    let narrow = app([480.0, 640.0], Arc::default()).await;
    let (years, rating) = (rect(&narrow, "years"), rect(&narrow, "rating"));
    assert!(rating[1] + rating[3] <= years[1], "{years:?} {rating:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_click_on_a_genre_fades_its_movies_and_reports_its_clause() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let mut app = app(SIZE, heard.clone()).await;
    let before = opacities(&app.scenes()[0].marks, "movies");
    assert!(before.iter().all(|opacity| *opacity > 0.5), "{before:?}");

    let at = text_at(&app, "Drama").expect("the label of Drama");
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
    // The dramas fade, and the movies of the other genres stay.
    let after = opacities(&app.scenes()[0].marks, "movies");
    assert!(after.iter().any(|opacity| *opacity < 0.1), "{after:?}");
    assert!(after.iter().any(|opacity| *opacity > 0.5), "{after:?}");
    let heard = heard.lock().unwrap();
    assert_eq!(
        heard.as_slice(),
        [vec![
            "genres: genre in [Action, Adventure, Comedy, Horror, Thriller/Suspense]".to_string()
        ]]
    );
}
