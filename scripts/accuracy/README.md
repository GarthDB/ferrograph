# Accuracy harness: `dead` and `pr` against compiler ground truth

Measures **recall** and **precision** of `ferrograph dead` (default and `--strict`) and the
"newly dead" part of `ferrograph pr` using rustc's `dead_code` lint as the oracle instead of
hand classification. The runs are not part of CI (they need external corpora and minutes of compile time);
the self-tests on the built-in synthetic corpus are (`cargo test -p xtask`).

The harness is the `xtask` crate in this repository: `cargo xtask <command>` (see `.cargo/config.toml`), so it needs
only the Rust toolchain, `git` and a ferrograph binary. `cargo xtask --help` lists the commands; each takes the same options as
the Python script it replaced.

## Run

```bash
# 1. Static ground truth (Part A). Corpora come from corpora.toml.
cargo xtask groundtruth --ferrograph /path/to/ferrograph \
    --work /tmp/fg-acc --only clap,ripgrep,fd \
    --out-md groundtruth-report.md --out-json groundtruth-data.json

# 2. `pr` mutation test (Part B), one corpus.
cargo xtask mutation --ferrograph /path/to/ferrograph \
    --work /tmp/fg-acc --corpus clap --n 30 \
    --out-md mutation-report.md --out-json mutation-data.json

# 2b. Same, with an option under test (the harness's model of `dead` follows --restricted-vis-not-root).
#     (`--bin-pub-not-root` is followed by the model too).
cargo xtask groundtruth --ferrograph /path/to/ferrograph --work /tmp/fg-acc \
    --extra-args=--restricted-vis-not-root
cargo xtask mutation --ferrograph /path/to/ferrograph --work /tmp/fg-acc --corpus clap \
    --extra-args=--restricted-vis-not-root

# 2c. Replay: false-alarm rate of `pr` on the last N (default 50) real commits of url corpora.
cargo xtask replay --ferrograph /path/to/ferrograph --work /tmp/fg-acc \
    --only clap,ripgrep --n 50 --extra-args=--restricted-vis-not-root

# 2d. Replay on commits that delete call sites (instead of the last N): the N most recent commits within the
#     last 10*N whose diff removes a line calling/referencing a fn defined in the repo.
cargo xtask replay --ferrograph /path/to/ferrograph --work /tmp/fg-acc \
    --only clap,ripgrep --n 50 --select calls --depth 600

# 2e. More mutation families (default is `call` only); --n applies per family.
cargo xtask mutation --ferrograph /path/to/ferrograph --work /tmp/fg-acc --corpus clap \
    --family all --n 15

# 2f. Paired comparison of two groundtruth runs (default vs an option): recall with Wilson intervals,
#     items found by only one arm (exact McNemar), extra reports by rustc verdict, sample size for a 5-point effect.
cargo xtask compare groundtruth-default.json groundtruth-variant.json

# 2f2. Precision of the `pr` impacted set by depth (spectrum-rune #47/#49/#50, ferrograph 6.1.0). In a worktree of each PR head:
#      ferrograph pr --base <base> --index-head --impact-depth {1,2,3} [--exclude-low-confidence] --blast-limit 100000 --json > pr<N>-d<D>[-x].json
#      (bases: 47 e4e4371, 49 488cdf5, 50 34c8e90). Then bucket, hand-label the `label` column (TP/FP), and score:
cargo xtask impact-sample bucket scripts/accuracy/impact 47 49 50 > labels.csv
cargo xtask impact-sample score scripts/accuracy/impact/labels.csv

# 2g. Find corpus candidates: shallow-clones HEAD, runs the oracle, prints rustc-dead fn counts (deletes the target dir).
cargo xtask scan --work /tmp/fg-scan owner/repo another/repo

# 2h. Whole matrix (all url corpora x arms), one corpus at a time with target dirs deleted afterwards:
cargo xtask run-matrix --ferrograph /path/to/ferrograph --work /tmp/fg-acc --out /tmp/fg-out \
    --arms default,restricted,binpub,both     # arms: default, restricted, binpub, both

# 3. Self-tests of the harness on the built-in synthetic corpus (seconds, no network).
cargo build && cargo test -p xtask   # uses $FERROGRAPH or target/{debug,release}/ferrograph
```

Use a released, checksum-verified binary for numbers you intend to quote
(`gh release download vX.Y.Z -p 'ferrograph-<target>.tar.gz' -p SHA256SUMS`, then `shasum -a 256 -c`).
Everything is written under `--work` (corpus copies, `CARGO_TARGET_DIR`s, graph databases) and the
scripts are idempotent: re-running reuses clones and target dirs. Audited repositories are never written
to: `path` corpora are copied (without `target/` and `.git`), `url` corpora are cloned into `--work`.
Every subprocess has a hard timeout (`--timeout` per cargo invocation).

Expected cost (Apple M-series, release binary): clap/ripgrep/fd/topgrade a few minutes each including
first builds, ~2-4 GB of `target/` in total for the supplementary set; a 17k-function workspace
(rune) indexes in ~2.5 min. Mutation test on clap: roughly 1 cargo check (~5-15 s) + 1 `pr` run per mutation.

## Differences from the earlier Python harness

The harness was ported from Python (`scripts/accuracy/*.py`, removed). `groundtruth`, `compare` and `impact-sample` produce the
same Markdown byte for byte and the same JSON values (compared on the synthetic corpus and on clap, default and
`--restricted-vis-not-root` arms, with the same release binary; the one exception is the order of the `text_refs` hints, below), so the numbers quoted in the README stay reproducible. Known differences: the `platform:` line of
the reports is `<os>-<arch>`; JSON files are pretty-printed with two spaces and sorted keys; `mutation` samples candidates with
`rand`'s `StdRng` seeded from `--seed`, so the same seed picks different candidates than the Python version did (no published
number depends on it); the `text_refs` lines in the JSON (hand-check aids) come in sorted file order instead of filesystem order.

## corpora.toml

`[[corpus]]` entries: `name`, `url` + `ref` (branch, tag or SHA) or `path`; optional `index_path`,
`group`, `cargo_args`. Pooling cohorts used so far: `original` (first nine), `held-out` (bat, hyperfine, zoxide, dust, eza, spectrum-rune) and `held-out-2`
(chosen by `cargo xtask scan` for >= 5 rustc-dead fns). `cargo xtask scan` sets `RUSTUP_TOOLCHAIN=stable` so a repo's `rust-toolchain` file cannot make rustup
download another toolchain; the benchmark scripts do not, so a corpus with a pinned toolchain builds with it.

The runner tries `cargo check --workspace --all-targets --all-features`, then without
`--all-features`, then without benches (`--lib --bins --tests --examples`, for crates whose benches need nightly),
and prints which variant ran.

Corpora with `heldout = true` were not used while tuning ferrograph; the reports pool them separately from the
others. `cohort = "<name>"` puts a corpus in its own pool (default: `original`, or `held-out` when `heldout = true`),
so later additions are never mixed with corpora that were already measured. Recall and precision carry Wilson 95% intervals (`[lo-hi]`).

## What is measured

**Ground truth** (`cargo xtask groundtruth`): `RUSTFLAGS="--force-warn dead_code" cargo check ... --message-format=json`.
`--force-warn` defeats in-source `#![allow(dead_code)]`. Diagnostics are kept when the primary span is a
`fn` inside the corpus root (dependencies dropped; structs, fields, variants, consts skipped). Because `--all-targets`
compiles a file once per unit (lib, bin, each test, example, ...), an item counts as dead (set **R**) only if it is
reported in **every** unit that compiles its file; unit membership comes from the `.d` dep-info files. Items dead
in only some units are counted separately ("dead in some unit only") and treated as live for rustc.

**Mapping**: ferrograph node ids are `./path#line:col` (start of the fn item). A rustc item maps to the node in the same
file with the same canonical name (`pub::`, `test::`, `bench::` prefixes stripped) whose start line is the greatest one
`<=` rustc's line within 10 lines. Unmapped items are reported (indexing misses).

**Metrics**, per corpus and pooled, for default and `--strict`, for lib vs bin crates (a file is `lib` if any non-test lib
unit compiles it, otherwise `bin`; test/example/bench units count as `bin`):

- *recall (all R)* = |F∩R| / |R|, F being ferrograph's dead set. *recall (eligible)* excludes R items that ferrograph treats
  as roots by policy (`pub` items, trait impls, test/bench/main, `#[no_mangle]` etc.), i.e. what it could find at all.
- *precision raw* = |F∩R| / |F|. *precision judgeable* removes F items rustc cannot judge: the file is compiled by no unit, or the
  fn is under a `#[cfg]` (text heuristic: own attributes, enclosing `mod`/`impl`/`trait`, or its `mod x;` declaration).
- False negatives (R minus F) are bucketed by a BFS over the graph that replicates `dead`'s reachability (the harness
  asserts its model equals the CLI output: "Self-check ... diff=0"). The bucket is how the item first became reachable:
  a `pub`/trait-impl/test/attribute root, a low-confidence call edge, a macro (`expands_to`) edge, or a call/reference edge
  from a root item or from non-root code; "collateral" means it inherits the cause of another rustc-dead node.
- False positives not in R: not judgeable (above), live in some unit, or "genuine disagreement" (the list to hand-check;
  the JSON carries source lines that mention each name).

**`pr` mutation test** (`cargo xtask mutation`): in a throwaway git repo, one commit per mutation, always starting from the pristine
base tree. Oracle: rustc's dead set after minus before. System under test: `ferrograph pr --base HEAD~1 --index-head --format json`.
Reports TP/FN/FP per mutation, per family and pooled, FN buckets (graph replay on the mutated tree), FP kinds, `pr` wall time,
and the `git worktree list` afterwards. `--family` selects families (default `call`; `all`); `--n` compiling mutations per family.
Edits blank or replace lines in place, so line numbers never shift; mutations that do not compile are skipped and counted.

| family | what is edited | picks |
|---|---|---|
| `call` | a single-line call statement `[recv.]f(args);` becomes `();` | uniquely named fn with exactly one high-confidence call edge |
| `ref` | a use site that is not a call: a `.adapter(f)` line or a table entry `f,` on its own line is blanked | uniquely named fn whose only incoming edge is one `references` edge |
| `impl` | a whole `impl Trait for T { .. }` block, with its attributes, is blanked | helper fn whose only caller is a method of that impl |
| `caller` | a whole fn (with attributes) that nothing calls or references is blanked | such a fn that is the only caller of some helper |

**Replay** (`cargo xtask replay`): for each of the last N first-parent commits that change a `.rs` file, check out the commit in its
own history-carrying clone, run `ferrograph pr --base <parent> --index-head`, and compare the newly-dead fns with rustc's
(dead-in-all-units at the commit minus at the parent; rustc answers are cached on disk per commit). Reports the
false-alarm commit rate (commits with an unconfirmed, judgeable report), report precision and recall. Commits that do
not build with the current toolchain are skipped, so the sample is biased toward buildable history. The table lists
how many commits had a rustc newly-dead fn at all (the real denominator for recall). With `--select calls` the commits are
those whose diff removes a line that calls or references a fn defined in the repo (`--depth` first-parent commits are scanned,
the first `--n` hits are measured); the default `--select last` keeps the last-N behaviour.

## UI-toolkit cohort (`ui`, #210)

Five toolkits (ratatui, egui, iced, xilem, cursive; see `corpora.toml`) whose code is closures in fields, message handlers, builders and
callbacks. `cargo xtask groundtruth --only ratatui,egui,iced,xilem,cursive`, ferrograph `main` after #263 (8.0.0 plus #243-#246, #251-#253,
#255-#257, #261-#263; measured on the head of #263), rustc 1.99.1, macOS arm64,
2026-11-17, `cargo check --workspace --all-targets --all-features` (cursive without `--all-features`). Wilson 95% intervals; percentages are
floored, as in the harness reports. "Genuine disagreements" are F minus the confirmed items minus the ones rustc cannot judge (cfg-gated, file
not compiled), so judgeable minus confirmed (21 - 5 - 10 = 6 in default mode; 99 - 52 - 11 = 36 under `--strict`).

| | R (rustc-dead) | F | F in R | recall | precision (judgeable) | genuine disagreements |
|---|---|---|---|---|---|---|
| default | 35 | 21 | 10 | 28.5% (10/35) [16.3-45.1] | 62.5% (10/16) [38.6-81.5] | 6 |
| `--strict` | 35 | 99 | 11 | 31.4% (11/35) [18.6-48.0] | 23.4% (11/47) [13.6-37.2] | 36 |

For comparison, the same cohort on ferrograph 6.2.1 (`fe9ef0d`, 2026-11-16): default F 54, precision 22.2% (10/45), 35 genuine disagreements; `--strict` F 157,
10.5% (11/104), 93 genuine. Recall is unchanged (R and F in R are the same): the fixes since then removed reports, not detections. The 7 older
corpora of the harness were measured with the same binary: recall 31.4% (17/54), unchanged.

Genuine disagreements per corpus (default): ratatui 0, egui 1, iced 2, xilem 1, cursive 2. R is small (these crates are CI-clean), so recall has wide
intervals, and the list is a candidate list, not a verdict. The 6 default ones were each read and traced to a cause:

- **iced `Content::diff`, `TitleBar::diff`:** called in a closure passed to `diff_children_custom`, whose parameter type comes from the callee's signature (#237).
- **cursive `CursiveRoot::layout`, `draw`:** called through `C: BorrowMut<Cursive>` in `CursiveRunner<C>` (#238).
- **egui `PassState::begin_pass`:** the receiver goes through a one-argument map alias defined in another file (`ViewportIdMap<V>`, #241).
- **xilem `AnyPod::as_mut`:** the receiver is the generic return of `VecSplice<T>::mutate` (#242).

Strict mode has 36 (ratatui 7, egui 22, iced 0, xilem 3, cursive 4); iced's 6 strict reports are `cfg`-gated `examples/todos` items (policy, not judgeable). The other strict items were not read.

How the 35 default ones went (6.2.1 to now): egui `assert_send_sync`, a `const fn` declared and called inside `const _: () = { .. };` (8, #218); cursive `find_closest_pair`
called as `super::find_closest_pair(..)` from a same-named wrapper in a child module (2, #219); iced `Content`/`TitleBar` methods called on a receiver bound by a closure or
zip/tuple pattern, because `zip` was not modelled and the lone same-file method of the same name was linked instead (11, #220/#236); iced `error_list` in
`#[error("..", error_list(.0))]` (#243); locals bound by struct patterns (`Self::Ready { controls, .. }`; #244), an `if` expression (#245) and an `IntMap` field (#246); the remaining unlooked-at items
(egui `id`/`ui`/`set_options`, iced `update`/`draw`) were instances of these. Strict-only fixes: `iter().enumerate()` (#251), `Self::Assoc` types (#252), inherent over trait methods (#253, #255),
`impl Iterator<Item = T>` parameters (#256), same-named types in several modules (#257).

**Harness self-check.** The harness's model of `dead` matches the CLI on all five corpora at the measured head (diff 0 in both modes). It did not at 8.0.0: egui was off by 8 (the model lacked the `const _: () = { .. };` root, #261) and cursive by 1 (`rgb`, the symlinked directory indexed twice, #263).

**Regression checks.** Mutation (clap, all families, 30 per family) and replay (clap and ripgrep, 50 commits that remove a call, `--select calls`), v8.0.0 against this `main`: replay identical
(0 false-alarm commits of 100; recall 1/12 on both); mutation sampled 19 mutations in common (the two runs differ in the rest) and gave identical results on them (14 TP, 7 FP, 3 FN).


**Default and strict report different sets.** `--strict` drops ambiguous call edges and also turns off `--restricted-vis-not-root` and `--bin-pub-not-root`, so `pub(crate)` items and `pub` items in bin/example files are entry points there. A fix can therefore move one arm and not the other: typing `zip` (#236) took iced default reports from 17 to 6 and left the strict row at 38, because the 11 items it removed (`pub(crate)` methods of `Content` and `TitleBar`) were never in the strict list (checked by diffing the node ids of the two releases). The strict list is mostly private functions. Strict false positives found this way (#240 follow-up, iced strict 33 to 16, ratatui strict 28 to 18): `iter().enumerate()` pairs, `Self::Assoc` types in a trait impl, and an inherent method shadowed by a same-named trait method. Left in iced strict: `cfg`-gated wasm/non-wasm code in `examples/todos` (policy items) and `downcast_mut::<T>()` receivers in `examples/loading_spinners`.

### `--lib-exported-only` on the same cohort (#207)

The harness's model of `dead` follows `--lib-exported-only` (`attr::pub_unexported`, a `Graph::prepare_model` flag; `run-matrix` has an arm `lib` = `both` plus the flag).
`cargo xtask groundtruth ... --extra-args=--lib-exported-only`, same binary, date and corpora as above (default mode, the model's flags otherwise on; the model's self-check applies here as well):

| | R | F | F in R | recall | precision (judgeable) | genuine disagreements |
|---|---|---|---|---|---|---|
| without the flag | 35 | 21 | 10 | 28.5% (10/35) [16.3-45.1] | 62.5% (10/16) [38.6-81.5] | 6 |
| `--lib-exported-only` | 35 | 27 | 14 | 40.0% (14/35) [25.6-56.4] | 70.0% (14/20) [48.1-85.5] | 6 |

The flag adds 6 reports, 4 of them confirmed by rustc (recall 10 to 14 of 35, all in library crates: 9 to 13 of 23). The other 2 are not judgeable (`#[cfg]` code), so the genuine disagreements stay at 6: the iced `whitespace` item that this section used to list as the one new false positive is no longer reported (not traced to a single fix). Small sample, wide intervals; the intervals of the two recall rows overlap, so this shows the direction, not a significant gain. Not measured: the 22 older corpora (run `run-matrix --arms both,lib` to compare), and methods (#217).

Reproduce: `cargo xtask groundtruth --ferrograph <binary> --work <dir> --only ratatui,egui,iced,xilem,cursive --out-md ui.md --out-json ui.json`
(about 5 minutes with warm clones; each crate needs 1-3 GB of `target/` while it runs).

## What is not measured / limitations

- `RUSTFLAGS` is set to `--force-warn dead_code`, which makes cargo ignore `build.rustflags`/`target.*.rustflags` from a corpus's `.cargo/config.toml`. In the pinned corpora those are Windows-only (fd, ripgrep), so nothing changes on macOS/Linux; check before adding a corpus that needs e.g. `--cfg tokio_unstable`.
- rustc's view depends on cfg, features, target and platform; ferrograph is cfg-agnostic. One platform, one feature set per run.
  Disagreements caused by that appear as "not judgeable (cfg)" or "dead in some unit only", not as errors.
- Trait-impl methods and `pub` items of library crates are invisible to rustc, so neither recall nor precision says anything
  about them. Only functions/methods/associated functions are compared.
- Well-maintained crates contain almost no dead code (CI usually denies warnings), so natural ground truth is small and
  recall figures have wide error bars; the mutation test creates dead code on purpose. The supplementary corpora were chosen
  because rustc reports dead code in them, which biases toward crates with dead code.
- The cfg heuristic is textual; hand-check the "genuine disagreement" list before quoting precision as a defect rate.
- Mutation families are synthetic. `ref` only sees fns with a single `references` edge, which the indexer creates mostly for
  attribute arguments and a few path positions, so many corpora yield no candidates; `impl` and `caller` blank blocks found by a
  naive brace count (strings and comments are stripped, raw strings and macros are not), and most candidates do not compile after
  the deletion (an impl is usually needed by something) and are skipped, so the surviving sample is biased toward impls that
  nothing uses (e.g. `Display`). None of the families covers signature changes, moved code or renames.
- Replay `--select calls` keys on removed text (a removed line that mentions a fn defined at the replayed HEAD), not on the call
  graph, so it overselects (deleted comments aside, renamed callees and moved lines count); it only raises the share of commits that
  can orphan something. Read the `commits with rustc newly dead` column for the real denominator.

Note: since 5.0 `--bin-pub-not-root` is on by default. The harness's model of `dead` (`xtask/src/graph.rs`) follows it, and the `default` and `restricted` arms of `run-matrix` pass `--no-bin-pub-not-root` to keep measuring the old baseline; with a pre-5.0 binary that flag is unknown, so drop it there.
