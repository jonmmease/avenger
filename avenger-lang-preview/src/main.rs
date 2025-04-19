use std::sync::{Arc, Mutex};

use avenger_app::{app::{AvengerApp, SceneGraphBuilder}, error::AvengerAppError};
use avenger_lang::{ast::AvengerFile, parser::AvengerParser, task_graph::runtime::TaskGraphRuntime};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_winit_wgpu::WinitWgpuAvengerApp;
use winit::event_loop::EventLoop;

#[derive(Clone)]
pub struct ChartState {
    pub ast: Arc<Mutex<Option<AvengerFile>>>,
    pub default_ast: AvengerFile,
    pub runtime: Arc<TaskGraphRuntime>,
}

impl ChartState {
    pub fn new() -> Self {

        // let parser = AvengerParser::new();
        let default_file_str = r#"
width := 840;
height := 440;

dataset data_0: SELECT * FROM (VALUES 
        (1, 'red'),
        (2, 'green'),
        (3, 'blue')
    ) foo("a", "b");

comp g1: Group {
    x := 20;
    y := 20;

    comp mark1: Rect {
        data := SELECT * FROM @data_0;
        x2 := @x + 10;
        x := "a" * 100;
        y := "a" * 10 + 10;
        y2 := 0;
        fill := "b";
        stroke_width := 4;
        stroke := 'black';

        clip := false;
        zindex := 1 + 2;
    }
}
        "#;
        let default_ast = AvengerParser::parse_single_file(
            default_file_str
        ).expect("Failed to parse default file");

        let runtime = TaskGraphRuntime::new();

        Self {
            ast: Arc::new(Mutex::new(None)),
            default_ast,
            runtime: Arc::new(runtime),
        }
    }
}


#[derive(Clone, Debug)]
struct LangSceneGraphBuilder;

#[async_trait::async_trait]
impl SceneGraphBuilder<ChartState> for LangSceneGraphBuilder {
    async fn build(&self, state: &mut ChartState) -> Result<SceneGraph, AvengerAppError> {
        // Clone the AST to avoid reference issues
        let ast = {
            let guard = state.ast.lock().unwrap();
            if let Some(ast) = &*guard {
                ast.clone()
            } else {
                state.default_ast.clone()
            }
        };

        let scene_graph = state.runtime.evaluate_file(&ast).await.map_err(
            |e| AvengerAppError::InternalError(e.to_string())
        )?;
        Ok(scene_graph)
    }
}



fn main() -> Result<(), AvengerAppError> {
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .expect("Failed to build tokio runtime");

    let avenger_app = tokio_runtime.block_on(AvengerApp::try_new(
        ChartState::new(),
        Arc::new(LangSceneGraphBuilder),
        vec![]
    )).expect("Failed to create avenger app");

    let mut app = WinitWgpuAvengerApp::new(avenger_app, 2.0, tokio_runtime);

    let event_loop = EventLoop::new().expect("Failed to build event loop");
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");

    Ok(())
}
