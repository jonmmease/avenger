#!/bin/bash
# Script to update visual test baselines for avenger-chart
#
# Usage: ./scripts/update_baselines.sh [test_name]
#
# If test_name is provided, only that baseline will be updated.
# Otherwise, all baselines will be updated.

set -e

cd "$(dirname "$0")/.."

if [ $# -eq 0 ]; then
    echo "Updating all visual test baselines..."
    cargo test --test visual_regression -- --ignored update
else
    echo "Updating baseline for test: $1"
    cargo test --test visual_regression "update_${1}_baseline" -- --ignored --nocapture
fi

echo "Baselines updated successfully!"
echo "Don't forget to:"
echo "  1. Review the updated images"
echo "  2. Run the tests to ensure they pass"
echo "  3. Commit the baseline changes"