#!/usr/bin/env python3

"""
IR Test Runner for LibJS

This script runs JavaScript files through the IR optimizer and compares
the IR output at each optimization pass against expected baselines.

Usage:
    LADYBIRD_SOURCE_DIR=. ./Build/release/bin/test-js-ir

To create/update expected output for a test:
    LADYBIRD_SOURCE_DIR=. ./Build/release/bin/test-js-ir --rebaseline test-name.js

Directory structure:
    Tests/LibJS/IR/
    ├── input/           # .js source files to compile
    ├── expected/        # .txt files with expected IR output
    └── output/          # .txt files with actual IR output (generated)
"""

import difflib
import os
import re
import subprocess
import sys

from argparse import ArgumentParser
from concurrent.futures import ThreadPoolExecutor
from concurrent.futures import as_completed
from pathlib import Path

LADYBIRD_SOURCE_DIR: Path
IR_TEST_DIR: Path
BUILD_DIR: Path
ANSI_COLOR_PATTERN = re.compile(r"\x1b\[\d+([;:]1)?m")


def setup() -> None:
    global LADYBIRD_SOURCE_DIR, IR_TEST_DIR, BUILD_DIR

    ladybird_source_dir = os.getenv("LADYBIRD_SOURCE_DIR")

    if ladybird_source_dir is None:
        print("LADYBIRD_SOURCE_DIR must be set!")
        sys.exit(1)

    LADYBIRD_SOURCE_DIR = Path(ladybird_source_dir)
    IR_TEST_DIR = LADYBIRD_SOURCE_DIR / "Tests/LibJS/IR/"

    # The script is copied to bin/test-js-ir, so the build dir is one level up
    BUILD_DIR = Path(__file__).parent.parent.resolve()


def strip_color(s: str) -> str:
    return ANSI_COLOR_PATTERN.sub("", s)


DIFF_PREFIX_ESCAPES = {
    "@": "\x1b[36m",
    "+": "\x1b[32m",
    "-": "\x1b[31m",
}


def diff(a: str, a_file: Path, b: str, b_file: Path) -> None:
    for line in difflib.unified_diff(a.splitlines(), b.splitlines(), fromfile=str(a_file), tofile=str(b_file)):
        line = line.rstrip()

        color_prefix = DIFF_PREFIX_ESCAPES.get((line or " ")[0], "")

        print(f"{color_prefix}{line}\x1b[0m")


def test(file: Path, rebaseline: bool = False) -> bool:
    """
    Run a single IR test.
    Returns True if the test failed, False if it passed.
    """
    args = [
        str(BUILD_DIR / "bin/js"),
        str(IR_TEST_DIR / "input" / file),
        "--disable-ansi-colors",
        "--dump-ir",  # Dump the IR
        "--dump-ir-passes",  # Dump IR after each optimization pass
        "--optimize-ir",  # Run the optimizer
        "--tier-up-threshold=1",  # Tier up functions on first call
    ]
    process = subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)

    stdout = process.stdout.decode().strip()

    # Non-zero exit is OK for parse-only mode sometimes, but we should still capture output
    stdout = strip_color(stdout)

    output_file = IR_TEST_DIR / "output" / file.with_suffix(".txt")
    expected_file = IR_TEST_DIR / "expected" / file.with_suffix(".txt")

    # Write actual output
    output_file.parent.mkdir(parents=True, exist_ok=True)
    output_file.write_text(stdout + "\n", encoding="utf8")

    if rebaseline:
        # Copy output to expected
        expected_file.parent.mkdir(parents=True, exist_ok=True)
        expected_file.write_text(stdout + "\n", encoding="utf8")
        print(f"Rebaselined: {file}")
        return False

    # Compare with expected
    if not expected_file.exists():
        print(f"\nNo expected file for {file}")
        print(f"Run with --rebaseline {file} to create it")
        return True

    expected = expected_file.read_text(encoding="utf8").strip()

    if stdout != expected:
        print(f"\nIR does not match for {file}!\n")

        diff(a=expected, a_file=expected_file, b=stdout, b_file=output_file)

        return True

    return False


def main() -> int:
    setup()

    parser = ArgumentParser(description="Run LibJS IR optimization tests")
    parser.add_argument("-j", "--jobs", type=int, help="Number of parallel jobs")
    parser.add_argument(
        "--rebaseline", nargs="*", metavar="FILE", help="Rebaseline expected output. If no files given, rebaseline all."
    )
    parser.add_argument("files", nargs="*", help="Specific test files to run")

    args = parser.parse_args()

    input_dir = IR_TEST_DIR / "input"

    if not input_dir.exists():
        print(f"No input directory found at {input_dir}")
        print("Create Tests/LibJS/IR/input/ and add .js test files")
        return 1

    # Get list of test files
    if args.files:
        js_files = [input_dir / f for f in args.files]
    else:
        js_files = [js_file for js_file in sorted(input_dir.iterdir()) if js_file.is_file() and js_file.suffix == ".js"]

    if not js_files:
        print("No test files found in", input_dir)
        return 0

    # Determine which files to rebaseline
    rebaseline_files = set()
    if args.rebaseline is not None:
        if len(args.rebaseline) == 0:
            # Rebaseline all
            rebaseline_files = {f.name for f in js_files}
        else:
            rebaseline_files = set(args.rebaseline)

    failed = 0

    with ThreadPoolExecutor(max_workers=args.jobs) as executor:
        futures = {
            executor.submit(test, js_file.relative_to(input_dir), js_file.name in rebaseline_files): js_file
            for js_file in js_files
        }

        for future in as_completed(futures):
            if future.result():
                failed += 1

    total = len(js_files)
    passed = total - failed

    if failed:
        print(f"\nTests: {passed} passed, {failed} failed, {total} total")
        return 1

    print(f"All IR tests passed! ({total} total)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
