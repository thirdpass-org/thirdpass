<h1 align="center">Vouch</h1>

<p align="center"><strong>Collaborative dependency review for open-source packages.</strong></p>

<p align="center">
  <a href="https://matrix.to/#/#vouch:matrix.org"><img src="https://img.shields.io/matrix/vouch:matrix.org?label=chat&logo=matrix" alt="Matrix"></a>
</p>

Most dependency code is shipped without meaningful review. Vouch enables
collaborative dependency review: contributors run reviews, publish structured
findings, and the ecosystem reuses that signal.

Think of it as channeling LLM inference tokens into open-source dependency due
diligence.

<p align="center">
  <img src="assets/vouch_review_is-even_v3.gif" alt="Using Vouch to review Javascript package is-even." />
</p>

## Why Vouch

- **Collaborative coverage:** many contributors review, everyone benefits.
- **Reusable security signal:** findings are structured and tied to package
  version + file scope.
- **Faster risk decisions:** `vouch check` helps teams evaluate dependency
  posture from shared reviews.

## 60-second quickstart

Run a dependency review and submit it:

```bash
cargo run -p vouch -- review d3 4.10.0
```

Review specific files in one run:

```bash
cargo run -p vouch -- review d3 4.10.0 \
  --file src/index.js \
  --file src/core.js
```

Evaluate a project's dependencies against available reviews:

```bash
cargo run -p vouch -- check
```

## How it works

1. Vouch fetches and unpacks the exact dependency artifact.
2. An agent (Codex or Claude) reviews selected target files.
3. Findings are saved locally and submitted to the shared review service.
4. Other users consume that signal via `vouch check`.

This is not one-off scanning. It is cumulative review intelligence.

## Core commands

Review a package version:

```bash
vouch review <package> <version>
```

Submit existing matching local work without re-running the agent:

```bash
vouch review d3 4.10.0 --file src/index.js --submit-existing
```

Check dependencies:

```bash
vouch check
```

## Agent configuration

Choose default reviewing agent:

```bash
vouch review d3 4.10.0 --agent codex
vouch review d3 4.10.0 --agent claude
```

Set Codex defaults:

```bash
vouch review d3 4.10.0 --agent codex --agent-model gpt-5.2-codex
vouch review d3 4.10.0 --agent codex --agent-reasoning-effort high
```

## Extensions

Vouch supports multiple ecosystems via extensions.

Install an extension:

```bash
vouch extension add py
```

Install from repository URL:

```bash
vouch extension add https://github.com/vouch-dev/vouch-py
```

List installed extensions:

```bash
vouch extension list
```

Official extensions:

| Name                                                        | Ecosystem      | Package Registries |
|-------------------------------------------------------------|----------------|--------------------|
| [vouch-py](https://github.com/vouch-dev/vouch-py)           | Python         | pypi.org           |
| [vouch-js](https://github.com/vouch-dev/vouch-js)           | Javascript     | npmjs.com          |
| [vouch-ansible](https://github.com/vouch-dev/vouch-ansible) | Ansible Galaxy | galaxy.ansible.com |

## Notes

- Setup runs automatically on first command.
- Source archives and reviews are stored in Vouch's per-user data directory.
- Use `vouch review --help` and `vouch check --help` for full flags.
- If you need a local-only run, use `--skip-coordination`.
