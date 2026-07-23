# Closing the cargo-vet Gaps in the Top 100 Rust Crates

Published: 2026-07-20

The June analysis measured public cargo-vet coverage for the crates.io 100
most-downloaded crates and the crate versions selected by their Linux
dependency graphs. The results were published in
[`cargo-vet` coverage in the top 100 Rust crates](cargo-vet-popular-dependency-coverage.md).
The baseline used 9 cargo-vet registry entries; the audit repositories are
listed in the [Sources section](cargo-vet-popular-dependency-coverage.md#sources)
of that post.

The baseline still had substantial gaps: 27 of 100 top-level crate versions
were not covered, and 148 of 314 (47.1%) distinct crate/version pairs across the
sampled dependency graphs were not covered.

## Generating Review Coverage

Thirdpass CLI was used with Codex `gpt-5.4-mini` at effort `high` to review
the 148 uncovered crate/version pairs.

For each crates.io crate version archive, the review procedure was
[file-focused review](https://thirdpass.dev/docs/cargo-vet#file-focused-review):

- Each agent session focused on one target file.
- The agent could inspect the rest of the crate archive to understand how that
  file was used.
- The review recorded what the agent inspected and summarized supply-chain
  relevant behavior: install/build execution, network or credential access,
  dynamic code loading, hidden intent, or file tampering.

A crate/version counted as covered only when accepted file reviews covered every
file in the crate archive manifest.

Under this review procedure, the accepted reviews did not report the
supply-chain indicators listed above.

## Published cargo-vet Repo

cargo-vet lets Rust projects track audit evidence for the crate versions they
depend on. Projects can import audit repositories and decide in their own
policy which evidence is enough for their dependencies.

Review evidence was published as a cargo-vet audit repo:

<https://github.com/thirdpass-org/cargo-vet-audits>

The Thirdpass audit repo records that, for a given crate version, every file in
the crates.io archive was reviewed with the Thirdpass file-focused review
procedure.

In cargo-vet, a criterion is the named audit result recorded for a crate
version. The Thirdpass repo uses
`thirdpass-full-crate-archive-reviewed/v1`. It is not a general security
certification, and it does not automatically mark the crate as `safe-to-run` or
`safe-to-deploy`. A project would have to make that choice explicitly in its
own cargo-vet policy.

Each audit points to a JSON evidence file so the cargo-vet entry is not just a
bare audit entry. The point is that readers and AI agents can examine the
review judgment, challenge it, and decide how much weight to give it.

The evidence shows:

- which archive and files were reviewed
- which procedure version and agent configuration were used
- the agent summaries and review comments
- what the review cost, including runtime and token metrics when available

That makes the audit easier to inspect, compare with future review runs, and
decide whether to use in a local cargo-vet policy.

Adding the Thirdpass repo to the public cargo-vet sources from the June analysis
covers all 148 previously uncovered crate/version pairs. In the sampled graph,
combined coverage is now 100% of crate/version pairs. The Thirdpass repo by
itself does not cover every crate in the graph, because this pass targeted the
missing pieces.

## Cost

The high-effort export contains:

| Metric | Value |
| ------ | ----- |
| Audited crate/version pairs | 148 |
| File review records | 9,360 |
| Archive data covered | 185.3 MB |
| Summed agent runtime | 48h 3m 22s |
| Input tokens | 456.4M |
| Cached input tokens | 389.8M |
| Output tokens | 6.1M |
| Reasoning output tokens | 2.9M |
| Total tokens | 462.5M |

Cached input tokens are included in the input total. Reasoning output tokens
are included in the output total.

---

# Notes

* This is a GitHub-readable mirror of the canonical website post:
<https://thirdpass.dev/blog/closing-cargo-vet-top-100-gaps>.
