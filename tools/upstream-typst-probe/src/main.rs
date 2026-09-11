use std::{
    error::Error,
    path::{Path, PathBuf},
    sync::Arc,
};

use typst_upstream::{
    Features, Library, LibraryExt, World,
    diag::{FileError, FileResult},
    foundations::{Bytes, Datetime, Duration, Smart},
    layout::{Abs, Margin, PageElem},
    syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot},
    text::{Font, FontBook, TextElem, TextSize},
    utils::LazyHash,
};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let output = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/upstream-typst-math-svg-probe/math-label.svg"));
    let font_dir = args
        .next()
        .map(PathBuf::from)
        .or_else(default_font_dir)
        .ok_or("missing font dir; pass scratch/font-subset-output or another font root")?;

    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let source = r#"
#set page(width: auto, height: auto, margin: 0pt, fill: none)
#set text(font: "Lato", size: 36pt, weight: 500)
#show math.equation: set text(font: "Lete Sans Math", weight: 500)
$y = sqrt(x) / (1 + x^2)$
"#;

    let world = ProbeWorld::new(source, &font_dir)?;
    let warned = typst_upstream::compile::<typst_layout_upstream::PagedDocument>(&world);
    if !warned.warnings.is_empty() {
        eprintln!("upstream Typst warnings: {}", warned.warnings.len());
    }
    let document = warned.output.map_err(|errors| {
        let messages = errors
            .into_iter()
            .map(|diagnostic| diagnostic.message.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        format!(
            "upstream Typst compile failed: {messages}; loaded font families: {}",
            world.font_summary
        )
    })?;
    let page = document
        .pages()
        .first()
        .ok_or("compiled document has no pages")?;
    let svg = typst_svg_upstream::svg(
        page,
        &typst_svg_upstream::SvgOptions {
            render_bleed: false,
            pretty: false,
        },
    );
    std::fs::write(&output, svg.as_bytes())?;

    println!(
        "{} svg_bytes={} pages={}",
        output.display(),
        svg.len(),
        document.pages().len()
    );
    Ok(())
}

struct ProbeWorld {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    font_summary: String,
    main: Source,
}

impl ProbeWorld {
    fn new(source: &str, font_dir: &Path) -> Result<Self, Box<dyn Error>> {
        let fonts = load_fonts(font_dir)?;
        let book = FontBook::from_fonts(&fonts);
        let font_summary = summarize_fonts(&book);
        let main_id = FileId::unique(RootedPath::new(
            VirtualRoot::Project,
            VirtualPath::new("main.typ")?,
        ));
        let main = Source::new(main_id, source.into());
        Ok(Self {
            library: LazyHash::new(library()),
            book: LazyHash::new(book),
            fonts,
            font_summary,
            main,
        })
    }
}

impl World for ProbeWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main.id()
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.main.id() {
            Ok(self.main.clone())
        } else {
            Err(FileError::NotFound(id.vpath().get_without_slash().into()))
        }
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        Err(FileError::NotFound(id.vpath().get_without_slash().into()))
    }

    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }

    fn today(&self, _: Option<Duration>) -> Option<Datetime> {
        None
    }
}

fn library() -> Library {
    let mut library = Library::builder().with_features(Features::all()).build();
    library
        .styles
        .set(PageElem::width, Smart::Custom(Abs::pt(120.0).into()));
    library.styles.set(PageElem::height, Smart::Auto);
    library.styles.set(
        PageElem::margin,
        Smart::Custom(Margin::splat(Some(Smart::Custom(Abs::pt(0.0).into())))),
    );
    library
        .styles
        .set(TextElem::size, TextSize(Abs::pt(36.0).into()));
    library
}

fn load_fonts(font_dir: &Path) -> Result<Vec<Font>, Box<dyn Error>> {
    let mut fonts = Vec::new();
    collect_fonts(font_dir, &mut fonts)?;
    if fonts.is_empty() {
        return Err(format!("no fonts found under {}", font_dir.display()).into());
    }
    Ok(fonts)
}

fn summarize_fonts(book: &FontBook) -> String {
    book.families()
        .map(|(family, ids)| format!("{family}:{}", ids.count()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn collect_fonts(dir: &Path, fonts: &mut Vec<Font>) -> Result<(), Box<dyn Error>> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_fonts(&path, fonts)?;
            continue;
        }
        let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
            continue;
        };
        if !matches!(
            extension.to_ascii_lowercase().as_str(),
            "ttf" | "otf" | "ttc"
        ) {
            continue;
        }
        let data = Bytes::new(Arc::<[u8]>::from(std::fs::read(&path)?));
        fonts.extend(Font::iter(data));
    }
    Ok(())
}

fn default_font_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scratch/font-subset-output");
    dir.is_dir().then_some(dir)
}
