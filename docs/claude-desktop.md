# Claude desktop extension

The release includes `ametller-0.1.0-beta.4-macos-arm64.mcpb`, a self-contained
extension for **Claude Desktop on Apple Silicon Macs** (M1 or later). It bundles
the compiled Rust server and needs no Homebrew, Node, Python, or browser to run.
It is a local extension; it does not deploy a cloud server or enable Claude web
or mobile. Intel Macs, Windows and Linux are not supported by this bundle.

## Install in Claude

1. Download the `.mcpb` asset from the private
   [beta.4 release](https://github.com/gilboz/ametller-mcp/releases/tag/v0.1.0-beta.4).
   Sign into GitHub with access to the repository first.
2. Double-click the file and choose **Install** in Claude. Alternatively, open
   **Settings → Desktop app → Extensions → Manage → Advanced settings → Install
   Extension…**, then select the file. Older Claude versions use **Settings →
   Extensions → Advanced settings**.
3. Set the catalogue language to `es` (Spanish) or `ca` (Catalan), and enable
   Ametller. No password or token belongs in the extension settings.
4. In a new chat, ask: “Show me Greek yogurts with photos and prices.”

The package is privately distributed and not listed in Anthropic's extension
directory. Install only the bundle built from this repository; it is not signed
with an Apple Developer ID or an extension publisher certificate. If a managed
Claude installation blocks custom/unsigned extensions, use its approved
administrator workflow; do not disable the policy.

These steps follow Anthropic's
[custom extension installation instructions](https://support.claude.com/en/articles/10949351-getting-started-with-local-mcp-servers-on-claude-desktop).

## Reuse your existing login

If `ametller-mcp status --live` already succeeds with the default settings on
this Mac, no new login is needed. Claude and the Homebrew/Codex installation
share the same application state and macOS Keychain entry. Keychain may ask you
to allow the bundled executable to access that existing entry.

Guest product searches work before sign-in. Cart reads and changes require your
registered account. When already installed through Homebrew or Cargo, sign in
privately with `ametller-mcp login` in your own Terminal.

For a fresh Mac without the CLI installed, the bundle includes a terminal login
helper. Extract the downloaded bundle into a directory you choose and run it:

```sh
unzip "$HOME/Downloads/ametller-0.1.0-beta.4-macos-arm64.mcpb" -d "$HOME/ametller-login-beta4"
"$HOME/ametller-login-beta4/login.command"
```

The helper prompts for email and a hidden password. It saves only session tokens
in Keychain, with account metadata in the standard private application directory.
The extracted folder can be deleted after successful login; account state lives
outside that folder and the Claude extension cache. Never paste credentials into
Claude or put them in command arguments.

The bundle uses the default state directory and Keychain backend. If your CLI
uses a custom `--state-dir` or the file backend, it is a separate configuration;
do not copy credential files into the extension. Use a matching manual stdio
configuration or sign in to the default Keychain-backed installation.

## Photos and shopping

The MCP returns real, bounded product thumbnails as standard MCP `image` content
blocks, alongside product names, prices, pack sizes and original photo URLs.
It does not rely solely on Markdown links to remote JPEGs. Presentation of image
blocks still depends on the host; these are not a custom carousel or shopping UI.
Unavailable photos leave the product details usable.

Only explicit product/cart instructions authorize changes. The server reads back
the resulting cart, and never places orders or makes payments. Prices are
estimates; delivery and checkout remain on Ametller's official website.

## Updates and removal

Download and install the newer `.mcpb` to update this private extension. It does
not automatically track Homebrew or the Codex plugin version. Remove it through
Claude's extension settings. Removal does not erase your shared Ametller login.
`ametller-mcp logout` explicitly removes the saved local session for all clients
using that same state directory; it does not revoke the session at Ametller.

## Build and check the bundle

On an Apple Silicon Mac with Rust 1.100+ and Python 3.11+:

```sh
cargo build --release --locked
python3 scripts/package_mcpb.py --binary target/release/ametller-mcp
python3 tests/check_mcpb.py --binary target/release/ametller-mcp
```

The packager uses an explicit six-file inventory and rejects non-arm64 Mach-O
executables or mismatched versions. The test extracts to a path containing
spaces, starts the exact manifest command with isolated test storage, checks all
five tools and missing-session behavior, and exercises the terminal-only helper.
No credentials, account state, research sources or build toolchains are bundled.
The format follows the upstream
[MCPB manifest specification](https://github.com/modelcontextprotocol/mcpb/blob/main/MANIFEST.md).
