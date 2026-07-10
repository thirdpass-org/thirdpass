# cargo-vet Coverage in the Top 100 Rust Crates

Published: 2026-06-25

This is a GitHub-readable mirror of the canonical website post:
<https://thirdpass.dev/blog/cargo-vet-popular-dependency-coverage>.

In the previous
[`cargo-vet` long-tail coverage post](cargo-vet-weak-long-tail-coverage.md),
we compared public `cargo-vet` registry data with crates.io as a whole. Coverage
across the full crates.io long tail was low.

This follow-up asks a narrower question: what happens for the crates that are
downloaded most often?

We selected the 100 crates with the highest all-time download counts in the
crates.io dump. For each crate, we took the current default version from the
dump.

For dependency coverage, we resolved the Linux dependency graph with the root
crate's default features and every other root feature enabled. Cargo selected
concrete dependency versions from the root crate's version requirements. We did
not replace dependencies with the latest available version of each dependency
crate.

The analysis has two parts. First, we checked whether the selected crate version
itself was covered. Then, for crates that had resolved dependencies, we checked
those dependency versions separately.

Headline results:

| Measure                                                            | Value     |
| ------------------------------------------------------------------ | --------- |
| Selected root crate versions covered                               | 73 of 100 |
| Crates with dependencies that had an uncovered dependency version   | 43 of 67  |

## Root Crate Coverage

Here, the root crate is the selected crate version from the top-100 list. This
section counts only those selected crate versions. The rows split them by
whether Cargo also found dependencies for the separate dependency coverage
analysis.

| Root crate group                           | Total | Covered | Not covered |
| ------------------------------------------ | ----- | ------- | ----------- |
| All selected root crate versions           | 100   | 73      | 27          |
| Selected root crates that had dependencies | 67    | 46      | 21          |
| Selected root crates without dependencies  | 33    | 27      | 6           |

Root crate coverage came through these cargo-vet paths:

| Coverage path          | Root crate versions |
| ---------------------- | ------------------- |
| Delta audit            | 38                  |
| Trusted publisher rule | 28                  |
| Direct version audit   | 6                   |
| Wildcard audit         | 1                   |
| No matched coverage    | 27                  |

## Dependency Coverage

33 of the 100 crates had no resolved dependencies under these settings.
The dependency analysis excludes those crates.

The canonical website post includes ranked charts for dependency coverage and
absolute uncovered dependency-version counts across the 67 crates with
dependencies. This GitHub mirror keeps the tabular results below.

## Dependency Counts

The table below counts dependency crate-version pairs. It counts a dependency
version once, even if it appeared under more than one selected crate.

| Measure                                           | Value  |
| ------------------------------------------------- | ------ |
| Crates with at least one resolved dependency      | 67     |
| Crates with no resolved dependencies              | 33     |
| Crates with an uncovered dependency version       | 43     |
| Crates with all dependency versions covered       | 24     |
| Unique dependency versions, excluding root crates | 281    |
| Covered dependency versions                       | 145    |
| Dependency-version coverage                       | 51.60% |

## Dependency Version Age

The resolved dependency versions were not all new. The median dependency
version age was 189.9 days at analysis time.

Fresh releases account for some missing coverage, but not all of it. Of the 136
uncovered dependency versions, 73 were at least 90 days old, 56 were at least
180 days old, and 36 were at least one year old.

The youngest resolved dependency version was `chacha20` `0.10.1`, published 1.9
days before the analysis run. It was not covered. That does not make the version
suspicious, but it shows that normal resolution can include very recent
dependency versions before cargo-vet coverage exists.

## Dependency Gaps

Some popular crates had only a small number of uncovered dependencies. Others
had larger gaps.

Selected rows with the largest uncovered dependency counts:

| Root crate              | Covered dependencies | Uncovered dependencies |
| ----------------------- | -------------------- | ---------------------- |
| `rustls` `0.23.40`      | 23 of 49             | 26                     |
| `idna` `1.1.0`          | 8 of 28              | 20                     |
| `url` `2.5.8`           | 14 of 34             | 20                     |
| `chrono` `0.4.45`       | 21 of 39             | 18                     |
| `generic-array` `1.4.3` | 10 of 27             | 17                     |
| `log` `0.4.32`          | 5 of 17              | 12                     |

These are not obscure crates in the download data. They are common
infrastructure crates that appear frequently as transitive dependencies.

## Selection and Resolution

The ranking used the crates.io dump from 2026-06-17. Download counts came from
the all-time crate download totals in `crate_downloads.csv`.

This is a download-ranked sample. The top 100 are dominated by infrastructure
crates such as `syn`, `hashbrown`, `getrandom`, `bitflags`, `rand_core`, `rand`,
and `libc`.

That matters for interpretation. Many top-download crates are small, and 33 had
no resolved dependencies under these settings. Root crate coverage and
dependency coverage are counted separately above.

The selected root crate version was the current default version in the dump. For
dependency coverage, Cargo resolved dependencies for `x86_64-unknown-linux-gnu`
with all root crate features enabled. The dependency versions are the concrete
versions Cargo selected for that root crate and feature set.

## Coverage Definition

A crate version counted as covered when one of the public cargo-vet registry
sources matched that exact version under `safe-to-deploy` or `safe-to-run`
through one of these paths:

- a direct version audit
- a delta audit ending at that version
- a trusted-publisher rule
- a wildcard audit

With a wildcard audit, the auditor is vouching that releases of that crate by a
specified publisher, within a specified date range, satisfy the stated
criteria.

"Covered" does not mean the crate is secure. It means the crate version matched
one of the cargo-vet coverage paths in this analysis.

## Sources

The analysis imported `audits.toml` data from the cargo-vet registry.

| Registry property | Value                                                                                                                               |
| ----------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| Registry file     | [mozilla/cargo-vet registry.toml](https://github.com/mozilla/cargo-vet/blob/fb5cc28663eb4ec5e7b136413c012457063b4d81/registry.toml) |
| Pinned commit     | `fb5cc28663eb4ec5e7b136413c012457063b4d81`                                                                                          |
| Entries used      | 9                                                                                                                                   |

| Registry entry      | Repository                                                                      |
| ------------------- | ------------------------------------------------------------------------------- |
| `actix`             | [actix/supply-chain](https://github.com/actix/supply-chain)                     |
| `ariel-os`          | [ariel-os/ariel-os](https://github.com/ariel-os/ariel-os)                       |
| `bytecode-alliance` | [bytecodealliance/wasmtime](https://github.com/bytecodealliance/wasmtime)       |
| `embark-studios`    | [EmbarkStudios/rust-ecosystem](https://github.com/EmbarkStudios/rust-ecosystem) |
| `fermyon`           | [fermyon/spin](https://github.com/fermyon/spin)                                 |
| `google`            | [google/rust-crate-audits](https://github.com/google/rust-crate-audits)         |
| `isrg`              | [divviup/libprio-rs](https://github.com/divviup/libprio-rs)                     |
| `mozilla`           | [mozilla/supply-chain](https://github.com/mozilla/supply-chain)                 |
| `zcash`             | [zcash/rust-ecosystem](https://github.com/zcash/rust-ecosystem)                 |

The registry is a discovery list, not a complete index of every public
`audits.toml` file. Other public cargo-vet sources may exist outside it.

## Data

Raw outputs:

- [summary JSON](https://thirdpass.dev/data/cargo-vet-popular-dependency-coverage-summary.json)
- [popular crates CSV](https://thirdpass.dev/data/cargo-vet-popular-crates.csv)
- [root and dependency coverage CSV](https://thirdpass.dev/data/cargo-vet-popular-graph-coverage.csv)
- [resolved package coverage CSV](https://thirdpass.dev/data/cargo-vet-popular-resolved-package-coverage.csv)
- [uncovered package frequency CSV](https://thirdpass.dev/data/cargo-vet-popular-uncovered-package-frequency.csv)
- [excluded failed candidates CSV](https://thirdpass.dev/data/cargo-vet-popular-excluded-failed-candidates.csv)

The top-100 run had no excluded failed candidates. The file is included so the
analysis format stays stable if the cutoff changes later.

## Notes

The root crate versions came from the 2026-06-17 crates.io dump.

Some resolved dependency versions were newer than the dump. For those rows, the
analysis used crates.io API metadata for publication date and publisher ID.

Version age should be read against the resolution model above. A dependency
version can be young because Cargo selected it for the current root crate
version and feature set.

Different feature choices, targets, lockfiles, or audit sources can change the
resolved dependencies and the matching cargo-vet coverage.

