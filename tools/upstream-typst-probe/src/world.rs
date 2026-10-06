//! The probe's `World`: one in-memory source and the fixture fonts.

use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde_json::{Value as Json, json};
use typst::{
    Library, LibraryExt, World, WorldExt,
    diag::{FileError, FileResult, SourceDiagnostic},
    foundations::{Bytes, Datetime, Duration, Target},
    math::EquationElem,
    syntax::{DiagSpan, FileId, RootedPath, Source, VirtualPath, VirtualRoot},
    text::{Font, FontBook},
    utils::LazyHash,
};

use crate::{Result, math};

/// The fixture fonts, in file-name order like typst-cli's `--font-path` scan.
pub struct Fonts {
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
}

/// Loads the same fonts the PNG generator gives typst-cli: Avenger's bundled Lato, DejaVu Sans
/// Mono and Lete Sans Math, plus `avenger-typst-label/tests/fixtures/fonts`.
pub fn load_fixture_fonts(repo_root: &Path) -> Result<Fonts> {
    let mut paths: Vec<PathBuf> = [
        "avenger-fonts/fonts/Lato/Lato-Light.ttf.br",
        "avenger-fonts/fonts/Lato/Lato-Italic.ttf.br",
        "avenger-fonts/fonts/Lato/Lato-Medium.ttf.br",
        "avenger-fonts/fonts/Lato/Lato-Bold.ttf.br",
        "avenger-fonts/fonts/DejaVu_Sans_Mono/DejaVuSansMono.ttf.br",
        "avenger-fonts/fonts/Lete_Sans_Math/LeteSansMath.otf.br",
        "avenger-fonts/fonts/Lete_Sans_Math/LeteSansMath-Bold.otf.br",
    ]
    .iter()
    .map(|path| repo_root.join(path))
    .collect();
    for entry in fs::read_dir(repo_root.join("avenger-typst-label/tests/fixtures/fonts"))? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "br") {
            paths.push(path);
        }
    }
    paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

    let mut fonts = Vec::new();
    for path in &paths {
        let mut data = Vec::new();
        brotli_decompressor::Decompressor::new(fs::File::open(path)?, 4096)
            .read_to_end(&mut data)
            .map_err(|err| format!("failed to decompress {}: {err}", path.display()))?;
        let loaded = Font::iter(Bytes::new(Arc::<[u8]>::from(data))).collect::<Vec<_>>();
        if loaded.is_empty() {
            return Err(format!("{} holds no font", path.display()).into());
        }
        fonts.extend(loaded);
    }
    Ok(Fonts {
        book: LazyHash::new(FontBook::from_fonts(&fonts)),
        fonts,
    })
}

/// The standard library, with the inline-equation rule replaced by one that also records the
/// equation's resolved math IR.
pub fn probe_library() -> LazyHash<Library> {
    let mut library = Library::builder().build();
    math::remember_rules(library.rules.clone());
    library
        .rules
        .replace::<EquationElem>(Target::Paged, math::CAPTURING_EQUATION_RULE);
    LazyHash::new(library)
}

pub struct ProbeWorld<'a> {
    library: LazyHash<Library>,
    fonts: &'a Fonts,
    main: Source,
}

impl<'a> ProbeWorld<'a> {
    pub fn new(library: LazyHash<Library>, fonts: &'a Fonts, text: &str) -> Result<Self> {
        let id = FileId::unique(RootedPath::new(
            VirtualRoot::Project,
            VirtualPath::new("main.typ")?,
        ));
        Ok(Self {
            library,
            fonts,
            main: Source::new(id, text.into()),
        })
    }
}

impl World for ProbeWorld<'_> {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.fonts.book
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
        self.fonts.fonts.get(index).cloned()
    }

    fn today(&self, _: Option<Duration>) -> Option<Datetime> {
        None
    }
}

/// Maps spans in the wrapped document to byte ranges in the label source.
pub struct SpanMapper<'a> {
    world: &'a ProbeWorld<'a>,
    offset: usize,
    len: usize,
}

impl<'a> SpanMapper<'a> {
    pub fn new(world: &'a ProbeWorld<'a>, offset: usize, len: usize) -> Self {
        Self { world, offset, len }
    }

    /// The label-relative range of a span's node, or `None` for detached spans and spans
    /// outside the label (the wrapper's own rules).
    pub fn range(&self, span: impl Into<DiagSpan>) -> Option<[usize; 2]> {
        let span = span.into();
        if span.id() != Some(self.world.main.id()) {
            return None;
        }
        let range = self.world.range(span)?;
        (range.start >= self.offset && range.end <= self.offset + self.len)
            .then(|| [range.start - self.offset, range.end - self.offset])
    }

    pub fn json(&self, span: impl Into<DiagSpan>) -> Json {
        self.range(span).map_or(Json::Null, |range| json!(range))
    }
}

pub fn diagnostics(diagnostics: &[SourceDiagnostic], mapper: &SpanMapper) -> Json {
    diagnostics
        .iter()
        .map(|diagnostic| {
            json!({
                "message": diagnostic.message.as_str(),
                "range": mapper.json(diagnostic.span),
                "hints": diagnostic.hints.iter().map(|hint| hint.v.as_str()).collect::<Vec<_>>(),
            })
        })
        .collect()
}
