# Obsidian CLI for Termux

CLI en Rust para vaults de Obsidian con sintaxis inspirada en la CLI oficial:

```bash
obsidian vault=Main files
obsidian read file=Inbox
obsidian append file=Inbox content="hola\nmundo"
obsidian search query="meeting notes" format=json
```

## Objetivo

- Mantener la gramática de la CLI oficial: `vault=<name>`, `param=value`, flags booleanos y modo interactivo.
- Ejecutar rápido en Termux sin depender de la app de Android.
- Ser usable por otros agentes con salidas de texto, TSV, CSV y JSON.

## Estado actual

- `local`: funciona sólo con el filesystem del vault.
- `hybrid`: opera sobre archivos/config local de `.obsidian` y deja espacio para bridge.
- `bridge`: registrado para compatibilidad, pero requiere un plugin/bridge de Obsidian todavía no implementado aquí.
- La TUI ya no es un REPL plano: ahora es una interfaz persistente con navegador de comandos, panel de salida, historial visual y barra de comandos.

Hoy están implementados los grupos locales principales:

- archivos y carpetas: `file`, `files`, `folder`, `folders`, `open`, `create`, `read`, `append`, `prepend`, `move`, `rename`, `delete`
- metadatos: `links`, `backlinks`, `unresolved`, `orphans`, `deadends`, `outline`, `tags`, `tag`, `tasks`, `task`, `aliases`, `properties`, `property:*`
- daily/templates/utilidades: `daily*`, `templates`, `template:*`, `random*`, `wordcount`, `recents`, `bases`
- config local del vault: `plugins`, `plugins:enabled`, `plugin`, `plugin:enable`, `plugin:disable`, `plugin:uninstall`, `themes`, `theme`, `theme:set`, `theme:uninstall`, `snippets`, `snippets:enabled`, `snippet:enable`, `snippet:disable`

Pendientes o parciales:

- `bridge` y parte de `hybrid`: sync, publish, workspace, devtools, installadores y cualquier comando que dependa del runtime vivo de Obsidian
- `bookmark`: lectura básica sí, escritura todavía no

## Limitaciones del corte local

Estos grupos siguen sin paridad completa con la CLI oficial porque dependen de una instancia viva de Obsidian o de APIs privadas/no expuestas:

- `sync`, `sync:*`
- `publish:*`
- `workspace`, `workspaces`, `workspace:*`, `tabs`, `tab:open`
- `devtools`, `dev:*`, `eval`
- `search:open`
- `plugin:install`, `plugin:reload`, `plugins:restrict`
- `theme:install`
- `bookmark` en modo escritura

En otras palabras: el backend local sirve para automatización fuerte sobre el vault y su configuración, pero la parte UI/runtime de Obsidian todavía requiere un bridge en mobile.

## Diseño

- Resolución de vault por `vault=<name>`, por directorio actual o por estado persistido.
- Índice incremental cacheado por vault para headings, tags, tasks, properties, aliases y wikilinks.
- TUI visual con navegador de comandos, sugerencias, historial persistente, scroll de salida y barra de comandos cuando se ejecuta `obsidian` sin subcomando.

## Build

```bash
cargo build --release
```

El binario queda como `target/release/obsidian`.

Build cruzada desde Windows a Android/Termux:

```powershell
./scripts/build-android-cross.ps1
```

Eso genera:

```text
target/aarch64-linux-android/release/obsidian
```

## Termux

Instalación nativa recomendada dentro de Termux:

```bash
git clone <repo>
cd <repo>
chmod +x scripts/*.sh
./scripts/setup-termux.sh
./scripts/build-termux.sh
```

Eso compila e instala la CLI en:

```text
$PREFIX/bin/obsidian
```

Instalación manual equivalente:

```bash
mkdir -p "$PREFIX/bin"
cp target/release/obsidian "$PREFIX/bin/obsidian"
chmod +x "$PREFIX/bin/obsidian"
```

La CLI intenta descubrir vaults desde `obsidian.json` y guarda su estado en:

```text
$XDG_CONFIG_HOME/obsidian-termux-cli
```

Si quieres forzar otra ubicación de configuración de Obsidian:

```bash
export OBSIDIAN_CONFIG_DIR=/ruta/a/obsidian
```

Notas:

- `rust-toolchain.toml` fija `stable` y el target `aarch64-linux-android` para entornos con `rustup`.
- En este workspace se verificó `cargo check --target aarch64-linux-android`.
- En este workspace sí se generó un binario Android AArch64 usando el NDK oficial en `.tooling/android-ndk-r29`.
