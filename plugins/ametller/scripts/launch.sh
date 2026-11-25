#!/bin/sh
set -eu

launch() {
  server_binary=$1
  set --
  if [ -n "${AMETLLER_MCP_STATE_DIR:-}" ]; then
    set -- "$@" --state-dir "$AMETLLER_MCP_STATE_DIR"
  fi
  if [ -n "${AMETLLER_MCP_CREDENTIAL_STORE:-}" ]; then
    case "$AMETLLER_MCP_CREDENTIAL_STORE" in
      file|keychain) set -- "$@" --credential-store "$AMETLLER_MCP_CREDENTIAL_STORE" ;;
      *) printf '%s\n' 'AMETLLER_MCP_CREDENTIAL_STORE must be file or keychain.' >&2; exit 126 ;;
    esac
  fi
  exec "$server_binary" "$@" serve
}

if [ -n "${AMETLLER_MCP_BIN:-}" ]; then
  if [ ! -x "$AMETLLER_MCP_BIN" ] || [ -d "$AMETLLER_MCP_BIN" ]; then
    printf '%s\n' 'AMETLLER_MCP_BIN must name an executable ametller-mcp file.' >&2
    exit 126
  fi
  launch "$AMETLLER_MCP_BIN"
fi

if command -v ametller-mcp >/dev/null 2>&1; then
  launch "$(command -v ametller-mcp)"
fi

for server_candidate in /opt/homebrew/bin/ametller-mcp /usr/local/bin/ametller-mcp "${HOME}/.cargo/bin/ametller-mcp"; do
  if [ -x "$server_candidate" ] && [ ! -d "$server_candidate" ]; then
    launch "$server_candidate"
  fi
done

printf '%s\n' \
  'Install ametller-mcp with Homebrew or Cargo, then restart this plugin.' \
  'brew tap gilboz/ametller-mcp https://github.com/gilboz/ametller-mcp.git' \
  'brew install gilboz/ametller-mcp/ametller-mcp' \
  'Private GitHub access is required. Instructions: https://github.com/gilboz/ametller-mcp/blob/main/docs/installation.md' >&2
exit 127
