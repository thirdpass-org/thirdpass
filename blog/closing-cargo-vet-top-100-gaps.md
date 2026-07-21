# Closing the cargo-vet Gaps in the Top 100 Rust Crates

Published: 2026-07-20

In June, we looked at
[`cargo-vet` coverage in the top 100 Rust crates](cargo-vet-popular-dependency-coverage.md).
That analysis measured public cargo-vet coverage for the crates.io 100 most-downloaded
crates and the crate versions selected by their Linux dependency graphs.
The baseline used 9 cargo-vet registry entries; the audit repositories are
listed in the [Sources section](cargo-vet-popular-dependency-coverage.md#sources)
of that post.

The starting crate versions were in decent shape: 73 of 100 had matched public
cargo-vet coverage. The dependency graph had larger gaps. Excluding the
starting crates, only 145 of 281 unique dependency versions were covered.

Counting the uncovered starting crate versions as well, the sample had 148
unique crate/version pairs with no matched public cargo-vet coverage.

## Generating Review Coverage

We used Codex `gpt-5.4-mini` with effort `high` to review the 148 uncovered
crate/version pairs.

For each crates.io crate version archive, we used
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
policy which kinds of evidence are enough for their dependencies.

We published the covered crate/version pairs as a cargo-vet audit repo:

<https://github.com/thirdpass-org/cargo-vet-audits>

In that system, a criterion is the named claim attached to an audit. The
Thirdpass audits use the criterion name
`thirdpass-full-crate-archive-reviewed/v1`: the claim is that the crate archive
has Thirdpass review coverage under the procedure above. It is not a general
security certification or an automatic cargo-vet `safe-to-run` or
`safe-to-deploy` judgment.

Each audit points to a JSON evidence file so the cargo-vet entry is not just a
bare assertion. The goal is to make the audit less opaque: readers and AI agents
can scrutinize the underlying review evidence instead of only trusting the
cargo-vet entry.

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
