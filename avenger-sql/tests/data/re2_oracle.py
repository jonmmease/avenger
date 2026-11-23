# /// script
# requires-python = ">=3.10"
# dependencies = ["google-re2"]
# ///
"""Records RE2's answers for the cases of avenger-sql's regular expressions, for `tests/re2.rs`.

GoogleSQL's regex functions run RE2, so RE2 is their reference: `REGEXP_CONTAINS` is an unanchored
match, `REGEXP_EXTRACT` the first match, or its one capturing group, and `REGEXP_REPLACE` RE2's
`GlobalReplace` with its rewrite string, where `\\0` to `\\9` are the matches of the groups and `\\\\`
is a backslash. The Python binding of RE2, `google-re2`, gives the matches, and this script ports
`GlobalReplace` and the rewrite's rules from RE2's `re2.cc`, since the binding has neither.

Run it with uv, which installs `google-re2`, the dependency that the header names, in an
environment of its own:

    uv run avenger-sql/tests/data/re2_oracle.py

It writes `re2.jsonl` beside itself: one case on each line, with RE2's result or `"error": true`.
"""

import json
import pathlib

import re2

_re2 = re2._re2
UNANCHORED = _re2.RE2.Anchor.UNANCHORED

# Each case: the function, the pattern, the text, and for REGEXP_REPLACE the rewrite string.
CASES = [
    # The Perl classes are ASCII: a digit, a word character, and white space of RE2's set, which
    # holds no vertical tab.
    ("contains", r"\d", "7"),
    ("contains", r"\d", "٣"),
    ("contains", r"\D", "٣"),
    ("contains", r"[\d]", "٣"),
    ("contains", r"[^\d]", "٣"),
    ("contains", r"[\D]", "٣"),
    ("contains", r"[a\d]", "٣"),
    ("contains", r"\w", "é"),
    ("contains", r"\W", "é"),
    ("contains", r"[\w]", "é"),
    ("contains", r"[^\w]", "é"),
    ("contains", r"^\w+$", "abc_123"),
    ("contains", r"\s", "\t"),
    ("contains", r"\s", "\v"),
    ("contains", r"\s", " "),
    ("contains", r"\s", " "),
    ("contains", r"\S", "\v"),
    ("contains", r"[\s]", "\v"),
    ("contains", r"[^\s]", "\v"),
    ("contains", r"[\S]", "\v"),
    ("contains", r"[x\s]", "\f"),
    ("contains", r"[x\S]", " "),
    # Word boundaries are ASCII too.
    ("contains", r"\bé", "é"),
    ("contains", r"é\b", "é"),
    ("contains", r"\Bé", "é"),
    ("contains", r"\bfoo\b", "éfoo"),
    ("contains", r"\bfoo\b", "a foo"),
    ("contains", r"a\Bb", "ab"),
    # Case folding of the classes.
    ("contains", r"(?i)\w", "K"),
    ("contains", r"(?i)\w", "ſ"),
    ("contains", r"(?i)\d", "٣"),
    ("contains", r"(?i)[[:word:]]", "K"),
    ("contains", r"(?i)k", "K"),
    # The other classes keep their sets.
    ("contains", r"[[:alpha:]]", "é"),
    ("contains", r"[[:space:]]", "\v"),
    ("contains", r"\pL", "é"),
    ("contains", r"\p{Greek}", "α"),
    ("contains", r"\PN", "٣"),
    ("contains", r".", "é"),
    # The first match, or its group.
    ("extract", r"\d+", "a٣1234"),
    ("extract", r"(\d+)", "a٣ 12"),
    ("extract", r"\w+", "éabc d"),
    ("extract", r"\s+(\S+)", "a b c"),
    ("extract", r"\b\w+\b", "éab cd"),
    ("extract", r"x", "abc"),
    # REGEXP_REPLACE's rewrite string.
    ("replace", r"b", "abc", r"$1x"),
    ("replace", r"(b)", "abc", r"$1"),
    ("replace", r"(b)", "abc", r"${1}"),
    ("replace", r"(b)", "abc", r"\1\1"),
    ("replace", r"(b)", "abc", r"\\1"),
    ("replace", r"(b)", "abc", r"\12"),
    ("replace", r"b", "abc", r"\0\0"),
    ("replace", r"b", "abc", r"\\"),
    ("replace", r"b", "abc", r"\\\\"),
    ("replace", r"(a)(b)", "abc", r"\2\1"),
    ("replace", r"(a)|(x)", "abc", r"[\2]"),
    ("replace", r"(b)", "abc", r"\2"),
    ("replace", r"b", "abc", r"\x"),
    ("replace", r"b", "abc", "\\"),
    ("replace", r"b", "abc", r"\n"),
    # Every match, and the empty matches between them.
    ("replace", r"x*", "abc", "-"),
    ("replace", r"b*", "abc", "-"),
    ("replace", r"", "aé", "-"),
    ("replace", r"\b", "ab cd", "|"),
    ("replace", r"a|", "baaa", "-"),
    ("replace", r"\d", "a٣1", "#"),
    ("replace", r"\s", "a b c\vd", "_"),
    ("replace", r"\w+", "été ok", "W"),
]


def rewrite(template: bytes, groups: list, text: bytes) -> bytes:
    """RE2's `Rewrite`: `\\N` is the match of group N, `\\\\` a backslash, and other text as is."""
    out = bytearray()
    i = 0
    while i < len(template):
        c = template[i : i + 1]
        if c != b"\\":
            out += c
            i += 1
            continue
        nxt = template[i + 1 : i + 2]
        if nxt.isdigit():
            start, end = groups[int(nxt)]
            if start >= 0:
                out += text[start:end]
            i += 2
        elif nxt == b"\\":
            out += b"\\"
            i += 2
        else:
            raise ValueError("invalid rewrite pattern")
    return bytes(out)


def check_rewrite(template: bytes, regexp) -> None:
    """RE2's `CheckRewriteString`: every escape is `\\N` or `\\\\`, and N is a group of `regexp`."""
    largest = -1
    i = 0
    while i < len(template):
        if template[i : i + 1] == b"\\":
            nxt = template[i + 1 : i + 2]
            if nxt == b"\\":
                i += 2
                continue
            if not nxt.isdigit():
                raise ValueError("invalid rewrite pattern")
            largest = max(largest, int(nxt))
            i += 2
            continue
        i += 1
    if largest > regexp.NumberOfCapturingGroups():
        raise ValueError("rewrite requests a group that the pattern lacks")


def utf8_char_length(text: bytes, at: int) -> int:
    """The length of the UTF-8 character at `at`."""
    first = text[at]
    for length, mask in ((1, 0x80), (2, 0xE0), (3, 0xF0), (4, 0xF8)):
        if length == 1 and first < 0x80:
            return 1
        if length > 1 and (first & mask) == ((mask << 1) & 0xFF):
            return length
    return 1


def global_replace(regexp, text: bytes, template: bytes) -> bytes:
    """RE2's `GlobalReplace`: each match, after an empty match at the end of the last one."""
    check_rewrite(template, regexp)
    out = bytearray()
    p = 0
    last_end = None
    count = 0
    while p <= len(text):
        spans = regexp.Match(UNANCHORED, text, p, len(text))
        start, end = spans[0]
        if start < 0:
            break
        if p < start:
            out += text[p:start]
        if start == last_end and start == end:
            # An empty match at the end of the last match: copy one character and go on.
            if p < len(text):
                n = utf8_char_length(text, p)
                out += text[p : p + n]
                p += n
            else:
                p += 1
            continue
        out += rewrite(template, spans, text)
        p = end
        last_end = p
        count += 1
    if count == 0:
        return text
    if p < len(text):
        out += text[p:]
    return bytes(out)


def answer(case):
    function, pattern, text = case[0], case[1], case[2]
    regexp = _re2.RE2(pattern.encode(), re2.Options())
    if not regexp.ok():
        return {"error": True}
    data = text.encode()
    if function == "contains":
        return {"result": regexp.Match(UNANCHORED, data, 0, len(data))[0][0] >= 0}
    if function == "extract":
        if regexp.NumberOfCapturingGroups() > 1:
            return {"error": True}
        spans = regexp.Match(UNANCHORED, data, 0, len(data))
        group = 1 if regexp.NumberOfCapturingGroups() == 1 else 0
        start, end = spans[group] if spans[0][0] >= 0 else (-1, -1)
        return {"result": data[start:end].decode() if start >= 0 else None}
    try:
        return {"result": global_replace(regexp, data, case[3].encode()).decode()}
    except ValueError:
        return {"error": True}


def main():
    lines = []
    for case in CASES:
        record = {"function": case[0], "pattern": case[1], "text": case[2]}
        if case[0] == "replace":
            record["replacement"] = case[3]
        record.update(answer(case))
        lines.append(json.dumps(record, ensure_ascii=False))
    path = pathlib.Path(__file__).with_name("re2.jsonl")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {len(lines)} cases to {path}")


if __name__ == "__main__":
    main()
