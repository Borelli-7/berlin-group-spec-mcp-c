#!/usr/bin/env bash
# Prints this checkout's config and merges its server into ~/.copilot/mcp-config.json.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
destination="${HOME}/.copilot/mcp-config.json"

if ! command -v jq >/dev/null 2>&1; then
	printf 'error: jq is required to merge the Copilot MCP configuration\n' >&2
	exit 1
fi

generated="$(sed -e "s#/absolute/path/berlin-group-spec-mcp#${root}#g" "${root}/examples/copilot/mcp-config.json")"
mkdir -p "$(dirname "$destination")"
temporary="$(mktemp "${destination}.XXXXXX")"
trap 'rm -f "$temporary"' EXIT

if [[ -f "$destination" ]]; then
	server="$(jq -c '.mcpServers["berlin-group-spec"]' <<<"$generated")"
	jq --argjson server "$server" \
		'.mcpServers = ((.mcpServers // {}) + {"berlin-group-spec": $server})' \
		"$destination" > "$temporary"
else
	printf '%s\n' "$generated" > "$temporary"
fi

chmod 600 "$temporary"
mv "$temporary" "$destination"
printf '%s\n' "$generated"
