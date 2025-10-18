#!/usr/bin/env bash
# Full documentation rebuild script
# Cleans all caches and build artifacts, then rebuilds the book from scratch

set -e  # Exit on error

# Get the directory where this script is located
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "🧹 Cleaning build artifacts..."

# Remove the built book directory
if [ -d "book" ]; then
    echo "  - Removing book/ directory"
    rm -rf book
fi

# Remove generated images in source (to ensure clean regeneration)
if [ -d "src/.generated" ]; then
    echo "  - Removing src/.generated/ directory"
    rm -rf src/.generated
fi

# Remove the target directory (preprocessor builds)
if [ -d "target" ]; then
    echo "  - Removing target/ directory"
    rm -rf target
fi

echo ""
echo "🔨 Building mdbook-avenger preprocessor..."
echo "  - Forcing build.rs to re-scan markdown files"
# Delete build script output to force build.rs to re-run and regenerate render_snippets.rs
# This is more reliable than cargo clean and much faster than cleaning dependencies
rm -rf ../../target/debug/build/avenger-chart-mdbook-*
rm -rf ../../target/release/build/avenger-chart-mdbook-*

# Build preprocessor binaries (mdbook-avenger and mdbook-avenger-render)
cargo build --release --manifest-path ../Cargo.toml --package avenger-chart-mdbook

echo ""
echo "📚 Building book..."
mdbook build

echo ""
echo "✅ Book rebuild complete!"
echo "   Output is in: $SCRIPT_DIR/book/"
echo ""
echo "To serve the book locally, run:"
echo "   cd $SCRIPT_DIR && mdbook serve"
