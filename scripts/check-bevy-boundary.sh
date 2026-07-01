#!/usr/bin/env bash
# Frontière architecturale (specs §1.4/§2.4) : les crates moteur ne dépendent
# JAMAIS de Bevy — le churn de version Bevy ne doit exposer que la crate `app`.
# Exécuté en CI ; utilisable en local : ./scripts/check-bevy-boundary.sh
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."

crates=(engine decode analysis midi mapping)
status=0
for crate in "${crates[@]}"; do
  tree=$(cargo tree -p "$crate" -e normal,build)
  if grep -qi bevy <<<"$tree"; then
    echo "ERREUR : la crate « $crate » dépend de Bevy :" >&2
    grep -i bevy <<<"$tree" >&2
    status=1
  fi
done

if [ "$status" -eq 0 ]; then
  echo "OK : aucune dépendance Bevy dans : ${crates[*]}"
fi
exit "$status"
