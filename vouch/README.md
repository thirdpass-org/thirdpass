<h1 align="center">Vouch</h1>

<p align="center">🔍 Crowd-powered dependency intelligence for the open-source supply chain. 🔍</p>

<p align="center">
  <a href="https://matrix.to/#/#vouch:matrix.org"><img src="https://img.shields.io/matrix/vouch:matrix.org?label=chat&logo=matrix" alt="Matrix"></a>
</p>

Open source software dependencies are commonly used without review. Running unreviewed code poses security risks. Vouch lets contributors donate AI tokens and compute to generate trusted code reviews at scale. The goal is simple: **make the software dependency supply chain safer for everyone**.

Vouch is built to:
1. minimize the cost of reviewing dependencies
2. generate and share verified review signals across ecosystems
3. help teams catch malicious or risky packages before they ship

<br>

<p align="center">
  <img src="assets/vouch_review_is-even_v3.gif" alt="Using Vouch to review Javascript package is-even." />
</p>

## Getting Started

### Setup

Setup runs automatically on first use and initializes local configuration and data directories.

### Extensions

Extensions enable Vouch to create reviews for packages from different ecosystems. For example, the [Python extension](https://github.com/vouch-dev/vouch-py) adds support for [pypi.org](https://pypi.org) packages. By default, Vouch includes extensions for Python and Javascript. Add an extension using the following command:

`vouch extension add py`

or via any GitHub repository URL:

`vouch extension add https://github.com/vouch-dev/vouch-py`

#### Official Extensions

| Name                                                        | Ecosystem      | Package Registries |
|-------------------------------------------------------------|----------------|--------------------|
| [vouch-py](https://github.com/vouch-dev/vouch-py)           | Python         | pypi.org           |
| [vouch-js](https://github.com/vouch-dev/vouch-js)           | Javascript     | npmjs.com          |
| [vouch-ansible](https://github.com/vouch-dev/vouch-ansible) | Ansible Galaxy | galaxy.ansible.com |

### Review

By default, Vouch runs an AI agent (Codex or Claude) to review a target file. Use `--manual` to review via [VSCode](https://code.visualstudio.com/) and the Vouch extension.

Lets review the [NPM](https://www.npmjs.com/) Javascript package [d3](https://www.npmjs.com/package/d3) at version `4.10.0`, targeting `src/index.js`:

`vouch review d3 4.10.0 --file src/index.js`

By default, reviews are submitted to the central API. Use `--no-submit` to keep a review local only.

Manual review:

`vouch review d3 4.10.0 --file src/index.js --manual`

### Check

Reviews created using Vouch can be used to evaluate software project dependencies. Vouch extensions can discover ecosystem specific dependency definition files. For example, the Python extension parses `Pipfile.lock` files.

The `check` command automatically pulls the latest reviews from the central API, then generates an evaluation report of local project dependencies:

`vouch check`

## Why Vouch

- **Crowd-powered coverage:** contribute your AI tokens and compute to review open-source code at scale.
- **High-signal outputs:** structured findings focused on security and complexity, not noisy vulnerability lists.
- **Actionable intelligence:** a growing review dataset designed to plug into tooling and CI workflows.
