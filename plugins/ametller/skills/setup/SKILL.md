---
name: setup
description: Set up or troubleshoot the local Ametller grocery MCP and private CLI sign-in. Use for a missing server, initial account connection, or an authentication error.
---

This plugin runs the separately installed `ametller-mcp` executable. Guest product search works without account sign-in. Reading or changing the user's real cart requires a registered session.

Check `ametller-mcp --version` and `ametller-mcp status`. If the desktop PATH is incomplete, the launcher also checks Homebrew's standard paths and `~/.cargo/bin`; `AMETLLER_MCP_BIN` can select an explicit executable.

For an installation request, use the user's preferred package manager. The private repository is also the Homebrew tap:

```sh
brew tap gilboz/ametller-mcp https://github.com/gilboz/ametller-mcp.git
brew install gilboz/ametller-mcp/ametller-mcp
```

Reuse existing GitHub access. `gh auth login` and `gh auth setup-git` are only needed if private Git downloads are not already configured. Cargo installation is documented in the repository's `docs/installation.md`.

For missing or expired Ametller authentication, have the user run `ametller-mcp login`. It opens an isolated Chrome window on Ametller's official website. The user signs in and completes any challenge there; the Rust helper captures the resulting session locally, closes its temporary browser, validates the account and saves the credentials. Closing the window or Ctrl-C cancels it. Never ask the user to paste a session token, password, cookies or HAR into chat, MCP arguments or command arguments. Retry the original requested operation after sign-in.

`ametller-mcp login --password` retains direct API login with a hidden terminal password prompt. Use it only when the user prefers it; it cannot complete website challenges. A non-redirect HTTP response is not automatically a CAPTCHA or incorrect password. Preserve the HTTP status and avoid repeated login attempts. Browser login uses installed Chrome; missing Chrome is reported without automatically installing a browser.

The login command and plugin must use the same state directory and credential backend. The launcher's optional `AMETLLER_MCP_STATE_DIR` and `AMETLLER_MCP_CREDENTIAL_STORE` select those settings. Credentials use macOS Keychain by default on macOS; file storage is explicit there and uses private permissions, not encryption. Never move another account's credentials into this setup or overwrite rotated tokens with older captures.

For shopping, show names, official photos/links, pack sizes, quantity units and price context from the tools. Confirm the user's chosen products when their request is ambiguous; suggestions alone do not authorize adding them. Explicit requests to change chosen items can be applied directly. Show the returned cart after changes, preserving estimated weight/price and delivery limitations. Never describe a cart change as a placed order: checkout/payment remains on Ametller's official site.
