# cargo-vet Shows Weak Long-Tail Coverage

Published: 2026-06-24

This is a GitHub-readable mirror of the canonical website post:
<https://thirdpass.dev/blog/cargo-vet-weak-long-tail-coverage>.

We compared the public `cargo-vet` registry sources with the crates.io dump
from 2026-06-17.

`cargo-vet` maintains a registry of audit sets published by well-known
organizations. The registry is used by cargo-vet when it suggests imports that
could reduce a project's audit backlog. We used it as the source list for this
run.

The analysis used a broad coverage definition based on cargo-vet's built-in
criteria. A crate version counted as covered when `cargo-vet` metadata could
justify accepting that version through any supported evidence path in the
selected registry sources for `safe-to-deploy` or `safe-to-run`. That includes
direct audits, delta audits, wildcard audits, and trusted-publisher rules.

Headline result: 0.59% of crates had at least one covered version. That is
1,628 crates out of 277,697.

This is the most generous coverage metric in the analysis. It counts a crate
as covered when any non-yanked version is covered. That version may be old,
and it may not be the version crates.io currently shows by default.

Even with that definition, coverage was below 1%.

## Result

The main count is deduplicated by crate name. A crate with many covered
versions still counts once.

| Count                                    | Value   |
| ---------------------------------------- | ------- |
| Eligible crates                          | 277,697 |
| Crates with at least one covered version | 1,628   |
| Crates with no covered version           | 276,069 |
| Crate-level coverage                     | 0.59%   |

Of the 1,628 covered crates, 744 did not have their current crates.io default
version covered.

## Current Versions

The current-version count is stricter. It only counts a crate when the version
crates.io marks as the default is covered.

| Count                      | Value   |
| -------------------------- | ------- |
| Eligible current versions  | 277,697 |
| Covered current versions   | 884     |
| Uncovered current versions | 276,813 |
| Current-version coverage   | 0.3183% |

Delta audits accounted for the largest share of covered current versions.
With a wildcard audit, the auditor is effectively vouching that releases
of that crate by a specified publisher, within a specified date range, satisfy
the stated criteria.

| Primary coverage path | Covered current versions |
| --------------------- | ------------------------ |
| Delta audit           | 266                      |
| Direct version audit  | 262                      |
| Wildcard audit        | 205                      |
| Trusted publisher     | 151                      |

## Age Filters

To avoid penalizing very new versions, we also filtered out younger versions.
The result did not change much.

| Current version age | Covered | Eligible | Coverage |
| ------------------- | ------- | -------- | -------- |
| At least 90 days    | 703     | 197,511  | 0.3559%  |
| At least 180 days   | 622     | 167,322  | 0.3717%  |
| At least 365 days   | 468     | 138,522  | 0.3379%  |

Age does not explain the low coverage rate in this run.

## Historical Version Rows

Counting each non-yanked release separately gives a larger denominator and a
higher coverage rate. This row count is not deduplicated by crate.

| Count                         | Value     |
| ----------------------------- | --------- |
| Historical crate-version rows | 2,418,303 |
| Covered historical rows       | 23,101    |
| Uncovered historical rows     | 2,395,202 |
| Historical-row coverage       | 0.9553%   |

## Sources

The analysis imported `audits.toml` data from the cargo-vet registry:

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

- [any-version crate summary JSON](https://thirdpass.dev/data/cargo-vet-any-version-crate-coverage-summary.json)
- [covered crates any-version CSV](https://thirdpass.dev/data/cargo-vet-covered-crates-any-version.csv)
- [default-version summary JSON](https://thirdpass.dev/data/cargo-vet-default-version-coverage-summary.json)
- [covered default versions CSV](https://thirdpass.dev/data/cargo-vet-covered-default-versions.csv)
- [uncovered default versions sample CSV](https://thirdpass.dev/data/cargo-vet-uncovered-default-versions-sample.csv)
- [all-version summary JSON](https://thirdpass.dev/data/cargo-vet-all-version-coverage-summary.json)
- [covered all versions CSV](https://thirdpass.dev/data/cargo-vet-covered-all-versions.csv)
- [uncovered all versions sample CSV](https://thirdpass.dev/data/cargo-vet-uncovered-all-versions-sample.csv)

The uncovered CSV files are samples, not full uncovered sets.

## Notes

The headline count counts crates. The current-version table counts one
crates.io default version per crate. The historical-row table counts every
non-yanked version row once.

The analysis counts the built-in `safe-to-deploy` and `safe-to-run` criteria.
It does not count custom criteria.

"Covered" does not always mean manually audited. It means covered by at least
one cargo-vet evidence path in the registry data.

