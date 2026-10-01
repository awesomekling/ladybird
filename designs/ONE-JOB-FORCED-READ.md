# One forced read, one owner job, one wait

A script API that forces style or layout (`offsetWidth`, `getBoundingClientRect()`, `getComputedStyle().width`, ...)
should cost the main thread exactly one wait on the render owner: main sends the typed changes before the job, the
owner runs the whole job (style, what style leaves the layout nodes, layout, paint preparation, the answer), and main
applies the job's typed main-only effects after it. Today it is two to four waits, with main's C++ style drain between
the owner's style and the owner's layout.

## 0. Measurements (trunk b6d47c1c006)

Method: an uncommitted trace (`OWNTRACE`, kept outside the tree) in `stage_thread::send_and_wait` logs every message the
main thread waits on: its kind, the wait, and the time the owner spent in the unit (`OwnerReplyTo::answer`). The
handoff latency is wait minus unit time: both thread wakeups plus any queueing behind other owner work. StyleBench
runner marks delimit the windows. One run, 1 iteration, 180 sync windows.

Waits in sync windows, per iteration: **425** (Style 173, Layout 173, Ask Engine 75, Paint 4). lindus's older trace
counted 487 (Paint 56); trunk has since moved most paint preparation into the layout commit.

| sync window shape | windows | waits | what it is |
|---|---|---|---|
| `u s S L` | 50 | 100 | a forced read: `update_layout` → `update_style` → **Style** → drain → **Layout** |
| `u s u s S L S L` | 55 | 220 | a forced read in the benchmark iframe: `StyleUpdate::begin` first lays out the embedding document (its own S+L) |
| `E` | 60 | 60 | `getComputedStyle` with nothing to restyle: a lone `Boundary(HasDeferredElementStyleInput)` ask |
| `E u s S L` (+`P`) | 15 | 49 | `getComputedStyle` that restyles and lays out (4 of them prepare rendering: **Paint**) |

(`u` = `Document::update_layout`, `s` = `Document::update_style`, `E` = an engine ask.) No forced read needed a second
layout round.

| | Style | Layout | Ask Engine | Paint |
|---|---|---|---|---|
| handoff p50 / p90 (all windows) | 21.6 / 164.8 µs | 10.6 / 37.5 µs | 17.3 / 57.0 µs | 7.2 / 16.6 µs |
| handoff total, sync windows | 11.2 ms | 9.9 ms | 2.2 ms | 0.1 ms |

- Sync wall is 1687 ms per iteration. The owner is busy for 1378 ms of it (Style 653, Layout 686, engine asks 40). Handoffs
  cost **23.3 ms/it** (1.4%). The main thread's gap between a Style answer and the Layout send is **43.2 ms/it**
  (250 µs per read).
- In the same run, perf (cpu-clock, dwarf, `--no-inline`) on the main thread splits `StyleUpdate::finish` (205 samples)
  into the per-row DOM install (`apply_engine_computed_style_record` 88) and the rest of `apply_style_engine_reactions`.
  The drain's render half (`StyleEffectDrain::apply_render_half`) is 35 samples.
- For the 173 forced-read transactions, `OwnerRenderHalf` (the owner applying the batch to the layout nodes itself) ended
  like this:

  | outcome | count |
  |---|---|
  | applied | 45 |
  | no rows to apply | 34 |
  | declined: `LayoutNode` (a box kind the flight does not style) | 54 |
  | declined: `Row` (pseudo-element rows) | 15 |
  | not requested (a tree build was pending) | 25 |

  So 79 of the 173 (46%) leave the owner nothing that main's drain must do before layout.
- Outside the forced reads, rendering updates (async windows) wait 2960 times per iteration: Paint 2055, Layout 694 and
  AskArena 173, with 37 ms of handoff. That is a separate target: the rendering update's host steps between owner units.

## 1. What main does between the Style answer and the Layout send

In order, after `run_style_transaction` returns (`css/style/bridge.rs` `style_engine_take_style_transaction`, then
`CSS/UpdateStyle.cpp` `StyleUpdate::finish`, then `Layout/UpdateLayout.cpp` `update_style_and_layout_once`). Classes:

- **O**: an arena or engine write the owner can do inside the job.
- **A**: a main-only (DOM/JS-visible) effect, which becomes a typed after-effect.
- **L**: something the layout job reads first.

| step | writes | class |
|---|---|---|
| `applied.hand_to_host` (`OwnerAppliedStyle`) | host tables: held owner damages, handback payment | A (host tables) |
| C++ `take_style_transaction` tail | `set_needs_accumulated_visual_contexts_update`, `repaint_document_after_owner_style_change` | A (document flags) |
| `accept_style_engine_transaction` | counters; `set_published_batch_waits` | A |
| `sample_animation_effects_needing_style_update` | animation host objects | A |
| per row: `Element::apply_engine_computed_style_record` | `m_style_record` (`replace_style_record`), `m_style_uses_*`, custom property data, root font metrics, display-none subtree, pseudo recompute, container-query dependents, SVG paint resources | A: layout never reads `Element::m_style_record`. It reads the arena row, and a tree build reads the engine's column (`pin_style_record_for_build`) |
| per row: engine acks (`acknowledge_environment_move`, `absorb_element_style_input`, `record_applied_style_reaction`, ...) | engine | O: the engine's own bookkeeping, sent back to the engine |
| per row: animation plans, transitions, samples (`apply_settled_animation_plan`, `run_transition_step_for_installed_record`, `sample_animations_for_installed_record`) | animation host objects, plus engine asks (`TakeSettledAnimationPlan`, `SampleInstalledRecords`, `DecideTransition*`) | A. The asks become part of the job's answer, since the owner can precompute them |
| drain `LayoutNodeStyle` → `apply_layout_node_style` | row style (`InstallRowStyle` / `ReplaceRowStyleRecord`), repaint marks, selection pseudo sync | O: `OwnerRenderHalf` already does it for eligible batches |
| (inside it) `did_update_box_style_record`, `attach_style_resources_to_box` | scroll-snap registration, image loads and observers | A: images load asynchronously, and snapping runs after layout |
| drain `LayoutInvalidation` | layout and visual-context marks, partial relayout escape | O |
| drain `RestoreRowDebts`, `AcknowledgeRecord`, `DiscardContainerQueryEffects` | engine | O |
| drain `LayoutTreeRebuild` → `Element::set_needs_layout_tree_rebuild` | picks the rebuild root from DOM state (top layer, `display: contents` parent, box presence changed in place), then host-table tree-update marks | **L**: the tree build reads the marks |
| drain `ExplicitInheritance` | DOM parent style-group marks | A, but the next wave reads it |
| drain `AnchorNames` → `register_anchor_names` | engine registration | O: the engine holds the record and the tree scope |
| drain `ContainerQueryEffects` | engine plus element container-query bookkeeping | O+A; the next wave reads it |
| drain `publish_anchor_names` | `ArenaChange::PublishAnchorNames` | O |
| main half: `AnimationNames`, `AnimationPlan`, `DisplayNoneAnimations` | animation host objects | A |
| wave loop: `has_pending_transaction` → another `take_style_engine_transaction` | another **Style** wait | L: see below |
| `~StyleUpdate` → `layout_arena_finish_owner_style_host_half` | `ArenaChange::FinishOwnerStyleHostHalf` (puts back rows the install did not adopt) | O |
| `read_layout_round` | list item renumbers, top-layer changes, document facts, selection, document style for build | L (all reads, except renumbers and top layer, which mark the tree) |

**What layout needs before it runs, and why.**

1. **Tree-update marks and the rebuild root** (`LayoutTreeRebuild`). The owner's tree build reads the marks lent to the
   frame. The root is chosen from DOM state. This is the one real ordering dependency. It moves to the owner once the
   arena decides the root from what it already holds: the style tree's flat-tree parents, the top-layer zone, and box
   presence. Until then, a batch with a rebuild row is a two-job read.
2. **A second style wave.** The install feedback (explicit inheritance marks, derived child reactions, container query
   effects) is main-side input to the next transaction. Each wave is another job. The engine already keeps its own
   explicit-inheritance mark ("the engine took its own mark as it published the row"). Taking the host's half into the
   engine makes the wave loop an owner loop.
3. **Round facts** (viewport, quirks, selection, document style for a viewport build). Style does not change them. Main
   can read them before the style job. `style_input_waits_on_document` after the install decides whether *another* job
   follows, not what the first layout reads.

Nothing else in the drain is read by layout. The render half is owner work that `OwnerRenderHalf` already does for
eligible batches. The rest is main-only and can run after the job.

## 2. The Paint wait and the lone engine asks

**Paint.** A layout job whose round lays nothing out sets `messages.prepare_for_rendering`. Main then runs
`Document::prepare_for_rendering`, which does `publish_visual_context_tree_inputs` and then **Paint(PrepareForRendering)**
(and, rarely, Paint(FinishRenderingPreparation) after clamped scroll offsets are stored). This is a separate message
only because the inputs live on main:
- `m_needs_accumulated_visual_contexts_update`
- the visual-viewport and DPR inputs

The commit path already does the same inside the job (`prepare_for_rendering_after_commit`). To ride the job:
- Send the visual-context inputs as a change before the job.
- Carry `visual_context_update_pending` in `FrameInput`.
- Return the outcome flags and clamped offsets as typed effects in `FfiLayoutFrameEffects`.

On StyleBench this is only 4 waits per iteration now.

**Lone engine asks.** `CSS::update_style_for_element` (getComputedStyle) asks `Boundary(HasDeferredElementStyleInput)`
for the element and its ancestors whenever the engine home's deferred-input mirror is not exact. The owner applies the
pending changes before it answers, which is why these asks show 530 µs of owner time each. In 60 of 75 cases the answer
is "no" and nothing else follows. In the other 15, a Style and a Layout wait follow. It is a separate message because
the C++ code decides between a targeted install, a full style update and nothing on main, from the answer. In the
one-job design, getComputedStyle sends one job: "the style of this element, current" (plus layout if the property needs
it). The owner applies the changes, answers the deferred check itself, runs the transaction if needed, and returns the
record with the after-effects. That makes it one wait in every case.

The drain's own engine asks (`TakeSettledAnimationPlan`, `SampleInstalledRecords`, `DecideTransitions`,
`PublishComputedGroups`, ...) are waits in the middle of main's install. None occur in StyleBench's sync windows, but
animated content hits them. The owner can answer them ahead, into the job's view, for the rows the batch installs.

## 3. Types that make a multi-wait forced read not compile

Today:
- `run_style_transaction`, `LayoutFrame::run_job_on_owner` (via `wait_for_owner`), `run_paint_pass_of` and `ask_engine`
  each wait, and **none takes a token**.
- `ScriptForcedRead` reaches only `ask`/`ask_owner`/`ask_arena`, and it is minted deep in Rust entries
  (`layout_arena_publish_query_snapshot`), not at the script entry.

So nothing stops one script call from waiting four times.

The target:

```rust
/// The one wait of a script API call that needs a current answer. Takes the call's token by value.
pub(crate) fn force_read(read: ScriptForcedRead, document: DocumentId, job: ForcedReadJob) -> ForcedReadAnswer;

/// Everything the owner runs for the read, all sent before it: the style transaction the document took (if any), the
/// layout frame's first round (read before style: nothing style computes changes it), whether paint preparation
/// follows, and the question.
pub(crate) struct ForcedReadJob { style: Option<OwnerStyleTransaction>, layout: Option<FirstRound>, query: Option<Query> }

/// What the job leaves: the style view (answers the host installs), the frame's end, the answer, and the typed
/// main-only effects (`AfterEffects`: element record installs, animation plans, image loads, snap registration, box
/// presence), which main applies after the wait and which nothing in the job read.
pub(crate) struct ForcedReadAnswer { style: Option<OwnerStyleTransactionView>, layout: Option<FrameJobAnswer>, answer: Option<Answer>, after: AfterEffects }
```

- `ScriptForcedRead` is `!Send`, not `Clone`, and moved into `force_read`, so a script call waits once. It is minted by
  the host entry a script API calls (the `UpdateLayoutReason`s of script APIs, `update_style_for_element`), and by
  nothing else.
- **Deleted as waits:**
  - `run_style_transaction` becomes owner-internal: the transaction travels only inside a `ForcedReadJob`, or in a
    rendering update's unit (`FinishSubmitted` runs on the owner already).
  - `LayoutFrame::run_job_on_owner` sends a job only under a `FrameJobPermit`, which `force_read`'s answer (for the
    next round) or a `LockstepProof` provides.
  - `run_paint_pass_of`'s `wait_for_owner` gets `PrepareForRendering` folded into the frame job, and keeps only held
    passes under `LockstepProof::recording_on_main`.
  - `ask_engine` takes `impl OwnerWait`.
  - `wait_for_owner` / `wait_for_owner_thread` become private to `render_owner`, and take the wait token.
- **Call sites that change:**
  - `bridge.rs` `style_engine_take_style_transaction` (it no longer waits; it packs the transaction into the job the
    frame sends).
  - `update_layout.rs` `run_with_owner`, `run_job_on_owner` and `start_first_round`.
  - `render_owner.rs` `ask_engine` and `run_style_transaction`.
  - `owner_pass.rs` `run_paint_pass_of`.
  - C++ `Document::update_style_and_layout_once` (read the first round before `update_style`) and
    `CSS::update_style_for_element`.
- **What stays a second job, by type:** a `FrameJobEnd::NextRound` (another style wave or a rebuild the owner cannot
  root) is a new `FrameJobPermit`. It is visible and counted, not an accident of the call graph.

**Spike (step 2): ride the first layout job on the style job.** The smallest honest cut:
1. `update_style_and_layout_once` reads the first round *before* `update_style` when no tree build, renumber or
   top-layer change is pending (exactly when `OwnerRenderHalf` is requested), and offers the frame's first job to the
   style transaction.
2. `ToOwner::Style` carries it. The owner runs the transaction. When it applied the render half, or there was nothing to
   apply, it runs the layout job in the same message and answers both.
3. Main runs its drain after the job and before taking the layout end in. The carried end stands only if nothing main
   sent after it alters published rows or marks layout. That is typed per change (`alters_published_rows` /
   `lays_out_again`). The drain's re-install of a row the owner applied becomes a typed adoption that alters nothing.
   Otherwise the frame runs the job again: the old two-wait path, correct by construction.

## 4. Expected gain

| | waits/it (sync windows) | est. ms/it |
|---|---|---|
| trunk | 425 | — |
| spike (ride where the render half applied or was empty, 79 reads) | ~346 | ~4.5 ms (79 × 57 µs mean Layout handoff); drain unchanged |
| target (one wait per window; iframe chain = 2 jobs until the child's viewport comes from the parent's job) | 180–235 | 12 ms of handoff + ~7 ms render half off main ≈ 19 ms (1.0%) |
| target + main's install pipelined beside the owner's layout (needs the drain to read a handed-over snapshot, not the engine) | same | up to ~50 ms (2.6%) |

The waits are cheap (tens of µs), and the owner's style and layout (1378 ms/it in sync windows) are the time. One job
per read is the structure that later lets main's install run beside the owner's layout instead of between it.

## 5. After the typed entry (`force_read`)

Done: every read of render state the main thread waits for is one `ForcedRead`, begun by the scope that brackets it:
the outermost `Document::JoinScope` of a document (`update_layout`, `update_layout_if_needed_for_node`, the CSSOM and
editing reads that open one) and `Document::update_style_for_element` call `render_owner_begin_forced_read`, which
mints a `ScriptForcedRead` for a reason a script API names and the host's read otherwise, and the scope's end drops it
if nothing spent it. A scope opened inside an open read of the same document belongs to it. The read's first owner
wait spends it on `render_owner::force_read`:

- a question to the style engine asked in the read (`owner_calls::ask_in_read`: the geometry read's
  `DeferPendingTransactionForGeometryRead`, a style read's `HasDeferredElementStyleInput`), which leaves the read's next
  job a `ForcedRead::AfterAsk`, whose `AskedFirst` only that question makes;
- or the first style transaction (a style read's, or a layout update's with its frame's first job riding);
- or the layout frame's first job.

A layout update that finds its read spent already lays out on `LockstepProof::read_lays_out_again()` (an image that
arrived, a scroll-state snapshot, a second update of the same call). Every other style or layout job spends a named
`FrameJobPermit` or `StyleJobPermit`, and `ToOwner::Style`/`ToOwner::Layout` carry a `SpentWait` only `render_owner`
makes. Wait counts are unchanged: `offsetWidth` and `getBoundingClientRect()` after a change still cost 2 waits per
read, now typed as `force_read(EngineQuestion)` then the update's job on `ForcedRead::AfterAsk`.

What still waits outside `force_read`, in the order to fold it in:

1. **The second wait the types now show.** `ForcedRead::AfterAsk`: the geometry read asks whether the pending
   transaction is paint-only before any update starts. To merge it, send "defer it if paint-only, else run the style
   transaction and the riding layout job" as one `ForcedReadJob`; this needs the host's style steps
   (`update_style` before the transaction is taken) to run before the question, which is §1's ordering dependency.
2. **Paint preparation** after a frame that laid nothing out (`Document::prepare_for_rendering` →
   `run_paint_pass_of` with `LockstepProof::host_paint_step`): fold into the frame job as §2 says.
3. **Second jobs:** `FrameJobPermit::after_style` (the ride was declined: a tree build, renumbers or top-layer changes
   pending, or no render half), `for_next_round` (another wave, a root only the document picks), and
   `read_lays_out_again`. These are §1's ordering dependencies.
4. **Engine questions outside a read**, which still spend `LockstepProof::engine_door`: the drain's
   (`TakeSettledAnimationPlan`, `SampleInstalledRecords`, ...) in the middle of main's install, and the questions a read
   asks once its read is spent.

Once 2 is folded in, `ToOwner::Ask` and `ToOwner::Paint` can carry `SpentWait` too, and `wait_for_owner` can become
private to `render_owner`.
