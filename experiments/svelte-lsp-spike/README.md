# svelte-lsp-spike

A narrow-scope Svelte language server, built to test whether the design
measured in the architecture note actually works: instead of holding a
TypeScript program for the whole workspace, build one for the open file's
import closure only.

Not shipped, not on any roadmap. It lives under `experiments/` and is built
separately — it declares its own `[workspace]`, so `cargo test --workspace` at
the repo root never touches it.

One change was needed in a tracked crate to make it work:
`svn_typecheck::map_raw_diagnostics`, which exposes the diagnostic mapper to
callers that obtain diagnostics without running tsgo themselves. That is
additive and shipped separately — see the reasoning in "Stage 3" below.

## Running it

```sh
cargo build --release

# One full pass with no editor attached — closure, timings, diagnostics.
./target/release/svelte-lsp-spike --selftest path/to/Component.svelte

# As a language server.
./target/release/svelte-lsp-spike
```

Point an editor at the binary with no arguments. It speaks LSP over stdio and
publishes diagnostics on open and save.

## What it does

`didOpen` / `didSave` on a `.svelte` file →

1. **Discover the project** — walk up for the nearest `tsconfig.json`.
2. **Compute the closure** — follow `.svelte` imports transitively from the
   open file. `.ts` and package imports are left to tsgo's own resolution.
3. **Emit** — parse, walk, emit each file in the closure, exactly as
   `crates/cli` does per file.
4. **Check** — hand them to `svn_typecheck::check` against a narrowed tsconfig.
5. **Publish** — the diagnostics come back already mapped to `.svelte`
   coordinates by the crate's own mapper, so positions match the CLI's by
   construction rather than by reimplementation.

Svelte compiler warnings run as a separate per-file pass with no tsgo in it.

## Measured on `bench/control-svelte-5/src/apps/admin-app` (1,207 components)

| | |
|---|---|
| Diagnostic parity vs the CLI | **18 / 18 files with problems match exactly**, line and column |
| False positives | none across a 15-file random sample |
| Closure size | 1–244 files depending on the component |
| Warm check | 54–72 ms typical, 343 ms on a 35-file closure |
| Cold check | 140 ms – 4 s (tsgo cache state dependent) |
| Server resident | 42 MB, with tsgo spawned per check and exiting |

The CLI checking the same workspace: 1,357 files, ~4 s, tsgo peaking at
2,176 MB. `svelte-language-server` holding it open: 1,990 MB resident, 13.7 s
to first answer.

## Two things the spike found that the measurements had not

**Composite projects break narrowing.** With `composite: true` inherited from
the workspace's base config, every `.ts` a component imports raises TS6307
("not listed within the file list of project") — because the whole point of
narrowing is to *not* list them. The narrow config turns composite off; it
exists for build orchestration and buys a language server nothing.

**Ambient declarations cannot be reached by following imports.** A
`declare module` in `src/global.d.ts`, or SvelteKit's generated `$env/*`
declarations, are global by nature and imported by nobody. Dropping them
produced confident, entirely wrong diagnostics — "module has no exported member",
"cannot find module `$env/dynamic/public`" — on code that is fine. The narrow
config lists every `.d.ts` in the workspace explicitly. There are few and they
are small.

**But "every `.d.ts`" is far too many.** The obvious fix to the above —
list every `.d.ts` in the workspace — quietly pulled in SvelteKit's 98
generated per-route `$types.d.ts` files, each importing its own route's
modules. A one-component scope went to 3,935 files. Those are reached by
import when a route is actually in scope; only `.svelte-kit/ambient.d.ts` and
the app's own hand-written declarations belong in the list. Excluding
`.svelte-kit/types/` took it from 103 ambient files to 7, with diagnostic
parity unchanged.

All three are the kind of thing only a running server surfaces.

## Stage 2: hover against a warm tsgo

`hoverProvider` is live. The first hover starts a `tsgo --lsp -stdio` child
against the narrow overlay project and keeps it; subsequent hovers reuse it.

A cursor is translated `.svelte` → overlay by inverting the emit maps — token
map first (byte-exact, and the only thing that works inside template
expressions), line map as the fallback for verbatim script blocks. tsgo's
answer comes back in overlay coordinates and the range is mapped home the same
way; a range spanning two source lines is a synthesized construct with no
single source span, so the range is dropped and the hover text kept.

| | measured |
|---|---|
| Hover in a script block | `let rocketIcon: DotLottie \| null` — correct |
| Hover in template markup | `(property) onpointerover?: PointerEventHandler<HTMLButtonElement>` — correct |
| First hover (spawn + build) | 191 ms light closure, 455 ms 35-file closure |
| Warm hover | **1 ms** |
| Warm resident, light closure | 185 MB tsgo + 39 MB server |
| Warm resident, 35-file closure | 477 MB tsgo + 30 MB server |

Against `svelte-language-server` on the same workspace and file: 1,990 MB
resident, 13.7 s to first answer, 245 ms per hover after that.

## Stage 3: one process for everything

Diagnostics used to run through a *separate* batch tsgo, so a save spawned a
second process that type-checked the whole closure while the warm one sat idle
— up to ~1.1 GB transient on a heavy closure. That is gone. tsgo advertises
`diagnosticProvider`, so the warm child answers `textDocument/diagnostic` for
each overlay and there is exactly one compiler process.

Emitting without checking uses public API throughout: `CheckSession::prepare`
writes one overlay, `overlay::build` produces the tsconfig listing them. The
crate's own `check` is no longer in the path, so position mapping and the
suppression rules are reimplemented here against `scan_ignore_regions` and the
emit maps — which makes the parity sweep load-bearing rather than reassuring.

| | stage 2 | stage 3 |
|---|---|---|
| tsgo processes | 2 | **1** |
| Server resident | 30–41 MB | **6–7 MB** |
| tsgo resident, light closure | 185 MB | 191 MB |
| tsgo resident, 35-file closure | 477 MB (+ ~1.1 GB batch transient) | **646 MB, no transient** |
| Diagnostics, light closure | 41 ms | **24 ms** |
| Diagnostics, 35-file closure | 343 ms | **49 ms** |
| Warm hover | 1 ms | 1 ms |
| Diagnostic parity | 18/18 | **18/18** |

### Two more bugs, both from running it

**Interleaving sync and pull costs a re-check per file.** Syncing one overlay
and immediately pulling its diagnostics, file by file, means every pull runs
against a program the next `didChange` invalidates — tsgo declares
`interFileDependencies`, so a 35-file scope paid 35 full re-checks and took
3.0 s. Syncing every overlay first and pulling afterwards took the same scope
to 49 ms. Skipping the `didChange` entirely when an overlay's text is unchanged
keeps a re-save of one file from disturbing the other 34.

**Hint-severity diagnostics have to be filtered by hand.** The batch path drops
them; pulling straight from tsgo does not. Without the filter, a file picked up
`'base' is deprecated` (TS6385) twice — diagnostics `svelte-check` reports only
under `--include-suggestions`. This is the shape of divergence to expect when
bypassing the crate's mapper, and it is exactly what the parity sweep is for.

## Stage 4: union scope across open tabs

The scope is now the union of every open tab's closure, in most-recently-used
order, capped at 600 files. One program for the window, not one per file: the
dependency declarations are ~88% of any program here, and paying for them once
is the whole reason this fits in memory.

Seven tabs on `admin-app`, opened one at a time:

| tab | open time | tsgo | server |
|---|---|---|---|
| 1 | 167 ms | 186 MB | 6 MB |
| 3 | 175 ms | 190 MB | 7 MB |
| 5 | 803 ms | 551 MB | 7 MB |
| 7 | 1,091 ms | 705 MB | 8 MB |

Hover in tab 1 after seven tabs: 17 ms. Diagnostics stayed at parity throughout.

### Restarting tsgo beats telling it the config changed

A new tab rewrites the project's file list, and tsgo has to be told. The
intended cheap path — `workspace/didChangeWatchedFiles` on the tsconfig —
does not work: tsgo keeps its old program, the newly listed overlay becomes an
orphan in an inferred project with no shims, and the file fills with
`Cannot find name '__svn_any'` and implicit-any errors. Ten confident,
completely wrong diagnostics on a file that is fine.

Restarting the process is correct, and on a narrow scope it is also *faster*
than the notification was supposed to be: 285 ms against 322 ms for a second
tab. That inverts the usual instinct, and it only holds because the program
being rebuilt is small. `SPIKE_KEEP_ON_SCOPE=1` keeps the notification path for
retesting against a future tsgo.

### And a bug that only routes could find

A tab whose path contained parentheses never received its diagnostics. The URI
encoder was encoding anything non-alphanumeric, but editors encode far less:
`(group)` stays literal while `[param]` is encoded, and SvelteKit route trees
use both, often in the same path. Over-encoding produces a URI the editor never
matches to the file it asked about, so the diagnostics go nowhere — silently,
with no error on either side.

## Parity against upstream's own LSP tests

`language-server/test/plugins/typescript/features/HoverProvider.test.ts` holds
upstream's hover expectations — exact `contents` strings at exact positions,
against fixtures in `testfiles/hover/`. Extracting them and replaying them
through this server:

**10 / 10 match**, including the `$store` cases, which are the Svelte-specific
ones a generic TypeScript hover gets wrong.

Nine matched immediately. The tenth — three cases, one cause — was upstream
separating a signature from its documentation with a markdown rule where tsgo
runs them together. Every type and every position already agreed.

### The diagnostics corpus

`features/diagnostics/fixtures/` holds 74 fixtures with expectations already in
LSP shape — `input.svelte` plus `expectedv2.json` (or `expected_svelte_5.json`)
carrying ranges, severities and codes. The main repo's `ls_diagnostics` suite
runs these through the CLI; they replay through this server unchanged.

**59 / 74 strict match** on `(line, character, code)`, against **57 / 74** for
the CLI on the same corpus. Thirteen of the remainder are fixtures the CLI also
skips or excludes; two are genuinely worse here.

Getting there took three fixes, and the first one is the interesting one.

**Stop cloning the mapper.** The first run scored 52, and every one of the nine
gaps traced to a filter in `svn_typecheck`'s `map_diagnostic` that the
hand-rolled version here did not have: emit-ignore regions, the Svelte-4
reactive-label label, duplicate element-attribute keys, transition-callback
arity, pug containers, the `bind:` message rewrite. Those filters *are* the
upstream parity, and a second implementation of them was never going to hold.
`svn-typecheck` now exposes `map_raw_diagnostics`, this server builds the
crate's own `MapData` and calls it, and the local mapper is gone. 52 → 58.

**tsgo's LSP type-checks JavaScript that its batch mode leaves alone.** A plain
`<script>` component came back with `Object is possibly 'null'` where the CLI
reported nothing. tsgo returns semantic diagnostics for any *open* `.js`
document regardless of `checkJs`, leaving the client to decide — so the client
has to. Gated on the resolved `checkJs` plus a `// @ts-check` opt-out. 58 → 59.

**Ambients are not just any `.d.ts`.** `$types.d.ts`, `components.d.ts`,
`Foo.svelte.d.ts` are ordinary modules that happen to hold only types —
something imports them. Only a file with `declare global`, `declare module`,
`declare namespace` or a triple-slash reference is unreachable by import and
therefore has to be listed. Corpus-neutral, but it shrinks every program.

### The two that remain

| fixture | upstream | here |
|---|---|---|
| `pug` | 3 diagnostics | those 3, plus TS6133 / TS6192 unused-import hints |
| `snippet-js.v5` | `16:12` TS2345 | `16:17` TS2345, plus a TS7006 |

Both are tsgo behaving differently through its language server than through
`tsc -p`, on byte-identical overlays — confirmed by running the CLI over the
same two fixtures, where batch tsgo produces upstream's answer exactly. The LSP
computes unused-import suggestions batch does not, and anchors an
overload mismatch on a different argument. Nothing here can map its way out of
that; it wants a tsgo issue.

### One surface, two rules about hints

Running this corpus surfaced a split worth stating. Hint-severity diagnostics
(TS6133 unused, TS6385 deprecated) are dropped by svelte-check's CLI writers
but *always* requested by its language server — `DiagnosticsProvider.ts` calls
`getSuggestionDiagnostics` beside `getSemanticDiagnostics`, which is why these
fixtures expect them. Filtering them was right for the CLI-parity sweep and
wrong for an editor. The server now includes them and `--selftest` does not,
which is what lets both comparisons stay honest. Including them moved this
corpus from 40 to 52.

`testfiles/completions/` has 46 more fixtures waiting for when completion lands.

## Stage 5: where the time actually goes

Phase timing, once the design settled, turned up something the earlier
end-to-end numbers had hidden. The check was dominated neither by tsgo nor by
disk, but by this server's own scan for ambient declaration files — a walk that
reads every `.d.ts` under the workspace to decide which ones are ambient, and
was re-running on every single check.

| | one-file scope | 35-file scope |
|---|---:|---:|
| before caching the scan | 23.3 ms | 41.0 ms |
| after | **3.9 ms** | **22.1 ms** |

What remains on the 35-file scope: emit 4.3 ms, overlay writes 5.7 ms, overlay
tsconfig 1.5 ms, tsgo 8.0 ms.

That breakdown also settles a design question worth recording, because the
answer is counterintuitive. Overlays could be kept entirely in memory and
handed to tsgo over `didOpen` — tsgo accepts an in-memory file during module
resolution as long as its parent directory exists — writing only the tsconfig
and shims to disk. It would remove the 5.7 ms of writes.

It is not worth it here. Those writes are 5.7 ms of a 22 ms check, and the
overlay text is currently held *off* this process's heap by
`LazyText::on_disk`, which is most of why the server sits at 6-7 MB while tsgo
holds hundreds. Going diskless trades a quarter of the check time for putting
every overlay back on our heap. A design that already holds its projections in
memory would get it for free; this one would pay for it.

## A note on stdout

The server speaks length-framed JSON-RPC on stdout, with no trailing newlines.
`--selftest` prints its results there too. Debug logging that notified the
editor unconditionally therefore glued a JSON frame onto the front of the
`DIAGS` line, and the parity harness reported all 18 bench files as
mismatches — a convincing-looking regression that was nothing of the sort.
`rpc::begin_serving()` now gates it. Worth knowing before adding a `println!`
anywhere in this tree.

## What is not here

- **Completion, go-to-definition, rename.** The hover path generalises to the
  first two; rename needs the wide scope and is a different problem.
- **Incremental typing.** Diagnostics refresh on save. Typing updates the buffer
  and nothing else — the frozen-workspace bargain.
- **Cross-file correctness after edits.** Change a component's props and its
  consumers refresh when their own scope is next built.
- **Alias resolution in the closure scan.** `$lib/` and relative specifiers
  resolve; tsconfig `paths` aliases do not. Missing an edge means a component
  without an overlay, not a wrong type.
- **Scope eviction.** The union is capped at 600 files and ordered
  most-recently-used, but nothing is evicted before the cap — a long session
  only grows.
- **One project per window.** Files under a different tsconfig are skipped
  rather than given their own scope.
