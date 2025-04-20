use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use avenger_app::{app::{AvengerApp, SceneGraphBuilder}, error::AvengerAppError};
use avenger_eventstream::{
    manager::EventStreamHandler,
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{EventStreamConfig, UpdateStatus},
    window::WindowEvent,
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_lang::{ast::AvengerFile, parser::AvengerParser, task_graph::runtime::TaskGraphRuntime};
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_winit_wgpu::{FileWatcher, WinitWgpuAvengerApp};

use log::{error, info};
use winit::event_loop::EventLoop;

const DEFAULT_FILE_NAME: &str = "app.avgr";

#[derive(Clone)]
pub struct ChartState {
    pub ast: Arc<Mutex<Option<AvengerFile>>>,
    pub default_ast: AvengerFile,
    pub runtime: Arc<TaskGraphRuntime>,
    pub file_path: PathBuf,
}

impl ChartState {
    pub fn new(file_path: PathBuf) -> Self {
        // Default avenger file content
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
            file_path,
        }
    }

    pub fn update_from_file(&self, content: &str) -> Result<(), String> {
        let parsed_file = AvengerParser::parse_single_file(content)
            .map_err(|e| format!("Failed to parse file: {}", e))?;
        
        let mut ast = self.ast.lock().unwrap();
        *ast = Some(parsed_file);
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct LangSceneGraphBuilder;

#[async_trait]
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

// Event handler for file changes
struct FileChangeHandler;

#[async_trait]
impl EventStreamHandler<ChartState> for FileChangeHandler {
    async fn handle(
        &self,
        event: &SceneGraphEvent,
        state: &mut ChartState,
        _rtree: &SceneGraphRTree,
    ) -> UpdateStatus {
        // Handle file change events
        if let SceneGraphEvent::FileChanged(e) = event {
            // Check if this is the file we're watching
            if Path::new(&e.file_path) == state.file_path {
                // Read file content when event is received
                match fs::read_to_string(&state.file_path) {
                    Ok(content) => {
                        match state.update_from_file(&content) {
                            Ok(_) => {
                                info!("Updated app from file: {:?}", e.file_path);
                                // Return true to indicate that a render is needed
                                return UpdateStatus {
                                    rerender: true,
                                    rebuild_geometry: true,
                                };
                            }
                            Err(err) => {
                                error!("Failed to update AST: {}", err);
                            }
                        }
                    }
                    Err(err) => {
                        error!("Error reading file {:?}: {}", e.file_path, err);
                    }
                }
            }
        }

        // No update needed by default
        UpdateStatus::default()
    }
}

fn ensure_file_exists(path: &Path) -> Result<String, std::io::Error> {
    if !path.exists() {
        let default_content = r#"
width := 440;
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
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        
        // Create the file with default content
        fs::write(path, default_content)?;
        info!("Created default file at: {}", path.display());
        Ok(default_content.to_string())
    } else {
        // Read existing file
        let content = fs::read_to_string(path)?;
        Ok(content)
    }
}

fn main() -> Result<(), AvengerAppError> {
    // Setup logger
    env_logger::init();

    // Parse command line arguments
    let args: Vec<String> = std::env::args().collect();
    
    // Get file path from command line or use default
    let file_path = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        PathBuf::from(DEFAULT_FILE_NAME)
    };

    // Ensure file exists or create with default content
    let content = match ensure_file_exists(&file_path) {
        Ok(content) => content,
        Err(e) => {
            error!("Failed to ensure file exists: {}", e);
            return Err(AvengerAppError::InternalError(e.to_string()));
        }
    };
    
    // Canonicalize file path after ensuring it exists
    let file_path = fs::canonicalize(&file_path).map_err(|e| {
        AvengerAppError::InternalError(format!("Failed to canonicalize path: {}", e))
    })?;
    
    // Setup tokio runtime
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build tokio runtime");

    // Create state and initial AST
    let mut chart_state = ChartState::new(file_path.clone());
    
    // Parse initial file
    if let Err(e) = chart_state.update_from_file(&content) {
        error!("Failed to parse initial file: {}", e);
        // Fall back to default AST
    }
    
    // Create file change event handler and config
    let file_handler = Arc::new(FileChangeHandler);
    let file_handler_config = EventStreamConfig {
        types: vec![SceneGraphEventType::FileChanged(file_path.clone())],
        consume: false,
        ..Default::default()
    };
    
    // Create app with chart state and file change handler
    let avenger_app = tokio_runtime.block_on(AvengerApp::try_new(
        chart_state,
        Arc::new(LangSceneGraphBuilder),
        vec![(file_handler_config, file_handler)],
    )).expect("Failed to create initial avenger app");

    // // Create event loop with AvengerWindowEvent as custom event type
    // let event_loop = EventLoop::<WindowEvent>::with_user_event().build()
    //     .expect("Failed to build event loop");
    // let event_proxy = event_loop.create_proxy();
    
    // // Create and initialize file watcher
    // let mut file_watcher = FileWatcher::new(event_proxy.clone());
    
    // // Watch the target file for changes
    // if let Err(e) = file_watcher.watch(&file_path) {
    //     error!("Failed to watch file {}: {}", file_path.display(), e);
    // } else {
    //     info!("Watching file for changes: {}", file_path.display());
    // }
    
    // Create app with file watcher
    let (mut app, event_loop) = WinitWgpuAvengerApp::new_and_event_loop(avenger_app, 2.0, tokio_runtime);

    // Run the app
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");

    Ok(())
}