#!/bin/sh
set -eu

workspace=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
cd "$workspace"

AVENGER_LANG_UPDATE_BASELINES=1 cargo test --release -p avenger-lang-core --test diagnostics
AVENGER_LANG_UPDATE_BASELINES=1 cargo test --release -p avenger-lang-core --test token_corpus token_golden_corpus_is_stable
AVENGER_LANG_UPDATE_BASELINES=1 cargo test --release -p avenger-lang-compiler --test phase0_contracts bootstrap_schema_round_trips_and_matches_version_snapshot

changed=$(git diff --name-only -- \
    avenger-lang-core/tests/baselines \
    avenger-lang-compiler/tests/baselines)

if [ -n "$changed" ]; then
    echo "Updated Avenger language baselines:"
    echo "$changed"
    echo "Review every diff before committing."
else
    echo "Avenger language baselines are already current."
fi
