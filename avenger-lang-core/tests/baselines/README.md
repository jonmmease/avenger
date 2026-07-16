# Frontend baselines

Baselines are reviewed outputs, never test inputs. Token baselines record token
kind/value/span/trivia; AST baselines use canonical semantic JSON; printed
baselines use canonical `.avenger`; diagnostic baselines contain normalized
source excerpts and labels. Normal test runs write only below
`target/tests/avenger-lang/`.

Run `../../scripts/update_baselines.sh` from this crate only when intentionally
reviewing output changes, then inspect every changed baseline before commit.
