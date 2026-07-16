# Compiler baselines

`compiled_interface/` contains explicitly debug-only stable host interfaces;
`expansion/` contains canonical expanded sources and source-map summaries;
`render/` is reserved for semantics without an existing `avenger-chart`
baseline. Phase 0 deliberately reuses the chart crate's
`symbol/simple_scatter_plot.png` instead of copying it.

Use `../../../avenger-lang-core/scripts/update_baselines.sh` for intentional
snapshot updates and review the printed changed-file list before committing.
