# Avenger Chart Book

User documentation for Avenger Chart built with [mdBook](https://rust-lang.github.io/mdBook/).

## Building the Book

### Prerequisites

Install mdBook:

```bash
cargo install mdbook
```

### Build

Build the book:

```bash
cd avenger-chart/book
mdbook build
```

Output will be in `book/book/`.

### Validate Code Examples

The Markdown snippets are mirrored into the `avenger-chart-mdbook` harness crate so they can be compiled against the workspace sources. Run the doctests with:

```bash
cargo test -p avenger-chart-mdbook --doc
```

This command surfaces any examples that no longer compile with the current APIs.

### Serve

Serve the book locally with live reload:

```bash
mdbook serve
```

Then open http://localhost:3000 in your browser.

### Watch

Automatically rebuild on file changes:

```bash
mdbook watch
```

## Structure

```
book/
├── book.toml           # mdBook configuration
├── src/                # Markdown source files
│   ├── SUMMARY.md      # Table of contents
│   ├── introduction.md # Introduction page
│   ├── getting-started/
│   ├── concepts/
│   ├── guides/
│   ├── advanced/
│   ├── api/
│   └── images/         # Images for inline display
└── book/               # Generated output (gitignored)
```

## Adding Content

1. Create new `.md` files in `src/`
2. Add entries to `src/SUMMARY.md`
3. Run `mdbook build` to generate HTML

## Images

Place images in `src/images/` and reference them with relative paths:

```markdown
![Description](images/my-image.png)
```

## Publishing

The book can be published to:
- GitHub Pages
- Netlify
- Any static hosting service

Just deploy the `book/book/` directory contents.

## Links

- [mdBook User Guide](https://rust-lang.github.io/mdBook/)
- [Avenger Chart Repository](https://github.com/vega/avenger)
