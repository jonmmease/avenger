#!/usr/bin/env python3
"""Compares avenger-typst-label's ported files with the upstream Typst files they come from.

Each ported file names its source on its first line:

    //! Ported from crates/<path> @ v<version>, modified for Avenger.

The comparison undoes what the port changes mechanically, so that only real changes remain:
the provenance header and the module's `avenger:` notes, imports, crate paths
(`crate::typst_library::` for `crate::` or `typst_library::`), the `elem!` and `func!` wrappers
around upstream's attribute-macro items, `name = ".."` in `#[elem(..)]`, and `ignore` on doc
fences. Both sides are then formatted with the crate's rustfmt settings, which are upstream's.

    upstream_diff.py diff [FILE ...]   unified diffs from upstream to Avenger
    upstream_diff.py status            per file: upstream items kept, changed and removed, and
                                       Avenger's added items
    upstream_diff.py paths             the upstream paths, for `git diff OLD..NEW -- PATHS`
    upstream_diff.py bump NEW          a dry run of upstream's changes since the pinned
                                       revision on the ported files, with crate paths ported

Upstream is read from a Typst checkout (`--typst`, default `../typst` next to this repository)
at the commit pinned in `avenger-typst-label/tests/fixtures/typst-pin.toml`. Needs python3 3.11
or later and rustfmt.
"""
import argparse, difflib, os, re, subprocess, sys, tomllib
from pathlib import Path

import rust_items

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
CRATE = REPO / 'avenger-typst-label'
HEADER = re.compile(r'^//! Ported from (crates/\S+?) @ v[^,]+(, modified for Avenger)?\.$')
# The crates whose modules the port nests under `crate::`. `typst_syntax` and `typst_utils`
# are dependencies, so their paths are upstream's.
CRATES = ['typst_eval', 'typst_layout', 'typst_library', 'typst_realize']


def ported_files():
    """(Avenger path, upstream path) for every ported file, in path order."""
    pairs = []
    for path in sorted((CRATE / 'src').rglob('*.rs')):
        with open(path) as file:
            match = HEADER.match(file.readline().rstrip('\n'))
        if match:
            pairs.append((path, match.group(1)))
    return pairs


def upstream_text(typst, rev, path):
    return subprocess.run(['git', '-C', typst, 'show', f'{rev}:{path}'],
                          capture_output=True, text=True, check=True).stdout


def source_crate(upstream_path):
    """The crate an upstream path belongs to, as a Rust identifier: `typst_library`."""
    return upstream_path.split('/')[1].replace('-', '_')


def drop_module_header(lines):
    """Drops the provenance line and the `//! avenger:` paragraphs of the module docs."""
    out, i = [], 1
    while i < len(lines) and lines[i].startswith('//!'):
        if lines[i].startswith('//! avenger:'):
            while i < len(lines) and lines[i].startswith('//!') and lines[i] != '//!':
                i += 1
            continue
        out.append(lines[i])
        i += 1
    # The blank `//!` lines left around removed paragraphs.
    while out and out[0] == '//!':
        out.pop(0)
    while out and out[-1] == '//!':
        out.pop()
    return out + lines[i:]


def drop_uses(lines):
    """Drops `use` declarations, which the port rewrites and regroups."""
    out, i = [], 0
    start = re.compile(r'^\s*(pub(\([^)]*\))? )?use [^;]*($|;)')
    while i < len(lines):
        if start.match(lines[i]):
            while not lines[i].rstrip().endswith(';'):
                i += 1
            i += 1
            continue
        out.append(lines[i])
        i += 1
    return out


def drop_wrappers(lines):
    """Unwraps `elem! { .. }` and `func! { .. }`, which stand in for attribute macros."""
    drop, i = set(), 0
    while i < len(lines):
        if lines[i] in ('elem! {', 'func! {'):
            end = rust_items.scan_end(lines, i)
            drop.update((i, end))
            i = end + 1
        else:
            i += 1
    return [line for k, line in enumerate(lines) if k not in drop]


def normalize_ours(text, crate):
    lines = drop_module_header(text.split('\n'))
    lines = drop_wrappers(drop_uses(lines))
    text = '\n'.join(lines)
    for other in CRATES:
        text = text.replace(f'crate::{other}::', 'crate::' if other == crate else f'{other}::')
    text = re.sub(r'#\[elem\(name = "[^"]*"\)\]', '#[elem]', text)
    text = re.sub(r'#\[elem\(name = "[^"]*", ', '#[elem(', text)
    text = re.sub(r'^(\s*///.*)```ignore$', r'\1```', text, flags=re.M)
    return text


def normalize_upstream(text):
    return '\n'.join(drop_uses(text.split('\n')))


def rustfmt(text):
    """The text formatted with the crate's rustfmt settings, or as is if it doesn't parse."""
    result = subprocess.run(
        ['rustfmt', '--edition', '2024', '--emit', 'stdout', '--quiet',
         '--config-path', str(CRATE / 'rustfmt.toml')],
        input=text, capture_output=True, text=True)
    return result.stdout if result.returncode == 0 and result.stdout else text


def compared(typst, rev, path, upstream_path):
    """The upstream text and Avenger's, normalized for comparison."""
    ours = normalize_ours(path.read_text(), source_crate(upstream_path))
    theirs = normalize_upstream(upstream_text(typst, rev, upstream_path))
    return rustfmt(theirs), rustfmt(ours)


def item_texts(text):
    """Each item's text with its whitespace collapsed, by selector. Repeated selectors (such
    as several `impl` blocks of one type) are numbered in order."""
    lines, items = rust_items.parse(text)
    texts, seen = {}, {}
    for prefix, item in rust_items.walk(items):
        if item.children:
            continue
        selector = prefix + item.selector
        seen[selector] = seen.get(selector, 0) + 1
        if seen[selector] > 1:
            selector += f' #{seen[selector]}'
        body = ' '.join(' '.join(lines[item.start:item.end + 1]).split())
        texts[selector] = body
    return texts


def status(typst, rev):
    print('| Avenger file | Upstream file | Kept | Changed | Removed | Added | Status |')
    print('|---|---|---:|---:|---:|---:|---|')
    for path, upstream_path in ported_files():
        theirs, ours = compared(typst, rev, path, upstream_path)
        theirs, ours = item_texts(theirs), item_texts(ours)
        kept = sum(1 for k, v in theirs.items() if ours.get(k) == v)
        changed = sum(1 for k, v in theirs.items() if k in ours and ours[k] != v)
        removed = sum(1 for k in theirs if k not in ours)
        added = sum(1 for k in ours if k not in theirs)
        mark = 'A' if changed or added else 'S' if removed else 'V'
        rel = path.relative_to(CRATE / 'src')
        print(f'| `{rel}` | `{upstream_path}` | {kept} | {changed} | {removed} | {added} | {mark} |')


def diff(typst, rev, files):
    pairs = ported_files()
    if files:
        wanted = {Path(f).resolve() for f in files}
        pairs = [(p, u) for p, u in pairs if p.resolve() in wanted]
    for path, upstream_path in pairs:
        theirs, ours = compared(typst, rev, path, upstream_path)
        rel = path.relative_to(REPO)
        sys.stdout.writelines(difflib.unified_diff(
            theirs.splitlines(keepends=True), ours.splitlines(keepends=True),
            fromfile=f'{upstream_path} @ {rev[:9]}', tofile=str(rel)))


def port_paths(text, crate):
    """Upstream code with crate paths as the port writes them: the reverse of
    `normalize_ours`'s path rewrite."""
    text = re.sub(r'(?<![\w:$])crate::', f'crate::{crate}::', text)
    for other in CRATES:
        if other != crate:
            text = re.sub(rf'(?<![\w:$]){other}::', f'crate::{other}::', text)
    return text


def bump(typst, old, new):
    """Dry-runs upstream's changes from `old` to `new` on each ported file, and reports the
    hunks that apply, that the file already has, and that fail."""
    for path, upstream_path in ported_files():
        patch = subprocess.run(
            ['git', '-C', typst, 'diff', '--no-ext-diff', '--no-color', f'{old}..{new}', '--',
             upstream_path],
            capture_output=True, text=True, check=True).stdout
        if not patch:
            continue
        hunks = patch.count('\n@@ ')
        if not hunks:
            sys.exit(f'git diff gave no unified hunks for {upstream_path}')
        rel = path.relative_to(REPO)
        crate = source_crate(upstream_path)
        lines = []
        for line in patch.splitlines(keepends=True):
            if line.startswith(('--- a/', '+++ b/')):
                line = f'{line[:6]}{rel}\n'
            elif line[:1] in ' +-' and not line.startswith(('---', '+++')):
                line = line[0] + port_paths(line[1:], crate)
            lines.append(line)
        # `-N` skips hunks that the file already has, and says so.
        result = subprocess.run(['patch', '--dry-run', '-N', '-p1', '-d', str(REPO)],
                                input=''.join(lines), capture_output=True, text=True)
        output = result.stdout + result.stderr
        count = lambda word: sum(int(n) for n in re.findall(
            rf'(\d+) out of \d+ hunks? {word}', output, flags=re.I))
        failed, applied = count('failed'), count('ignored')
        parts = [f'{hunks - failed - applied} apply'] if hunks - failed - applied else []
        parts += [f'{applied} already applied'] if applied else []
        parts += [f'{failed} fail'] if failed else []
        print(f'{rel} ({upstream_path}): {hunks} hunks: ' + ', '.join(parts))


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('command', choices=['diff', 'status', 'paths', 'bump'])
    parser.add_argument('files', nargs='*',
                        help='diff: Avenger files (default: all); bump: the new revision')
    parser.add_argument('--from', dest='old', help='bump: the old revision (default: pinned)')
    parser.add_argument('--typst', default=os.environ.get('TYPST_DIR', REPO.parent / 'typst'),
                        help='a Typst git checkout')
    args = parser.parse_args()
    with open(CRATE / 'tests' / 'fixtures' / 'typst-pin.toml', 'rb') as file:
        rev = tomllib.load(file)['commit']
    if args.command == 'paths':
        print('\n'.join(sorted({upstream for _, upstream in ported_files()})))
    elif args.command == 'bump':
        if len(args.files) != 1:
            parser.error('bump takes the new revision')
        bump(str(args.typst), args.old or rev, args.files[0])
    elif args.command == 'status':
        status(str(args.typst), rev)
    else:
        diff(str(args.typst), rev, args.files)


if __name__ == '__main__':
    main()
