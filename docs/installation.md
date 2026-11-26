# Installation

The private repository is [gilboz/ametller-mcp](https://github.com/gilboz/ametller-mcp).
Install the Rust executable with Cargo or Homebrew, then add the small Codex
plugin. Login uses a hidden terminal password prompt and direct HTTPS requests.
No browser, Node.js, extension, or background service is required.

## Private GitHub access

Reuse existing GitHub authentication. On a new machine, `gh auth login` and
`gh auth setup-git` configure access for private HTTPS Git downloads. Never put
a token into a repository URL or the formula. Homebrew supports the GitHub CLI
credential helper for private Git sources; see [Homebrew's guidance](https://docs.brew.sh/FAQ#how-do-i-authenticate-private-git-source-downloads).

## Homebrew

```sh
brew tap gilboz/ametller-mcp https://github.com/gilboz/ametller-mcp.git
brew install gilboz/ametller-mcp/ametller-mcp
```

This repository is also the private tap. The formula builds `v0.1.0-beta.3` from
source with locked dependencies. It does not include a prebuilt bottle or sign
into Ametller during installation.

## Cargo

Requires Rust 1.100 or newer and a working platform linker:

```sh
CARGO_NET_GIT_FETCH_WITH_CLI=true cargo install \
  --git https://github.com/gilboz/ametller-mcp.git \
  --tag v0.1.0-beta.3 --locked ametller-mcp
```

From a trusted checkout, `cargo install --path . --locked` also works. The release
provides a standard `.crate` source archive: extract it, enter its directory and
run the same command. The package is private (`publish = false`) and is not
published on crates.io. Rust dependencies are downloaded on a fresh build.

## Sign in locally

Run this in your own terminal:

```sh
ametller-mcp login
```

The command prompts for your email and a hidden password in the terminal. It
signs in directly to Ametller over HTTPS using PKCE, verifies account access,
and saves only the session tokens. It does not open a browser. `login --password`
is accepted as a compatibility alias for the same flow. Never paste credentials
into chat or pass them as command arguments.

Session credentials use macOS Keychain by default on macOS. The explicit
`--credential-store file` backend uses owner-only files, not encryption, and is
the available backend on Linux. Keep the selected backend and `--state-dir`
consistent across CLI and MCP processes. `ametller-mcp status` reports local
connection status; `status --live` checks the saved session with Ametller.

Guest search/product details work without this step. Registered login and account
cart compatibility require a successful first sign-in with your account; mock
tests do not establish that. Existing credentials are kept until a new login has
passed account validation. Website challenges cannot be completed through the
CLI; an HTTP failure alone does not establish that a challenge is present.
Failures identify sign-in or account verification and preserve the HTTP status
without printing response bodies.

## Codex plugin

```sh
codex plugin marketplace add gilboz/ametller-mcp --ref main
codex plugin add ametller@ametller-mcp
```

Restart or refresh the client after installing. The plugin uses the supported
Codex compatibility layout: `.codex-plugin/plugin.json`, `.mcp.json`, a launcher,
and one setup skill. It contains no credentials. The Git marketplace is the
recommended install route; the release ZIP contains the same plugin folder.
See [official plugin packaging documentation](https://developers.openai.com/plugins/build/plugins).

The launcher checks PATH, `/opt/homebrew/bin`, `/usr/local/bin` and `~/.cargo/bin`.
Optional settings contain paths/backend choices only:

| Environment variable | Purpose |
| --- | --- |
| `AMETLLER_MCP_BIN` | Explicit installed executable path |
| `AMETLLER_MCP_STATE_DIR` | Same private state directory used for login |
| `AMETLLER_MCP_CREDENTIAL_STORE` | `file` or `keychain`, matching login |

The server starts and lists tools before account login. It never prompts for a
password through MCP, so an unconnected plugin does not hang on terminal input.
Photos are returned as official URLs with product metadata; actual rendering
depends on the client. No interactive shopping UI is bundled in this release.

For any local stdio MCP client, an equivalent connection is:

```json
{
  "mcpServers": {
    "ametller": {
      "command": "/absolute/path/to/ametller-mcp",
      "args": ["serve"]
    }
  }
}
```

## Updates and removal

Use `brew upgrade gilboz/ametller-mcp/ametller-mcp` for a newer formula, or repeat
Cargo installation with a newer release tag. Refresh the plugin source with
`codex plugin marketplace upgrade ametller-mcp` and restart the client.

`ametller-mcp logout` removes this installation's saved Ametller session.
`codex plugin remove ametller@ametller-mcp` removes the plugin, and `brew uninstall
ametller-mcp` or `cargo uninstall ametller-mcp` removes the executable. Removing
the executable/plugin does not automatically erase private application state.
