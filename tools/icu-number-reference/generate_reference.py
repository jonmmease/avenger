#!/usr/bin/env python3
"""Regenerate ICU4J labels or exception names, retaining exact f64 input bits and authored fields."""
import argparse
import base64
import hashlib
import json
from pathlib import Path
import subprocess
from tempfile import TemporaryDirectory
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
ICU_VERSION = '78.1'
ICU_SHA256 = 'bbb70d3be23110d7295823eee0c2e896ac3b619b3c0f26168f65eb972df51d2a'


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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--java', default='java')
    args = parser.parse_args()
    fixture = ROOT / 'avenger-format-number-icu/tests/fixtures/icu4j.json'
    data = json.loads(fixture.read_text())
    results = reference(args.java, [(r['locale'], r['skeleton'], r['bits']) for r in data['cases']])
    order = ['locale', 'skeleton', 'bits', 'expected', 'icu4j_error', 'error', 'position', 'compatibility_exception']
    for i, (row, (kind, text)) in enumerate(zip(data['cases'], results)):
        row = {k: v for k, v in row.items() if k not in ('expected', 'icu4j_error')}
        row['expected' if kind == 'OK' else 'icu4j_error'] = text
        data['cases'][i] = {k: row[k] for k in order if k in row}
    data['java'] = subprocess.run([args.java, '-version'], capture_output=True, text=True, check=True).stderr.splitlines()[0]
    data['icu4j'] = ICU_VERSION
    fixture.write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n')

if __name__ == '__main__':
    main()
