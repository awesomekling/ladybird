# StyleEngine: testing and debugging

How to test style engine changes, how the verification machinery works, and how to debug. The engine itself is described in [StyleEngine.md](StyleEngine.md).

The correctness bar: **incremental results must be identical to full recomputation from authoritative inputs.** Over-invalidation (recomputing something that did not change) is a performance problem; under-invalidation (missing something that did change) is always a bug.

## 1. The test stack

Run these from fastest to slowest; a style change is not done until all of them pass.

**Rust unit and differential tests.**

```sh
cd Libraries/LibWeb/Rust && cargo test --release -p libweb_rust
```

`tests.rs` drives engine operators directly through the same entry points the bridge uses. `differential_tests.rs` compares incremental evaluation against the exact cold evaluator over generated mutation sequences. In unit tests, every maintained prefix relation is also compared with scalar prefix matching after construction and updates.

**Web tests.** The style-engine and style-invalidation suites under `Tests/LibWeb/Text/input/css/` assert observable behavior (computed styles after mutations) and, via internals hooks, engine behavior (recompute counts, invalidation reach). The full LibWeb suite is the outer gate:

```sh
./bin/test-web
```

A focused loop for style work:

```sh
./bin/test-web -f Text/input/css/style-engine/ -f Text/input/css/style-invalidation/
```

## 2. Record and replay

The engine's complete input stream (every transaction, boundary call, and atom allocation) can be captured from a real browsing session and replayed deterministically.

* Set `LIBWEB_STYLE_RECORD=<path>` to capture a session into a `.sg` stream; `Meta/record-style.py` wraps this. Recording support is included in ordinary builds and does no serialization work unless the environment variable is set.
* Replay with the `style-replay` binary (it also supports `--list`, `--suite`, and `--subtest` for scoping):

```sh
style-replay --assert-digests capture.sg
```

`--assert-digests` verifies per-event payload checksums and recomputes the output digest over every published style transaction (match-answer identities, old and new style-record identities, damage, and reactions, per node). Cascade winners, exact cascade publications, and style-record payloads are checked by direct comparison events during the same replay, and those comparisons name the diverging node and event index. Any divergence from the recorded run, down to a single changed identity, fails. Replay wall time is also a low-noise perf harness: iterating on an engine change takes seconds per run instead of a browser session.

**Determinism is a maintained property.** Replay identity depends on fixed-seed hashing everywhere iteration order can reach published state (`fast_hash.rs`; never introduce a randomly seeded map there), on identity allocation and reuse order (recycled IDs cross the FFI boundary), and on the absence of wall-clock or random inputs in engine paths. Recording frames and replay decoders are generated together with the boundary calls themselves, so a new call is recordable automatically.

**When digests legitimately change.** A behavior fix that changes published outputs is rejected by existing captures. First prove the new behavior correct (the differential tests must show equality with full recomputation), then replace the affected captures with freshly recorded live sessions; each new stream must pass its own digest replay before it replaces the old one. Never hand-edit a capture.

## 3. Debugging playbook

**A page styles wrong.**

1. Reduce to a test under `Tests/LibWeb/Text/input/css/style-engine/` if you can, and to a differential test in `differential_tests.rs` if the incremental answer differs from the cold one.
2. If the engine answers right but the page is still wrong, the bug is upstream of the engine (input collection, C++ integration) or downstream (consumers): check that the mutation reached the engine as a typed delta, and that consumers read the published record.

**A real site styles wrong and resists reduction.** Capture a recording of the interaction and iterate in replay with `--assert-digests`. The direct-comparison events (cascade publications, style-record responses) report the diverging node and event index; a digest-only mismatch narrows by scoping the replay with `--suite`/`--subtest`.

**Performance.** A browser profile shows C++ materialization cost; profiling `style-replay` on a captured interaction isolates engine cost. The counters (§16 of StyleEngine.md) are exposed through the internals object (`--expose-internals-object` works on live sites via WebDriver) and answer the first triage question: how many reactions and recomputations did this interaction produce, and how many computed styles changed? A large gap is amplification, and the fix is precision, not micro-optimization. Performance claims are settled by measuring the real workload A/B; counter improvements and synthetic benchmarks are supporting evidence, not the verdict.

**Suspected memory-accounting drift.** Every cache and arena reports exact bytes (invariant 8). Capacity tests assert charges reach zero after eviction, and the replay accounting report prints per-category bytes for a captured run, so a drifting category is visible without a debugger.

## 4. Writing tests

* Behavior fixes land with a test that failed before the fix. For engine-internal properties (invalidation reach, recompute counts, retention), assert through the internals hooks rather than timing; timing-based tests are forbidden.
* Structural refactors that claim "no behavior change" are proven by the standard stack plus unchanged replay digests, not by new tests that would only assert the refactor's own shape.
* When a test needs the engine to be in a specific retained state (warm caches, retained answers), drive it there through ordinary DOM/CSSOM operations in the test body; tests never reach into engine internals to preload state.
