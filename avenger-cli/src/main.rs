use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use avenger_app::{app::{AvengerApp, SceneGraphBuilder}, error::AvengerAppError};
use avenger_common::canvas::CanvasDimensions;
use avenger_eventstream::scene::SceneFileChangedEvent;
use avenger_eventstream::stream::EventStreamFilter;
use avenger_eventstream::window::Key;
use avenger_eventstream::{
    manager::EventStreamHandler,
    scene::{SceneGraphEvent, SceneGraphEventType},
    stream::{EventStreamConfig, UpdateStatus},
    window::WindowEvent,
};
use avenger_geometry::rtree::SceneGraphRTree;
use avenger_lang2::error::AvengerLangError;
use avenger_lang2::imports::load_main_component_file;
use avenger_lang2::{ast::AvengerFile, parser::AvengerParser};
use avenger_runtime::cache::RuntimeCacheConfig;
use avenger_runtime::runtime::TaskGraphRuntime;
use avenger_scenegraph::scene_graph::SceneGraph;
use avenger_wgpu::canvas::{Canvas, CanvasConfig, PngCanvas};
use avenger_winit_wgpu::{FileWatcher, WinitWgpuAvengerApp};
use walkdir::WalkDir;

use clap::{Parser, Subcommand};
use log::{error, info};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use winit::event_loop::EventLoop;

const DEFAULT_FILE_NAME: &str = "App.avgr";

/// Avenger CLI for visualization preview and rendering
#[derive(Parser)]
#[command(author, version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Launch an interactive preview window for an Avenger language file
    Preview {
        /// Path to the Avenger language file (.avgr)
        #[arg(default_value = DEFAULT_FILE_NAME)]
        file_path: String,
    },
    
    /// Save an Avenger language file to a PNG image without launching a preview window
    Save {
        /// Path to the Avenger language file (.avgr)
        file_path: String,
        
        /// Output path for the PNG image (defaults to input file with .png extension)
        #[arg(short, long)]
        output: Option<String>,
        
        /// Scale factor for the output image (default: 2.0)
        #[arg(short, long, default_value_t = 2.0)]
        scale: f32,
    },
}

#[derive(Clone)]
pub struct ChartState {
    pub ast: Arc<Mutex<Option<AvengerFile>>>,
    pub default_ast: AvengerFile,
    pub runtime: Arc<TaskGraphRuntime>,
    pub main_component_path: PathBuf,
    pub current_scene: Option<SceneGraph>,
}

impl ChartState {
    pub fn new(file_path: PathBuf) -> Self {
        // Default avenger file content
        let default_file_str = r#"
width := 234;
height := 234;
        "#;
        let mut parser = AvengerParser::new(default_file_str, "App", ".").expect("Failed to create parser");
        let default_ast = parser.parse().expect("Failed to parse default file");
        let runtime = TaskGraphRuntime::new(RuntimeCacheConfig::default());

        Self {
            ast: Arc::new(Mutex::new(None)),
            default_ast,
            runtime: Arc::new(runtime),
            main_component_path: file_path,
            current_scene: None,
        }
    }

    pub fn update_from_file(&self) -> Result<(), AvengerLangError> {
        let file_ast = load_main_component_file(self.main_component_path.clone(), true)?;
        let mut ast = self.ast.lock().unwrap();
        *ast = Some(file_ast);
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

        let scene_graph = state.runtime.clone().evaluate_file(&ast).await.map_err(
            |e| AvengerAppError::InternalError(e.to_string())
        )?;
        state.current_scene = Some(scene_graph.clone());
        Ok(scene_graph)
    }
}


struct SavePngTask {
    scene_graph: SceneGraph,
    file_path: PathBuf,
    scale: f32,
}


#[derive(Clone)]
pub struct SavePngHandler {
    sender: UnboundedSender<SavePngTask>,
}

impl SavePngHandler {
    pub fn new() -> Self {
        let (sender, mut recv) = unbounded_channel::<SavePngTask>();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        std::thread::spawn(move || {
            let local = tokio::task::LocalSet::new();

            local.spawn_local(async move {
                while let Some(new_task) = recv.recv().await {
                    tokio::task::spawn_local(save_png(new_task));
                }
            });

            // This will return once all senders are dropped and all
            // spawned tasks have returned.
            rt.block_on(local);
        });
    
        Self { sender }
    }
}

async fn save_png(task: SavePngTask) {
    let mut canvas = PngCanvas::new(
        CanvasDimensions {
            size: [task.scene_graph.width, task.scene_graph.height],
            scale: task.scale,
        },
        CanvasConfig::default(),
    )
    .await.expect("Failed to create canvas");

    canvas.set_scene(&task.scene_graph).expect("Failed to set scene");
    let generated_image = canvas.render().await.expect("Failed to render scene");
    generated_image.save(&task.file_path).expect("Failed to save PNG");
    
    info!("Saved PNG to {}", task.file_path.display());
}


#[async_trait]
impl EventStreamHandler<ChartState> for SavePngHandler {
    async fn handle(&self, _event: &SceneGraphEvent, state: &mut ChartState, _rtree: &SceneGraphRTree) -> UpdateStatus {
        if let Some(scene_graph) = &state.current_scene {
            // Ignore send errors if receiver has been dropped
            let file_path = state.main_component_path.clone().with_extension("png");
            let scale = 2.0;
            let _ = self.sender.send(SavePngTask {
                scene_graph: scene_graph.clone(),
                file_path,
                scale,
            });
        }
        UpdateStatus::default()
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
        if let SceneGraphEvent::FileChanged(SceneFileChangedEvent { file_path, .. }) = event {
            println!("file changed: {}", file_path.to_string_lossy());
            if let Err(e) = state.update_from_file() {
                error!("Failed to update AST: {}", e);
                UpdateStatus::default()
            } else {
                UpdateStatus {
                    rerender: true,
                    rebuild_geometry: true,
                }
            }
        } else {
            UpdateStatus::default()
        }
    }
}

fn ensure_file_exists(path: &Path) -> Result<(), std::io::Error> {
    let parent = path.parent().unwrap();

    // Create parent directory if it doesn't exist
    if !parent.exists() {
        fs::create_dir_all(parent)?;
    }

    // Write file if it doesn't exist
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
    }
    Ok(())
}

/// Parse and render an Avenger file to a PNG, useful for batch processing or headless environments
async fn render_to_png(input_path: &Path, output_path: &Path, scale: f32) -> Result<(), AvengerAppError> {
    // Ensure file exists and read it
    if input_path.is_dir() {
        return Err(AvengerAppError::InternalError(
            format!("Input path is a directory: {}", input_path.display())
        ));
    }

    let input_path = input_path.canonicalize().map_err(|e| {
        AvengerAppError::InternalError(format!("Failed to canonicalize path: {}", e))
    })?;

    let file_ast = load_main_component_file(input_path, true).map_err(|e| {
        AvengerAppError::InternalError(format!("Failed to load file: {}", e))
    })?;

    // Create runtime and evaluate file
    let runtime = Arc::new(TaskGraphRuntime::new(RuntimeCacheConfig::default()));

    let scene_graph = runtime.evaluate_file(&file_ast).await.map_err(|e| 
        AvengerAppError::InternalError(format!("Failed to evaluate file: {}", e))
    )?;
    
    // Create canvas and render to PNG
    let mut canvas = PngCanvas::new(
        CanvasDimensions {
            size: [scene_graph.width, scene_graph.height],
            scale,
        },
        CanvasConfig::default(),
    )
    .await.expect("Failed to create canvas");

    canvas.set_scene(&scene_graph).expect("Failed to set scene");
    let generated_image = canvas.render().await.expect("Failed to render scene");
    
    // Ensure the output directory exists
    if let Some(parent) = output_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(|e| 
                AvengerAppError::InternalError(format!("Failed to create output directory: {}", e))
            )?;
        }
    }
    
    // Save the image
    generated_image.save(output_path).map_err(|e| 
        AvengerAppError::InternalError(format!("Failed to save PNG: {}", e))
    )?;
    
    info!("Saved PNG to {}", output_path.display());
    
    Ok(())
}

/// Launch the interactive preview window
fn run_preview(file_path: &str) -> Result<(), AvengerAppError> {
    // Convert string path to PathBuf
    let file_path = PathBuf::from(file_path);

    // Check if the path is a directory
    if file_path.is_dir() {
        return Err(AvengerAppError::InternalError(
            format!("Cannot preview a directory: {}", file_path.display())
        ));
    }

    // Canonicalize file path after ensuring it exists
    let file_path = fs::canonicalize(&file_path).map_err(|e| {
        AvengerAppError::InternalError(format!("Failed to canonicalize path: {}", e))
    })?;

    // Ensure file exists or create with default content
    ensure_file_exists(&file_path).map_err(|e| {
        AvengerAppError::InternalError(format!("Failed to ensure file exists: {}", e))
    })?;
    
    // Setup tokio runtime
    let tokio_runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("Failed to build tokio runtime");
    
    // Create state and initial AST
    let chart_state = ChartState::new(file_path.clone());
    
    // Parse initial file
    if let Err(e) = chart_state.update_from_file() {
        error!("Failed to parse initial file: {}", e);
        // Fall back to default AST
    }
    
    // Create file change event handler and config
    let file_handler = Arc::new(FileChangeHandler);

    // Gather all *.avgr files in the directory recursively
    let dir_path = file_path.parent().unwrap();

    let file_event_types = WalkDir::new(dir_path)
        .into_iter()
        .filter_map(|entry| {
            if let Ok(entry) = entry {
                if entry.path().is_file() && entry.path().extension() == Some(OsStr::new("avgr")) {
                    let file_path = entry.path().to_path_buf();
                    println!("watch: {}", file_path.display());
                    Some(SceneGraphEventType::FileChanged(file_path))
                } else {
                    None
                }
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let file_handler_config = EventStreamConfig {
        types: file_event_types,
        consume: false,
        ..Default::default()
    };

    let save_png_handler = Arc::new(SavePngHandler::new());
    let save_png_handler_config = EventStreamConfig {
        types: vec![SceneGraphEventType::KeyPress],
        consume: false,
        filter: Some(vec![EventStreamFilter(Arc::new(|event| {
            if let SceneGraphEvent::KeyPress(e) = event {
                e.key == Key::Character('s') && (e.modifiers.control || e.modifiers.meta)
            } else {
                false
            }
        }))]),
        ..Default::default()
    };

    // Create app with chart state and file change handler
    let avenger_app = tokio_runtime.block_on(AvengerApp::try_new(
        chart_state,
        Arc::new(LangSceneGraphBuilder),
        vec![
            (file_handler_config, file_handler),
            (save_png_handler_config, save_png_handler),
        ],
    )).expect("Failed to create initial avenger app");
    
    // Create app with file watcher
    let (mut app, event_loop) = WinitWgpuAvengerApp::new_and_event_loop(avenger_app, 2.0, tokio_runtime);

    // Run the app
    event_loop
        .run_app(&mut app)
        .expect("Failed to run event loop");

    Ok(())
}

fn main() -> Result<(), AvengerAppError> {
    // Setup logger
    env_logger::init();
    
    // Parse command line arguments with clap
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Preview { file_path } => {
            // Run the interactive preview window in a blocking context
            run_preview(&file_path)
        },
        Commands::Save { file_path, output, scale } => {
            // Create a runtime for the save command
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("Failed to build runtime");
            
            // Determine output path
            let input_path = PathBuf::from(&file_path);
            let output_path = match output {
                Some(output) => PathBuf::from(output),
                None => input_path.with_extension("png"),
            };
            
            // Render to PNG within the runtime
            rt.block_on(render_to_png(&input_path, &output_path, scale))
        }
    }
}