# apkg

Package manager for AI tooling — skills, agents, commands, and rules for Claude Code, Cursor, Codex, and other AI coding assistants.

`apkg` brings the `npm`/`cargo` workflow to AI developer tooling: a manifest, a lockfile, a registry, semver-resolved dependencies, signed releases, and reproducible setup across the tools your team actually uses.

## Install

### One-line installer (macOS, Linux)

```sh
curl -fsSL https://raw.githubusercontent.com/apkg-ai/cli/main/install.sh | sh
```

Detects your platform, verifies the SHA-256 checksum, installs to `~/.apkg/bin/apkg`, and adds it to your `PATH`. Options:

- `APKG_VERSION=v0.1.0 sh install.sh` — pin a specific version.
- `sh install.sh --install-dir ~/bin` — install to a custom directory (skips shell RC edits).
- `sh install.sh --no-modify-path` — install without touching any shell RC file.

### Manual download

Grab a tarball from the [releases page](https://github.com/apkg-ai/cli/releases/latest):

| Platform | Asset |
|---|---|
| macOS (Apple Silicon) | `apkg-darwin-arm64.tar.gz` |
| macOS (Intel) | `apkg-darwin-amd64.tar.gz` |
| Linux (x86_64) | `apkg-linux-amd64.tar.gz` |
| Linux (arm64) | `apkg-linux-arm64.tar.gz` |
| Windows (x86_64) | `apkg-windows-amd64.zip` |
| Windows (arm64) | `apkg-windows-arm64.zip` |

Each asset ships with a `.sha256` checksum. After extracting, add `apkg` to your `PATH` (or run `apkg add-to-path`).

### From source

```sh
git clone https://github.com/apkg-ai/cli.git
cd cli
cargo install --path .
```

## Quick start

```sh
# Create a manifest in the current directory
apkg init

# Log in to the registry
apkg login

# Add a dependency — resolves, downloads, and wires it into your AI tool of choice
apkg add @acme/review-skill

# Install everything from apkg.json (CI-friendly; use --frozen-lockfile to fail on drift)
apkg install --frozen-lockfile

# Publish your package
apkg publish
```

## Commands

### Project

- `apkg init` — create `apkg.json` interactively
- `apkg add <pkg>[@version]` — add and install a dependency (`-D` for dev, `-P` for peer)
- `apkg remove <pkg>` — remove a dependency
- `apkg install [<pkg>]` — install one package or everything from `apkg.json`
- `apkg update [<pkg>]` — bump within semver ranges (`--latest` to ignore ranges, `--dry-run` to preview)
- `apkg info <pkg>` — show package metadata
- `apkg search <query>` — search the registry

### Packaging & publishing

- `apkg pack` — build a `.tar.zst` tarball of the current package
- `apkg publish` — pack and upload to the registry
- `apkg deprecate <pkg>[@version] "message"` — mark a version deprecated (`--undo` to revert)
- `apkg dist-tag add|rm|ls` — manage distribution tags (`latest`, `beta`, `canary`, …)

### Authentication & keys

- `apkg login` / `apkg logout` / `apkg whoami`
- `apkg token create|list|revoke` — long-lived tokens for CI
- `apkg key generate|list|register|revoke|rotate` — Ed25519 signing keys

### Verification & local dev

- `apkg verify [<pkg>]` — verify signatures and integrity (`--strict` exits non-zero on unverified)
- `apkg link [<target>]` — symlink a local package into `apkg_packages/` (or register it globally)
- `apkg unlink [<pkg>]` — the inverse

### CLI tooling

- `apkg cache clean|list|verify` — manage the global package cache
- `apkg config set|get|list|delete` — CLI configuration (registry URL, service URLs, default setup targets)
- `apkg completions <shell>` — generate shell completion scripts
- `apkg add-to-path` — symlink `apkg` onto your `PATH`
- `apkg check-update` — check whether a newer release is available

Run `apkg --help` or `apkg <command> --help` for full flag details.

## Configuration

`apkg` reads configuration from several sources (in decreasing precedence):

1. Per-command flag: `--registry <url>`, `--offline`
2. Environment variables: `APKG_REGISTRY`, `APKG_TOKEN`, `APKG_CACHE_DIR`, `APKG_OFFLINE`, `APKG_NO_METADATA_CACHE`, `APKG_METADATA_TTL_SECS`, `APKG_MAX_CONCURRENT_DOWNLOADS`, `APKG_NO_PROXY`
3. `~/.apkg/settings.json` (managed via `apkg config`)
4. Compiled-in defaults

Credentials are stored in `~/.apkg/credentials.json` after `apkg login`.

### HTTP proxy

`apkg` honors the standard `HTTP_PROXY`, `HTTPS_PROXY`, and `NO_PROXY`
environment variables via reqwest's built-in system proxy detection — no
CLI flag or config required. Proxy credentials use the conventional
`http://user:pass@host:port` form.

Set `APKG_NO_PROXY=1` to opt out of proxy detection entirely. Useful in
CI or sandboxed environments where an inherited proxy setting shouldn't
apply to apkg's own traffic.

## Supported AI tools

`apkg add` and `apkg install` automatically wire installed packages into the AI tools it detects in your project. Target a specific tool with `--setup <tool>`:

- `claude-code`
- `cursor`
- `codex`

Skip the auto-setup with `--no-setup`.

## Development

```sh
# Build
cargo build

# Run the full test suite
cargo test

# Unit-test coverage (gate at 85%)
cargo llvm-cov --bins --summary-only --fail-under-lines 85 --fail-under-regions 85 --fail-under-functions 85

# Lints and formatting
cargo fmt --all -- --check
cargo clippy --all-targets

# Security advisories
cargo audit
```

CI runs formatting, clippy, tests, coverage, and `cargo audit` on every pull request (`.github/workflows/quality-gates.yml`). Releases are built for six target triples and attached to GitHub Releases automatically (`.github/workflows/release-assets.yml`).

## License

MIT — see [LICENSE](LICENSE).
