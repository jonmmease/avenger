# Compiler project fixtures

The numbered directories are stable cross-phase examples. Each becomes a real
Avenger project in its named phase; until then its marker records the intended
coverage. Tests may copy fixtures below `target/tests/avenger-lang/`, but must
not mutate these reviewed sources.

`08_multi_chart_project` is the Phase 10 incremental-compilation fixture: three
chart roots have isolated parameters while the cartesian and polar charts share
one catalog table and one imported transform definition.
