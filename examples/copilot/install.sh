#!/usr/bin/env bash
# Prints (does not write) a Copilot CLI MCP configuration with absolute paths for this checkout.
# Usage: examples/copilot/install.sh > /tmp/bg-spec.json   then merge into ~/.copilot/mcp-config.json
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
sed -e "s#/absolute/path/berlin-group-spec-mcp#${root}#g" "${root}/examples/copilot/mcp-config.json"
