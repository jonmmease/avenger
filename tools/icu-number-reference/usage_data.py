"""Compile unit preferences, including precision omitted by the CLDR JSON export."""
import hashlib
import json
import re
import urllib.request
from generate_data import rust

ICU_COMMIT = '049e0d6a420629ac7db77256987d083a563287b5'
# Usage requires a label for every output unit, so keep categories with ICU4X unit names.
LABELED_BASE_UNITS = {'length': 'meter', 'area': 'square-meter', 'volume': 'cubic-meter', 'mass': 'kilogram', 'duration': 'second'}


def source(path, checksum):
    url = f'https://raw.githubusercontent.com/unicode-org/icu/{ICU_COMMIT}/{path}'
    content = urllib.request.urlopen(url).read()
    if hashlib.sha256(content).hexdigest() != checksum:
        raise ValueError(f'ICU source checksum mismatch: {url}')
    return content.decode()


def resource(tokens):
    """Read the table and array subset used by ICU unit preferences."""
    assert next(tokens) == '{'
    fields, values = {}, []
    for token in tokens:
        if token == '}':
            return fields if fields else values
        if token == '{':
            tokens.push(token)
            values.append(resource(tokens))
        else:
            following = next(tokens)
            if following == '{':
                tokens.push(following)
                fields[token.strip('"')] = resource(tokens)
            else:
                values.append(token.strip('"'))
                tokens.push(following)
    raise ValueError('unterminated ICU resource')


class Tokens:
    def __init__(self, text):
        self.tokens = iter(re.findall(r'"[^"\\]*(?:\\.[^"\\]*)*"|[{}]|[A-Za-z0-9_-]+', text))
        self.saved = []
    def __iter__(self): return self
    def __next__(self): return self.saved.pop() if self.saved else next(self.tokens)
    def push(self, token): self.saved.append(token)


def generate(z, dest, header):
    text = source('icu4c/source/data/misc/units.txt', '365e4cc1398de1591550e294696cc10dcdaccc9615ec7e584f6373e4a6aaf8e2')
    preferences = resource(Tokens(text.split('unitPreferenceData',1)[1]))
    out = list(header) + [f'// Precision: ICU {ICU_COMMIT}.']
    out += ['static PREFERENCES: &[(&str, &[Preference])] = &[']
    rows_by_key = {category+'/'+usage+'/'+region: rows for category, usages in preferences.items() if category in LABELED_BASE_UNITS for usage, regions in usages.items() for region, rows in regions.items()}
    for key, rows in sorted(rows_by_key.items()):
        out += [f'    ({rust(key)}, &[']
        for row in rows:
            unit = row['unit'][0]
            threshold = row.get('geq', ['1'])[0]
            precision = row.get('skeleton', [''])[0]
            out += [f'        Preference {{ unit: {rust(unit)}, threshold: {rust(threshold)}, precision: {rust(precision)} }},']
        out += ['    ]),']
    out += ['];']
    units = json.loads(z.read('cldr-core/supplemental/units.json'))['supplemental']
    out += ['static BASE_UNITS: &[(&str, &str, &[&str])] = &[']
    for name, row in sorted(units['convertUnits'].items()):
        if row['_baseUnit'] in LABELED_BASE_UNITS.values():
            systems = ', '.join(rust(s) for s in row.get('_systems', []))
            out += [f'    ({rust(name)}, {rust(row["_baseUnit"])}, &[{systems}]),']
    out += ['];', 'static CATEGORIES: &[(&str, &str)] = &[']
    out += [f'    ({rust(name)}, {rust(row["_quantity"])}),' for name,row in units['unitQuantities'].items()]
    out += ['];']
    (dest/'src/generated_usage.rs').write_text('\n'.join(out)+'\n')
