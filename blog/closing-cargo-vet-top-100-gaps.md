# Closing the cargo-vet Gaps in the Top 100 Rust Crates

Published: 2026-07-10

This is a GitHub-readable mirror of the canonical website post:
<https://thirdpass.dev/blog/closing-cargo-vet-top-100-gaps>.

In June, we looked at
[`cargo-vet` coverage in the top 100 Rust crates](cargo-vet-popular-dependency-coverage.md).
That post measured public cargo-vet coverage for the 100 most-downloaded
crates and the crate versions selected by their Linux dependency graphs. The
100 most-downloaded crate versions were the starting points for the dependency
resolution.

Those starting crate versions were in decent shape: 73 of 100 were already
covered by public cargo-vet data, or 73.0%. The dependency graph had larger
gaps. Across the resolved dependency versions, excluding the starting crates,
only 145 of 281 unique dependency versions were covered, or 51.6%.

That left 136 uncovered dependency versions. Counting the uncovered starting
crate versions as well, the top-100 run had 148 unique crate/version pairs
with no matched public cargo-vet coverage.

## What We Added

We created a Thirdpass cargo-vet audit repository for those missing
crate/version pairs:

<https://github.com/thirdpass-org/cargo-vet-audits>

That repository exports Thirdpass review evidence as cargo-vet-compatible
`audits.toml` entries. It is meant to be inspectable. Each audit points to an
evidence JSON file with the package hash, reviewed files, review summaries,
agent details, available runtime and token metrics, and links back to the
Thirdpass review page.

The current export contains 148 crate/version audits. Those entries are the 148
crate/version pairs that were uncovered in the previous top-100 analysis.

With the new Thirdpass audit repo added to the public cargo-vet sources from
the June analysis, the sampled graph is fully covered:

| Scope                                                 | Before         | After          |
| ----------------------------------------------------- | -------------- | -------------- |
| Top-100 starting crate versions                       | 73/100 (73.0%) | 100/100 (100%) |
| Unique dependency versions, excluding starting crates  | 145/281 (51.6%) | 281/281 (100%) |
| Unique crate/version pairs, including starting crates  | 166/314 (52.9%) | 314/314 (100%) |

The "after" column is the union of the original public cargo-vet sources and
the new Thirdpass audits. The Thirdpass repo by itself does not cover every
crate in the graph, because the work targeted the missing pieces. Within the
targeted set, every exported Thirdpass audit is backed only by
`codex/gpt-5.4-mini/high` review records.

## What the Coverage Means

The Thirdpass criterion is:

`thirdpass-full-crate-archive-reviewed/v1`

For each emitted audit, Thirdpass has 100% byte coverage of the crate archive
according to the authoritative crates.io package archive manifest.

Every file in the crate archive was assigned to a review, and the accepted
reviews together covered every byte in the package archive. The review
procedure is file-focused: an agent reviews one or more selected files, records
what it looked at, and reports any concrete supply-chain indicators it found.
The cargo-vet export composes those file-focused reviews into a crate-level
audit entry once the whole archive is covered.

This is evidence toward safety in a narrow sense. It is especially aimed at
supply-chain behavior that can be seen in the package contents:

- install-time execution
- network or exfiltration behavior
- credential or environment access
- dynamic code loading
- obfuscation or packing
- persistence or tampering behavior

It is not a proof that the crate is bug-free. It is not a proof that the crate
is cryptographically correct, memory safe, or suitable for a particular
production system. It is also not the same as a maintainer or domain expert
reviewing the crate's design.

The claim is narrower: for these crate archives, there is recorded,
file-by-file evidence that the package contents were reviewed for concrete
supply-chain risk indicators.

## Why Use cargo-vet

cargo-vet gives Rust projects a way to track audit evidence in source control.
It also has an import model, so projects can choose whether to trust or ignore
external audit sources.

Thirdpass does not need to ask every project to use a new tool. Instead, the
review results can be published as a cargo-vet audit repo. A project can then
inspect the criterion, inspect the evidence, and decide whether that evidence
is useful for its own policy.

The exported criterion is deliberately specific. It records Thirdpass
full-crate archive review coverage. It does not automatically claim cargo-vet's
built-in `safe-to-run` or `safe-to-deploy` criteria. A consuming project can
map it into its own policy if that is appropriate.

## What Is in the Evidence Bundle

For each audited crate version, the repository includes:

- the crate name and version
- the package archive hash
- the Thirdpass review page URL
- the reviewed byte count and total byte count
- the review procedure name and version
- the agent model and reasoning effort used for the reviews
- available runtime and token metrics
- review-level agent summaries
- per-file summaries
- structured review comments, when comments were recorded
- whether the reviewer was an official Thirdpass reviewer

A bare "reviewed" flag is hard to inspect. The evidence files now include the
same kind of review summaries shown on the Thirdpass website, so someone can
see what the agent understood about each file and what kind of risk indicators
it checked for.

## Model and Token Spend

The current export contains 3,355 accepted review records covering 9,360 file
records across 148 crate versions. Those reviews cover 8,920 distinct files and
185,276,571 bytes from the crate archives.

This export is deliberately narrower than the earlier working set. It includes
only `codex/gpt-5.4-mini/high` reviews with full-file scope. Runtime and token
metrics are available for every file record in the exported evidence.

| Agent/model/effort             | Review records | File records | File records with metrics |
| ------------------------------ | -------------- | ------------ | ------------------------- |
| `codex/gpt-5.4-mini/high`      | 3,355          | 9,360        | 9,360                     |

For those records, the measured agent runtime and token use were:

| Metric | Value |
| ------ | ----- |
| File records with metrics | 9,360 |
| Agent attempts | 9,406 |
| Sum of measured agent wall-clock runtime | 48h 3m 22s |
| Failed-attempt runtime | 3h 35m 57s |
| Retry wait time | 1h 22m 52s |
| Input tokens | 456,431,013 |
| Cached input tokens | 389,790,336 |
| Output tokens | 6,109,397 |
| Reasoning output tokens | 2,869,401 |
| Total tokens | 462,540,410 |

The runtime number is the sum of measured agent run durations, not calendar
time from the start of the whole project to the end.

Closing the top-100 sample gap with one consistent high-effort profile took a
few thousand file-focused review records and about 463 million measured tokens.

## The Token Budget Becomes the Main Question

The previous post showed a coverage gap. Some common dependency versions had
no matched public cargo-vet evidence at all. For this sample, that gap was
small enough to close with the current review loop.

The next question is how much review effort to spend:

- How many agent runs should review each file?
- Which models should be used?
- Should high-risk files get stronger or repeated review?
- Should large or complex crates get a second pass with a more capable model?
- Should comments be required even when the review finds no issue?
- How much runtime and token spend is justified for different dependency tiers?

The current repo uses file-focused agent reviews and exports the resulting
evidence. The same system could be extended with more runs, more advanced
models, stricter review procedures, targeted human review, or different
criteria for different kinds of crates.

That turns the Rust audit backlog into a budgeting problem: where do additional
review tokens provide the most value?

## Reproducing the Check

The comparison used the same data files from the previous top-100 post:

- `cargo-vet-popular-crates.csv`
- `cargo-vet-popular-resolved-package-coverage.csv`
- `cargo-vet-popular-uncovered-package-frequency.csv`

We compared the unique crate/version pairs in those files with the generated
Thirdpass `audits.toml`. We also checked the evidence JSON files to make sure
the exported audits used only `codex/gpt-5.4-mini/high` reviews with full-file
scope.

The verification checked that every crate/version pair uncovered in the
previous top-100 analysis now has a matching Thirdpass audit entry, and that the
combined cargo-vet plus Thirdpass view fully covers the sampled dependency
graphs.

The result:

| Check | Result |
| ----- | ------ |
| Previously uncovered unique crate/version pairs | 148 |
| Now present in Thirdpass cargo-vet audits | 148 |
| Still missing | 0 |
| Previously uncovered resolved package rows | 249 |
| Still uncovered resolved package rows | 0 |
| Combined unique crate/version coverage | 314/314 |
| Fully covered dependency graphs | 100/100 |

## Notes

The original top-100 analysis used the crates.io dump from 2026-06-17 and
resolved dependencies for `x86_64-unknown-linux-gnu` with all starting crate
features enabled.

The result here should be read against that same sample. Different targets,
features, lockfiles, newer crate releases, or different cargo-vet sources can
produce a different dependency graph and a different coverage result.

The result is limited, but concrete: a cargo-vet coverage gap from a
popular-crate sample has been closed with a public, inspectable evidence
bundle.
