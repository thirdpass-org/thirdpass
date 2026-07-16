# Closing the cargo-vet Gaps in the Top 100 Rust Crates

Published: 2026-07-10

This is a GitHub-readable mirror of the canonical website post:
<https://thirdpass.dev/blog/closing-cargo-vet-top-100-gaps>.

In June, we looked at
[`cargo-vet` coverage in the top 100 Rust crates](cargo-vet-popular-dependency-coverage.md).
That analysis measured public cargo-vet coverage for the 100 most-downloaded
crates and the crate versions selected by their Linux dependency graphs.

The starting crate versions were in decent shape: 73 of 100 had matched public
cargo-vet coverage. The dependency graph had larger gaps. Excluding the
starting crates, only 145 of 281 unique dependency versions were covered.

Counting the uncovered starting crate versions as well, the sample had 148
unique crate/version pairs with no matched public cargo-vet coverage.

## What We Added

We created a Thirdpass cargo-vet audit repository for those missing
crate/version pairs:

<https://github.com/thirdpass-org/cargo-vet-audits>

The current export contains 148 audits: one for each crate/version pair that was
uncovered in the previous top-100 analysis. The export is intentionally narrow.
Every audit is backed only by `codex/gpt-5.4-mini/high` reviews with full-file
scope.

With the new Thirdpass audit repo added to the public cargo-vet sources from the
June analysis, the sampled graph is fully covered:

| Scope                                                | Before          | After          |
| ---------------------------------------------------- | --------------- | -------------- |
| Top-100 starting crate versions                      | 73/100 (73.0%)  | 100/100 (100%) |
| Dependency versions, excluding starting crates       | 145/281 (51.6%) | 281/281 (100%) |
| Crate/version pairs, including starting crates       | 166/314 (52.9%) | 314/314 (100%) |

The "after" column is the union of the original public cargo-vet sources and
the new Thirdpass audits. The Thirdpass repo by itself does not cover every
crate in the graph, because this pass targeted the missing pieces.

## What the Audit Means

Each Thirdpass audit says that the crate archive was reviewed with 100% byte
coverage against the authoritative crates.io package archive manifest. In
cargo-vet, that evidence is recorded under the criterion name
`thirdpass-full-crate-archive-reviewed/v1`.

The procedure is file-focused:

- Each agent session focuses on one file at a time. The agent can inspect the
  rest of the crate archive to understand how that file is used.
- The file review records what the agent looked at and summarizes
  supply-chain relevant behavior: code that runs during install or build,
  accesses the network or credentials, loads code dynamically, hides intent, or
  tampers with files.
- The cargo-vet export creates a crate-level audit only when accepted file
  reviews cover 100% of the archive bytes.

This is evidence toward safety in a narrow sense. It is not a proof that the
crate is bug-free, memory safe, cryptographically correct, or suitable for a
particular production system. It is also not automatically cargo-vet
`safe-to-run` or `safe-to-deploy`. A project can inspect the evidence and decide
how, or whether, to map this criterion into its own policy.

Each audit points to a JSON evidence file with the package hash, reviewed files,
review summaries, agent details, available runtime and token metrics, and a link
back to the Thirdpass review page.

## Cost

The high-effort export contains:

| Metric | Value |
| ------ | ----- |
| Audited crate/version pairs | 148 |
| Accepted review records | 3,355 |
| File review records | 9,360 |
| Distinct files covered | 8,920 |
| Bytes covered | 185,276,571 |
| Measured agent runtime, summed across runs | 48h 3m 22s |
| Measured tokens | 462,540,410 |

That is the main tradeoff. The system can produce more audit evidence by
spending more agent runs and tokens. The question becomes where the next review
budget should go: more crates, repeated review, stronger models for higher-risk
files, or human follow-up for selected results.

## Verification

We compared the previous top-100 analysis data with the generated Thirdpass
`audits.toml`. We also checked the evidence JSON files to confirm that the
exported audits used only `codex/gpt-5.4-mini/high` reviews with full-file
scope.

| Check | Result |
| ----- | ------ |
| Previously uncovered unique crate/version pairs | 148 |
| Now present in Thirdpass cargo-vet audits | 148 |
| Previously uncovered resolved package rows | 249 |
| Still uncovered resolved package rows | 0 |
| Combined unique crate/version coverage | 314/314 |
| Fully covered dependency graphs | 100/100 |

This result is scoped to the same sample as the previous post: the crates.io
dump from 2026-06-17, dependency resolution for `x86_64-unknown-linux-gnu`, and
the selected root crate features from that analysis. Different targets,
features, lockfiles, newer crate releases, or different cargo-vet sources can
produce a different result.

The limited claim is that this specific cargo-vet coverage gap is now closed
with a public, inspectable evidence bundle.
