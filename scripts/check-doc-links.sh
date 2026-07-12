#!/usr/bin/env bash

set -euo pipefail

status=0

while IFS=: read -r file line target; do
    case "$target" in
        ""|\#*|http://*|https://*|mailto:*|app://*)
            continue
            ;;
    esac

    path="${target%%#*}"
    if [[ ! -e "$(dirname "$file")/$path" ]]; then
        printf '%s:%s: broken relative link: %s\n' "$file" "$line" "$target" >&2
        status=1
    fi
done < <(
    rg --line-number --with-filename --only-matching \
        --replace '$1' '\]\(([^)]*)\)' README.md docs
)

if [[ "$status" -eq 0 ]]; then
    echo "documentation links: ok"
fi

exit "$status"
