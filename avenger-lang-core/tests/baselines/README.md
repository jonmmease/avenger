# Frontend baselines

Baselines are reviewed outputs, never test inputs. Token baselines record token
kind/value/span/trivia; AST baselines use canonical semantic JSON; printed
baselines use canonical `.avenger`; diagnostic baselines contain normalized
source excerpts and labels. Normal test runs write only below
`target/tests/avenger-lang/`.

Phase 2 baselines live in `parse/`. Every accepted root has paired
`*.ast.json` and `*.printed.avenger` files; `parse/invalid/` pins diagnostic
codes, messages, and byte ranges.

Run `../../scripts/update_baselines.sh` from this crate only when intentionally
reviewing output changes, then inspect every changed baseline before commit.
