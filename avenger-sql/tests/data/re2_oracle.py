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

It writes `re2.jsonl` beside itself: one case on each line, with RE2's result or `"error": true`,
and `src/library/re2/properties.txt`, the property names that RE2 accepts, exactly and in case:
`Any`, the general categories, and the scripts.
"""

import json
import pathlib

import re2

_re2 = re2._re2
UNANCHORED = _re2.RE2.Anchor.UNANCHORED

# The candidate names of properties, which the script tries one by one and writes those that RE2
# accepts to `src/library/re2/properties.txt`: `Any`, the general categories, and the scripts of
# Unicode, as the `regex` crate's tables name them, with names that RE2 rejects among them.
GENERAL_CATEGORIES = "C Cc Cf Cn Co Cs L LC Ll Lm Lo Lt Lu M Mc Me Mn N Nd Nl No P Pc Pd Pe Pf Pi Po Ps S Sc Sk Sm So Z Zl Zp Zs".split()
SCRIPTS = """
Adlam Ahom Anatolian_Hieroglyphs Arabic Armenian Avestan Balinese Bamum
Bassa_Vah Batak Bengali Bhaiksuki Bopomofo Brahmi Braille Buginese
Buhid Canadian_Aboriginal Carian Caucasian_Albanian Chakma Cham Cherokee Chorasmian
Common Coptic Cuneiform Cypriot Cypro_Minoan Cyrillic Deseret Devanagari
Dives_Akuru Dogra Duployan Egyptian_Hieroglyphs Elbasan Elymaic Ethiopic Garay
Georgian Glagolitic Gothic Grantha Greek Gujarati Gunjala_Gondi Gurmukhi
Gurung_Khema Han Hangul Hanifi_Rohingya Hanunoo Hatran Hebrew Hiragana
Imperial_Aramaic Inherited Inscriptional_Pahlavi Inscriptional_Parthian Javanese Kaithi Kannada Katakana
Kawi Kayah_Li Kharoshthi Khitan_Small_Script Khmer Khojki Khudawadi Kirat_Rai
Lao Latin Lepcha Limbu Linear_A Linear_B Lisu Lycian
Lydian Mahajani Makasar Malayalam Mandaic Manichaean Marchen Masaram_Gondi
Medefaidrin Meetei_Mayek Mende_Kikakui Meroitic_Cursive Meroitic_Hieroglyphs Miao Modi Mongolian
Mro Multani Myanmar Nabataean Nag_Mundari Nandinagari New_Tai_Lue Newa
Nko Nushu Nyiakeng_Puachue_Hmong Ogham Ol_Chiki Ol_Onal Old_Hungarian Old_Italic
Old_North_Arabian Old_Permic Old_Persian Old_Sogdian Old_South_Arabian Old_Turkic Old_Uyghur Oriya
Osage Osmanya Pahawh_Hmong Palmyrene Pau_Cin_Hau Phags_Pa Phoenician Psalter_Pahlavi
Rejang Runic Samaritan Saurashtra Sharada Shavian Siddham SignWriting
Sinhala Sogdian Sora_Sompeng Soyombo Sundanese Sunuwar Syloti_Nagri Syriac
Tagalog Tagbanwa Tai_Le Tai_Tham Tai_Viet Takri Tamil Tangsa
Tangut Telugu Thaana Thai Tibetan Tifinagh Tirhuta Todhri
Toto Tulu_Tigalari Ugaritic Vai Vithkuqi Wancho Warang_Citi Yezidi
Yi Zanabazar_Square
""".split()
OTHER_NAMES = "Any Assigned ASCII Letter Greek_And_Coptic greek GREEK Alphabetic Emoji".split()

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
    # RE2's syntax that `regex` reads otherwise or rejects.
    ("contains", r"\Q.*\E", "a.*b"),
    ("contains", r"\Q.*\E", "ab"),
    ("extract", r"\Qa.b", "xa.b"),
    ("extract", r"a\Q\E+", "aaa"),
    ("contains", r"\E", "E"),
    ("contains", r"[\Q]\E]", "]"),
    ("extract", r"\101\0123", "A\n3"),
    ("extract", r"\0", "a\0b"),
    ("extract", r"\12\1234", "\nS4"),
    ("extract", r"\400", "\u0100"),
    ("contains", r"\1", "1"),
    ("contains", r"\8", "8"),
    ("contains", r"\18", "18"),
    ("contains", r"\p{^Greek}", "a"),
    ("contains", r"\p{^Greek}", "\u03b1"),
    ("contains", r"\P{^Greek}", "\u03b1"),
    ("contains", r"[\p{^Greek}]", "\u03b1"),
    ("contains", r"\p{Letter}", "a"),
    ("contains", r"\p{greek}", "\u03b1"),
    ("contains", r"\p{Cn}", "a"),
    ("contains", r"\p{script=Greek}", "\u03b1"),
    ("contains", r"\p{Emoji}", "a"),
    ("contains", r"\pX", "a"),
    ("contains", r"\p{Any}", "a"),
    ("contains", r"\p{^Any}", "a"),
    ("extract", r"a{,3}", "a{,3}"),
    ("extract", r"x{foo}", "x{foo}"),
    ("extract", r"a{1", "a{1"),
    ("extract", r"{", "x{"),
    ("extract", r"a{ 2}", "a{ 2}"),
    ("extract", r"\<a\>", "<a>"),
    ("contains", r"\b{start}", "a{start}"),
    ("extract", r"\b{2}", "a"),
    ("extract", r"[[]", "["),
    ("contains", r"[a-z&&[^aeiou]]", "e"),
    ("extract", r"[a-z&&[^aeiou]]", "&]"),
    ("extract", r"[a[bc]]", "[]"),
    ("extract", r"[a-z--c]", "-"),
    ("extract", r"[a-z--c]", "Z"),
    ("extract", r"[a~~b]", "~"),
    ("extract", r"[\d-z]", "-"),
    ("contains", r"[\d-z]", "y"),
    ("extract", r"[]a]", "]"),
    ("contains", r"[^]a]", "]"),
    ("extract", r"[a-]", "-"),
    ("contains", r"[[:foo:]]", "a"),
    ("contains", r"[a-\d]", "a"),
    ("contains", r"[z-a]", "a"),
    ("extract", r"[\101-\132]+", "aMZb"),
    ("extract", r"\_\#\ \:\%\@", "_# :%@"),
    ("contains", r"\e", "e"),
    ("contains", r"\x4", "x"),
    ("contains", r"\x{110000}", "x"),
    ("contains", r"\x{D800}", "a"),
    ("contains", r"[\x{D800}]", "a"),
    ("contains", r"[^\x{D800}]", "a"),
    ("extract", r"[\x{D7FF}-\x{E000}]", "a\ue000"),
    ("contains", r"\C", "a"),
    ("extract", r"(?)a", "a"),
    ("contains", r"(?x)a", "a"),
    ("contains", r"(?R)a", "a"),
    ("contains", r"(?u)a", "a"),
    ("contains", r"(?-)a", "a"),
    ("contains", r"(?i-)a", "a"),
    ("extract", r"(?i-s)A.", "aa"),
    ("extract", r"(?U)a+", "aaa"),
    ("extract", r"(?P<n>a)b", "ab"),
    ("extract", r"(?<n>a)b", "ab"),
    ("extract", r"(?P<a>x)(?P<a>y)", "xy"),
    ("extract", r"(?P<1a>x)", "x"),
    ("contains", r"(?P<>x)", "x"),
    ("contains", r"(?=a)", "a"),
    ("contains", r"(?<=a)b", "ab"),
    ("contains", r"(a)\1", "aa"),
    # Repetitions that RE2 rejects.
    ("contains", r"a**", "a"),
    ("contains", r"a*+", "a"),
    ("contains", r"a*?+", "a"),
    ("contains", r"a{2}*", "a"),
    ("contains", r"x{2}{3}", "x"),
    ("extract", r"a+?", "aaa"),
    ("extract", r"a??", "a"),
    ("contains", r"a{1001}", "a"),
    ("extract", r"a{1000}", "a"),
    ("contains", r"a{3,2}", "a"),
    ("extract", r"a{1000,}", "a"),
    ("extract", r"(?:a{500}){2}", "a"),
    ("contains", r"(?:a{500}){3}", "a"),
    ("extract", r"(?:a{2}){500}", "a"),
    ("contains", r"(?:a{2}){501}", "a"),
    ("contains", r"(a*){1001}", "a"),
    ("contains", r"{2}", "a"),
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
    options = re2.Options()
    options.log_errors = False
    regexp = _re2.RE2(pattern.encode(), options)
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
    options = re2.Options()
    options.log_errors = False
    names = [
        name
        for name in GENERAL_CATEGORIES + SCRIPTS + OTHER_NAMES
        if _re2.RE2(("\\p{%s}" % name).encode(), options).ok()
    ]
    properties = pathlib.Path(__file__).parents[2] / "src/library/re2/properties.txt"
    properties.write_text("\n".join(names) + "\n", encoding="utf-8")
    print(f"wrote {len(names)} property names to {properties}")


if __name__ == "__main__":
    main()
