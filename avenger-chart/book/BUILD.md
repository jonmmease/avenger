# Documentation Build Guide

## Quick Reference

| Script | When to Use | Speed |
|--------|-------------|-------|
| `bash build.sh` | Text-only markdown changes | ⚡ ~5-10 sec |
| `bash rebuild.sh` | Added new `rust,render` examples | 🐢 ~1-2 min |
| `mdbook serve` | Live preview while editing | ⚡ Instant |

## Workflows

### Editing Existing Documentation
```bash
# Start live preview server
mdbook serve --port 3000

# Edit markdown files
# Changes appear instantly in browser
```

### Adding New Rust Examples
```bash
# After adding rust,render code blocks:
bash rebuild.sh

# This forces preprocessor to re-scan markdown files
# and compile new examples
```

### Text-Only Changes
```bash
# Quick rebuild without preprocessor recompilation:
bash build.sh
```

## How It Works

**`rebuild.sh`:**
1. Cleans build artifacts (book/, src/.generated/)
2. Cleans preprocessor package only (preserves dependency cache)
3. Rebuilds preprocessor in release mode
4. Builds book with all examples

**`build.sh`:**
1. Runs `mdbook build` directly
2. Uses existing preprocessor binary
3. Fast for text changes, but won't pick up new examples

**`mdbook serve`:**
1. Watches markdown files for changes
2. Rebuilds only changed pages
3. Live browser reload
4. Best for iterative editing

## Troubleshooting

**New example not rendering?**
- Use `bash rebuild.sh` (not `build.sh`)

**Build takes 5+ minutes?**
- First build after adding examples is slow
- Subsequent builds reuse cargo cache

**Want even faster builds?**
- Use `mdbook serve` for live preview
- Only use rebuild.sh when adding new examples
