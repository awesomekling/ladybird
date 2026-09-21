#!/usr/bin/env python3
"""Keeps process-wide state out of the Rust crates that are compiled into more than one library.

A Rust crate that another Rust crate depends on is compiled into both of their libraries, and so
is every `static` in it. A dynamic linker with a flat namespace binds every call to one of the
copies and the duplication never shows; one with two-level namespaces, as macOS has, gives each
library its own copy of the state. A list one library pushes to is then not the list the other
drains, and nothing about the Rust says so. That is how the wanted-pending-face handover broke on
macOS and nowhere else, twice.

So state that has to be shared lives on the C++ side, in a library there is only one of, and the
crate reaches it through `extern "C"` accessors. Everything that stays in such a crate has to be
listed below with the reason it is allowed to be per copy.
"""

import pathlib
import re
import sys

# `file:name` -> why a per-copy instance is harmless. A cache of a pure function, per-thread
# scratch, or something whose whole purpose is to be per copy.
ALLOWED = {
    "Libraries/LibGfx/Rust/src/lib.rs:MARKER": "per copy on purpose: registering its address is how LibGfx counts the copies",
    "Libraries/LibGfx/Rust/src/lib.rs:WANTED": "test stub for the C++ store, in the cargo test binary that has no C++ side",
    "Libraries/LibGfx/Rust/src/lib.rs:NEXT": "test stub for the C++ store, in the cargo test binary that has no C++ side",
    "Libraries/LibGfx/Rust/src/image_frame.rs:STORAGE": "per-copy handle dedup; the id-to-frame map that has to be shared is LibGfx's",
    "Libraries/LibGfx/Rust/src/text_layout.rs:SHAPING_CACHE": "per-thread memo of a pure function; a second copy costs memory, never an answer",
    "Libraries/LibWeb/HTML/Parser/Rust/src/token.rs:SPARE_ATTRIBUTE_LISTS": "per-thread allocation pool; a second copy costs memory, never an answer",
    "Libraries/LibWeb/HTML/Parser/Rust/src/token.rs:SPARE_ATTRIBUTE_VALUES": "per-thread allocation pool; a second copy costs memory, never an answer",
    "Libraries/LibJS/Flap/src/low_ir/lowering.rs:LABELS": "per-thread scratch buffer; a second copy costs memory, never an answer",
}

STATE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?static\s+(?:mut\s+)?([A-Z_][A-Z0-9_]*)\s*:\s*(.*)$")
THREAD_LOCAL_STATE = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?static\s+([A-Z_][A-Z0-9_]*)\s*:\s*(.*)$")


def crates_compiled_into_more_than_one_library(root):
    """Every crate some other crate depends on: its code, and its statics, end up in both."""
    shared = set()
    for manifest in sorted(root.glob("**/Cargo.toml")):
        if "target" in manifest.parts:
            continue
        for match in re.finditer(r"^([a-z_0-9]+)\s*=\s*\{\s*path\s*=", manifest.read_text(), re.M):
            shared.add(match.group(1))
    return shared


def main():
    root = pathlib.Path(__file__).resolve().parent.parent.parent
    shared_crates = crates_compiled_into_more_than_one_library(root)
    failures = []
    for manifest in sorted(root.glob("**/Cargo.toml")):
        if "target" in manifest.parts:
            continue
        name = re.search(r"^name\s*=\s*\"([^\"]+)\"", manifest.read_text(), re.M)
        if not name or name.group(1) not in shared_crates:
            continue
        for source in sorted((manifest.parent / "src").glob("**/*.rs")):
            relative = source.relative_to(root).as_posix()
            in_thread_local = False
            for number, line in enumerate(source.read_text().splitlines(), start=1):
                if re.match(r"^\s*thread_local!\s*\{", line):
                    in_thread_local = True
                    continue
                pattern = THREAD_LOCAL_STATE if in_thread_local else STATE
                if in_thread_local and re.match(r"^\s*\}", line):
                    in_thread_local = False
                match = pattern.match(line)
                if not match:
                    continue
                # A shared reference to constant data is not state: nothing can write it, so the
                # copies cannot disagree.
                if match.group(2).lstrip().startswith("&"):
                    continue
                if f"{relative}:{match.group(1)}" in ALLOWED:
                    continue
                failures.append(f"{relative}:{number}: {match.group(1)}")
    if not failures:
        print(f"No process-wide state in the {len(shared_crates)} multiply-linked Rust crates.")
        return 0
    print("A Rust crate compiled into more than one library must keep no process-wide state:")
    for failure in failures:
        print(f"  {failure}")
    print("Move it behind C++-owned storage (see Libraries/LibGfx/RustProcessState.cpp), or add it")
    print("to ALLOWED in this script with the reason a per-copy instance is harmless.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
