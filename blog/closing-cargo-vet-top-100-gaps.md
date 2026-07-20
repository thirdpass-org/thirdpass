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

Adding the Thirdpass repo to the public cargo-vet sources from the June analysis
covers all 148 previously uncovered crate/version pairs. In the sampled graph,
combined coverage is now 314/314 crate/version pairs. The Thirdpass repo by
itself does not cover every crate in the graph, because this pass targeted the
missing pieces.

## What the Audit Means

Each Thirdpass audit says that the crate archive was reviewed with 100% byte
coverage against the authoritative crates.io package archive manifest. In
cargo-vet, that evidence is recorded under the criterion name
`thirdpass-full-crate-archive-reviewed/v1`.

The review procedure was:

- Each agent session focused on one target file.
- The agent could inspect the rest of the crate archive to understand how that
  file was used.
- The review recorded what the agent inspected and summarized supply-chain
  relevant behavior, including install/build execution, network or credential
  access, dynamic code loading, hidden intent, or file tampering.

A crate/version was included in the Thirdpass cargo-vet repo only when:

- Accepted file reviews matched the crate archive and file hashes.
- Those accepted reviews covered 100% of the archive bytes.
- For this export, the reviews matched the high-effort review set:
  `codex/gpt-5.4-mini/high` with full-file scope.

Read the result narrowly: under this review procedure, the accepted reviews did
not report the supply-chain indicators listed above. It is not a general
security certification or an automatic cargo-vet `safe-to-run` or
`safe-to-deploy` judgment.

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
