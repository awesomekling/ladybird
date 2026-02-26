#!/bin/bash
# Run HTML tokenizer tests by comparing dump-html-tokens output against expectations.
# Usage: run-tests.sh <path-to-dump-html-tokens>

set -euo pipefail

DUMP_TOOL="${1:?Usage: $0 <path-to-dump-html-tokens>}"
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
INPUT_DIR="$SCRIPT_DIR/input"
EXPECTED_DIR="$SCRIPT_DIR/expected"

pass=0
fail=0
errors=""

for input_file in "$INPUT_DIR"/*.html; do
    name="$(basename "$input_file" .html)"
    expected_file="$EXPECTED_DIR/$name.txt"

    if [ ! -f "$expected_file" ]; then
        echo "SKIP $name (no expected file)"
        continue
    fi

    actual=$("$DUMP_TOOL" "$input_file" 2>&1) || true

    if diff -u "$expected_file" <(printf '%s\n' "$actual") > /dev/null 2>&1; then
        echo "PASS $name"
        pass=$((pass + 1))
    else
        echo "FAIL $name"
        diff -u "$expected_file" <(printf '%s\n' "$actual") || true
        fail=$((fail + 1))
        errors="$errors $name"
    fi
done

echo ""
echo "$pass passed, $fail failed"
if [ "$fail" -gt 0 ]; then
    echo "Failed:$errors"
    exit 1
fi
