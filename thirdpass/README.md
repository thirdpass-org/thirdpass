<h1 align="center">Thirdpass</h1>

<p align="center"><strong>Collaborative dependency review for open-source packages.</strong></p>

<p align="center">
  <a href="https://matrix.to/#/#thirdpass:matrix.org"><img src="https://img.shields.io/matrix/thirdpass:matrix.org?label=chat&logo=matrix" alt="Matrix"></a>
</p>

Most dependency code is shipped without meaningful review. Thirdpass enables
collaborative dependency review: contributors run reviews, publish structured
findings, and the ecosystem reuses that signal.

Think of it as channeling LLM inference tokens into open-source dependency due
diligence.

<p align="center">
  <img src="assets/thirdpass_review_is-even_v3.gif" alt="Using Thirdpass to review Javascript package is-even." />
</p>

## Why Thirdpass

- **Collaborative coverage:** many contributors review, everyone benefits.
- **Reusable security signal:** findings are structured and tied to package
  version + file scope.
- **Faster risk decisions:** `thirdpass check` helps teams evaluate dependency
  posture from shared reviews.

## 60-second quickstart

Run a dependency review and submit it:

```bash
cargo run -p thirdpass -- review d3 4.10.0
```

Review specific files in one run:

```bash
cargo run -p thirdpass -- review d3 4.10.0 \
  --file src/index.js \
  --file src/core.js
```

Evaluate a project's dependencies against available reviews:

```bash
cargo run -p thirdpass -- check
```

## How it works

1. Thirdpass fetches and unpacks the exact dependency artifact.
2. An agent (Codex or Claude) reviews selected target files.
3. Findings are saved locally and submitted to the shared review service.
4. Other users consume that signal via `thirdpass check`.

This is not one-off scanning. It is cumulative review intelligence.

## Core commands

Review a package version:

```bash
thirdpass review <package> <version>
```

Submit existing matching local work without re-running the agent:

```bash
thirdpass review d3 4.10.0 --file src/index.js --submit-existing
```

Check dependencies:

```bash
thirdpass check
```

## Agent configuration

Choose default reviewing agent:

```bash
thirdpass review d3 4.10.0 --agent codex
thirdpass review d3 4.10.0 --agent claude
```

Set Codex defaults:

```bash
thirdpass review d3 4.10.0 --agent codex --agent-model gpt-5.2-codex
thirdpass review d3 4.10.0 --agent codex --agent-reasoning-effort high
```

## Extensions

Thirdpass supports multiple ecosystems via extensions.

Install an extension:

```bash
thirdpass extension add py
```

Install from repository URL:

```bash
thirdpass extension add https://github.com/thirdpass-org/thirdpass-py
```

List installed extensions:

```bash
thirdpass extension list
```

Official extensions:

| Name                                                        | Ecosystem      | Package Registries | Availability |
|-------------------------------------------------------------|----------------|--------------------|--------------|
| [thirdpass-rs](https://github.com/thirdpass-org/thirdpass-rs)           | Rust           | crates.io          | Inbuilt      |
| [thirdpass-py](https://github.com/thirdpass-org/thirdpass-py)           | Python         | pypi.org           | Inbuilt      |
| [thirdpass-js](https://github.com/thirdpass-org/thirdpass-js)           | Javascript     | npmjs.com          | Inbuilt      |
| [thirdpass-ansible](https://github.com/thirdpass-org/thirdpass-ansible) | Ansible Galaxy | galaxy.ansible.com | External     |
