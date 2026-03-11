#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail

if ! command -v pkg >/dev/null 2>&1; then
  echo "Este script está pensado para ejecutarse dentro de Termux." >&2
  exit 1
fi

WITH_API=0
if [[ "${1:-}" == "--with-api" ]]; then
  WITH_API=1
fi

pkg update -y
pkg install -y rust clang pkg-config git

if command -v rustup >/dev/null 2>&1; then
  rustup component add rustfmt
else
  pkg install -y rustfmt || true
fi

if [[ "$WITH_API" -eq 1 ]]; then
  pkg install -y termux-api
fi

echo
echo "Dependencias base instaladas (incluyendo rustfmt para \`cargo fmt\`)."
echo "Siguiente paso:"
echo "  ./scripts/build-termux.sh"
