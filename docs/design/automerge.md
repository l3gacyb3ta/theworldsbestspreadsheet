# Design note: collaboration on Automerge (#18)

Status: **proposal, for review**. Nothing here is implemented yet except the throwaway
spike in `spikes/automerge/` (tests + one benchmark example) that checks the Automerge
behaviours this note relies on. Section numbers (§) are referenced from the spike.

## 0. Summary

- One workbook = one Automerge document. The document is the source of truth; the
  `Workbook` struct becomes a cache that is kept equal to it, and the `Engine` keeps
  computing from the `Workbook` exactly as today.
- **Cells are a flat map keyed by `row:col` ids, holding the whole stored formula as one
  value.** Two edits to the same cell never merge character by character; one wins, and
  the cell is flagged.
- **Row/column order is a map `id → position key` (fractional indexing), not an Automerge
  list.** Automerge has no move operation, and a sort written into a list can merge into a
  list that duplicates one row and loses another (spike proves it). With position keys a
  merge is always a permutation.
- **Rows beyond the used area are virtual, with deterministic ids**, so scrolling never
  writes to the document and two people typing into the same empty cell are typing into
  the same cell.
- Nothing is ever really removed: deleted rows, columns and sheets are hidden by a flag,
  so a concurrent edit inside them survives and restoring brings it back.
- Undo is local: it undoes *your* last change, as a new change, and skips anything
  someone else has changed since.
- Transport: `samod` (the Rust automerge-repo, wire-compatible with the JS one) over
  WebSocket to a sync server; presence (who's here, selections, in-progress edits) as
  automerge-repo ephemeral messages.
- The owner needs to decide hosting, the default file format, and the privacy defaults
  (§11).

## 1. Goals and non-goals for 1.0

The issue title is "Automerge sharing, live edit viewing, etc.". I read that as:

**Goals**

1. **Live co-editing**: two or more people have the same workbook open and see each
   other's committed edits within about a second, including structural edits (insert,
   delete, sort, move, sheets, names, sizes).
2. **Offline edits merge**: edit on a plane, reconnect, everything merges without a
   prompt. The same mechanism merges two copies of a file.
3. **Presence**: who is here (name + colour), which sheet they are on, their selection,
   drawn in the grid.
4. **Live edit viewing**: while someone is typing in a cell, others see the text they are
   typing as a ghost in that cell (not committed, not in the document); while someone
   scrubs a number, others see the dependents move, at a lower rate (§9).
5. **Predictable conflicts**: every merge outcome is explainable in one sentence (§4), and
   anything that might surprise is flagged on the cell, tab or row where it happened.

**Non-goals for 1.0** (each could come later): accounts, permissions and read-only
links; end-to-end encryption; a history/time-travel browser; comments; a web/wasm
client; collaboration on goal-seek, the playground or other scratch evaluations (they
never touch the document); merging character-level edits inside one formula.

## 2. Document mapping

### 2.1 Schema

Ids are written as 16-digit lowercase hex strings (map keys must be strings).

```
ROOT
  schema: "wbs/1"
  sheets: Map<sheet-id, Map>            -- a sheet object is created once, by one peer
    name:     Str
    pos:      Str                       -- tab order (position key, see 2.2)
    deleted:  Bool                      -- tombstone; never removed
    seed:     Uint                      -- seed for this sheet's virtual row/col ids (2.3)
    row_pos:  Map<row-id, Str>          -- position key of each materialised row
    row_dead: Map<row-id, Bool>
    col_pos:  Map<col-id, Str>
    col_dead: Map<col-id, Bool>
    row_h:    Map<row-id, F64>          -- only rows with a non-default height
    col_w:    Map<col-id, F64>
    cells:    Map<"row-id:col-id", Str> -- serialised Vec<Piece>; absent = empty
  names: Map<name, Str>                 -- "sheet-id:row-id:col-id" or "...:input"
  people: Map<actor-id, Str>            -- display name + colour for attribution (optional)
```

Rules of thumb behind it:

- **Everything per row, column and cell is a flat scalar in a map**, never a nested
  object. Two peers creating the same nested object at once get two objects and one of
  them (with its fields) is hidden — `concurrently_created_nested_objects_lose_fields` in
  the spike. Flat scalars merge per field: one person resizing row 7 while another
  deletes it gives a deleted row that remembers its new height.
- A sheet object *is* nested, but its key is a fresh random id created by exactly one
  peer, so it is never created concurrently.
- `schema` lets a later version migrate a document in place.

### 2.2 Row and column order: position keys, not Automerge lists

SPEC §1 suggested "row/column order as list CRDTs". Automerge's list *is* a list CRDT and
keeps tombstones internally, but its API doesn't expose them, and it has no move
operation (checked in the 0.12.0 source: `Transactable` has `insert`, `put`, `delete`,
`splice`, no move). Our operations map onto it like this:

| our op | Automerge list | position-key map |
|---|---|---|
| insert rows | `insert` — native, runs stay contiguous (spike `list_runs_do_not_interleave`) | new ids with keys between the neighbours, one random prefix per insert so runs stay contiguous |
| delete rows | would have to keep our own `dead` set anyway (range corners need the dead id's place) | `row_dead[id] = true` |
| sort (`PermuteRows`) | per-slot `put` (no move); merges can **duplicate one row and lose another** (spike `sorting_a_list_by_slot_puts_can_lose_a_row`) | each sorted row gets a new key; every row always has exactly one key, so any merge is a permutation (spike `position_keys_keep_concurrent_sorts_a_permutation`) |
| undo of delete | flip `dead` | flip `dead` |

So the proposal is **a map from id to a string position key** ("fractional indexing";
`fractional_index` 2.0.2 and `jittered-fractional-indexing` 0.1.2 exist on crates.io,
or ~60 lines of our own). Visible order = alive ids sorted by `(key, id)`. This is still
a list CRDT, just one with a move. In memory the `Axis` keeps its current shape (`order`,
`dead`, indexes); only the representation in the document changes. I'd update the SPEC
wording when this lands.

A **sort** writes, for the sorted block, fresh keys `prefix + slot` between the keys of
the rows just before and after the block (`prefix` random per sort), writing rows in
ascending id order. Because both sorts then write the same rows in the same order,
Automerge's last-writer-wins picks the same sort for every row, so two concurrent sorts of
the same block give exactly one of the two sorts — the spike checks this. Overlapping
but different blocks can give an order that is neither sort, but never a lost or
duplicated row.

Sheet tab order uses the same scheme (`sheets[id].pos`).

### 2.3 Virtual rows: the sheet no longer grows by writing

Today the sheet grows by appending fresh random ids: `ensure_size` from scrolling and
selection (`app.rs` `select`, `grid.rs`), `key_grow` when a formula references a cell
past the end (`Workbook::parse_text`), and spills (`engine.rs`). None of these go through
`Edit`. In a shared document that breaks: if Alice scrolls to row 300 (appending 100
rows with her ids) and Bob types in row 260 (appending 60 rows with his), the merged
sheet has 160 new rows and Bob's cell is no longer at row 260 on anyone's screen.

Proposal: row *k* past the materialised rows has the deterministic id
`mix(seed, "row", k)` and the position key `t` + base-36 *k* (fixed width). The
document only stores rows that need storing ("materialised"): rows that were inserted,
sorted or deleted, or that rows were inserted in front of. (As built in PR 2: holding a
cell, being referenced or resized doesn't materialise a row, because `mix` is a
bijection, so a virtual id gives back its *k* and finds its row without a stored key.)
Materialising row *k* writes
rows `0..=k` with their canonical keys; two peers doing it at once write identical
values, so it is idempotent (spike `materialising_the_same_virtual_row_is_idempotent`).
Inserted and sorted rows get keys between materialised neighbours, so every
materialised key sorts before every virtual one, and the visible order is "materialised
rows by key, then virtual rows by *k*".

Effects: a new sheet stores no rows at all (today: 200 × 26 random ids); scrolling,
spills and goal-seek trials never write; two people typing into the empty `B260` of the
same sheet write the same cell key and get the same-cell rule (§4.1). Duplicating a
sheet copies its `seed`, which matches today's "the copy keeps the row and column ids".
Files saved before this change keep their random ids, materialised with keys `m…`
(sorting before `t…`), in their current order.

### 2.4 Cells and formulas

A cell is one string: `serde_json` of today's `Vec<Piece>` (the same shape as in the
current JSON file). References stay as row/column ids, so inserts, deletes and sorts
elsewhere never touch a cell — the property the model was built for.

**Text vs structured**: storing the formula as an Automerge `Text` would let two people
edit one formula at once and merge characters. For a program that is the wrong default:
`=A1 2 *` edited to `=A1 3 *` by one person and `=A1 2 * 1 +` by another merges into
`=A1 3 * 1 +` or worse, a program nobody wrote, and references are ids inside pieces,
not characters. The predictability principle says one of the two programs wins and the
other is kept for the user to pick (§4.1). Last-writer-wins on a whole cell also makes
the number-literal scrub fast path (only a number changed, so no static pass) still
apply after a merge.

### 2.5 Names, units, words, sizes

- **Names**: `names[name] = "sheet:row:col[:input]"`. Today `Edit::Names` replaces the
  whole map; it becomes per-name puts and deletes, so naming `B2` and naming `C5` at
  the same time both survive.
- **Units, dimensions and words** are declared in cells, so they need no mapping of
  their own; the existing rule "first definition in sheet order wins, later ones show an
  error" decides duplicates (§4.7).
- **Row heights / column widths** are flat maps (above). Today resizing writes
  `row_heights` directly from `grid.rs` without an `Edit`; it becomes an `Edit` (and
  gains undo).

### 2.6 Ids across peers

Random u64 is fine. With *n* ids the collision probability is about n² / 2⁶⁵: 10⁶ ids
(a very large shared workbook) gives ~3 × 10⁻⁸. Two things to fix: `ids.rs` seeds its
generator from the clock's nanoseconds, so two processes started in the same
nanosecond (or on a coarse VM clock) generate the same sequence — seed it from OS
entropy (`getrandom`, or std's `RandomState` to stay dependency-free) instead; and
virtual ids come from the same 64-bit mixer, so they share the same odds.

Automerge's own actor ids (random 16 bytes per session) are separate from ours.

## 3. Change flow

```
local:  App ──Edit──▶ Session::exec ──▶ Engine::apply (as today; returns inverse)
                         └──▶ DocWriter: one Automerge transaction with the touched
                              keys copied from the Workbook, + change message
remote: sync ──▶ doc changed ──▶ diff_incremental() patches ──▶ touched keys
                         └──▶ Workbook updated for those keys ──▶ Engine::apply_external
                              (cells only → cells_changed; anything else → rebuild)
```

- **Local edits are written state-based**: after `Engine::apply`, the writer copies the
  touched entries (cell keys, axis entries, names, sheet fields) from the `Workbook`
  into the document in one transaction. `apply_raw` already collects touched cells;
  it is extended to collect the other entries. This avoids writing a second
  interpretation of every `Edit` variant.
- **The writer lives above the engine** (a `Session` that owns `Engine` + document), not
  in `Engine::apply`. `Engine::goal_seek` writes trial values through `set_text` and then
  restores them; the playground and step-through evaluate without writing. Keeping the
  document out of `Engine` means none of that can ever reach a shared document. A test
  asserts that goal-seek on a session produces no document change.
- **Each change carries a message**: a short JSON summary of the `Edit` (`{"op":"sort",
  "sheet":…, "rows":…}`). It costs a few bytes and lets the conflict detection in §4 and
  the toasts ("Alice deleted row 7") work from the change, not by diffing.
- **Remote changes**: patches name the keys that changed. A cell key → `cells_changed`
  (the incremental path, which already rebuilds itself when a declaration changes);
  anything in `row_*`/`col_*`/`names`/`sheets` → `rebuild()` (today's rule for
  structural edits). Patches from all pending remote changes are applied once per frame,
  so a burst of remote changes costs one recalc.
- **Derived state** (symbols, spills, static dimensions, cycles) is always derived from
  the `Workbook`, so it is correct as long as (a) the `Workbook` equals the document and
  (b) the incremental path equals a rebuild. Both are tested by a convergence fuzz test
  (§10, PR 4): random edits on 3 peers, random merge orders, then every peer's
  `Workbook` must equal a fresh load of the document and every cell result must equal
  `Engine::new` on it.
- **The undo stack holds no positions** (see §5), and the selection and presence store
  ids, so a remote insert above you doesn't move your cursor to another cell.

## 4. Conflict semantics

Automerge resolves concurrent writes to one key by a deterministic order of operations
(the same on every machine; not wall-clock time) and keeps the losing values readable
(`get_all`) until someone writes the key again. Each rule below is one sentence; the
example says what everyone sees after the merge.

**4.1 Same cell, same time.** *When two people change the same cell at once, one
version wins everywhere and the cell is flagged until someone edits it.* Alice types
`=B2 3 *` in `C1`, Bob types `=B2 2 * 1 +`. Both see the same winner, and `C1` gets a
conflict mark (a corner triangle in the other person's colour); the inspector says
"Bob also wrote `=B2 2 * 1 +` at the same time" with a "Use this" button. Picking either
is a normal edit, which clears the conflict (spike `same_cell_conflict_is_deterministic_and_visible`).

**4.2 Row deleted while someone edits a cell in it.** *Deleting a row hides it with
everything in it, including edits made to it at the same time; restoring it brings them
back.* Alice deletes row 7 while Bob types `42` in `B7`. Row 7 disappears for both; the
cells are still in the document (a put survives a concurrent delete — spike
`put_survives_concurrent_delete` — and in this design nothing is deleted anyway).
References to `B7` show `#ref!` as they do today. Bob gets a toast: "Alice deleted row
7, which had your edit to B7 — Restore row". This is a behaviour change from today,
where deleting a row removes its cells from the map (the undo entry carries them): the
cells now stay, hidden, so the file keeps them.

**4.3 Two people sorting.** *If two people sort the same rows at once, one sort wins;
whatever happens, every row is still there exactly once.* Same block → exactly one of
the two orders. Overlapping blocks (Alice sorts rows 2–20 by A, Bob rows 10–30 by C) →
a permutation that may be neither; the rows get a "sorted by two people at once — sort
again?" note. The sort also rewrites range corners so ranges keep covering the same
block (`PermuteRows`); a range typed concurrently by someone who hadn't seen the sort
keeps the ids its author saw, which may now be a different block. That case is
detected from the change messages (a sort concurrent with a range edit on that block)
and the cell is flagged "a concurrent sort moved this range's rows".

**4.4 Inserting rows at the same place.** *Rows inserted at the same place at once all
appear, each person's together.* Alice inserts 2 rows above row 5, Bob 3: the result has
all 5, Alice's two together and Bob's three together, in an order that is the same
everywhere. Bob typing into the row he saw as row 5 while Alice inserts above it writes
into that row (now row 7 for both): edits follow ids, not positions.

**4.5 Moving cells while someone edits a formula that points at them.** *A move rewrites
the formulas that existed when it was made; a formula written at the same time still
points where its author pointed.* Alice moves `B2:B4` to `D2:D4`, which rewrites `C1`
from `=B2` to `=D2`. Bob concurrently changes `C1` to `=B2 2 *`. If Bob's version wins
(4.1), `C1` reads the now-empty `B2` and shows the existing "B2 is empty" error; if
Alice's wins, `C1` is flagged with Bob's version. Bob typing into `B3` (a cell being
moved) at the same time keeps his text in `B3`, while `D3` gets what was in `B3` when
Alice moved it. This is the most surprising rule and the main argument for one day
giving cells their own id independent of position; that is a much larger change and
not proposed for 1.0.

**4.6 Sheet deleted while edited.** *Deleting a sheet hides it with everything in it,
including edits made at the same time; restoring it brings them back.* References into
it show the existing "reference to a deleted sheet" error. The editor of the deleted
sheet is moved to a neighbouring tab, with a toast and a Restore button. Two people
renaming one sheet: one name wins (4.1 applied to the name). Two sheets that end up with
the same name (both created "Sheet2", or renamed to "Rates" at once) keep it — the sheet
never renames anything by itself — but both tabs are marked "another sheet has this
name", and typing `Rates!A1` is an error naming the ambiguity until one is renamed.

**4.7 Names, words, units created twice.** *A name is a key: if two people give the same
name to different cells at once, one wins and the names panel shows the clash.* Formulas
store names as text and resolve them when compiled, so all users of `growth` follow the
winner. Two names for one cell (Alice names `B2` `growth`, Bob names it `rate`) are both
kept; the "one name per cell" check applies only to local edits. Words, units and
dimensions are declared in cells, so two `: npv … ;` cells, or two `[EUR] = …` cells,
are just the existing duplicate rule: *the first definition in sheet order wins and later
ones show an error* — no new mechanism.

**Surfacing, generally**: conflicts and "your edit landed somewhere hidden" are shown
where they happened (cell mark, tab mark, row note) plus one toast; nothing is
silently rewritten to "fix" a merge.

## 5. Undo and redo

- Undo stays **local**: ⌘Z undoes *your* last change, never someone else's. It is applied
  as a new change (spike `undo_is_a_new_change`), so others see it like any edit.
- **Positional inverses must go first.** Today's inverses are positional:
  `InsertRows { at }`, `DeleteRows { at, n }`, `PermuteRows { at }`, `MoveSheet { to }`,
  `InsertSheet { at }`. After a remote insert above, `at` points at the wrong rows. PR 1
  makes every `Edit` id-based: delete/restore rows by id (flip `dead`), sort as
  `(id, new key)` pairs whose inverse is the old pairs, sheets by id and key. This is
  also what SPEC §1 asks for ("nothing should assume positional addressing").
- **Someone else changed it since**: *undo only reverts what still looks the way your
  change left it; anything someone else changed since is left alone, and the status bar
  says so.* Each undo entry records the values its change wrote; at undo time, keys
  whose current value differs (someone overwrote your `C1`, or typed into the rows you
  inserted) are skipped. If everything was overwritten the undo step is dropped with a
  message rather than doing nothing silently. Redo uses the same check.
- Remote changes do not clear your redo stack.

## 6. Transport, hosting, sharing, privacy

What I checked (2026-10-10, `cargo search` / `cargo info` / crates.io API, and reading
the downloaded sources):

| crate | version | last release | notes |
|---|---|---|---|
| `automerge` | 0.12.0 | 2026-09-16 | MIT, needs Rust ≥ 1.90 (we have 1.95); sync protocol in `automerge::sync`; no move op |
| `autosurgeon` | 0.14.0 | 2026-09-17 | derive `Hydrate`/`Reconcile` for Rust structs |
| `samod` / `samod-core` | 0.15.0 | 2026-09-30 | Rust automerge-repo, "wire compatible with the automerge-repo JavaScript library"; depends on automerge 0.12; runtimes tokio/gio/local pool; filesystem storage; WebSocket via tungstenite or axum; `DocHandle::broadcast`/`ephemera` for ephemeral messages |
| `automerge_repo` | 0.3.0 | 2025-10-03 | the older Rust automerge-repo; samod looks like its successor |
| `sync.automerge.org` | — | — | public sync server; answered `200 👍 running` today |

I did not build or run samod, the public server, or anything newer (Subduction /
sedimentree / Keyhive crates appear on crates.io but I could not judge their maturity).

**Options**

1. **samod + WebSocket to a sync server (recommended).** The app runs a samod `Repo` on a
   background tokio thread with filesystem storage; the server relays and stores. Works
   offline (local storage, sync on reconnect), interoperates with any automerge-repo
   server, and ephemeral messages (presence) come free. The server can be:
   - the **public `sync.automerge.org`**: zero setup, but no auth, no guarantees, and its
     operator can read every document;
   - **self-hosted**: samod with its `axum` feature is a ~100-line binary we could ship as
     `wbs-sync-server`, or the JS automerge-repo sync server.
2. **Raw `automerge::sync` over our own WebSocket relay.** Fewer dependencies, but we would
   rewrite storage, reconnection, multiplexing and presence that samod already has, with
   no interop.
3. **Peer-to-peer / LAN** (mDNS discovery, or iroh-style hole punching). No server, but NAT
   traversal, discovery and "both must be online" make it a poor default. samod's
   transport trait means it can be added later without touching the document code.
4. **File-based sync** (Dropbox/iCloud folder): the `.wbs` Automerge file in a shared
   folder. Not live, but merging is trivial: on open, merge the file with any
   "conflicted copy" siblings (`Automerge::merge`). Cheap enough to include anyway.

**Sharing and identity.** "Share…" creates (or reuses) the document id and shows a link
such as `wbs://share/<automerge-url>?server=wss://…`; "Open link…" (and a URL handler)
joins it. Identity is a local display name + colour (preferences), unauthenticated, as in
most automerge apps; changes can carry it via Automerge's author field (0.12 has
`with_author`; read in the source, not tried) or our `people` map.

**Privacy and security** — what users must be told on the Share dialog:

- The link is the key: **anyone with the link can read and edit**, forever (links can't be
  revoked; making a fresh copy is the workaround).
- The sync server stores the document **unencrypted**; on the public server, its
  operator can read it.
- Sharing shares **history**: a cell deleted last month is still in the document. The
  first time an existing file is shared, offer "share without history" (a fresh document
  from the current state).
- Presence names are self-declared.

End-to-end encryption and read-only links need either server-side auth (samod supports
binding a transport to an authenticated peer id) or a capability system like Keyhive;
both are out of 1.0.

## 7. File format and migration

- **Native format becomes the Automerge binary** (`doc.save()`), extension `.wbs`. JSON
  (`.wbs.json`) stays readable forever — opening one migrates it into a new document
  (rows keep their ids, keyed `m…` in order) — and "Export JSON…" writes it.
- Shared documents live in samod's storage (an app-data folder) and are saved
  continuously; the title bar shows sync state ("synced", "offline — 12 changes not yet
  sent") instead of the dirty dot. Local files keep today's explicit Save: ⌘S writes the
  document's bytes, and `files.rs`'s fingerprint keeps deciding "dirty", unchanged.
- A shared document can also be saved to a `.wbs` file (a snapshot with history); opening
  that file again offers to reconnect.

## 8. Presence and live edit viewing

Presence is sent with `DocHandle::broadcast` as small CBOR messages (the JS
automerge-repo only accepts CBOR) and is never stored:

```
{ peer, name, colour, sheet,                      // every 5 s as a heartbeat, and on change
  sel: [anchor-key, cursor-key],                  // ids, not positions
  editing: { cell, text, caret } | null,          // while typing, throttled to 10 Hz
}
```

A peer not heard from for 15 s disappears. In the grid: each person's selection is a
2 px outline in their colour with a name tag on its corner; a tab shows coloured dots
for who is on that sheet; a cell someone is editing shows their uncommitted text in
italics in their colour (not evaluated, not referenced). The toolbar shows avatars
(initial + colour); clicking one jumps to their selection.

## 9. Performance

Measured with `cargo run --release -p automerge-spike --example size` on an Apple M1 Pro,
for a 3,000-cell sheet of formula-sized values:

| | |
|---|---|
| build the document | 40 ms |
| saved size | 46 KiB |
| load | 3 ms |
| one cell change + full sync round trip between two in-process peers | 0.15 ms, 376 bytes on the wire |
| merge one remote change and compute its patches | 0.17 ms |
| 600 per-frame changes (10 s scrub at 60 fps): growth of the saved file | ~1 KiB |

So Automerge itself is far inside the ~10 ms scrub budget; the costs that matter are
ours: a remote structural change runs `rebuild()` (as a local one does today), and every
remote change runs a recalc. Hence:

- **Scrubbing** writes the document at most every 100 ms (and on release), not every
  frame, and pushes one undo step. Others see the dependents move at ~10 Hz; the
  scrubber's own grid still updates every frame from the engine. The spike shows
  per-frame writing would also be affordable, so this is about remote recalc load and
  history noise, not Automerge cost.
- **Remote patches are batched per frame** (one `cells_changed` for all of them).
- **Hydrating the whole `Workbook`** happens only on open/join; afterwards only touched
  keys are read.
- PR 4 adds a benchmark of `rebuild()` on the demo workbook so we know what a remote
  insert costs.

## 10. Implementation plan (PR-sized)

1. **Id-based edits** (core only, no Automerge): `Edit` variants by id (restore/hide rows
   and columns, sort as id → key, sheets by id), deleting keeps cells (hidden), names as
   per-key changes, resizes as edits with undo, entropy-seeded ids. Single-user behaviour
   unchanged except the resize undo. Tests: undo/redo of each op after an unrelated
   insert above still targets the right rows.
2. **Position-key axes and virtual rows**: `Axis` backed by position keys; scrolling,
   spills, `parse_text` and goal-seek stop growing the sheet; old JSON files load with
   identical positions. Tests: positions of every cell in the demo and test workbooks are
   unchanged after load; fingerprint/dirty unchanged by scrolling.
3. **Document schema + `wbs-sync` crate** (new crate, depends on `automerge`; core stays
   free of it): `Workbook ↔ document` (full hydrate, state-based writer), `.wbs` open/save,
   JSON import/export, change messages. Tests: round-trip property test; every `Edit` in the
   existing test suites leaves `hydrate(doc) == workbook`.
4. **Merging**: `Session`, remote patches → `Engine::apply_external`, conflict tracking,
   the convergence fuzz test (3 peers, random edits incl. structural, random merge orders,
   compare against a fresh engine), the §4 scenarios as named tests, goal-seek-writes-
   nothing test, `rebuild()` benchmark. File-based merging of conflicted copies.
5. **Shared undo and scrub coalescing** (§5, §9) with tests for the skip rule.
6. **Networking**: samod repo on a background thread, Share / Open link, sync status in the
   title bar, server setting; an in-process two-repo test over samod's channel transport.
   Optionally the `wbs-sync-server` binary.
7. **Presence and live edit viewing** (§8), with UI snapshot tests using two in-process
   sessions.
8. **Conflict UI**: cell/tab/row marks, inspector "use this version", toasts with Restore,
   help pages ("Working together") and README section.

The spike crate is deleted in PR 3.

## 11. Open questions for the owner

Each has my recommendation; a plain "agree" on the list is enough to start PR 1.

1. **Scope**: is §1 the right reading of "live edit viewing" (others' selections, ghost
   text while typing, scrubs at ~10 Hz)? *Recommend: yes; history browsing, comments and
   permissions after 1.0.*
2. **Row/column order as position keys instead of Automerge lists**, and a SPEC §1 wording
   update to match? *Recommend: yes* — lists can lose rows under concurrent sorts (spike).
3. **Virtual rows with deterministic ids** (scrolling never writes; files stop storing
   200 × 26 ids per sheet)? *Recommend: yes.*
4. **Deleted rows/columns/sheets keep their cells hidden** in the file instead of
   removing them? *Recommend: yes* — it's what makes concurrent edits recoverable.
5. **Whole-cell last-writer-wins with a conflict mark**, no character-level merging of
   formulas? *Recommend: yes.*
6. **Hosting default**: public `sync.automerge.org`, a server we run, or "bring your own"
   only? *Recommend: ship a self-hostable `wbs-sync-server`, default to the public server
   so sharing works out of the box, and say on the Share dialog that its operator can read
   the document.* This is the decision I most need from you.
7. **Native file format**: Automerge `.wbs` for every file, JSON as import/export?
   *Recommend: yes* (one code path; any file can be shared later and merged from a synced
   folder).
8. **Privacy defaults**: no read-only links or encryption in 1.0, and "share without
   history" offered on first share? *Recommend: yes to both.*
9. **Undo** skips what others changed since and says so (§5)? *Recommend: yes.*
10. **Scrubbing** writes to the shared document at 10 Hz rather than every frame?
    *Recommend: yes.*
11. **Duplicate sheet names after a merge** are flagged, never auto-renamed, and
    ambiguous `Name!A1` references are errors? *Recommend: yes* (no silent rewriting).
12. **Identity**: self-declared display name + colour, no accounts? *Recommend: yes for
    1.0.*
13. **New dependencies**: `automerge` + `samod` (+ tokio, tungstenite) in a new
    `wbs-sync` crate, keeping `wbs-core` dependency-free? *Recommend: yes.*
