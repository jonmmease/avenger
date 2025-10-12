use anyhow::{anyhow, Context, Result};
use avenger_chart_mdbook::render_snippets::{RenderEntry, RENDER_ENTRIES};
use avenger_wgpu::error::AvengerWgpuError;
use mdbook::book::{Book, BookItem, Chapter};
use mdbook::errors::Error;
use mdbook::preprocess::{CmdPreprocessor, Preprocessor, PreprocessorContext};
use pathdiff::diff_paths;
use std::collections::HashMap;
use std::fs;
use std::path::Path;

const GENERATED_DIR: &str = ".generated/images";

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let preprocessor = AvengerPreprocessor::default();

    if let Some(sub) = args.next() {
        if sub == "supports" {
            let renderer = args.next().unwrap_or_default();
            std::process::exit(if preprocessor.supports_renderer(&renderer) {
                0
            } else {
                1
            });
        } else {
            return Err(anyhow!("unknown argument {}", sub));
        }
    }

    let (ctx, book) =
        CmdPreprocessor::parse_input(std::io::stdin()).context("failed to parse mdBook input")?;
    let processed = preprocessor.run(&ctx, book).map_err(|err| anyhow!(err))?;
    serde_json::to_writer(std::io::stdout(), &processed)
        .context("failed to write preprocessed book")?;
    Ok(())
}

#[derive(Default)]
struct AvengerPreprocessor;

impl Preprocessor for AvengerPreprocessor {
    fn name(&self) -> &str {
        "avenger"
    }

    fn run(&self, ctx: &PreprocessorContext, mut book: Book) -> Result<Book, Error> {
        let src_dir = ctx.root.join(&ctx.config.book.src);
        let images_dir = src_dir.join(GENERATED_DIR);
        fs::create_dir_all(&images_dir)
            .with_context(|| format!("failed to create {}", images_dir.display()))?;

        render_all(&images_dir, &src_dir)?;

        let mut by_file: HashMap<&str, Vec<&RenderEntry>> = HashMap::new();
        for entry in RENDER_ENTRIES {
            by_file.entry(entry.markdown_path).or_default().push(entry);
        }

        let mut avenger = AvengerBookProcessor {
            src_dir: &src_dir,
            images_dir: &images_dir,
            by_file,
        };
        avenger.process_book(&mut book)?;
        Ok(book)
    }

    fn supports_renderer(&self, renderer: &str) -> bool {
        renderer == "html"
    }
}

struct AvengerBookProcessor<'a> {
    src_dir: &'a Path,
    images_dir: &'a Path,
    by_file: HashMap<&'static str, Vec<&'static RenderEntry>>,
}

impl<'a> AvengerBookProcessor<'a> {
    fn process_book(&mut self, book: &mut Book) -> Result<()> {
        for item in &mut book.sections {
            self.process_item(item)?;
        }
        Ok(())
    }

    fn process_item(&mut self, item: &mut BookItem) -> Result<()> {
        match item {
            BookItem::Chapter(chapter) => self.process_chapter(chapter),
            BookItem::Separator | BookItem::PartTitle(_) => Ok(()),
        }
    }

    fn process_chapter(&mut self, chapter: &mut Chapter) -> Result<()> {
        if let Some(path) = chapter.path.as_ref() {
            let rel_path = path.to_string_lossy().replace('\\', "/");
            if let Some(entries) = self.by_file.get_mut(rel_path.as_str()) {
                entries.sort_by_key(|entry| entry.fence_index);
                let chapter_path = self.src_dir.join(path);
                let rewritten = rewrite_content(
                    &chapter.content,
                    &chapter_path,
                    entries.clone(),
                    self.images_dir,
                )?;
                if rewritten != chapter.content {
                    chapter.content = rewritten;
                }
            }
        }

        for sub in &mut chapter.sub_items {
            self.process_item(sub)?;
        }

        Ok(())
    }
}

fn render_all(images_dir: &Path, src_dir: &Path) -> Result<()> {
    for entry in RENDER_ENTRIES {
        let output = images_dir.join(format!("{}.png", entry.slug));
        let source = src_dir.join(&entry.markdown_path);
        if needs_render(&output, &source)? {
            (entry.render)(&output).map_err(|err| match err.downcast::<AvengerWgpuError>() {
                Ok(avenger_err) => match *avenger_err {
                    AvengerWgpuError::MakeWgpuAdapterError => anyhow!(
                        "failed to render snippet {}: {}\n\
                         Ensure a compatible GPU backend is available (for example, `WGPU_BACKEND=gl`).",
                        entry.slug,
                        AvengerWgpuError::MakeWgpuAdapterError
                    ),
                    other => anyhow!("failed to render snippet {}: {}", entry.slug, other),
                },
                Err(other) => anyhow!("failed to render snippet {}: {}", entry.slug, other),
            })?;
        }
    }
    Ok(())
}

fn rewrite_content(
    content: &str,
    path: &Path,
    entries: Vec<&RenderEntry>,
    images_dir: &Path,
) -> Result<String> {
    if entries.is_empty() {
        return Ok(content.to_string());
    }

    let mut result = Vec::new();
    let mut lines = content.lines().peekable();
    let mut stack: Vec<Option<&RenderEntry>> = Vec::new();
    let mut render_counter = 0usize;
    let entry_map: HashMap<usize, &RenderEntry> = entries
        .iter()
        .map(|entry| (entry.fence_index, *entry))
        .collect();
    let slugs: Vec<&str> = entries.iter().map(|entry| entry.slug).collect();

    while let Some(line) = lines.next() {
        if let Some(info) = line.strip_prefix("```") {
            if let Some(frame) = stack.pop() {
                result.push(line.to_string());
                if let Some(entry) = frame {
                    insert_image_if_needed(
                        &mut result,
                        &mut lines,
                        entry,
                        images_dir,
                        path.parent(),
                    )?;
                }
                continue;
            }

            let is_render = is_render_fence(info.trim());
            if is_render {
                let entry = entry_map
                    .get(&render_counter)
                    .copied()
                    .context("render fence count exceeded registered snippets")?;
                stack.push(Some(entry));
                render_counter += 1;
            } else {
                stack.push(None);
            }
            result.push(line.to_string());
            continue;
        }

        if let Some(slug) = extract_render_image_slug(line) {
            if slugs.iter().any(|expected| *expected == slug) {
                continue;
            }
        }

        result.push(line.to_string());
    }

    Ok(result.join("\n"))
}

fn insert_image_if_needed(
    output: &mut Vec<String>,
    remaining_lines: &mut std::iter::Peekable<std::str::Lines<'_>>,
    entry: &RenderEntry,
    images_dir: &Path,
    chapter_dir: Option<&Path>,
) -> Result<()> {
    if let Some(next_line) = remaining_lines.peek() {
        if extract_render_image_slug(next_line).is_some() {
            return Ok(());
        }
    }

    let image_path = images_dir.join(format!("{}.png", entry.slug));
    let relative = if let Some(chapter_dir) = chapter_dir {
        diff_paths(&image_path, chapter_dir).unwrap_or_else(|| image_path.clone())
    } else {
        image_path.clone()
    };

    let rel_str = relative.to_string_lossy().replace('\\', "/");
    output.push(String::new());
    output.push(format!("![Rendered plot]({})", rel_str));
    output.push(String::new());
    Ok(())
}

fn extract_render_image_slug(line: &str) -> Option<String> {
    if let Some(rest) = line.trim().strip_prefix("![Rendered plot](") {
        let candidate = rest.trim_end_matches(')').trim();
        if candidate.contains(".generated/images") {
            if let Some(name) = Path::new(candidate).file_stem() {
                return Some(name.to_string_lossy().to_string());
            }
        }
    }
    None
}

fn is_render_fence(info: &str) -> bool {
    let normalized = info.replace(',', " ");
    let mut saw_rust = false;
    let mut saw_render = false;
    for token in normalized.split_whitespace() {
        match token {
            "rust" | "" => saw_rust = true,
            "render" => saw_render = true,
            _ => {}
        }
    }
    saw_rust && saw_render
}

fn needs_render(output: &Path, source: &Path) -> Result<bool> {
    let output_meta = match fs::metadata(output) {
        Ok(meta) => meta,
        Err(_) => return Ok(true),
    };

    let source_meta = match fs::metadata(source) {
        Ok(meta) => meta,
        Err(_) => return Ok(false),
    };

    let output_time = output_meta
        .modified()
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    let source_time = source_meta
        .modified()
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    Ok(source_time > output_time)
}
