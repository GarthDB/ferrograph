# ferrograph `dead` vs rustc `dead_code` ground truth

- ferrograph: `v6.1.1` (`../../target/debug/ferrograph`)
- extra `dead` args for every row: `--restricted-vis-not-root`
- rustc: `rustc 1.99.1 (58cf8f9c3 2026-10-13)`; platform: `macOS-27.0.1-arm64-arm-64bit-Mach-O`; date: 2026-11-15
- ground truth: functions/methods/associated functions that rustc reports as `dead_code` in **every** compilation unit that includes their file (lib, bin, test, example, bench units of `--all-targets`), with `--force-warn dead_code`; dependencies dropped.

## Corpora

| corpus | source | commit | fn nodes | rustc fn items (all units) | R mapped | R unmapped | dead in some unit only | cargo s | index s |
|---|---|---|---|---|---|---|---|---|---|
| synthetic | scripts/accuracy/synthetic | `0123456789ab` (dirty tree) | 26 | 8 | 8 | 0 | 0 | 0.3 | 0.1 |
| second | https://example.com/x/y | `0123456789ab` | 26 | 8 | 8 | 3 | 0 | 12.0 | 0.25 |
| broken | https://example.com/broken | `?` | — | — | — | — | — | — | — |
| third | https://example.com/x/y | `0123456789ab` | 26 | 8 | 8 | 3 | 0 | 12.0 | 0.25 |

### synthetic

`RUSTFLAGS="--force-warn dead_code" cargo check --workspace --all-targets --all-features --message-format=json`

Self-check (model of `dead` vs CLI): default: cli=5 model=5 diff=0, strict: cli=7 model=7 diff=0

### second

`RUSTFLAGS="--force-warn dead_code" cargo check --workspace --all-targets --all-features --message-format=json`  
(**fallback used**: default features (--all-features failed))

Self-check (model of `dead` vs CLI): default: cli=5 model=5 diff=0, strict: cli=7 model=7 diff=0

Unmapped rustc items (indexing misses): no fn node with that name within 10 lines: 2; file has no indexed fn nodes: 1

### broken

`cargo check`

**Not measured**: cargo check failed: boom

### third

`RUSTFLAGS="--force-warn dead_code" cargo check --workspace --all-targets --all-features --message-format=json`

Self-check (model of `dead` vs CLI): default: cli=5 model=5 diff=0, strict: cli=7 model=7 diff=0

Unmapped rustc items (indexing misses): no fn node with that name within 10 lines: 2; file has no indexed fn nodes: 1

## Recall and precision

R = rustc-dead fns mapped to nodes; F = ferrograph dead set. **Recall (all R)** = |F∩R|/|R|. **Recall (eligible)** = |F∩R| / |R minus items ferrograph treats as roots by policy (`pub`, trait impls, tests, attributes)|. **Precision raw** = |F∩R|/|F|. **Precision judgeable** = |F∩R| / |F minus items rustc cannot judge (file never compiled, or under `#[cfg]`)|. Percentages are floored; counts given; brackets are Wilson 95% intervals. Cohorts (`original`, `held-out`, and any `cohort =` set in corpora.toml) are pooled separately; held-out ones were not used while tuning.

| corpus | class | mode | R | R eligible | F | F∩R | recall (all R) | recall (eligible) | precision raw | F not judgeable | F judgeable | precision judgeable |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| synthetic | lib | default | 1 | 1 | 1 | 1 | 100.0% (1/1) [20.7-100.0] | 100.0% (1/1) | 100.0% (1/1) [20.7-100.0] | 0 | 1 | 100.0% (1/1) [20.7-100.0] |
| synthetic | lib | strict | 1 | 1 | 1 | 1 | 100.0% (1/1) [20.7-100.0] | 100.0% (1/1) | 100.0% (1/1) [20.7-100.0] | 0 | 1 | 100.0% (1/1) [20.7-100.0] |
| synthetic | bin | default | 7 | 5 | 4 | 4 | 57.1% (4/7) [25.0-84.2] | 80.0% (4/5) | 100.0% (4/4) [51.0-100.0] | 0 | 4 | 100.0% (4/4) [51.0-100.0] |
| synthetic | bin | strict | 7 | 5 | 6 | 5 | 71.4% (5/7) [35.9-91.8] | 100.0% (5/5) | 83.3% (5/6) [43.6-97.0] | 0 | 6 | 83.3% (5/6) [43.6-97.0] |
| synthetic | all | default | 8 | 6 | 5 | 5 | 62.5% (5/8) [30.6-86.3] | 83.3% (5/6) | 100.0% (5/5) [56.6-100.0] | 0 | 5 | 100.0% (5/5) [56.6-100.0] |
| synthetic | all | strict | 8 | 6 | 7 | 6 | 75.0% (6/8) [40.9-92.9] | 100.0% (6/6) | 85.7% (6/7) [48.7-97.4] | 0 | 7 | 85.7% (6/7) [48.7-97.4] |
| second (held-out) | lib | default | 9 | 5 | 4 | 1 | 11.1% (1/9) [2.0-43.5] | 20.0% (1/5) | 25.0% (1/4) [4.6-69.9] | 1 | 3 | 33.3% (1/3) [6.1-79.2] |
| second (held-out) | lib | strict | 9 | 5 | 5 | 1 | 11.1% (1/9) [2.0-43.5] | 20.0% (1/5) | 20.0% (1/5) [3.6-62.4] | 0 | 5 | 20.0% (1/5) [3.6-62.4] |
| second (held-out) | bin | default | 8 | 5 | 6 | 4 | 50.0% (4/8) [21.5-78.5] | 80.0% (4/5) | 66.6% (4/6) [30.0-90.3] | 1 | 5 | 80.0% (4/5) [37.6-96.4] |
| second (held-out) | bin | strict | 8 | 5 | 6 | 4 | 50.0% (4/8) [21.5-78.5] | 80.0% (4/5) | 66.6% (4/6) [30.0-90.3] | 1 | 5 | 80.0% (4/5) [37.6-96.4] |
| second (held-out) | all | default | 17 | 10 | 10 | 6 | 35.2% (6/17) [17.3-58.7] | 60.0% (6/10) | 60.0% (6/10) [31.3-83.2] | 2 | 8 | 75.0% (6/8) [40.9-92.9] |
| second (held-out) | all | strict | 17 | 10 | 11 | 6 | 35.2% (6/17) [17.3-58.7] | 60.0% (6/10) | 54.5% (6/11) [28.0-78.7] | 1 | 10 | 60.0% (6/10) [31.3-83.2] |
| third (held-out-2) | lib | default | 9 | 5 | 4 | 1 | 11.1% (1/9) [2.0-43.5] | 20.0% (1/5) | 25.0% (1/4) [4.6-69.9] | 1 | 3 | 33.3% (1/3) [6.1-79.2] |
| third (held-out-2) | lib | strict | 9 | 5 | 5 | 1 | 11.1% (1/9) [2.0-43.5] | 20.0% (1/5) | 20.0% (1/5) [3.6-62.4] | 0 | 5 | 20.0% (1/5) [3.6-62.4] |
| third (held-out-2) | bin | default | 8 | 5 | 6 | 4 | 50.0% (4/8) [21.5-78.5] | 80.0% (4/5) | 66.6% (4/6) [30.0-90.3] | 1 | 5 | 80.0% (4/5) [37.6-96.4] |
| third (held-out-2) | bin | strict | 8 | 5 | 6 | 4 | 50.0% (4/8) [21.5-78.5] | 80.0% (4/5) | 66.6% (4/6) [30.0-90.3] | 1 | 5 | 80.0% (4/5) [37.6-96.4] |
| third (held-out-2) | all | default | 17 | 10 | 10 | 6 | 35.2% (6/17) [17.3-58.7] | 60.0% (6/10) | 60.0% (6/10) [31.3-83.2] | 2 | 8 | 75.0% (6/8) [40.9-92.9] |
| third (held-out-2) | all | strict | 17 | 10 | 11 | 6 | 35.2% (6/17) [17.3-58.7] | 60.0% (6/10) | 54.5% (6/11) [28.0-78.7] | 1 | 10 | 60.0% (6/10) [31.3-83.2] |
| **pooled (original)** | lib | default | 1 | 1 | 1 | 1 | 100.0% (1/1) [20.7-100.0] | 100.0% (1/1) | 100.0% (1/1) [20.7-100.0] | 0 | 1 | 100.0% (1/1) [20.7-100.0] |
| **pooled (original)** | lib | strict | 1 | 1 | 1 | 1 | 100.0% (1/1) [20.7-100.0] | 100.0% (1/1) | 100.0% (1/1) [20.7-100.0] | 0 | 1 | 100.0% (1/1) [20.7-100.0] |
| **pooled (original)** | bin | default | 7 | 5 | 4 | 4 | 57.1% (4/7) [25.0-84.2] | 80.0% (4/5) | 100.0% (4/4) [51.0-100.0] | 0 | 4 | 100.0% (4/4) [51.0-100.0] |
| **pooled (original)** | bin | strict | 7 | 5 | 6 | 5 | 71.4% (5/7) [35.9-91.8] | 100.0% (5/5) | 83.3% (5/6) [43.6-97.0] | 0 | 6 | 83.3% (5/6) [43.6-97.0] |
| **pooled (original)** | all | default | 8 | 6 | 5 | 5 | 62.5% (5/8) [30.6-86.3] | 83.3% (5/6) | 100.0% (5/5) [56.6-100.0] | 0 | 5 | 100.0% (5/5) [56.6-100.0] |
| **pooled (original)** | all | strict | 8 | 6 | 7 | 6 | 75.0% (6/8) [40.9-92.9] | 100.0% (6/6) | 85.7% (6/7) [48.7-97.4] | 0 | 7 | 85.7% (6/7) [48.7-97.4] |
| **pooled (held-out)** | lib | default | 9 | 5 | 4 | 1 | 11.1% (1/9) [2.0-43.5] | 20.0% (1/5) | 25.0% (1/4) [4.6-69.9] | 1 | 3 | 33.3% (1/3) [6.1-79.2] |
| **pooled (held-out)** | lib | strict | 9 | 5 | 5 | 1 | 11.1% (1/9) [2.0-43.5] | 20.0% (1/5) | 20.0% (1/5) [3.6-62.4] | 0 | 5 | 20.0% (1/5) [3.6-62.4] |
| **pooled (held-out)** | bin | default | 8 | 5 | 6 | 4 | 50.0% (4/8) [21.5-78.5] | 80.0% (4/5) | 66.6% (4/6) [30.0-90.3] | 1 | 5 | 80.0% (4/5) [37.6-96.4] |
| **pooled (held-out)** | bin | strict | 8 | 5 | 6 | 4 | 50.0% (4/8) [21.5-78.5] | 80.0% (4/5) | 66.6% (4/6) [30.0-90.3] | 1 | 5 | 80.0% (4/5) [37.6-96.4] |
| **pooled (held-out)** | all | default | 17 | 10 | 10 | 6 | 35.2% (6/17) [17.3-58.7] | 60.0% (6/10) | 60.0% (6/10) [31.3-83.2] | 2 | 8 | 75.0% (6/8) [40.9-92.9] |
| **pooled (held-out)** | all | strict | 17 | 10 | 11 | 6 | 35.2% (6/17) [17.3-58.7] | 60.0% (6/10) | 54.5% (6/11) [28.0-78.7] | 1 | 10 | 60.0% (6/10) [31.3-83.2] |
| **pooled (held-out-2)** | lib | default | 9 | 5 | 4 | 1 | 11.1% (1/9) [2.0-43.5] | 20.0% (1/5) | 25.0% (1/4) [4.6-69.9] | 1 | 3 | 33.3% (1/3) [6.1-79.2] |
| **pooled (held-out-2)** | lib | strict | 9 | 5 | 5 | 1 | 11.1% (1/9) [2.0-43.5] | 20.0% (1/5) | 20.0% (1/5) [3.6-62.4] | 0 | 5 | 20.0% (1/5) [3.6-62.4] |
| **pooled (held-out-2)** | bin | default | 8 | 5 | 6 | 4 | 50.0% (4/8) [21.5-78.5] | 80.0% (4/5) | 66.6% (4/6) [30.0-90.3] | 1 | 5 | 80.0% (4/5) [37.6-96.4] |
| **pooled (held-out-2)** | bin | strict | 8 | 5 | 6 | 4 | 50.0% (4/8) [21.5-78.5] | 80.0% (4/5) | 66.6% (4/6) [30.0-90.3] | 1 | 5 | 80.0% (4/5) [37.6-96.4] |
| **pooled (held-out-2)** | all | default | 17 | 10 | 10 | 6 | 35.2% (6/17) [17.3-58.7] | 60.0% (6/10) | 60.0% (6/10) [31.3-83.2] | 2 | 8 | 75.0% (6/8) [40.9-92.9] |
| **pooled (held-out-2)** | all | strict | 17 | 10 | 11 | 6 | 35.2% (6/17) [17.3-58.7] | 60.0% (6/10) | 54.5% (6/11) [28.0-78.7] | 1 | 10 | 60.0% (6/10) [31.3-83.2] |

## False negatives (rustc-dead, ferrograph-live): why they are kept alive

Cause = how the item first became reachable in a BFS from ferrograph's roots (strict edges first). *collateral* = reached through another rustc-dead node, so it inherits that node's cause.

### default, lib crates (16 missed)

| bucket | count | of which collateral |
|---|---|---|
| calls edge from root item | 8 | 2 |
| pub root: pub | 4 | 0 |
| other | 2 | 0 |
| low-confidence call edge | 2 | 2 |

- **calls edge from root item**: `./m.rs:3` `m3`: ./m.rs#3:1 (calls) ← ./m.rs#4:1 (calls) ← ./m.rs#5:1 (calls) ← ./m.rs#6:1 (calls)
- **calls edge from root item**: `./m.rs:5` `m5`: ./m.rs#5:1 (calls) ← ./m.rs#6:1 (calls) ← ./m.rs#7:1 (calls) ← ./m.rs#8:1 (calls)
- **calls edge from root item**: `./m.rs:6` `m6`: ./m.rs#6:1 (calls) ← ./m.rs#7:1 (calls) ← ./m.rs#8:1 (calls) ← ./m.rs#9:1 (calls)
- **pub root: pub**: `./m.rs:2` `m2`: ./m.rs#2:1 (calls) ← ./m.rs#3:1 (calls) ← ./m.rs#4:1 (calls) ← ./m.rs#5:1 (calls)
- **pub root: pub**: `./m.rs:4` `m4`: ./m.rs#4:1 (calls) ← ./m.rs#5:1 (calls) ← ./m.rs#6:1 (calls) ← ./m.rs#7:1 (calls)
- **pub root: pub**: `./m.rs:2` `m2`: ./m.rs#2:1 (calls) ← ./m.rs#3:1 (calls) ← ./m.rs#4:1 (calls) ← ./m.rs#5:1 (calls)
- **other**: `./m.rs:7` `m7`: ./m.rs#7:1 (calls) ← ./m.rs#8:1 (calls) ← ./m.rs#9:1 (calls) ← ./m.rs#10:1 (calls)
- **other**: `./m.rs:7` `m7`: ./m.rs#7:1 (calls) ← ./m.rs#8:1 (calls) ← ./m.rs#9:1 (calls) ← ./m.rs#10:1 (calls)
- **low-confidence call edge**: `./m.rs:8` `m8`: ./m.rs#8:1 (calls) ← ./m.rs#9:1 (calls) ← ./m.rs#10:1 (calls) ← ./m.rs#11:1 (calls)
- **low-confidence call edge**: `./m.rs:8` `m8`: ./m.rs#8:1 (calls) ← ./m.rs#9:1 (calls) ← ./m.rs#10:1 (calls) ← ./m.rs#11:1 (calls)

### default, bin crates (5 missed)

| bucket | count | of which collateral |
|---|---|---|
| low-confidence call edge | 3 | 0 |
| pub root: pub | 1 | 0 |
| pub root: pub(crate)/pub(super) | 1 | 0 |

- **low-confidence call edge**: `./bin_app/src/main.rs:16` `run`: ./bin_app/src/main.rs#16:5 (calls(low)) ← ./bin_app/src/main.rs#73:1 (root)
- **low-confidence call edge**: `./m.rs:9` `m9`: ./m.rs#9:1 (calls) ← ./m.rs#10:1 (calls) ← ./m.rs#11:1 (calls) ← ./m.rs#12:1 (calls)
- **low-confidence call edge**: `./m.rs:9` `m9`: ./m.rs#9:1 (calls) ← ./m.rs#10:1 (calls) ← ./m.rs#11:1 (calls) ← ./m.rs#12:1 (calls)
- **pub root: pub**: `./bin_app/src/util.rs:5` `pub_dead`: ./bin_app/src/util.rs#5:1 (root)
- **pub root: pub(crate)/pub(super)**: `./bin_app/src/util.rs:9` `crate_dead`: ./bin_app/src/util.rs#9:1 (root)

### strict, lib crates (16 missed)

| bucket | count | of which collateral |
|---|---|---|
| calls edge from root item | 8 | 2 |
| pub root: pub | 4 | 0 |
| other | 2 | 0 |
| low-confidence call edge | 2 | 2 |

- **calls edge from root item**: `./m.rs:3` `m3`: ./m.rs#3:1 (calls) ← ./m.rs#4:1 (calls) ← ./m.rs#5:1 (calls) ← ./m.rs#6:1 (calls)
- **calls edge from root item**: `./m.rs:5` `m5`: ./m.rs#5:1 (calls) ← ./m.rs#6:1 (calls) ← ./m.rs#7:1 (calls) ← ./m.rs#8:1 (calls)
- **calls edge from root item**: `./m.rs:6` `m6`: ./m.rs#6:1 (calls) ← ./m.rs#7:1 (calls) ← ./m.rs#8:1 (calls) ← ./m.rs#9:1 (calls)
- **pub root: pub**: `./m.rs:2` `m2`: ./m.rs#2:1 (calls) ← ./m.rs#3:1 (calls) ← ./m.rs#4:1 (calls) ← ./m.rs#5:1 (calls)
- **pub root: pub**: `./m.rs:4` `m4`: ./m.rs#4:1 (calls) ← ./m.rs#5:1 (calls) ← ./m.rs#6:1 (calls) ← ./m.rs#7:1 (calls)
- **pub root: pub**: `./m.rs:2` `m2`: ./m.rs#2:1 (calls) ← ./m.rs#3:1 (calls) ← ./m.rs#4:1 (calls) ← ./m.rs#5:1 (calls)
- **other**: `./m.rs:7` `m7`: ./m.rs#7:1 (calls) ← ./m.rs#8:1 (calls) ← ./m.rs#9:1 (calls) ← ./m.rs#10:1 (calls)
- **other**: `./m.rs:7` `m7`: ./m.rs#7:1 (calls) ← ./m.rs#8:1 (calls) ← ./m.rs#9:1 (calls) ← ./m.rs#10:1 (calls)
- **low-confidence call edge**: `./m.rs:8` `m8`: ./m.rs#8:1 (calls) ← ./m.rs#9:1 (calls) ← ./m.rs#10:1 (calls) ← ./m.rs#11:1 (calls)
- **low-confidence call edge**: `./m.rs:8` `m8`: ./m.rs#8:1 (calls) ← ./m.rs#9:1 (calls) ← ./m.rs#10:1 (calls) ← ./m.rs#11:1 (calls)

### strict, bin crates (4 missed)

| bucket | count | of which collateral |
|---|---|---|
| low-confidence call edge | 2 | 0 |
| pub root: pub | 1 | 0 |
| pub root: pub(crate)/pub(super) | 1 | 0 |

- **low-confidence call edge**: `./m.rs:9` `m9`: ./m.rs#9:1 (calls) ← ./m.rs#10:1 (calls) ← ./m.rs#11:1 (calls) ← ./m.rs#12:1 (calls)
- **low-confidence call edge**: `./m.rs:9` `m9`: ./m.rs#9:1 (calls) ← ./m.rs#10:1 (calls) ← ./m.rs#11:1 (calls) ← ./m.rs#12:1 (calls)
- **pub root: pub**: `./bin_app/src/util.rs:5` `pub_dead`: ./bin_app/src/util.rs#5:1 (root)
- **pub root: pub(crate)/pub(super)**: `./bin_app/src/util.rs:9` `crate_dead`: ./bin_app/src/util.rs#9:1 (root)

## False positives: ferrograph-dead, rustc-not-dead

| mode | corpus | F | policy: file not compiled | policy: cfg-gated | rustc-live in some unit only | genuine disagreement | of which only dead under `--strict` |
|---|---|---|---|---|---|---|---|
| default | synthetic | 5 | 0 | 0 | 0 | 0 | - |
| default | second | 10 | 1 | 1 | 1 | 1 | - |
| default | third | 10 | 1 | 1 | 1 | 1 | - |
| strict | synthetic | 7 | 0 | 0 | 0 | 1 | 1 |
| strict | second | 11 | 0 | 1 | 0 | 2 | 1 |
| strict | third | 11 | 0 | 1 | 0 | 2 | 1 |

Genuine disagreements (default mode; text references for hand-checking are in the JSON, first 25 per corpus):

- second `./m.rs:53` `f53`
- third `./m.rs:53` `f53`
