"""Extracts the inputs of GoogleSQL's parser tests that fall in avenger-sql's subset, for the
conformance check of `tests/conformance.rs`.

GoogleSQL's parser tests are the `.test` files of `googlesql/parser/testdata` at the tag below, the
tag of the `execute_query` binary that the check runs. Each file holds cases separated by lines of
`==`; a case is option lines in brackets, the input, and after a line of `--` the expected output,
which the check ignores, since it runs the binary itself. An input can hold alternations,
`{{a|b}}`, which give one input for each choice.

An input falls in the subset when its first statement starts as a statement of avenger-sql does: a
query that starts with `FROM`, or `DECLARE`, `SET`, `CREATE TEMP`, `CREATE TEMPORARY`,
`CREATE OR REPLACE TEMP`, or `IMPORT`. Most tests of the pipe operators start with a standard
query, as `select x |> SET x = 1`, which isn't in the subset, so the corpus also holds each of them
with the text before its first pipe operator as `FROM t`, labeled `:from`. The check compares the
binary's verdict and ours on each input, so an input that both reject still checks the place of
the first error.

The script clones the tag into `scratch/tools/googlesql/googlesql-<TAG>`, unless it is there, and
writes the corpus to `scratch/tools/googlesql/parser-corpus.txt`: a line `## <file>:<case>[:<alt>]`
before each input. avenger doesn't commit GoogleSQL's text, so the corpus stays in `scratch/`.

Run from the repository's root: python3 avenger-sql-syntax/tests/conformance/extract.py
"""
import itertools, os, re, subprocess

TAG = "2026.11.1"
TOOLS = "scratch/tools/googlesql"
CHECKOUT = f"{TOOLS}/googlesql-{TAG}"
TESTDATA = f"{CHECKOUT}/googlesql/parser/testdata"
CORPUS = f"{TOOLS}/parser-corpus.txt"

# The first words of a statement of the subset.
STARTS = re.compile(
    r"^\(*\s*(FROM\b|DECLARE\b|SET\b|IMPORT\b|CREATE\s+(OR\s+REPLACE\s+)?TEMP(ORARY)?\b)",
    re.IGNORECASE,
)
# The most inputs that the alternations of one case give.
MOST = 64


def checkout():
    if os.path.isdir(TESTDATA):
        return
    subprocess.run(
        ["git", "clone", "--quiet", "--depth", "1", "--branch", TAG, "--filter=blob:none",
         "--sparse", "https://github.com/google/googlesql", CHECKOUT],
        check=True,
    )
    subprocess.run(["git", "-C", CHECKOUT, "sparse-checkout", "set", "googlesql/parser/testdata"],
                   check=True)


def inputs(case):
    """The input of a case, before its first `--` line, without its comments and options."""
    lines = []
    for line in case.split("\n"):
        if line == "--":
            break
        if not lines and (line.startswith("#") or line.startswith("[") or not line.strip()):
            continue
        lines.append(line)
    return "\n".join(lines).rstrip("\n")


def alternatives(text):
    """Each choice of the alternations of `text`."""
    parts = re.split(r"\{\{(.*?)\}\}", text, flags=re.S)
    if len(parts) == 1:
        return [("", text)]
    fixed, groups = parts[0::2], [group.split("|") for group in parts[1::2]]
    out = []
    for index, choice in enumerate(itertools.islice(itertools.product(*groups), MOST)):
        text = fixed[0] + "".join(c + f for c, f in zip(choice, fixed[1:]))
        out.append((str(index), text))
    return out


def first_statement(text):
    """`text` without its leading comments and white space."""
    while True:
        stripped = text.lstrip()
        if stripped.startswith("--") or stripped.startswith("#"):
            text = stripped.split("\n", 1)[1] if "\n" in stripped else ""
        elif stripped.startswith("/*") and "*/" in stripped:
            text = stripped.split("*/", 1)[1]
        else:
            return stripped


def from_query(text):
    """`text`, a standard query with pipe operators, with the text before its first pipe operator
    as `FROM t`, or None when its first `|>` is inside parentheses or it has none."""
    depth, quote = 0, None
    for i, c in enumerate(text):
        if quote:
            quote = None if c == quote else quote
        elif c in "'\"`":
            quote = c
        elif c == "(":
            depth += 1
        elif c == ")":
            depth -= 1
        elif text.startswith("|>", i):
            return None if depth else "FROM t\n" + text[i:]
    return None


def main():
    checkout()
    count = 0
    with open(CORPUS, "w") as out:
        out.write(f"# GoogleSQL {TAG}'s parser test inputs in avenger-sql's subset, by extract.py.\n")
        for name in sorted(os.listdir(TESTDATA)):
            if not name.endswith(".test"):
                continue
            # A few files test bytes that aren't UTF-8, which a `&str` can't hold.
            text = open(os.path.join(TESTDATA, name), encoding="utf-8", errors="replace").read()
            cases = re.split(r"^==$", text, flags=re.M)
            for index, case in enumerate(cases):
                source = inputs(case)
                for alt, text in alternatives(source):
                    if "\ufffd" in text or not text.strip():
                        continue
                    label = f"{name}:{index}" + (f":{alt}" if alt else "")
                    first = first_statement(text)
                    if not STARTS.match(first):
                        if not re.match(r"select\b", first, re.IGNORECASE):
                            continue
                        text = from_query(first)
                        if text is None:
                            continue
                        label += ":from"
                    out.write(f"## {label}\n{text}\n")
                    count += 1
    print(f"{count} inputs in {CORPUS}")


if __name__ == "__main__":
    main()
