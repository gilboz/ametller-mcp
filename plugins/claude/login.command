#!/bin/sh
set -eu
extension_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
exec "$extension_dir/server/ametller-mcp" login
