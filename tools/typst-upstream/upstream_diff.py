#!/usr/bin/env python3
"""Compares avenger-typst-label's ported files with the upstream Typst files they come from.

Each ported file names its source on its first line:

    //! Ported from crates/<path> @ v<version>, modified for Avenger.

and a marker names the upstream item that an item ports, in an Avenger file or where a ported
file moves or reshapes it:

    // upstream: crates/<path>::<Item>[::<method>] @ v<version>[, <note>]

The comparison undoes what the port changes mechanically, so that only real changes remain:
the provenance header and the module's `avenger:` notes, imports, crate paths
(`crate::typst_library::` for `crate::` or `typst_library::`), the `elem!` and `func!` wrappers
around upstream's attribute-macro items, `name = ".."` in `#[elem(..)]`, and `ignore` on doc
fences. Both sides are then formatted with the crate's rustfmt settings, which are upstream's,
and split into items with `rust_items.py`.

    upstream_diff.py status [--counts] [--write]
        UPSTREAM.md's Mirror table: each ported file's status, V, S or A. --counts adds the
        items kept, changed, removed and added; --write replaces the table in UPSTREAM.md.
    upstream_diff.py diff [FILE ...]
        Unified diffs from upstream to Avenger.
    upstream_diff.py paths
        The upstream files the crate ports from, for `git diff OLD..NEW -- PATHS`.
    upstream_diff.py deps [REV]
        The crate's dependencies at versions that upstream's crates don't share at REV.
    upstream_diff.py bump NEW [--from OLD] [--apply]
        What upstream's changes from OLD to NEW mean for each ported item. --apply applies
        them to the ported files.

`bump` sorts the upstream items that change between OLD and NEW by what the port did with them:

    take    the port has the item verbatim, so upstream's change applies as is
    port    the port changed the item: port upstream's change by hand
    decide  upstream deleted an item the port keeps
    new     upstream added the item; "used" if the new form of an item the port keeps names it
    skip    the port removed the item

It also lists the markers in other files whose upstream item changed or disappeared, and the
ported files that NEW renames or deletes upstream. `--apply` applies upstream's diff to the
ported files with their crate paths ported, without fuzz, and leaves `.rej` files for the hunks
that don't apply.

Upstream revisions are read from a Typst git repository: `--typst`, or `TYPST_DIR`, default
`../typst` next to this repository. Its checkout can be at any commit. OLD and the revision of
`status`, `diff` and `deps` default to the commit pinned in
`avenger-typst-label/tests/fixtures/typst-pin.toml`. Needs python3 3.11 or later, rustfmt, and
patch for `--apply`.
"""
import argparse, collections, difflib, os, re, subprocess, sys, tomllib
from pathlib import Path

import rust_items

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
CRATE = REPO / 'avenger-typst-label'
REFERENCES = HERE / 'references'
UPSTREAM_MD = CRATE / 'UPSTREAM.md'
HEADER = re.compile(r'^//! Ported from (crates/\S+) @ v[^,]+, modified for Avenger\.$')
MARKER = re.compile(r'^\s*// upstream: (crates/\S+?\.rs)::(\S+) @ v[^,\s]+')
# The crates whose modules the port nests under `crate::`. `typst_syntax` and `typst_utils`
# are dependencies, so their paths are upstream's.
CRATES = ['typst_eval', 'typst_layout', 'typst_library', 'typst_realize']
MIRROR_HEADER = '| Avenger file | Upstream file | Status |'
TYPE_KINDS = ('struct', 'enum', 'trait', 'type', 'union')


def ported_files():
    """(Avenger path, upstream path) for every ported file, in path order."""
    pairs = []
    for path in sorted((CRATE / 'src').rglob('*.rs')):
        with open(path) as file:
            match = HEADER.match(file.readline().rstrip('\n'))
        if match:
            pairs.append((path, match.group(1)))
    return pairs


def markers():
    """(Avenger path, line number, upstream path, item) for every marker, in path order."""
    found = []
    for root in (CRATE / 'src', REFERENCES / 'src'):
        for path in sorted(root.rglob('*.rs')):
            for number, line in enumerate(path.read_text().split('\n'), 1):
                match = MARKER.match(line)
                if match:
                    found.append((path, number, match.group(1), match.group(2)))
    return found


def git(typst, *args):
    return subprocess.run(['git', '-C', typst, *args], capture_output=True, text=True,
                          check=True).stdout


def short(rev):
    """A revision for display: commits abbreviated, names as they are."""
    return rev[:9] if re.fullmatch(r'[0-9a-f]{40}', rev) else rev


def upstream_text(typst, rev, path):
    """The upstream file at `rev`, or None where it doesn't exist."""
    result = subprocess.run(['git', '-C', typst, 'show', f'{rev}:{path}'],
                            capture_output=True, text=True)
    return result.stdout if result.returncode == 0 else None


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


def upstream_items(text):
    return item_texts(rustfmt(normalize_upstream(text)))


def our_items(path, upstream_path):
    return item_texts(rustfmt(normalize_ours(path.read_text(), source_crate(upstream_path))))


def impl_sides(selector):
    """The trait and the type of the `impl` block a selector is in, or None."""
    head = selector.split(' :: ')[0]
    if not head.startswith('impl '):
        return None
    trait, _, ty = head[5:].rpartition(' for ')
    return trait, ty


def resolve(texts, item):
    """The selectors of the upstream items a marker's `item` names: a function, a type with
    its impl blocks, or a method. None when it names none, or several methods."""
    names = re.sub(r'<.*>', '', item).split('::')
    bare = lambda selector: selector.split(' #')[0]
    if len(names) == 1:
        (name,) = names
        if any(bare(s) == f'{kind} {name}' for s in texts for kind in TYPE_KINDS):
            return sorted(s for s in texts if bare(s) in {f'{k} {name}' for k in TYPE_KINDS}
                          or (impl_sides(s) or (None, None))[1] == name)
        hits = [s for s in texts if bare(s) == f'fn {name}']
        return hits if len(hits) == 1 else None
    if len(names) != 2:
        return None
    owner, method = names
    hits = [s for s in texts if bare(s).endswith(f' :: fn {method}')
            and owner in (impl_sides(s) or ())]
    if not hits:
        hits = [s for s in texts if bare(s) == f'trait {owner} :: fn {method}']
    return hits if len(hits) == 1 else None


def check_markers(typst, rev):
    """Exits unless every marker names exactly one upstream item at `rev`."""
    cache, bad = {}, []
    for path, number, upstream_path, item in markers():
        if upstream_path not in cache:
            text = upstream_text(typst, rev, upstream_path)
            cache[upstream_path] = upstream_items(text) if text is not None else None
        texts = cache[upstream_path]
        if texts is None or not resolve(texts, item):
            bad.append(f'{path.relative_to(REPO)}:{number}: `{upstream_path}::{item}` names '
                       f'no single item at {short(rev)}')
    if bad:
        sys.exit('\n'.join(bad))


def status(typst, rev, counts, write):
    check_markers(typst, rev)
    if counts:
        rows = ['| Avenger file | Upstream file | Kept | Changed | Removed | Added | Status |',
                '|---|---|---:|---:|---:|---:|---|']
    else:
        rows = [MIRROR_HEADER, '|---|---|---|']
    for path, upstream_path in ported_files():
        theirs, ours = compared(typst, rev, path, upstream_path)
        theirs, ours = item_texts(theirs), item_texts(ours)
        kept = sum(1 for k, v in theirs.items() if ours.get(k) == v)
        changed = sum(1 for k, v in theirs.items() if k in ours and ours[k] != v)
        removed = sum(1 for k in theirs if k not in ours)
        added = sum(1 for k in ours if k not in theirs)
        mark = 'A' if changed or added else 'S' if removed else 'V'
        rel = path.relative_to(CRATE / 'src')
        counted = f' {kept} | {changed} | {removed} | {added} |' if counts else ''
        rows.append(f'| `{rel}` | `{upstream_path}` |{counted} {mark} |')
    if not write:
        print('\n'.join(rows))
        return
    lines = UPSTREAM_MD.read_text().split('\n')
    start = lines.index(MIRROR_HEADER)
    end = start
    while end < len(lines) and lines[end].startswith('|'):
        end += 1
    lines[start:end] = rows
    UPSTREAM_MD.write_text('\n'.join(lines))


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
            fromfile=f'{upstream_path} @ {short(rev)}', tofile=str(rel)))


def upstream_paths():
    return sorted({u for _, u in ported_files()} | {m[2] for m in markers()})


def locked_versions(lock, consumer):
    """The versions of each dependency of the lock's packages that `consumer` accepts."""
    present = collections.defaultdict(set)
    for package in lock['package']:
        present[package['name']].add(package['version'])
    versions = collections.defaultdict(set)
    for package in lock['package']:
        if consumer(package['name']):
            for entry in package.get('dependencies', []):
                name, *version = entry.split(' ')
                versions[name] |= {version[0]} if version else present[name]
    return versions


def compatible(a, b):
    """Whether Cargo treats the two versions as compatible: the same leftmost nonzero part."""
    a, b = a.split('.'), b.split('.')
    significant = next((i for i, part in enumerate(a[:2]) if part != '0'), 2)
    return a[:significant + 1] == b[:significant + 1]


def deps(typst, rev):
    with open(REPO / 'Cargo.lock', 'rb') as file:
        ours = locked_versions(tomllib.load(file), lambda name: name == 'avenger-typst-label')
    theirs = locked_versions(tomllib.loads(git(typst, 'show', f'{rev}:Cargo.lock')),
                             lambda name: name.startswith('typst'))
    shared = sorted(name for name in ours if name in theirs)
    differ = [(name, ', '.join(sorted(ours[name])), ', '.join(sorted(theirs[name])))
              for name in shared
              if not any(compatible(a, b) for a in ours[name] for b in theirs[name])]
    if differ:
        print(f'| Crate | Avenger | Upstream at {short(rev)} |')
        print('|---|---|---|')
        for name, mine, upstream in differ:
            print(f'| `{name}` | {mine} | {upstream} |')
    print(f'{len(shared) - len(differ)} of the {len(shared)} dependencies that upstream\'s '
          f'crates share match their versions.')


def moved_files(typst, old, new):
    """{upstream path: new path, or None if deleted} for ported paths that NEW moves."""
    paths, moved = set(upstream_paths()), {}
    for line in git(typst, 'diff', '--name-status', '-M', f'{old}..{new}', '--',
                    'crates').splitlines():
        status, *names = line.split('\t')
        if names[0] in paths and status[0] in 'RD':
            moved[names[0]] = names[1] if status[0] == 'R' else None
    return moved


def bump(typst, old, new, apply):
    check_markers(typst, old)
    moved = moved_files(typst, old, new)
    triage, kept_new, added = {}, [], []
    for path, upstream_path in ported_files():
        new_path = moved.get(upstream_path, upstream_path)
        if new_path is None:
            continue
        old_text = upstream_text(typst, old, upstream_path)
        new_text = upstream_text(typst, new, new_path)
        if old_text == new_text:
            continue
        before, after = upstream_items(old_text), upstream_items(new_text)
        ours = our_items(path, upstream_path)
        kept_new += [text for selector, text in after.items() if selector in ours]
        classes = triage.setdefault(path, collections.defaultdict(list))
        for selector in sorted(set(before) | set(after)):
            if before.get(selector) == after.get(selector):
                continue
            if selector not in before:
                added.append((path, selector))
            elif selector not in ours:
                classes['skip'].append(selector)
            elif selector not in after:
                classes['decide'].append(selector)
            else:
                classes['take' if ours[selector] == before[selector] else 'port'].append(selector)
    kept_new = '\n'.join(kept_new)
    for path, selector in added:
        name = selector.split(' #')[0].split()[-1]
        used = re.search(rf'\b{re.escape(name)}\b', kept_new)
        triage[path]['new' if used else 'unused'].append(selector)

    totals = collections.Counter()
    sources = dict(ported_files())
    for path, classes in triage.items():
        totals.update({kind: len(selectors) for kind, selectors in classes.items()})
        if not any(classes[kind] for kind in ('port', 'decide', 'take', 'new', 'unused')):
            continue
        print(f'{path.relative_to(CRATE / "src")} ({sources[path]})')
        for kind in ('port', 'decide', 'take', 'new'):
            for selector in classes[kind]:
                print(f'  {kind:<7} {selector}{"  (used)" if kind == "new" else ""}')
        if classes['unused']:
            print(f'  new, unused: {", ".join(classes["unused"])}')
        if classes['skip']:
            print(f'  skip: {plural(len(classes["skip"]), "item")} the port removed')

    copies = []
    for path, number, upstream_path, item in markers():
        header = HEADER.match(path.read_text().split('\n', 1)[0])
        if header and header.group(1) == upstream_path:
            continue
        before = upstream_items(upstream_text(typst, old, upstream_path))
        new_path = moved.get(upstream_path, upstream_path)
        new_text = new_path and upstream_text(typst, new, new_path)
        after = upstream_items(new_text) if new_text else {}
        old_hits, new_hits = resolve(before, item), resolve(after, item)
        if not new_hits:
            copies.append(('gone', path, number, upstream_path, item))
        elif [before[s] for s in old_hits] != [after[s] for s in new_hits]:
            copies.append(('changed', path, number, upstream_path, item))
    if copies:
        print('Copies, marked in other files:')
        for change, path, number, upstream_path, item in copies:
            print(f'  {change:<7} {path.relative_to(REPO)}:{number} {upstream_path}::{item}')
    for upstream_path, new_path in sorted(moved.items()):
        print(f'Upstream {"renames" if new_path else "deletes"} {upstream_path}'
              f'{f" to {new_path}" if new_path else ""}')

    print(f'take {totals["take"]}, port {totals["port"]}, decide {totals["decide"]}, '
          f'new {totals["new"]} used and {totals["unused"]} unused, skip {totals["skip"]}; '
          f'{plural(len(copies), "copy", "copies")} changed or gone; '
          f'{plural(len(moved), "file")} moved upstream')
    if apply:
        apply_changes(typst, old, new, moved)


def plural(count, noun, nouns=None):
    return f'{count} {noun if count == 1 else nouns or noun + "s"}'


def port_paths(text, crate):
    """Upstream code with crate paths as the port writes them: the reverse of
    `normalize_ours`'s path rewrite."""
    text = re.sub(r'(?<![\w:$])crate::', f'crate::{crate}::', text)
    for other in CRATES:
        if other != crate:
            text = re.sub(rf'(?<![\w:$]){other}::', f'crate::{other}::', text)
    return text


def apply_changes(typst, old, new, moved):
    """Applies upstream's diff from `old` to `new` to each ported file, with crate paths
    ported and no fuzz, and reports the hunks that apply, that the file already has, and that
    are left in a `.rej` file."""
    for path, upstream_path in ported_files():
        rel = path.relative_to(REPO)
        if upstream_path in moved:
            print(f'{rel}: moved upstream, so port it by hand')
            continue
        patch = git(typst, 'diff', '--no-ext-diff', '--no-color', f'{old}..{new}', '--',
                    upstream_path)
        if not patch:
            continue
        hunks = patch.count('\n@@ ')
        crate = source_crate(upstream_path)
        lines = []
        for line in patch.splitlines(keepends=True):
            if line.startswith(('--- a/', '+++ b/')):
                line = f'{line[:6]}{rel}\n'
            elif line[:1] in ' +-' and not line.startswith(('---', '+++')):
                line = line[0] + port_paths(line[1:], crate)
            lines.append(line)
        # `-N` skips hunks that the file already has, and says so.
        result = subprocess.run(
            ['patch', '-p1', '-N', '-F', '0', '--no-backup-if-mismatch', '-d', str(REPO)],
            input=''.join(lines), capture_output=True, text=True)
        output = result.stdout + result.stderr
        count = lambda word: sum(int(n) for n in re.findall(
            rf'(\d+) out of \d+ hunks? {word}', output, flags=re.I))
        failed, already = count('failed'), count('ignored')
        parts = [f'{hunks - failed - already} applied']
        parts += [f'{already} already applied'] if already else []
        parts += [f'{failed} rejected to {rel}.rej'] if failed else []
        print(f'{rel}: {hunks} hunks: ' + ', '.join(parts))


def main():
    parser = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    parser.add_argument('--typst', default=os.environ.get('TYPST_DIR', REPO.parent / 'typst'),
                        help='a Typst git repository (default: TYPST_DIR, or ../typst)')
    commands = parser.add_subparsers(dest='command', required=True)
    status_parser = commands.add_parser('status', help="UPSTREAM.md's Mirror table")
    status_parser.add_argument('--counts', action='store_true',
                               help='add the items kept, changed, removed and added')
    status_parser.add_argument('--write', action='store_true',
                               help="replace UPSTREAM.md's Mirror table")
    diff_parser = commands.add_parser('diff', help='unified diffs from upstream')
    diff_parser.add_argument('files', nargs='*', help='Avenger files (default: all)')
    commands.add_parser('paths', help='the upstream files the crate ports from')
    deps_parser = commands.add_parser('deps', help="dependencies off upstream's versions")
    deps_parser.add_argument('rev', nargs='?', help='the upstream revision (default: pinned)')
    bump_parser = commands.add_parser('bump', help="upstream's changes, item by item")
    bump_parser.add_argument('new', help='the new revision')
    bump_parser.add_argument('--from', dest='old', help='the old revision (default: pinned)')
    bump_parser.add_argument('--apply', action='store_true',
                             help='apply the changes to the ported files')
    args = parser.parse_args()
    if args.command == 'status' and args.counts and args.write:
        parser.error("--write replaces UPSTREAM.md's table, which has no counts")
    with open(CRATE / 'tests' / 'fixtures' / 'typst-pin.toml', 'rb') as file:
        rev = tomllib.load(file)['commit']
    typst = str(args.typst)
    if args.command == 'paths':
        print('\n'.join(upstream_paths()))
    elif args.command == 'deps':
        deps(typst, args.rev or rev)
    elif args.command == 'bump':
        bump(typst, args.old or rev, args.new, args.apply)
    elif args.command == 'status':
        status(typst, rev, args.counts, args.write)
    else:
        diff(typst, rev, args.files)


if __name__ == '__main__':
    main()
