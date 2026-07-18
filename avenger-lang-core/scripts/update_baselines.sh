#!/bin/sh
set -eu

workspace=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$workspace"

AVENGER_LANG_UPDATE_BASELINES=1 cargo test --release -p avenger-lang-core --test diagnostics
AVENGER_LANG_UPDATE_BASELINES=1 cargo test --release -p avenger-lang-core --test token_corpus token_golden_corpus_is_stable
AVENGER_LANG_UPDATE_BASELINES=1 cargo test --release -p avenger-lang-core --test language_corpus parse_
AVENGER_LANG_UPDATE_BASELINES=1 cargo test --release -p avenger-chart-lang-registry checked_full_v1_schema_and_documentation_do_not_drift
AVENGER_LANG_UPDATE_BASELINES=1 cargo test --release -p avenger-lang-compiler --test phase0_contracts full_v1

changed=$(git diff --name-only -- \
    avenger-lang-core/tests/baselines \
    avenger-lang-compiler/tests/baselines \
    avenger-chart-lang-registry/snapshots/full-v1-authoring-schema.json \
    avenger-chart-lang-registry/docs/full-v1-native-kinds.md)

if [ -n "$changed" ]; then
    echo "Updated Avenger language baselines:"
    echo "$changed"
    echo "Review every diff before committing."
else
    echo "Avenger language baselines are already current."
fi
