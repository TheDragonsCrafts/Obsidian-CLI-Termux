#!/data/data/com.termux/files/usr/bin/bash
set -euo pipefail

if ! command -v pkg >/dev/null 2>&1; then
  echo "Este script está pensado para ejecutarse dentro de Termux." >&2
  exit 1
fi

if ! command -v cargo >/dev/null 2>&1 || ! command -v rustc >/dev/null 2>&1; then
  echo "Instala Rust en Termux primero: pkg install rust" >&2
  exit 1
fi

if ! command -v clang >/dev/null 2>&1; then
  echo "Instala clang en Termux primero: pkg install clang" >&2
  exit 1
fi

HOST_TARGET="$(rustc -vV | awk '/^host: / { print $2 }')"
case "$HOST_TARGET" in
  aarch64-linux-android|armv7-linux-androideabi|i686-linux-android|x86_64-linux-android)
    ;;
  *)
    echo "Target host no soportado para Termux: $HOST_TARGET" >&2
    exit 1
    ;;
esac

echo "Compilando para $HOST_TARGET"
cargo install --path . --locked --force --root "${PREFIX}"

echo
echo "Instalado en: ${PREFIX}/bin/obsidian"
echo "Prueba rápida:"
echo "  obsidian version"
echo "  obsidian help"
