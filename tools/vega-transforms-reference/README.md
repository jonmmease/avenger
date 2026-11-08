# Vega transform reference fixtures

This tool writes the fixtures of `avenger-datafusion-transforms`: bins from vega-statistics, stacks from the code of Vega's stack transform, and time units from vega-time in four time zones. From the repository root, run these commands with Node 20.19.6:

```sh
cd tools/vega-transforms-reference
npm ci --ignore-scripts
npm run generate
```

The package lock pins the reference versions. The generator writes `bin.json`, `stack.json`, and `timeunit.json` in `avenger-datafusion-transforms/tests/fixtures`. Rust tests read these files without Node or network access.

The stack transform needs Vega's dataflow to run, so the generator copies its offset functions. Its `center` offset starts each stack at minus half its total, the one change from Vega. vega-time computes in the local time of the process, so each time zone runs in a child process with `TZ` set.
