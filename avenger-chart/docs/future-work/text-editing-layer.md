# The Text-Editing Layer

## Status

Implemented design, studied 2026-07-10 and completed through W5.6 on
2026-07-14. This is the deliverable of the widget plan's **W5.1 prior-art
study** (`scratch/2026-07-09/02-widgets/plan.md`) and the governing contract
for W5.2–W5.6. Grounded in source-level studies of parley `PlainEditor`
(the structural template), cosmic-text `Editor` (the semantics
checklist), iced `TextInput`, egui `TextEdit`/`Undoer`, Slint
`TextInput`/`LineEdit` (the native-primitive precedent), winit 0.30
IME/keyboard, arboard, and the eframe web text/clipboard agents — plus
a deep read of the in-repo stack (avenger-text / avenger-typst-label).
Companion: `widgets.md` (the TextInput contract this design
implements). Corrections to earlier assumptions are flagged inline
with **[correction]**.

## Architecture: Two Layers Behind a Geometry Firewall

Slint's division is the right law: **editor state owns byte offsets;
the text stack owns geometry** — the two meet through exactly two kinds
of query (position→offset, offset→rect). Concretely:

1. **`avenger-text/src/text_edit/`** (widget-agnostic, no params, no
   scene marks):
   - `shaped_line.rs` — the public exposure of the existing shaping
     output (`ShapedLine`, below).
   - `editor.rs` — `SingleLineEditor`: buffer + selection + compose
     state and the action vocabulary, borrowing the engine for
     relayout — **eager after edits, lazy after config changes** (the
     parley discipline; the `PlainEditor`/driver split collapses to
     one type since our `TextEngine` is `Arc`-shared and needs no
     split borrows).
2. **`avenger-chart-widgets/src/text_input/`** (the widget): params
   and commit policy, the `Undoer`, focus/clipboard/IME plumbing,
   event mapping, scroll offset, and `scene()` building marks from a
   `VisualRepresentation`.

## Native Runtime And Lifecycle Contract

TextInput uses the widget system's three-way split; this document does not
define a parallel runtime:

- `NativeWidget` is the serializable authoring description (`id`, `kind`,
  schema version, canonicalizable JSON payload, declarative-or-registry
  measure spec, ordered param-only state spec).
- The injected `NativeWidgetFactory` for `text-input` validates the version and
  payload, evaluates registry measurement, and creates the editor instance.
- `NativeWidgetInstance` owns the `SingleLineEditor`, undo state, scroll,
  focus/composition state, dirty flags, scene parts, and lifecycle callbacks.

Instances live in the host's `NativeWidgetInstanceStore`, keyed by stable
`{document_id, plot/member_id, widget_id}`. `PlotSessionOptions` receives one
immutable registry/store pair plus an explicit namespace; app runtime resources
bundle them for reuse. Every session attach gets a monotonic attachment epoch.
Focus, IME, clipboard, and wakeup commands carry that epoch, so cleanup from a
replaced session cannot affect its successor. Rebuilding with the same
namespace preserves editor/undo state; only committed params are serialized or
guaranteed across a different namespace.

Lifecycle meanings are exact:

- `on_deactivate` handles a retained-but-hidden owner: apply the focus-loss
  commit policy, disable IME, settle/cancel wakes, keep editor and undo state;
- `on_session_detach` removes only the current session lease and session-scoped
  services, performs no late document write, and preserves logical pending
  deadlines so a replacement can re-arm them;
- `on_unmount` is final cleanup, runs exactly once on store eviction, performs
  no document-state write, and detaches first when necessary.

Event callbacks receive frame-local logical coordinates and a testable
monotonic `now()` from the dispatcher. They return a host-neutral outcome with
resolved param assignments, evaluation intent, dirty flags, focus/cursor,
dynamic consume, and `RuntimeHostCommand`s. Exact keyed wakeups drive debounce;
IME enable/cursor-area and clipboard writes use the same channel. IME rectangles
start widget-local, are composed through widget/concat/panel/page offsets into
root-canvas logical pixels, then native hosts apply device scale and Wasm hosts
apply the full canvas-to-CSS-client affine transform. Merely adding the canvas
DOM origin is incorrect under responsive CSS sizing or DPR.

Before measurement/render, `on_environment_sync` receives frame width/height,
the shared resolved-widget-style digest and CSS-variable params, presentation
state, text config, and relevant data/registry revisions. X/y-only moves remain
compositor-only. `scene()` returns a stable ordered map of named parts; every
rebuild validates unique ids and refreshes only that native sub-index. External
chart event bindings cannot target runtime-discovered native parts; the native
dispatcher owns them.

## The avenger-text Exposure API

The substrate study's key fact: the non-PDF text→positioned-glyphs
path **already exists end to end** — `TextItem { byte_range, style,
metrics, glyphs }` with `Glyph { text_range, x, y, x_advance,
transform }` is populated on every frame build (label/mod.rs:513) and
merely never leaves the crates. The exposure is visibility plus two
normalizations:

```rust
// avenger-text — implemented over the crate-private text_line::typeset_line;
// CompiledLabel/LabelFrame are NOT exposed raw. Plain syntax mode only in v1
// (math runs have style: None and path-only content — un-caretable).
impl TextEngine {
    pub fn shape_line(&self, config: &TextMeasurementConfig)
        -> Result<ShapedLine, AvengerTextError>;
}

pub struct ShapedLine {
    pub bounds: TextBounds,        // real shaped metrics — NOT FontMetrics::fallback
    pub baseline: f32,             // top → baseline
    pub runs: Vec<ShapedRun>,      // VISUAL order (bidi-reordered), x monotone
}
pub struct ShapedRun {
    pub byte_range: Range<usize>,  // LOGICAL, source-relative
    pub is_rtl: bool,              // NEW — see normalization 2
    pub glyphs: Vec<ShapedGlyph>,
}
pub struct ShapedGlyph {
    pub text_range: Range<usize>,  // source-relative, cluster-merged (ligatures/ZWJ)
    pub left: f32,                 // transform.tx
    pub x_advance: f32,
}
```

**Mandatory normalizations at the seam** (both are latent hazards
found in the substrate study):

1. **`Glyph.text_range` frames of reference disagree today**: the
   single-face whole-line path yields source-relative ranges, but the
   segmented multi-font/bidi path
   (`plain_pdf_text_from_segmented`, inline/mod.rs:1357) yields
   run-relative ranges that are never offset. The seam offsets by
   `TextItem.byte_range.start` (valid because plain runs satisfy
   `run.text == source[byte_range]`), verified by a mixed-script test.
2. **Run direction is computed but discarded**:
   `unicode_bidi::BidiInfo::visual_runs` reorders (font.rs:578) but no
   direction field survives. Add `is_rtl` where `bidi_visual_ranges`
   already knows it (`PositionedTextLineRun`, frame.rs:86) — cheaper
   and truer than recomputing `BidiInfo` in text_edit.

**Geometry queries** (in text_edit, over `ShapedLine` — the firewall's
two functions plus selection):

- `byte_offset_for_x(&ShapedLine, x: f32) -> (usize, Affinity)` — the
  hit test: find the run/glyph by x, then subdivide the cluster's
  advance **proportionally across its grapheme count** (cosmic-text's
  rule; ligatures and emoji land mid-cluster correctly), with the
  **midpoint rule** (left half → before). Affinity assigned by which
  side of a run boundary was hit.
- `cursor_rect_for_offset(&ShapedLine, offset: usize, affinity)
  -> Rect` — caret: 1px wide, from `baseline − ascent` to
  `baseline + descent`; affinity picks the run at bidi boundaries.
- `selection_rects(&ShapedLine, range: Range<usize>) -> Vec<Rect>` —
  per-run contiguous x-spans from glyphs whose `text_range` overlaps
  the selection, grapheme-subdivided at the ends (cosmic-text's
  `run.highlight`); a bidi-crossing selection yields multiple rects
  by construction.
- Grapheme/word arithmetic: `prev/next_grapheme(text, offset)` and
  word boundaries via `unicode-segmentation`
  (`grapheme_indices(true)`, `unicode_word_indices` — cosmic-text's
  exact functions; the crate is **already a workspace dependency**,
  declared in avenger-text). Boundary code lives here, never in the
  widget.

**Empty text**: shapes to width 0 with real face metrics
(font.rs:481) — the placeholder caret height comes from the same
query path, no special case.

## The Editor Core (`SingleLineEditor`)

**Cursor model** — byte offsets everywhere (parley, cosmic-text, and
Slint agree; iced's grapheme-index `Vec<String>` is the allocation-
heavy outlier we avoid):

```rust
pub struct Cursor { pub index: usize, pub affinity: Affinity } // byte index
pub enum Affinity { #[default] Downstream, Upstream }          // bidi boundaries
                                                               // (no soft wrap in v1)
pub struct SelectionState {
    pub anchor: Cursor,      // press point — pinned while shift held
    pub head: Cursor,        // moving end
    pub granularity: Granularity, // Char | Word — sticky through drag
}
```

Anchor/head are **unnormalized** (order encodes drag direction);
normalization is read-time via a clamping accessor (Slint's
`safe_byte_offset` + sort — makes stale-index bugs structurally
impossible when the buffer changes underneath).

**State**: `buffer: String` (preedit **spliced in** — see IME),
`compose: Option<Range<usize>>`, `selection`, `show_cursor: bool`,
`generation: u32` (parley's damage counter — the widget redraws when
it changes), plus a cached `ShapedLine` with a dirty flag; edits
relayout eagerly (subsequent cursor math needs fresh geometry),
config changes lazily.

**Action vocabulary** (cosmic-text's enum shape — testable and
replayable — reduced to single-line; vertical motions and
`GotoLine` deliberately absent):

```rust
pub enum Action {
    InsertText(String),                  // from KeyEvent.text or paste (sanitized)
    Backspace, Delete,
    DeleteWordBack, DeleteWordForward,
    DeleteToStart, DeleteToEnd,          // Cmd+Backspace / ctrl+K family
    Motion { motion: Motion, extend: bool },
    Click { x: f32 }, DoubleClick { x: f32 }, TripleClick,
    Drag { x: f32 },
    SelectAll, Escape,
    Preedit { text: String, cursor: Option<(usize, usize)> },
    Commit(String),
}
pub enum Motion { Left, Right, WordLeft, WordRight, Start, End }
```

Delete collapses to one path (Slint): if no selection, apply the
matching motion with `extend`, then delete the selection. Insert over
a selection deletes it first. `InsertText` filters control characters
and newlines (single-line; Enter is handled above the editor as
commit). All deletions/motions are grapheme-correct via the text_edit
boundary functions — except plain Backspace, which may use
`PreviousCharacter` semantics inside a grapheme only if the study of
real usage demands it (default: grapheme-wise; Slint's
`PreviousCharacter` is backspace-only and optional).

## Interaction Spec

**Clicks** (iced's cycle, verified against platform feel):
Single→Double→Triple→**Double** (a fourth fast click is a double);
consecutive iff same button ∧ distance < 6px ∧ gap ≤ 300ms. Single:
collapse to hit (shift+click: keep anchor, move head — anchor
preserved across repeated shift-clicks). Double: select word at hit,
`granularity = Word`; drag then extends **word-snapped and
direction-aware** (below/above anchor-word). Triple: select all
(≡ line for single-line) and **disables drag-extend**. Drag with
`granularity = Char` extends head to hit.

**Keybinding table** — Slint's two-stage abstraction, matched at the
widget layer against `SceneKeyPressEvent { key, modifiers }`:

1. `text_shortcut(event) -> Option<TextShortcut>` where
   `TextShortcut = Move(Motion, extend) | Delete(variant)` — with a
   **runtime `is_apple` table**: word motion = Alt(mac)/Ctrl(else) +
   arrows; line start/end = Cmd+←/→ (mac), Home/End (all);
   delete-word = Alt/Ctrl+Backspace/Delete; delete-to-start =
   Cmd+Backspace (mac).
2. `standard_shortcut(event)` → SelectAll (Cmd/Ctrl+A), Copy/Cut/Paste
   (Cmd/Ctrl+C/X/V — **matched only natively**; on wasm the DOM
   events are the source, decision below), Undo (Cmd/Ctrl+Z), Redo
   (Cmd+Shift+Z, Ctrl+Y on Windows).

Then: Enter → commit per `TextCommit` policy (+ blur for
`OnEnterOrBlur`); Escape → collapse selection — **composition cancel
is not the editor's Escape**: during a composition the platform IME
typically consumes Escape itself and emits an empty `Preedit`, which
the splice model already handles; the app-initiated cancel paths are
exactly two — the `set_ime_allowed` off/on toggle (click-away, focus
loss) and the platform's own. Otherwise **insert `KeyEvent.text`**
when pressed, not a shortcut, and not composing.

**Scroll-to-caret**: a **persisted** offset (avoid iced's stateless
snap-left): on caret change, if `caret_x − offset > width − pad` set
`offset = caret_x − width + pad`; if `caret_x − offset < pad` set
`offset = max(0, caret_x − pad)`; clamp to
`[0, max(0, text_width − width)]`; `pad ≈ 4px`. Hit-testing adds the
offset back.

## Undo — Copy egui's `Undoer` Verbatim

**[correction]** Earlier notes assumed "time/edit-distance batched"
undo; egui's `Undoer` has **no edit-distance threshold — batching is
purely time-based**, and that is sufficient: `Settings { max_undos:
100, stable_time: 1.0s, auto_save_interval: 30.0s }`. A new undo
point is created when the state has changed **and been stable for
`stable_time`** ("not until you stop typing"), or every
`auto_save_interval` during continuous change. ~150 self-contained
LOC; reimplement in the widget crate. `State = (SelectionRange,
String)` full snapshots (`Clone + PartialEq` — trivially correct at
single-line sizes); feed before and after each event dispatch (egui
feeds twice a frame). Undo/redo restore both text and selection.
**Clear redos on every new undo point** (Slint fails to and positions
go stale — a recorded avoid). The Undoer lives in the TextInput
instance (ephemeral, never serialized).

## IME

**Model: preedit is spliced into the buffer** (parley and egui
converge on this; Slint keeps it separate and pays a display-remap
tax): `Preedit { text, cursor }` replaces the current compose range
(or the selection, on composition start), records the new byte range
in `compose`, and sets show_cursor from `cursor.is_some()`; the
winit-payload cursor is **byte offsets into the preedit string**
(collapsed caret = `(n, n)`) — converted at the boundary once.
`Commit(text)`: replace compose range with text, clear compose.
winit sends a synthetic `Preedit("", None)` right before `Commit`, so
replace-then-insert needs no special case. **Guards copied from egui
(they encode real bug fixes)**: ignore empty Preedit/Commit unless
composing (Wayland/Safari emit spurious ones that would destroy
selections); ignore `"\n"`/`"\r"` compositions.

**The committed-text param excludes preedit** — parley's `SplitString`
trick: the logical text is the buffer split around `compose`, zero
copies. So a debounced `OnChange` param never observes half-composed
text, and `on_state_sync` external writes are rejected while
composing (the widgets.md contract).

**Window plumbing** (native): `set_ime_allowed(true/false)` on focus
enter/leave — mandatory on macOS even for dead keys ("IME must be
enabled … where dead-key sequences are combined");
`set_ime_cursor_area(caret-or-preedit rect)` after `Enabled`, updated
**only when the rect changes** (egui caches `ime_rect_px` — don't spam
the platform); the off/on `set_ime_allowed` toggle force-cancels a
composition on click-away; on Windows, filter `KeyboardInput` for
physical keys the IME consumed (winit 0.30 still delivers them).
Rendering: preedit range gets an underline rule mark; the candidate
window is the OS's, positioned via the cursor area.

## Platform Contract (Eventstream Work)

**[correction] The typed-text source is `KeyEvent.text`, not
`Key::Character`.** winit documents `text` as the produced text —
possibly multi-code-point (Windows dead-key fallout ships two chars;
Enter yields `"\r"`); `logical_key` is for shortcut matching only.
Today avenger-eventstream drops `KeyEvent.text` entirely and truncates
`Key::Character(SmolStr)` to one `char` (window/winit.rs:84). The fix:
`WindowKeyboardInput`/`SceneKeyPressEvent` gain
`text: Option<SmolStr>`; the char-truncating conversion is removed;
insertion consumes `text`.

**Editor-facing primitives are exactly three** — `InsertText`,
`Preedit`, `Commit` — chosen so both hosts lower identically:

- **Native**: winit `KeyboardInput.text` → InsertText; winit `Ime::*`
  → Preedit/Commit; arboard (`get_text`/`set_text`, `&mut self`;
  enable `wayland-data-control`; **no wasm support — cfg-gated out**)
  behind Cut/Copy/Paste chord matching.
- **wasm**: **[correction — winit is not the vehicle]** the web
  backend supports neither `Ime` events nor `set_ime_*` (a canvas
  cannot receive composition events — winit #4424), and `Key::Dead` is
  always `None`. The wasm harness therefore grows an eframe-style
  **TextAgent**: a hidden 1×1 `<input>` focused while a TextInput has
  focus (canvas keeps focus otherwise), `compositionupdate` →
  Preedit, `compositionend` → Commit, `input` → InsertText, moved to
  the caret so the IME popup lands correctly. Clipboard on wasm:
  document-level `cut`/`copy`/`paste` listeners — paste reads
  `clipboardData.getData("text")` **synchronously in the handler**
  (works everywhere, no permission); copy/cut must determine the
  payload synchronously in the callback (Safari) and write via
  `navigator.clipboard.writeText` (secure context);
  `navigator.clipboard.readText` is not a viable paste path. The
  semantic `Cut`/`Copy`/`Paste(text)` eventstream events from
  widgets.md are the neutral carrier both hosts feed.

Paste is **sanitized** (strip `\n`/`\r`/`\t` and control chars) — iced
0.13 did; its master regression is a recorded avoid.

## Params, Controlled Input, Cadence

- Committed-text param per `TextCommit::{OnChange (debounced),
  OnEnterOrBlur}`; the value never includes preedit.
- **Opt-in cursor/selected-text params** (widgets.md amendment):
  minted only when `cursor_position()`/`selected_text()` accessors are
  consumed. **Units at the mint boundary**: the editor is byte-offset
  internally, but the cursor-position param surfaces a **grapheme
  index into the committed text** (per widgets.md) — converted once
  per write by counting grapheme boundaries up to the byte offset
  (trivial for a single line); selected-text is the committed-text
  substring. **Cadence**: coalesced to one write per event dispatch,
  riding the same `throttle_ms`/debounce policy as `OnChange` text —
  prior art exposes cursor state by direct query, not reactively, so
  reactive consumers explicitly accept evaluation ticks by opting in.
- `on_state_sync` (controlled input): an external committed-text write
  replaces the buffer **unless composing** (external write loses
  during composition); on accept, cursor and anchor are **clamped to
  grapheme boundaries** and **both undo stacks are cleared** — Slint
  learned both the hard way (`align_to_text`, issues #331/#9024).

## Rendering (`scene()` Contract)

A Slint-style `VisualRepresentation { display_text /*preedit
spliced*/, selection_range, preedit_range, cursor_offset:
Option<usize> }` drives the marks, in z-order: background/border
rects (theme tokens; focus ring when focused) → **selection rects
behind the text** (from `selection_rects`) → the text as one
`SceneTextMark` (plain syntax; placeholder text as a separate dimmed
mark when `buffer.is_empty() && !composing` — Slint hosts placeholder
outside the primitive; ours renders it in the widget) → preedit
underline rule → caret rule (solid; **blink stays out of v1**, and
when it arrives it copies egui's schedule-exact-wakeup pattern —
repaint requested precisely at the next phase flip, phase reset on
interaction — never a polling loop). All clipped to the frame with
the scroll offset applied.

**Sizing** (the widgets.md contract, restated so the mapping is
complete): `WidgetSizeHints = FillAvailable width (Fixed override) ×
Content(line height + padding)` — line height from the shaped metrics
(the empty-string path yields real face metrics, so the placeholder
case sizes identically). **Pointer feedback**: mark enter/leave write
the existing cursor-kind param to I-beam — the same machinery every
composed widget uses; nothing editing-specific.

**Perf note** (substrate finding): measurement/raster caches key on
the full text string with bounded wholesale-clear behavior —
per-keystroke reshaping of a short line is fine, but each keystroke
strands one raster-cache entry; acceptable in v1, and the editing
layer calls `shape_line` directly (its own single-line cache) rather
than round-tripping the keyed measurement memo.

## Deliberate v1 Limits (carried from widgets.md, restated here)

- **Password masking: deferred** — when it comes, Slint's pattern is
  the design: a visual-representation remap (all glyphs → the mask
  character, with byte ranges mapped between actual and display text
  in both directions); double-click then selects all, never a word
  (don't leak word boundaries).
- **Drag-and-drop text: deferred** (no drag-out/drop-in of text
  content; drag inside the widget is selection only).
- **LTR-biased keybinding table first**: motion and selection are
  bidi-correct from the stack (visual-order runs, affinity), but the
  binding table ships with LTR-biased Left/Right = Previous/Next; a
  visual-motion table for RTL-primary locales is a follow-on.
- **Read-only and input-type filtering** (Slint's property checklist):
  not in v1's surface; recorded as natural TextInput properties when
  demanded.
- Single line only; Enter commits; caret is solid (blink = later
  runtime presentation policy) — as stated throughout.

## Study-Question Checklist (widgets.md → answers)

| Question | Answer |
| --- | --- |
| avenger-text API to expose; does it fall out of the Typst structures? | `shape_line` → `ShapedLine` + the three geometry queries; glyph runs with byte ranges already exist — exposure + two normalizations (source-relative ranges; run direction) |
| Grapheme-vs-byte, who owns | Byte-offset cursors + Affinity; grapheme/word boundaries owned by text_edit via unicode-segmentation (already a dep) |
| Undo batching | egui `Undoer` verbatim: time-only (1s settle / 30s auto-save / 100 deep), full snapshots, redo cleared on new points |
| Keybinding table | Two-stage (`TextShortcut` then `StandardShortcut`), runtime `is_apple`, delete = move-with-extend + delete-selection |
| Double/triple-click | 300ms/6px cycle Single→Double→Triple→Double; word-snapped direction-aware drag; triple = all, no drag-extend |
| Scroll-to-caret | Persisted clamped offset with ~4px pad (not iced's stateless snap) |
| Selection across bidi runs | Per-run rects from visual-order `ShapedRun`s, grapheme-subdivided ends — multiple rects by construction |
| IME preedit needs | Splice + compose range + underline; candidate window via cached `set_ime_cursor_area`; winit byte-offset conversion; egui's empty-preedit guards |
| Opt-in param cadence | Coalesced per dispatch, debounce/throttle shared with `OnChange`; query-not-push is the prior-art norm |

## W5 Implementation Mapping

- **W5.2 (eventstream) — complete**: `KeyEvent.text` carried through (replaces
  the char-truncation fix as framed); `Ime` events + window plumbing
  (native); semantic `Cut`/`Copy`/`Paste(text)` events; **the wasm
  TextAgent + document clipboard listeners land with the wasm example
  harness** (winit web cannot deliver IME — new scope, now explicit).
- **W5.3 (services) — complete**: focus routing, arboard-backed clipboard service
  (cfg-gated), IME rect caching + composition-cancel toggle + Windows
  key filter.
- **W5.4 (avenger-text) — complete**: the two normalizations, `ShapedLine` +
  geometry queries + grapheme/word helpers, `SingleLineEditor` with
  the Action vocabulary; mixed-script/bidi/ligature test set.
- **W5.5 (native tier) — complete**: evaluation-time kind registry,
  store-owned instances, explicit runtime namespaces, typed part-theme
  queries, lifecycle and host-command outcomes, focus/localization routing,
  and registry-aware headless/vector export.
- **W5.6 (TextInput) — complete**: Undoer, click cycle, keybinding tables,
  scroll offset, `VisualRepresentation` rendering, revision-aware controlled
  inputs, params, commit policies, paired visual baselines, and vector-PDF
  coverage.
