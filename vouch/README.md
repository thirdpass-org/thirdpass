<h1 align="center">Vouch</h1>

<p align="center">🔍 A multi-ecosystem package code review system. 🔍</p>

<p align="center">
  <a href="https://matrix.to/#/#vouch:matrix.org"><img src="https://img.shields.io/matrix/vouch:matrix.org?label=chat&logo=matrix" alt="Matrix"></a>
</p>

Open source software dependencies are commonly used without review. Running unreviewed code poses security risks. Vouch is a package code review system built to solve this problem by:

1. minimizing the costs of reviewing software
2. checking software dependencies against reviews.

<br>

<p align="center">
  <img src="assets/vouch_review_is-even_v3.gif" alt="Using Vouch to review Javascript package is-even." />
</p>

## Getting Started

### Setup

First, lets setup Vouch. This initializes local configuration and data directories.

`vouch setup`

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

(Note: Vouch currently requires [VSCode](https://code.visualstudio.com/) to create reviews.)

Vouch supports multiple ecosystems and is extendable. For now, Python and Javascript support comes built-in. Lets review the [NPM](https://www.npmjs.com/) Javascript package [d3](https://www.npmjs.com/package/d3) at version `4.10.0`:

`vouch review d3 4.10.0`

### Check

Reviews created using Vouch can be used to evaluate software project dependencies. Vouch extensions can discover ecosystem specific dependency definition files. For example, the Python extension parses `Pipfile.lock` files.

The `check` command generates an evaluation report of local project dependencies based on available reviews:

`vouch check`
