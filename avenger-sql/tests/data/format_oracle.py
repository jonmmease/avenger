# /// script
# requires-python = ">=3.10"
# ///
"""Records GoogleSQL's answers for the cases of avenger-sql's `FORMAT`, for `tests/format.rs`.

GoogleSQL's `execute_query` runs each case as `SELECT FORMAT(...)` in its default time zone,
America/Los_Angeles, which the text of a `TIMESTAMP` follows. Run it with the path of
`execute_query`, a binary of GoogleSQL's releases, which the repository doesn't hold:

    uv run avenger-sql/tests/data/format_oracle.py path/to/execute_query

It writes `format.jsonl` beside itself: one case on each line, its arguments as SQL, and
GoogleSQL's result, `null` for `NULL`, or its error.
"""

import json
import pathlib
import re
import subprocess
import sys

# Each case: the arguments of FORMAT, as SQL.
CASES = [
    # Literal text and `%%`.
    ["'abc'"],
    ["'100%%'"],
    ["''"],
    ["'%%d'"],
    ["CAST(NULL AS STRING)", "1"],
    # Integers, exactly at any size.
    ["'%d'", "42"],
    ["'%i'", "-42"],
    ["'%d'", "9007199254740993"],
    ["'%d'", "-9223372036854775808"],
    ["'%5d|%-5d|%05d'", "1", "2", "3"],
    ["'%+d % d'", "5", "5"],
    ["'%.3d'", "5"],
    ["'%.0d'", "0"],
    ["'%08.3d'", "5"],
    ["'%-8.3d|'", "-5"],
    ["\"%'d\"", "1234567"],
    ["\"%'d\"", "-1234567"],
    ["\"%'.8d\"", "42"],
    ["\"%'010d\"", "1234567"],
    ["'%o'", "8"],
    ["'%#o'", "8"],
    ["'%#o'", "0"],
    ["'%x %X'", "255", "255"],
    ["'%#x %#X'", "255", "255"],
    ["'%#x'", "0"],
    ["'%x'", "-255"],
    ["'%o'", "-8"],
    ["\"%'x\"", "3735928559"],
    ["\"%'o\"", "1234567"],
    ["'%#010x'", "255"],
    ["'%#.6x'", "255"],
    ["'%#.6o'", "8"],
    ["'%08.3x|%08.3o|%-08.3x|%#08.3x'", "255", "8", "255", "255"],
    ["\"%'08.3d|%'+08.3d|%+08.3d\"", "5", "5", "5"],
    ["'%.0x|%.0o|%#.0o|%#.0x'", "0", "0", "0", "0"],
    ["\"%'.0d\"", "0"],
    ["'%d'", "CAST(NULL AS INT64)"],
    # Doubles.
    ["'%f'", "3.14159"],
    ["'%.2f'", "2.675"],
    ["'%.0f %.0f'", "2.5", "3.5"],
    ["'%.20f'", "1.1"],
    ["'%#.0f'", "3.0"],
    ["'%10.3f|%-10.3f|%010.3f'", "3.14159", "3.14159", "-3.14159"],
    ["'%+.1f % .1f'", "1.0", "1.0"],
    ["\"%'.2f\"", "1234567.891"],
    ["\"%'f\"", "-1234567.0"],
    ["\"%'015.2f\"", "1234567.891"],
    ["'%f'", "1e300"],
    ["'%e %E'", "12345.678", "12345.678"],
    ["'%.0e %#.0e'", "12345.678", "12345.678"],
    ["'%e'", "0.0"],
    ["'%g %g %g %g'", "100000.0", "1000000.0", "0.0001", "0.00001"],
    ["'%g %G'", "1e-10", "1e-10"],
    ["'%.3g %#.3g'", "1.0", "1.0"],
    ["'%.0g'", "123.0"],
    ["\"%'g\"", "1234567.0"],
    ["\"%'.10g\"", "1234567.0"],
    ["'%f %f %f'", "CAST('nan' AS FLOAT64)", "CAST('inf' AS FLOAT64)", "CAST('-inf' AS FLOAT64)"],
    ["'%F %E %G'", "CAST('nan' AS FLOAT64)", "CAST('inf' AS FLOAT64)", "CAST('-inf' AS FLOAT64)"],
    ["'%08f|%-8f|'", "CAST('inf' AS FLOAT64)", "CAST('nan' AS FLOAT64)"],
    ["'%+f'", "CAST('inf' AS FLOAT64)"],
    ["'%f %.1f'", "-0.0", "-0.04"],
    ["'%e %g'", "-0.0", "-0.0"],
    ["'%.1074f'", "5e-324"],
    ["'%f'", "CAST(NULL AS FLOAT64)"],
    # NUMERIC and BIGNUMERIC, exactly.
    ["'%f'", "NUMERIC '1.23456789'"],
    ["'%.20f'", "NUMERIC '1.1'"],
    ["'%.1f %.1f %.1f'", "NUMERIC '1.25'", "NUMERIC '1.35'", "NUMERIC '-1.25'"],
    ["'%.0f %.0f'", "NUMERIC '2.5'", "NUMERIC '3.5'"],
    ["\"%'f\"", "NUMERIC '1234567.5'"],
    ["\"%'015.2f\"", "NUMERIC '-1234567.5'"],
    ["'%e %E'", "NUMERIC '12345.678'", "NUMERIC '12345.678'"],
    ["'%.2e'", "NUMERIC '0.000123'"],
    ["'%g %g %g'", "NUMERIC '100000'", "NUMERIC '1000000'", "NUMERIC '0.00001'"],
    ["'%#g %#.0f'", "NUMERIC '1'", "NUMERIC '3'"],
    ["'%10.2f|%-10.2f|%010.2f|%+.1f|% .1f'", "NUMERIC '3.14159'", "NUMERIC '3.14159'", "NUMERIC '-3.14159'", "NUMERIC '1'", "NUMERIC '1'"],
    ["'%f'", "NUMERIC '99999999999999999999999999999.999999999'"],
    ["'%f %e'", "NUMERIC '0'", "NUMERIC '0'"],
    ["'%f'", "BIGNUMERIC '1.12345678901234567890123456789012345678'"],
    ["'%.40f'", "BIGNUMERIC '1.12345678901234567890123456789012345678'"],
    ["'%.37f'", "BIGNUMERIC '0.00000000000000000000000000000000000005'"],
    ["'%e'", "BIGNUMERIC '-123456789012345678901234567890'"],
    ["\"%'g\"", "BIGNUMERIC '1234567'"],
    ["'%f'", "CAST(NULL AS NUMERIC)"],
    # Strings.
    ["'%s'", "'abc'"],
    ["'%10s|%-10s|'", "'ab'", "'ab'"],
    ["'%.2s|%10.3s|'", "'abc'", "'abcdef'"],
    ["'%5s|%.1s|'", "'été'", "'été'"],
    ["'%05s|'", "'ab'"],
    ["'%s'", "''"],
    ["'%s'", "CAST(NULL AS STRING)"],
    ["'%s %d'", "CAST(NULL AS STRING)", "1"],
    # `*` widths and precisions.
    ["'%*d'", "5", "42"],
    ["'%-*d|'", "5", "42"],
    ["'%.*f'", "2", "3.14159"],
    ["'%*.*f|'", "10", "2", "3.14159"],
    ["'%*s|'", "-5", "'ab'"],
    ["'%.*s'", "-1", "'abc'"],
    ["'%*d'", "CAST(NULL AS INT64)", "42"],
    ["'%.*f'", "CAST(NULL AS INT64)", "1.5"],
    # NULLs: a typed conversion's NULL makes the result NULL, and %t and %T print NULL.
    ["'%d %t'", "1", "NULL"],
    ["'%t %d'", "NULL", "CAST(NULL AS INT64)"],
    ["'%T'", "CAST(NULL AS INT64)"],
    ["'%T'", "CAST(NULL AS ARRAY<INT64>)"],
    # %t of each type.
    ["'%t|%t|%t'", "1", "-1", "TRUE"],
    ["'%t|%t|%t|%t|%t'", "1.0", "1.5", "0.1", "1e300", "1e-300"],
    ["'%t|%t|%t'", "CAST('nan' AS FLOAT64)", "CAST('inf' AS FLOAT64)", "CAST('-inf' AS FLOAT64)"],
    ["'%t|%t'", "-0.0", "123456789012345680000.0"],
    ["'%t|%t|%t'", "NUMERIC '1.50'", "NUMERIC '-0.001'", "BIGNUMERIC '2'"],
    ["'%t|%t'", "'abc'", "'say \"hi\"'"],
    ["'%t'", "b'ab'"],
    ["'%t'", "b'\\x00\\xff\\n'"],
    ["'%t'", "b'a\\\\b\\'c\"d\\x7f\\x80 ~'"],
    ["'%t'", "[b'a', b'\\x01']"],
    ["'%t|%t'", "DATE '2024-01-02'", "TIME '03:04:05'"],
    ["'%t|%t'", "TIME '03:04:05.5'", "TIME '03:04:05.123456'"],
    ["'%t|%t'", "DATETIME '2024-01-02 03:04:05'", "DATETIME '2024-01-02 03:04:05.25'"],
    ["'%t'", "TIMESTAMP '2024-01-02 03:04:05+00'"],
    ["'%t'", "TIMESTAMP '2024-07-02 03:04:05.5+00'"],
    ["'%t|%T'", "TIMESTAMP '2024-01-02 03:04:05-08'", "TIMESTAMP '2024-01-02 03:04:05.123456+05:30'"],
    ["'%t|%t'", "INTERVAL 1 DAY", "INTERVAL -3 MONTH"],
    ["'%t'", "INTERVAL '1-2 3 4:5:6.789' YEAR TO SECOND"],
    ["'%t|%t'", "[1, 2]", "['a', 'b']"],
    ["'%t|%t'", "[CAST(NULL AS INT64)]", "CAST([] AS ARRAY<INT64>)"],
    ["'%t'", "STRUCT(1 AS a, 'x' AS b, [1.5] AS c)"],
    ["'%t'", "[STRUCT(1 AS a)]"],
    ["'%10t|%-10t|%.2t|'", "1", "1", "'abc'"],
    # %T of each type.
    ["'%T|%T|%T'", "1", "-1", "FALSE"],
    ["'%T|%T|%T|%T|%T'", "1.0", "1.5", "0.1", "1e300", "1e-300"],
    ["'%T|%T|%T'", "CAST('nan' AS FLOAT64)", "CAST('inf' AS FLOAT64)", "CAST('-inf' AS FLOAT64)"],
    ["'%T'", "-0.0"],
    ["'%T|%T|%T'", "NUMERIC '1.50'", "NUMERIC '-0.001'", "BIGNUMERIC '2'"],
    ["'%T'", "'abc'"],
    ["'%T'", "'say \"hi\"'"],
    ["'%T'", "\"it's\""],
    ["'%T'", "\"it's \\\"q\\\"\""],
    ["'%T'", "'a\\nb\\tc\\\\d'"],
    ["'%T'", "'é\\x01\\x7f'"],
    ["'%T'", "'\\u00a0\\u2028'"],
    ["'%T'", "'\\r\\a\\b\\f\\v\\x1b?'"],
    ["'%T'", "'a\\'b\"c'"],
    ["'%T'", "'it\\'s'"],
    ["'%T'", "'\\x00'"],
    ["'%s|%t'", "'a\\x00b'", "'a\\x00b'"],
    ["'%T'", "'💡'"],
    ["'%T'", "b'ab'"],
    ["'%T'", "b'\\x00\\xff\\n\"\\''"],
    ["'%T'", "b'a\\\\b\\x7f\\x80 ~\\r\\t'"],
    ["'%T'", "b\"it's\""],
    ["'%T'", "b'say \"hi\"'"],
    ["'%T|%T'", "DATE '2024-01-02'", "TIME '03:04:05.5'"],
    ["'%T'", "DATETIME '2024-01-02 03:04:05.25'"],
    ["'%T'", "TIMESTAMP '2024-01-02 03:04:05+00'"],
    ["'%T'", "INTERVAL 1 DAY"],
    ["'%T'", "INTERVAL '1-2 3 4:5:6.789' YEAR TO SECOND"],
    ["'%T|%T'", "TIME '03:04:05'", "DATETIME '2024-01-02 03:04:05'"],
    ["'%T|%T'", "[1, 2]", "['a']"],
    ["'%T'", "[DATE '2024-01-01']"],
    ["'%T'", "CAST([] AS ARRAY<INT64>)"],
    ["'%T'", "[CAST(NULL AS STRING)]"],
    ["'%T'", "STRUCT(1 AS a, 'x' AS b)"],
    ["'%T'", "STRUCT(1 AS a)"],
    ["'%T'", "[STRUCT(1 AS a)]"],
    ["'%T'", "STRUCT(STRUCT(1 AS x) AS a, [STRUCT(2 AS y, 3 AS z)] AS b)"],
    ["'%10T|%-10T|%.3T|'", "'ab'", "1", "'abc'"],
    # Errors.
    ["'%d'", "2.5"],
    ["'%d'", "TRUE"],
    ["'%d'", "'1'"],
    ["'%f'", "1"],
    ["'%e'", "1"],
    ["'%s'", "1"],
    ["'%s'", "DATE '2024-01-02'"],
    ["'%s'", "b'ab'"],
    ["'%u'", "1"],
    ["'%d'", "CAST(NULL AS FLOAT64)"],
    ["'%t %d'", "1", "1.5"],
    ["'%p'", "1"],
    ["'%a'", "1.0"],
    ["'%d %s'", "1"],
    ["'%d'", "1", "2"],
    ["'abc'", "1"],
    ["'%'"],
    ["'abc%'"],
    ["'%5%'"],
    ["'%*d'", "1.5", "42"],
    ["'%.*d'", "'a'", "42"],
    ["'%.1075f'", "1.0"],
    ["'%2000000d'", "1"],
    ["'%.*f'", "-1", "1.5"],
    ["'%.*f'", "2000", "1.5"],
]

# The location that GoogleSQL appends to the message of an error.
LOCATION = re.compile(r" \[type\.googleapis\.com/[^\]]*\]$")


def answer(execute_query: str, case: list[str]) -> dict:
    query = f"SELECT FORMAT({', '.join(case)}) AS r"
    run = subprocess.run(
        [execute_query, "--output_mode=json", query],
        capture_output=True,
        text=True,
    )
    if not run.stdout.strip():
        # In JSON mode, an error prints nothing, so the text mode gives its message.
        run = subprocess.run([execute_query, query], capture_output=True, text=True)
        output = run.stdout + run.stderr
        line = next(line for line in output.splitlines() if line.startswith("ERROR:"))
        message = LOCATION.sub("", line.removeprefix("ERROR: "))
        message = message.split(": ", 1)[1] if message.startswith(("INVALID_ARGUMENT", "OUT_OF_RANGE")) else message
        # `execute_query` names FLOAT64 DOUBLE, its internal name.
        return {"error": message.replace("DOUBLE", "FLOAT64")}
    rows = json.loads(run.stdout)["row"]
    return {"result": rows[0].get("r")}


def main():
    execute_query = sys.argv[1]
    lines = []
    for case in CASES:
        record = {"args": case}
        record.update(answer(execute_query, case))
        lines.append(json.dumps(record, ensure_ascii=False))
    path = pathlib.Path(__file__).with_name("format.jsonl")
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"wrote {len(lines)} cases to {path}")


if __name__ == "__main__":
    main()
