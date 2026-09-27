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

It also keeps count of the state in libweb_rust, the crate the render stages live in. Those stages
are moving to threads of their own, and a stage thread either shares a `static` with the document
thread or silently gets a fresh `thread_local!` of its own. Either can be wrong, so every one there
has to be listed below with the reason it is fine.
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
    "Libraries/LibCompositing/Rust/src/display_list/replay.rs:WARM_REPLAY_SCRATCH_STORAGE": "per-thread replay scratch; a second copy costs memory, never an answer",
    # FIXME: Two copies hand out overlapping epochs; a tree one copy built and a plan the other copy
    #        prepared could then agree on an epoch they do not share. Move the counter to C++.
    "Libraries/LibCompositing/Rust/src/visual_context/mod.rs:NEXT_STRUCTURAL_EPOCH": "process-wide identity counter, per copy until it moves to C++ storage",
}

# libweb_rust: `file:name` -> why the state is safe for a render stage on another thread.
RENDER_STAGE_CRATE = "Libraries/LibWeb/Rust"

SCRATCH = "per-thread allocation pool or scratch; a thread without one only allocates more"
DIAGNOSTIC = "seal, census or counter diagnostics; off unless an environment variable turns them on"
ENVIRONMENT_SWITCH = "read-once environment switch; every thread sees the same answer"
IDENTITY = "process-wide atomic counter handing out unique identities"
BUILT_ONCE = "built once and read-only after; every thread shares the same table"
STAGE_THREAD = "the stage thread itself, and the caller waiting on it"
STYLE_UPDATE_SCOPE = "the document thread's open style update; a stage run carries it to the stage thread and back"
LOCKED = "process-wide and behind a mutex"
CLOCK_HANDOFF = "the clock ticks' handoff between the RenderClock, Rendering and main threads: shared by design, behind a mutex, condvar or atomic"
MAIN_THREAD_ONLY = "a thread_local the main thread keeps for itself; a stage thread never reads or writes it"
PRESENTED_COUNTER = "process-wide atomic count of presented frames that tests read; nothing branches on it"
REPLAY = "style replay capture; replay builds only, or off unless an environment variable turns it on"
TEST_ONLY = "test only"
MAIN_SIDE_COUNTER = "counter kept by the main side's doors, which a render stage never passes"
GROW_ONLY = "process-wide behind a lock or atomic, and only grows; growing asks readers to keep more, never less"
SUBMITTED_COUNT = "process-wide atomic count of live submitted stages; a thread counts its own before it asks, so zero means none of its own is in flight"


def render_stage_entries(reason, entries):
    return {f"{RENDER_STAGE_CRATE}/src/{entry}": reason for entry in entries}


RENDER_STAGE_ALLOWED = {
    **render_stage_entries(
        MAIN_SIDE_COUNTER,
        ["layout/layout_node_arena.rs:DOOR_COUNTERS", "layout/layout_node_arena.rs:COUNTS_DOOR_PASSES"],
    ),
    **render_stage_entries(
        MAIN_THREAD_ONLY,
        [
            "flight.rs:TAKEN_BACK_OUTCOME",
            "flight.rs:FLIGHT_ENDS",
            "flight.rs:FLIGHT_STYLE_ENDS",
            "css/style/bridge.rs:STYLE_PASS_FOR_FLIGHT",
        ],
    ),
    **render_stage_entries(PRESENTED_COUNTER, ["clock_frames.rs:CLOCK_TICKS_PRESENTED"]),
    **render_stage_entries(STAGE_THREAD, ["flight.rs:FLIGHT_STYLE_DECISION"]),
    **render_stage_entries(SUBMITTED_COUNT, ["stage_thread.rs:SUBMITTED_STAGES"]),
    **render_stage_entries(
        CLOCK_HANDOFF,
        [
            "clock_frames.rs:GATE",
            "clock_frames.rs:TICKS_TO_ADOPT",
            "clock_frames.rs:INJECTED_TICKS_PENDING",
        ],
    ),
    **render_stage_entries(
        BUILT_ONCE,
        [
            "clock_frames.rs:NEEDS_MAIN",
            "clock_frames.rs:PRESENT",
            "clock_frames.rs:LEND_TAKEN_BACK",
            "clock_frames.rs:WAKE_MAIN",
        ],
    ),
    **render_stage_entries(
        GROW_ONLY,
        [
            "css/parser/arbitrary_substitution.rs:ATTR_NAMES_READ",
            "css/parser/arbitrary_substitution.rs:ATTR_NAMES_READ_GENERATION",
        ],
    ),
    **render_stage_entries(
        SCRATCH,
        [
            "css/cascaded_properties.rs:STORE_POOL",
            "css/style/column.rs:STAMPED_INDEX_POOL",
        ],
    ),
    **render_stage_entries(
        DIAGNOSTIC,
        [
            "css/ffi_stats.rs:COUNTERS_ENABLED",
            "css/ffi_stats.rs:COUNTER_CONTEXT",
            "css/ffi_stats.rs:CPP_CALLBACK_COUNT",
            "css/ffi_stats.rs:REGISTRY",
            "css/ffi_stats.rs:THREAD_UNSAFE_CPP_CALLBACK_COUNT",
            "css/style/engine_sample_check.rs:LOG",
            "css/style/engine_sample_check.rs:MODE",
            "css/style/seal.rs:MODE",
            "font_seal.rs:INSTALLED",
            "layout/main_side_census.rs:CENSUS",
            "layout/main_side_census.rs:COUNTED_PASSAGE_DEPTH",
            "layout/main_side_census.rs:ENABLED",
            "layout/main_side_census.rs:LAST_FLUSH",
            "layout/seal.rs:MODE",
            "layout/seal.rs:PASS_DEPTH",
            "layout/seal.rs:REPORTED",
            "layout/tree_build_seal.rs:BUILD_DEPTH",
            "layout/tree_build_seal.rs:COUNTS",
            "layout/tree_build_seal.rs:MODE",
            "layout/tree_build_seal.rs:REPORTED",
            "painting/published_immutable.rs:CURRENT_WRITER",
            "painting/published_immutable.rs:ENABLED",
            "painting/published_immutable.rs:STATE",
            "painting/record/verify.rs:ENABLED",
            "painting/seal.rs:CURRENT_PASS",
            "painting/seal.rs:ENABLED",
            "painting/seal.rs:MODE",
            "painting/seal.rs:REPORTED",
            "painting/seal.rs:REPORTED_MAIN_SIDE_READS",
        ],
    ),
    **render_stage_entries(
        ENVIRONMENT_SWITCH,
        [
            "stage_thread.rs:LOGS",
            "clock_frames.rs:ENABLED",
            "css/style/flush.rs:ENABLED",
            "css/style/mod.rs:CASCADE_WINNERS",
            "css/style/mod.rs:PREFIX_RELATION",
            "css/style/mod.rs:PUBLISHED_STYLE_TRANSACTION",
            "css/style/mod.rs:SELECTOR_TRUTH_DERIVATION",
            "css/style/mod.rs:STYLE_ANSWER_PATCH",
            "css/style/mod.rs:STYLE_PLAN_PROVENANCE",
            "layout/fc_run_cache.rs:MODE",
            "layout/update_layout.rs:ENABLED",
        ],
    ),
    **render_stage_entries(
        IDENTITY,
        [
            "css/declaration_block.rs:NEXT_DECLARATION_BLOCK_IDENTITY",
            "css/rule.rs:NEXT_RULE_IDENTITY",
            "css/selector.rs:NEXT_SELECTOR_ID",
            "css/style/index.rs:NEXT",
            "css/style/prefix.rs:NEXT",
            "css/style_sheet.rs:NEXT_SHEET_IDENTITY",
            "layout/fragment_tree.rs:NEXT_IDENTITY",
        ],
    ),
    **render_stage_entries(
        BUILT_ONCE,
        [
            "css/computed_values.rs:FIELD_DESCRIPTORS",
            "css/computed_values.rs:PROPERTY_DEPENDENCY_MASKS",
            "css/computed_values.rs:REGISTRY",
            "css/counter_representation.rs:DECIMAL",
            "css/css_string.rs:EMPTY",
            "css/parser/stylesheet_cache.rs:HASHER",
            "css/style/publication.rs:REMAINING",
            "css/style_compute.rs:INITIAL_VALUE_TABLE",
            "css/style_compute.rs:KINDS",
            "css/style_compute.rs:LONGHANDS",
            "css/style_compute.rs:PHASE_BOUNDARIES",
            "css/style_compute.rs:PX",
        ],
    ),
    **render_stage_entries(
        STYLE_UPDATE_SCOPE,
        [
            "css/style/seal.rs:STATE",
        ],
    ),
    **render_stage_entries(
        STAGE_THREAD,
        [
            "stage_thread.rs:STAGE_THREAD",
            "stage_thread.rs:PAINT_LANE",
            "stage_thread.rs:THREAD",
            "stage_thread.rs:WAITING_CALLER",
            "stage_thread.rs:INCOMING",
            "stage_thread.rs:MODE",
            "stage_thread.rs:STAGES",
            "stage_thread.rs:FRAME_SCHEDULER_HOST",
            "stage_thread.rs:THREAD_SETUP",
            "stage_thread.rs:STAGE_HOLD",
            "stage_thread.rs:SUBMITTED",
            "stage_thread.rs:FLIGHT_PREEMPTED",
            "stage_thread.rs:RUNNING_FLIGHT_STAGE",
            "flight.rs:TAKEN_BACK_OUTCOME",
            "flight.rs:TAKEN_BACK_PAINT",
            "flight.rs:FLIGHT_ENDS",
            "painting/ffi.rs:SEALED_FLIGHT_PAINT",
            "stage_thread.rs:RUNNING_JOIN_WORK",
            "stage_thread.rs:STYLE_ENGINE_ENTRANCES_ONLY_WAIT",
            "stage_thread.rs:FORCED_JOIN_SITES",
            "stage_thread.rs:STYLE_PASS_FORCED_JOINS",
            "stage_thread.rs:FORCED_JOINS",
            "stage_thread.rs:SITES_REACHED_FROM_COLLECTION",
            "layout/frame_retirement.rs:HOLDS",
            "layout/frame_retirement.rs:COUNTERS",
            "stage_thread.rs:NEXT_SUBMITTED_RUN",
            "stage_thread.rs:RUNNING_SUBMITTED_RUN",
            "stage_thread.rs:PRESENTATIONS_SUBMITTED",
            "stage_thread.rs:TAKE_BACKS_THAT_WAITED_FOR_PRESENTATION",
        ],
    ),
    **render_stage_entries(
        LOCKED,
        [
            "clock_frames.rs:LEASES",
            "css/ffi_stats.rs:COMPLETE_STYLE_UPDATE_STATE",
            "css/parser/stylesheet_cache.rs:CACHE",
            "css/style/atoms.rs:GLOBAL_ATOMS",
            "css/style/matching.rs:DISPATCH_POOLS",
            "css/style/native_rules/targets.rs:TARGET_PAGES",
            "css/style/program.rs:RULE_DECLARATIONS",
            "css/style/program/rule_records.rs:RULE_RECORD_PAGES",
            "css/style/program/rule_versions.rs:RULE_VERSION_PAGES",
            "css/style/selector.rs:ROUTING_POOLS",
            "css/style/selector.rs:SELECTOR_PROGRAM_POOLS",
        ],
    ),
    **render_stage_entries(
        REPLAY,
        [
            "css/computed_values.rs:REPLAY_STYLE_GROUPS",
            "css/computed_values.rs:REPLAY_STYLE_GROUP_SIZES",
            "css/style/record_replay.rs:CAPTURE",
            "css/style_value.rs:REPLAY_STYLE_VALUES",
        ],
    ),
    **render_stage_entries(
        TEST_ONLY,
        [
            "css/computed_values.rs:GROUPS",
            "css/declaration_block.rs:DECLARATION_OWNER_ALLOCATIONS",
            "css/descriptor_block.rs:DESCRIPTOR_OWNER_ALLOCATIONS",
            "css/rule.rs:RULE_OWNER_ALLOCATIONS",
            "css/style/atoms.rs:GLOBAL_ATOM_TEST_LOCK",
            "css/style/font_resolution.rs:RESOLVES",
            "css/style/mod.rs:SELECTOR_TRUTH_DERIVATION_OVERRIDE",
            "css/style_compute.rs:FLY_STRINGS",
            "css/style_compute.rs:FONT_CASCADE_LIST_UNREFS",
            "layout/layout_node_arena.rs:TOLD_BOX_PRESENCE",
            "lib.rs:NEXT",
            "lib.rs:WANTED",
        ],
    ),
    f"{RENDER_STAGE_CRATE}/src/css/parser/stylesheet_cache.rs:PARSE_DEPENDENCIES": "per-thread record of the parse running on this thread; scoped to that parse",
    f"{RENDER_STAGE_CRATE}/src/css/style_value.rs:VALUES": "built-once keyword values, and a replay-only table of the same name",
    f"{RENDER_STAGE_CRATE}/src/painting/record/vector_images.rs:RAWS": "per-thread memo of two interned keywords; a second copy leaks one more reference each",
}

# A `static` whose type has none of these cannot be written, so it is constant data, not state.
INTERIOR_MUTABILITY = re.compile(r"Cell|Atomic|Mutex|RwLock|Once|Lazy|Condvar")

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


def state_in(root, source_directory):
    """Yields `(relative path, line number, name, line, type and initializer, in a thread_local!)`."""
    for source in sorted(source_directory.glob("**/*.rs")):
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
            yield relative, number, match.group(1), line, match.group(2), in_thread_local


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
        for relative, number, name, _, _, _ in state_in(root, manifest.parent / "src"):
            if f"{relative}:{name}" in ALLOWED:
                continue
            failures.append(f"{relative}:{number}: {name}")

    render_stage_failures = []
    render_stage_state = set()
    for relative, number, name, line, rest, in_thread_local in state_in(root, root / RENDER_STAGE_CRATE / "src"):
        if not in_thread_local and not re.search(r"\bstatic\s+mut\b", line) and not INTERIOR_MUTABILITY.search(rest):
            continue
        render_stage_state.add(f"{relative}:{name}")
        if f"{relative}:{name}" in RENDER_STAGE_ALLOWED:
            continue
        render_stage_failures.append(f"{relative}:{number}: {name}")
    # The list only shrinks: an entry whose state has moved away must go too, or it would let the
    # state come back unnoticed.
    for entry in sorted(RENDER_STAGE_ALLOWED.keys() - render_stage_state):
        render_stage_failures.append(f"{entry}: listed in RENDER_STAGE_ALLOWED but no longer present")

    if not failures and not render_stage_failures:
        print(
            f"No process-wide state in the {len(shared_crates)} multiply-linked Rust crates, and all "
            f"{len(render_stage_state)} statics and thread_locals in libweb_rust are accounted for."
        )
        return 0
    if failures:
        print("A Rust crate compiled into more than one library must keep no process-wide state:")
        for failure in failures:
            print(f"  {failure}")
        print("Move it behind C++-owned storage (see Libraries/LibGfx/RustProcessState.cpp), or add it")
        print("to ALLOWED in this script with the reason a per-copy instance is harmless.")
    if render_stage_failures:
        print("A static or thread_local in libweb_rust is state a render stage thread shares or copies:")
        for failure in render_stage_failures:
            print(f"  {failure}")
        print("Move it into the state the stage is handed, or add it to RENDER_STAGE_ALLOWED in this")
        print("script with the reason it is safe on a stage thread.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
