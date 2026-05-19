<h1 align="center">Thirdpass</h1>

<p align="center"><strong>Collaborative dependency review for open-source packages.</strong></p>

<p align="center">
  <a href="https://discord.gg/ZG4NjxXcB"><img src="https://img.shields.io/badge/chat-Discord-5865F2?logo=discord&logoColor=white" alt="Discord"></a>
</p>

Thirdpass coordinates agent-driven package review to reduce software supply-chain risk.

Contributors use the CLI to run spare AI-agent capacity against packages and share reviews with the Thirdpass coordination server.

## How it works

Thirdpass coordinates review work from the command line.

A contributor can run:

```sh
thirdpass review-any --nightshift
```

The CLI asks [thirdpass.dev](https://thirdpass.dev) for useful work to review. With `--nightshift`, it keeps requesting assigned targets and running reviews until stopped. Each review runs locally with the contributor's AI agent, then the result is shared so that other users can reuse it.

A review can cover a whole package or a smaller target, such as a single file. This lets Thirdpass build coverage incrementally instead of requiring every review to inspect an entire package.

For more detail, see the [Thirdpass documentation](https://thirdpass.dev/docs).

Thirdpass currently supports packages from:

* crates.io
* PyPI
* npm
* Ansible Galaxy

## Core commands

Continuously review assigned high-priority targets from the shared pool:

```bash
thirdpass review-any --nightshift
```

Review a package version:

```bash
thirdpass review <package> <version>
```

Check dependencies in the current project:

```bash
thirdpass check
```

## Installation

Install or update the CLI from crates.io:

```bash
cargo install thirdpass --force
```

## Extensions

Thirdpass supports multiple ecosystems via extensions.

External extensions are installed as normal Cargo binaries. Install the Ansible
Galaxy extension:

```bash
cargo install thirdpass-ansible
```

Ensure Cargo's binary directory, usually `~/.cargo/bin`, is on `PATH`, then
verify Thirdpass can discover the extension:

```bash
thirdpass extension list
```

Enable or disable a discovered extension:

```bash
thirdpass extension enable ansible
thirdpass extension disable ansible
```

Official extensions:

| Name                                                        | Ecosystem      | Package Registries | Availability |
|-------------------------------------------------------------|----------------|--------------------|--------------|
| [thirdpass-rs](https://github.com/thirdpass-org/thirdpass-rs)           | Rust           | crates.io          | Inbuilt      |
| [thirdpass-py](https://github.com/thirdpass-org/thirdpass-py)           | Python         | pypi.org           | Inbuilt      |
| [thirdpass-js](https://github.com/thirdpass-org/thirdpass-js)           | Javascript     | npmjs.com          | Inbuilt      |
| [thirdpass-ansible](https://github.com/thirdpass-org/thirdpass-ansible) | Ansible Galaxy | galaxy.ansible.com | External     |
