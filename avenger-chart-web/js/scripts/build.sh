#!/usr/bin/env bash
# Builds the package: the module and its glue into wasm/, the bindings of the
# chart format into src/gen/, the TypeScript into dist/, and the licenses of the
# package and of the fonts that the module embeds into LICENSE and licenses/.
set -euo pipefail
cd "$(dirname "$0")/.."
../scripts/size.sh wasm
# The package is this directory: wasm-pack's own manifest, README, and `.gitignore`, which ignores
# everything and would keep the module out of the package, do not belong in it.
rm -f wasm/.gitignore wasm/package.json wasm/README.md
rm -rf src/gen dist licenses
npx buf generate ../../avenger-chart/proto --template buf.gen.yaml
npx tsc
cp ../../LICENSE LICENSE
mkdir -p licenses
for font in Lato DejaVu_Sans_Mono Lete_Sans_Math; do
  cp "../../avenger-fonts/fonts/$font/LICENSE.txt" "licenses/$font.txt"
done
