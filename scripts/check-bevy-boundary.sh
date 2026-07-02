#!/usr/bin/env bash
# Architectural boundary (specs §1.4/§2.4, CONSTITUTION-DEV Rule 2): the engine
# crates NEVER depend on Bevy — Bevy's version churn must only expose the `app`
# crate. Run in CI; usable locally: ./scripts/check-bevy-boundary.sh
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

crates=(engine decode analysis midi mapping)
status=0
for crate in "${crates[@]}"; do
  tree=$(cargo tree -p "$crate" -e normal,build)
  if grep -qi bevy <<<"$tree"; then
    echo "ERROR: crate \"$crate\" depends on Bevy:" >&2
    grep -i bevy <<<"$tree" >&2
    status=1
  fi
done

if [ "$status" -eq 0 ]; then
  echo "OK: no Bevy dependency in: ${crates[*]}"
fi
exit "$status"
