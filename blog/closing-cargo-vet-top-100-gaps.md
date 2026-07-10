# Closing the cargo-vet Gaps in the Top 100 Rust Crates

Published: 2026-07-10

This is a GitHub-readable mirror of the canonical website post:
<https://thirdpass.dev/blog/closing-cargo-vet-top-100-gaps>.

In June, we looked at
[`cargo-vet` coverage in the top 100 Rust crates](cargo-vet-popular-dependency-coverage.md).
That post asked a simple question: if you take the 100 most-downloaded crates
and resolve their Linux dependency graphs, how much of that crate/version
surface already has public cargo-vet coverage?

The answer was mixed.

The roots were in decent shape: 73 of the 100 selected root crate versions
were already covered by public cargo-vet data. The dependency graph had larger
gaps. Across the resolved dependency versions, excluding the root crates, only
145 of 281 unique dependency versions were covered.

That left 136 uncovered dependency versions. Counting root crate gaps as well,
the top-100 run had 148 unique crate/version pairs with no matched public
cargo-vet coverage.

We used that list as a target.

## What We Added

We created a Thirdpass cargo-vet audit repository:

<https://github.com/thirdpass-org/cargo-vet-audits>

That repository exports Thirdpass review evidence as cargo-vet-compatible
`audits.toml` entries. It is meant to be inspectable. Each audit points to an
evidence JSON file with the package hash, reviewed files, review summaries,
agent details, available runtime and token metrics, and links back to the
Thirdpass review page.

The current export contains 176 crate/version audits. Those entries cover all
148 crate/version pairs that were uncovered in the previous top-100 analysis.

Combining the original public cargo-vet coverage from the June analysis with
the new Thirdpass audit repo gives full coverage for that sampled graph:

| Scope                                      | Before | After |
| ------------------------------------------ | ------ | ----- |
| Top-100 root crate versions                | 73/100 | 100/100 |
| Unique dependency versions, excluding roots | 145/281 | 281/281 |
| Unique crate/version pairs, including roots | 166/314 | 314/314 |

The "after" column is the union of the original public cargo-vet sources and
the new Thirdpass audits. The Thirdpass repo by itself does not cover every
crate in the graph, because the work targeted the missing pieces.

## What the Coverage Means

The Thirdpass criterion is:

`thirdpass-full-crate-archive-reviewed/v1`

For each emitted audit, Thirdpass has 100% byte coverage of the crate archive
according to the authoritative crates.io package archive manifest.

That means every file in the crate archive was assigned to a review, and the
accepted reviews together covered every byte in the package archive. The review
procedure is file-focused: an agent reviews one or more selected files, records
what it looked at, and reports any concrete supply-chain indicators it found.
The cargo-vet export then composes those file-focused reviews into a
crate-level audit entry once the whole archive is covered.

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

The useful claim is narrower: for these crate archives, there is now recorded,
file-by-file evidence that the package contents were reviewed for concrete
supply-chain risk indicators.

## Why Use cargo-vet

cargo-vet already gives Rust projects a way to track audit evidence in source
control. It also has an import model, so projects can choose whether to trust
or ignore external audit sources.

That makes it a good distribution format for this work.

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

The summaries are important. A bare "reviewed" flag is hard to inspect. The
evidence files now include the same kind of review summaries shown on the
Thirdpass website, so someone can see what the agent understood about each
file and what kind of risk indicators it checked for.

## Model and Token Spend

The current export contains 3,935 accepted review records covering 10,618 file
records across 176 crate versions.

Those reviews were not all produced under the same telemetry setup. Runtime and
token metrics are available for 2,583 of the 10,618 reviewed file records. The
remaining 8,035 file records were submitted before or without metric reporting.

The model mix in the exported evidence is:

| Agent/model/effort             | Review records | File records | File records with metrics |
| ------------------------------ | -------------- | ------------ | ------------------------- |
| `codex/gpt-5.4-mini/high`      | 822            | 2,583        | 2,583                     |
| `codex/gpt-5.4-mini/medium`    | 3,091          | 7,948        | 0                         |
| `codex/gpt-5.5/high`           | 22             | 87           | 0                         |

For the records with metrics, the measured agent runtime and token use were:

| Metric | Value |
| ------ | ----- |
| File records with metrics | 2,583 |
| Agent attempts | 2,584 |
| Sum of measured agent wall-clock runtime | 12h 58m 22s |
| Failed-attempt runtime | 48s |
| Retry wait time | 15s |
| Input tokens | 102,369,874 |
| Cached input tokens | 84,852,224 |
| Output tokens | 1,662,800 |
| Reasoning output tokens | 787,212 |
| Total tokens | 104,032,674 |

The runtime number is the sum of measured agent run durations, not calendar
time from the start of the whole project to the end. The token count is also a
lower bound for the full project, because older records do not have metrics.

Even with that caveat, the order of magnitude is useful. Closing the top-100
sample gap was a bounded review workload: a few thousand measured file-focused
agent runs, about 104 million measured tokens, plus older unmetered review
records.

## The Token Budget Becomes the Main Question

This changes the shape of the problem.

The previous post showed a coverage gap. Some common dependency versions had
no matched public cargo-vet evidence at all. The next question was whether that
gap was practical to close.

For this sample, it was.

The work is now less about whether coverage can exist, and more about how much
review effort we want to spend:

- How many agent runs should review each file?
- Which models should be used?
- Should high-risk files get stronger or repeated review?
- Should large or complex crates get a second pass with a more capable model?
- Should comments be required even when the review finds no issue?
- How much runtime and token spend is justified for different dependency tiers?

The current repo is one point in that design space. It uses file-focused agent
reviews and exports the resulting evidence. The same system could be extended
with more runs, more advanced models, stricter review procedures, targeted
human review, or different criteria for different kinds of crates.

That is a more tractable problem. Instead of asking the Rust ecosystem to
manually audit everything at once, we can ask where additional review tokens
provide the most value.

## Reproducing the Check

The comparison used the same data files from the previous top-100 post:

- `cargo-vet-popular-crates.csv`
- `cargo-vet-popular-resolved-package-coverage.csv`
- `cargo-vet-popular-uncovered-package-frequency.csv`

We compared the unique crate/version pairs in those files with the generated
Thirdpass `audits.toml`.

The key verification was that every crate/version pair that was uncovered in
the previous top-100 analysis now has a matching Thirdpass audit entry.

Result:

| Check | Result |
| ----- | ------ |
| Previously uncovered unique crate/version pairs | 148 |
| Now present in Thirdpass cargo-vet audits | 148 |
| Still missing | 0 |

## Notes

The original top-100 analysis used the crates.io dump from 2026-06-17 and
resolved dependencies for `x86_64-unknown-linux-gnu` with all root crate
features enabled.

The result here should be read against that same sample. Different targets,
features, lockfiles, newer crate releases, or different cargo-vet sources can
produce a different dependency graph and a different coverage result.

The practical conclusion is still useful: a concrete cargo-vet coverage gap
from a real popular-crate sample has been closed with a public, inspectable
evidence bundle.
