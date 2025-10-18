#!/usr/bin/env bash
# Fast documentation build script
#
# Use this when: You've only edited markdown text (no new rust,render examples)
# Use rebuild.sh when: You've added new rust,render code blocks
#
# This skips preprocessor rebuild for maximum speed

set -e  # Exit on error

# Get the directory where this script is located
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

echo "📚 Building book (fast mode - skipping preprocessor rebuild)..."
mdbook build

echo ""
echo "✅ Book build complete!"
echo "   Output is in: $SCRIPT_DIR/book/"
echo ""
echo "To serve the book locally, run:"
echo "   cd $SCRIPT_DIR && mdbook serve"
