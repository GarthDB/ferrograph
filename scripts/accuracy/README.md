# Accuracy harness: `dead` and `pr` against compiler ground truth

Measures **recall** and **precision** of `ferrograph dead` (default and `--strict`) and the
"newly dead" part of `ferrograph pr` using rustc's `dead_code` lint as the oracle instead of
hand classification. Not run in CI (it needs external corpora and minutes of compile time).
Only `test_harness.py`, which uses the built-in synthetic corpus, is meant for CI.

Python 3.11+ (uses `tomllib`), standard library only. Needs `cargo`, `git` and a ferrograph binary.

## Run

```bash
# 1. Static ground truth (Part A). Corpora come from corpora.toml.
python3 scripts/accuracy/groundtruth.py --ferrograph /path/to/ferrograph \
    --work /tmp/fg-acc --only clap,ripgrep,fd \
    --out-md groundtruth-report.md --out-json groundtruth-data.json

# 2. `pr` mutation test (Part B), one corpus.
python3 scripts/accuracy/mutation.py --ferrograph /path/to/ferrograph \
    --work /tmp/fg-acc --corpus clap --n 30 \
    --out-md mutation-report.md --out-json mutation-data.json

# 2b. Same, with an option under test (the python model of `dead` follows --restricted-vis-not-root).
#     (`--bin-pub-not-root` is followed by the model too). Use the `=` form: argparse reads a bare `--extra-args --flag` as two options.
python3 scripts/accuracy/groundtruth.py --ferrograph /path/to/ferrograph --work /tmp/fg-acc \
    --extra-args=--restricted-vis-not-root
python3 scripts/accuracy/mutation.py --ferrograph /path/to/ferrograph --work /tmp/fg-acc --corpus clap \
    --extra-args=--restricted-vis-not-root

# 2c. Replay: false-alarm rate of `pr` on the last N (default 50) real commits of url corpora.
python3 scripts/accuracy/replay.py --ferrograph /path/to/ferrograph --work /tmp/fg-acc \
    --only clap,ripgrep --n 50 --extra-args=--restricted-vis-not-root

# 2d. Replay on commits that delete call sites (instead of the last N): the N most recent commits within the
#     last 10*N whose diff removes a line calling/referencing a fn defined in the repo.
python3 scripts/accuracy/replay.py --ferrograph /path/to/ferrograph --work /tmp/fg-acc \
    --only clap,ripgrep --n 50 --select calls --depth 600

# 2e. More mutation families (default is `call` only); --n applies per family.
python3 scripts/accuracy/mutation.py --ferrograph /path/to/ferrograph --work /tmp/fg-acc --corpus clap \
    --family all --n 15

# 2f. Paired comparison of two groundtruth runs (default vs an option): recall with Wilson intervals,
#     items found by only one arm (exact McNemar), extra reports by rustc verdict, sample size for a 5-point effect.
python3 scripts/accuracy/compare.py groundtruth-default.json groundtruth-variant.json

# 2g. Find corpus candidates: shallow-clones HEAD, runs the oracle, prints rustc-dead fn counts (deletes the target dir).
python3 scripts/accuracy/scan.py --work /tmp/fg-scan owner/repo another/repo

# 2h. Whole matrix (all url corpora x arms), one corpus at a time with target dirs deleted afterwards:
python3 scripts/accuracy/run_matrix.py --ferrograph /path/to/ferrograph --work /tmp/fg-acc --out /tmp/fg-out \
    --arms default,restricted,binpub,both     # arms: default, restricted, binpub, both

# 3. Self-test of the harness on the built-in synthetic corpus (~2 s, no network).
python3 scripts/accuracy/test_harness.py       # uses $FERROGRAPH or target/{debug,release}/ferrograph
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

## corpora.toml

`[[corpus]]` entries: `name`, `url` + `ref` (branch, tag or SHA) or `path`; optional `index_path`,
`group`, `cargo_args`. Pooling cohorts used so far: `original` (first nine), `held-out` (bat, hyperfine, zoxide, dust, eza, spectrum-rune) and `held-out-2`
(chosen by `scan.py` for >= 5 rustc-dead fns). `scan.py` sets `RUSTUP_TOOLCHAIN=stable` so a repo's `rust-toolchain` file cannot make rustup
download another toolchain; the benchmark scripts do not, so a corpus with a pinned toolchain builds with it.

The runner tries `cargo check --workspace --all-targets --all-features`, then without
`--all-features`, then without benches (`--lib --bins --tests --examples`, for crates whose benches need nightly),
and prints which variant ran.

Corpora with `heldout = true` were not used while tuning ferrograph; the reports pool them separately from the
others. `cohort = "<name>"` puts a corpus in its own pool (default: `original`, or `held-out` when `heldout = true`),
so later additions are never mixed with corpora that were already measured. Recall and precision carry Wilson 95% intervals (`[lo-hi]`).

## What is measured

**Ground truth** (`groundtruth.py`): `RUSTFLAGS="--force-warn dead_code" cargo check ... --message-format=json`.
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
- False negatives (R minus F) are bucketed by a BFS over the graph that replicates `dead`'s reachability (the script
  asserts its model equals the CLI output: "Self-check ... diff=0"). The bucket is how the item first became reachable:
  a `pub`/trait-impl/test/attribute root, a low-confidence call edge, a macro (`expands_to`) edge, or a call/reference edge
  from a root item or from non-root code; "collateral" means it inherits the cause of another rustc-dead node.
- False positives not in R: not judgeable (above), live in some unit, or "genuine disagreement" (the list to hand-check;
  the JSON carries source lines that mention each name).

**`pr` mutation test** (`mutation.py`): in a throwaway git repo, one commit per mutation, always starting from the pristine
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

**Replay** (`replay.py`): for each of the last N first-parent commits that change a `.rs` file, check out the commit in its
own history-carrying clone, run `ferrograph pr --base <parent> --index-head`, and compare the newly-dead fns with rustc's
(dead-in-all-units at the commit minus at the parent; rustc answers are cached on disk per commit). Reports the
false-alarm commit rate (commits with an unconfirmed, judgeable report), report precision and recall. Commits that do
not build with the current toolchain are skipped, so the sample is biased toward buildable history. The table lists
how many commits had a rustc newly-dead fn at all (the real denominator for recall). With `--select calls` the commits are
those whose diff removes a line that calls or references a fn defined in the repo (`--depth` first-parent commits are scanned,
the first `--n` hits are measured); the default `--select last` keeps the last-N behaviour.

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
