#!/usr/bin/env python3
"""Line-level item model for rustfmt-formatted Rust source.

Used by the typst-sync prototype to remove whole items (with their attached doc
comments/attributes) from pinned upstream Typst files, so that the vendored copy
is "upstream minus listed items" plus mechanical path rewrites.
"""
import re, sys

class Item:
    def __init__(self, kind, name, start, header, end, indent, children=None):
        self.kind, self.name, self.start, self.header, self.end = kind, name, start, header, end
        self.indent, self.children = indent, children or []
    @property
    def selector(self):
        return f"{self.kind} {self.name}"
    def __repr__(self):
        return f"{self.selector} [{self.start+1}-{self.end+1}]"

def scan_end(lines, i):
    """Return the index of the last line of the item whose header starts at line i."""
    depth = 0; opened_block = False
    j = i
    while j < len(lines):
        s = lines[j]; k = 0; n = len(s)
        while k < n:
            c = s[k]
            if s.startswith('//', k):
                break
            if s.startswith('/*', k):
                # block comment (nested), may span lines
                lvl = 0
                while True:
                    if s.startswith('/*', k): lvl += 1; k += 2; continue
                    if s.startswith('*/', k):
                        lvl -= 1; k += 2
                        if lvl == 0: break
                        continue
                    if k >= n:
                        j += 1; s = lines[j]; k = 0; n = len(s); continue
                    k += 1
                continue
            m = re.match(r'b?r(#*)"', s[k:])
            if m and (k == 0 or not (s[k-1].isalnum() or s[k-1] == '_')):
                close = '"' + m.group(1); k += m.end()
                while True:
                    idx = s.find(close, k)
                    if idx >= 0: k = idx + len(close); break
                    j += 1; s = lines[j]; k = 0; n = len(s)
                continue
            if c == '"' or (c == 'b' and s.startswith('b"', k)):
                if c == 'b': k += 1
                k += 1
                while True:
                    if k >= n:
                        j += 1; s = lines[j]; k = 0; n = len(s); continue
                    if s[k] == '\\': k += 2; continue
                    if s[k] == '"': k += 1; break
                    k += 1
                continue
            if c == "'":
                if k + 1 < n and s[k+1] == '\\':
                    end = s.find("'", k + 2)
                    # handle '\''
                    if s.startswith("'\\''", k): end = k + 3
                    k = end + 1; continue
                if k + 2 < n and s[k+2] == "'":
                    k += 3; continue
                # multi-byte char literal like '⋯' is still one char in Python
                k += 1; continue  # lifetime
            if c in '{([':
                depth += 1
                if c == '{' and depth == 1: opened_block = True
            elif c in '})]':
                depth -= 1
                if depth == 0 and c == '}' and opened_block:
                    rest = s[k+1:].strip()
                    if rest.startswith(';') or rest.startswith(')'):
                        pass  # `};` / `});` -> keep scanning until ';'
                    else:
                        return j
            elif c == ';' and depth == 0:
                return j
            k += 1
        j += 1
    raise ValueError(f"unterminated item at line {i+1}: {lines[i]!r}")

HEADER = re.compile(r'^(?:pub(?:\([^)]*\))?\s+)?(?:(?:const|unsafe|async|extern\s+"C")\s+)*'
                    r'(fn|struct|enum|trait|type|const|static|mod|union)\s+([A-Za-z_][A-Za-z0-9_]*)')

def impl_name(text):
    # text: header up to '{' (may span lines, joined)
    t = text.strip()
    t = re.sub(r'\s+', ' ', t)
    t = re.sub(r'^unsafe ', '', t)
    assert t.startswith('impl'), t
    t = t[4:]
    # strip leading generics
    if t.startswith('<'):
        d = 0
        for idx, ch in enumerate(t):
            if ch == '<': d += 1
            elif ch == '>':
                d -= 1
                if d == 0: t = t[idx+1:]; break
    t = t.split(' where ')[0].split('{')[0].strip()
    def strip_generics(s):
        out = ''; d = 0
        for ch in s:
            if ch == '<': d += 1; continue
            if ch == '>': d -= 1; continue
            if d == 0: out += ch
        return out.strip()
    if ' for ' in t:
        tr, ty = t.split(' for ', 1)
        return f"{strip_generics(tr)} for {strip_generics(ty)}"
    return strip_generics(t)

def classify(lines, i, indent):
    s = lines[i][indent:]
    m = HEADER.match(s)
    if m:
        return m.group(1), m.group(2)
    if s.startswith('impl') and (s[4:5] in ' <'):
        # collect header text until '{' or ';'
        hdr = ''; j = i
        while True:
            hdr += ' ' + lines[j].strip()
            if lines[j].rstrip().endswith('{') or lines[j].rstrip().endswith('{}') or lines[j].rstrip().endswith(';'): break
            j += 1
        return 'impl', impl_name(hdr)
    if s.startswith('unsafe impl'):
        return 'impl', impl_name(s)
    m = re.match(r'macro_rules!\s+(\w+)', s)
    if m: return 'macro', m.group(1)
    m = re.match(r'(\w+)!\s*\{', s)
    if m:
        # node! { struct Name }
        j = i + 1
        while j < len(lines):
            mm = re.match(r'\s*struct\s+(\w+)', lines[j])
            if mm: return m.group(1) + '!', mm.group(1)
            if lines[j].startswith(' ' * indent + '}'): break
            j += 1
        return m.group(1) + '!', f'@{i+1}'
    m = re.match(r'(?:pub(?:\([^)]*\))?\s+)?use\s+(.*)', s)
    if m: return 'use', m.group(1).rstrip()
    m = re.match(r'(\w+)!', s)
    if m: return m.group(1) + '!', f'@{i+1}'
    return None

def parse_block(lines, lo, hi, indent):
    """Parse items in lines[lo:hi] whose header is at exactly `indent` spaces."""
    items = []; i = lo; pending = None
    while i < hi:
        line = lines[i]
        if not line.strip():
            pending = None; i += 1; continue
        cur = len(line) - len(line.lstrip(' '))
        st = line.strip()
        if cur != indent:
            i += 1; continue
        if st.startswith('//') or st.startswith('#[') or st.startswith('#!['):
            if st.startswith('#![') or st.startswith('//!'):
                i += 1; continue  # inner attrs / module docs belong to the file
            if pending is None: pending = i
            if st.startswith('#[') and not st.endswith(']'):
                # multi-line attribute
                j = scan_end_attr(lines, i); i = j + 1; continue
            i += 1; continue
        cls = classify(lines, i, indent)
        if cls is None:
            i += 1; pending = None; continue
        kind, name = cls
        end = scan_end(lines, i)
        start = pending if pending is not None else i
        it = Item(kind, name, start, i, end, indent)
        if kind in ('impl', 'mod', 'trait') and not lines[end].strip().endswith(';'):
            it.children = parse_block(lines, i + 1, end, indent + 4)
        items.append(it)
        pending = None
        i = end + 1
    return items

def scan_end_attr(lines, i):
    depth = 0; j = i
    while True:
        for ch in lines[j]:
            if ch == '[': depth += 1
            elif ch == ']': depth -= 1
        if depth == 0: return j
        j += 1

def parse(text):
    lines = text.split('\n')
    return lines, parse_block(lines, 0, len(lines), 0)

def walk(items, prefix=''):
    for it in items:
        yield prefix, it
        if it.children:
            yield from walk(it.children, prefix + it.selector + ' :: ')

def select(items, sel):
    """Selector grammar: 'kind name' or 'parent kind name :: kind name' (exact)."""
    hits = [it for p, it in walk(items) if (p + it.selector) == sel]
    return hits

if __name__ == '__main__':
    lines, items = parse(open(sys.argv[1]).read())
    for p, it in walk(items):
        print(f"{it.end - it.start + 1:5d}  {p}{it.selector}   [{it.start+1}-{it.end+1}]")
