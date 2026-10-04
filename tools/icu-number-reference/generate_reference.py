#!/usr/bin/env python3
"""Regenerate ICU4J's results for the reference cases, retaining exact f64 input bits and authored fields,
and ICU4J's labels for the reference grid."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from tempfile import TemporaryDirectory
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
FIXTURES = ROOT / 'avenger-format-number-icu/tests/fixtures'
ICU_VERSION = '78.1'
ICU_SHA256 = 'bbb70d3be23110d7295823eee0c2e896ac3b619b3c0f26168f65eb972df51d2a'

# The grid formats every skeleton in every locale at every value.
LOCALES = [
    'en-US', 'en-GB', 'en-IN', 'de', 'de-CH', 'fr', 'fr-CA', 'fr-CH', 'es', 'es-419', 'it', 'pt', 'pt-PT', 'pt-AO', 'nl', 'nb', 'sv',
    'fi', 'da', 'pl', 'cs', 'ru', 'uk', 'sr-Latn', 'bs-Cyrl', 'tr', 'el', 'he', 'ar', 'ar-EG', 'fa', 'ur', 'hi', 'bn', 'mr', 'te',
    'ta', 'th', 'my', 'zh', 'zh-Hant', 'ja', 'ko', 'vi', 'id', 'sw', 'ha', 'am', 'cy', 'ga', 'lt', 'sl', 'ka', 'kk', 'ps',
]
VALUES = [0.0, -0.0, 0.5, 1.0, 1.5, -2.5, 21.0, 999.5, 1234.5678, -1234.5678, 999999.5, 1e15, 0.000123, float('nan'), float('inf')]
SKELETONS = [
    # Precision
    '', '.00', '@@#', '.00/@##r', '.00/w', 'precision-increment/0.05', 'rounding-mode-floor .0', 'scale/0.5',
    'integer-width/##00', 'integer-width-trunc', '+! ,?', '()',
    # Notation
    'E0', 'EE0 .00', 'E+!00', 'E+?0', 'K', 'KK', 'KK .0', 'K @@@',
    # Units
    'percent', 'permille', '%x100 .0', 'currency/USD', 'currency/EUR unit-width-full-name', 'currency/JPY ()', 'unit/meter',
    'unit/kilogram unit-width-full-name', 'unit/hour unit-width-narrow', 'unit/foot-and-inch', 'unit/meter usage/person-height',
    'unit/square-meter usage/floor', 'unit/liter usage/fluid', 'unit/kilogram usage/person', 'unit/second usage/media',
]


def reference(java, rows):
    """Run ICU4J on (locale, skeleton, bits) rows, returning ('OK', label) or ('ERR', exception) pairs."""
    request = ''.join(f'{locale}\t{skeleton}\t{bits}\n' for locale, skeleton, bits in rows)
    url = f'https://repo.maven.apache.org/maven2/com/ibm/icu/icu4j/{ICU_VERSION}/icu4j-{ICU_VERSION}.jar'
    with TemporaryDirectory(prefix='avenger-icu4j-') as directory:
        jar = Path(directory) / 'icu4j.jar'
        urllib.request.urlretrieve(url, jar)
        if hashlib.sha256(jar.read_bytes()).hexdigest() != ICU_SHA256:
            raise ValueError('ICU4J checksum mismatch')
        proc = subprocess.run([java, '--class-path', str(jar), str(Path(__file__).with_name('Reference.java'))], input=request, text=True, check=True, capture_output=True)
    results = [line.split('\t') for line in proc.stdout.splitlines()]
    assert len(results) == len(rows)
    return [(kind, base64.b64decode(text).decode()) for kind, text in results]


def write_grid(java, java_version):
    """Write ICU4J's labels for the grid, one line per skeleton and locale."""
    bits = [format(struct.unpack('<Q', struct.pack('<d', value))[0], '016x') for value in VALUES]
    results = iter(reference(java, [(l, s, b) for s in SKELETONS for l in LOCALES for b in bits]))
    skeletons = []
    for skeleton in SKELETONS:
        locales = []
        for locale in LOCALES:
            labels = []
            for b in bits:
                kind, text = next(results)
                if kind != 'OK':
                    raise ValueError(f'ICU4J rejects grid row {locale} {skeleton!r} {b}: {text}')
                labels.append(text)
            locales.append(f'      {json.dumps(locale)}: {json.dumps(labels, ensure_ascii=False)}')
        skeletons.append(f'    {json.dumps(skeleton)}: {{\n' + ',\n'.join(locales) + '\n    }')
    (FIXTURES / 'icu4j_grid.json').write_text(
        '{\n'
        f'  "icu4j": {json.dumps(ICU_VERSION)},\n'
        f'  "java": {json.dumps(java_version)},\n'
        f'  "bits": {json.dumps(bits)},\n'
        '  "labels": {\n' + ',\n'.join(skeletons) + '\n  }\n}\n'
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--java', default='java')
    args = parser.parse_args()
    java_version = subprocess.run([args.java, '-version'], capture_output=True, text=True, check=True).stderr.splitlines()[0]
    fixture = FIXTURES / 'icu4j.json'
    data = json.loads(fixture.read_text())
    results = reference(args.java, [(r['locale'], r['skeleton'], r['bits']) for r in data['cases']])
    order = ['locale', 'skeleton', 'bits', 'expected', 'icu4j_error', 'error', 'position', 'compatibility_exception']
    for i, (row, (kind, text)) in enumerate(zip(data['cases'], results)):
        row = {k: v for k, v in row.items() if k not in ('expected', 'icu4j_error')}
        row['expected' if kind == 'OK' else 'icu4j_error'] = text
        data['cases'][i] = {k: row[k] for k in order if k in row}
    data['java'] = java_version
    data['icu4j'] = ICU_VERSION
    fixture.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n')
    # ICU4J's output can depend on earlier rows in the same JVM, so the grid gets its own.
    write_grid(args.java, java_version)

if __name__ == '__main__':
    main()
