# Obsidian CLI for Termux

CLI en Rust para vaults de Obsidian con sintaxis inspirada en la CLI oficial:

```bash
obsidian vault=Main files
obsidian read file=Inbox
obsidian append file=Inbox content="hola\nmundo"
obsidian search query="meeting notes" format=json
obsidian --no-update commands format=json
obsidian doctor
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

- diagnóstico e inventario: `doctor`, `commands`
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
- Índice incremental cacheado por vault para headings, tags, tasks, properties, aliases y wikilinks. Una lectura caliente no reescribe el cache si ningún archivo cambió y el grafo usa búsquedas indexadas por path/stem.
- TUI visual con navegador de comandos, sugerencias, historial persistente, scroll de salida y barra de comandos cuando se ejecuta `obsidian` sin subcomando.
- Las rutas de usuario se mantienen dentro del vault incluso ante `..`, rutas absolutas o enlaces simbólicos que apunten fuera.
- Las escrituras de notas, frontmatter y configuración reemplazan archivos atómicamente para evitar estados parciales.

## Uso por LLMs y scripts

Para agentes de terminal se recomienda `--agent`. Desactiva auto-update, selecciona JSON compacto, envuelve incluso resultados vacíos de forma estable y emite errores JSON por `stderr` con exit code distinto de cero. Los diagnósticos o lotes cuyo payload tenga `ok=false` terminan con código 2:

```bash
obsidian --agent --vault /ruta/al/vault files
obsidian --agent search --query="meeting notes" --vault Main
```

También se aceptan `--json`, `--format json`, `--vault Main` y parámetros convencionales `--query=texto`, además de la sintaxis compatible `format=json`, `vault=Main` y `query=texto`.

Para scripts que necesitan conservar la salida original, usa `--no-update`:

```bash
obsidian --no-update vault=/ruta/al/vault read file=Inbox
```

Comandos útiles para descubrir capacidades sin parsear ayuda humana:

```bash
obsidian --agent commands available
obsidian --no-update commands support=local format=json
obsidian --agent doctor deep
```

`commands` incluye `usage` y `aliases` por comando en sus salidas estructuradas, de modo que un agente puede descubrir la invocación correcta en una sola llamada.

Para evitar el costo de iniciar un proceso por operación, `batch` ejecuta varias líneas en la misma sesión. Devuelve JSONL por defecto o un resumen JSON cuando se usa `--agent`/`format=json`:

```bash
printf '%s\n' \
  'files ext=md total' \
  'search query="project alpha" limit=10 format=json' \
  'tasks todo format=json' \
  | obsidian --agent --vault Main batch

obsidian --agent --vault Main batch file=commands.txt fail-fast
```

Si acabas de crear o mover vaults y quieres ignorar el cache de descubrimiento:

```bash
obsidian --no-update vaults --refresh
obsidian --no-update vaults --refresh format=json
```

`doctor` revisa entorno Termux, rutas de runtime/cache, herramientas disponibles (`pkg`, `termux-open-url`, `termux-clipboard-set`, `cargo`, `rustc`, etc.), vault activo y vaults conocidos. Ya no reporta éxito incondicional: entrega `status`, conteos, checks y reparaciones concretas.

```bash
obsidian doctor                 # diagnóstico rápido
obsidian --agent doctor deep    # además carga y verifica el índice del vault
obsidian --agent doctor fix     # recrea runtime, refresca vaults y verifica/actualiza el índice activo
```


## Auto-update desde GitHub

Al iniciar, la CLI revisa cada 12 horas si existe una release estable más nueva. Por
defecto solo avisa: nunca descarga código fuente ni ejecuta `cargo install`. El
comando `update` descarga el binario precompilado exacto para Termux, verifica su
SHA-256 y reemplaza el ejecutable de forma atómica.

Variables útiles:

- `OBSIDIAN_CLI_GITHUB_REPO=<owner>/<repo>` para indicar el repositorio exacto.
- `OBSIDIAN_CLI_AUTO_UPDATE=0` para desactivar el auto-update.
- `OBSIDIAN_CLI_AUTO_UPDATE_APPLY=1` para permitir que el chequeo periódico aplique
  la actualización automáticamente. Sin esta variable solo notifica.
- `OBSIDIAN_CLI_UPDATE_PIN=vX.Y.Z` para instalar una release concreta.

También puedes forzar una actualización manual con:

```bash
obsidian update
obsidian update --force
```

`--force` vuelve a descargar y verificar el binario aunque la versión publicada
sea la misma.

Por defecto se usa `TheDragonsCrafts/Obsidian-CLI-Termux` si no se define `OBSIDIAN_CLI_GITHUB_REPO`.

Si no existe una release o no contiene un asset compatible, el binario instalado
se conserva sin cambios. Actualmente se publican assets para Termux AArch64 y
x86_64.

## Releases

Cada push a `master` actualiza una PR de release mediante Release Please. Los
títulos de PR/commits deben seguir Conventional Commits (`feat:`, `fix:`, etc.).
Al fusionar esa PR se actualizan `Cargo.toml`, `Cargo.lock` y `CHANGELOG.md`, se
crea el tag SemVer y GitHub Actions adjunta los binarios Android junto con sus
checksums. Las releases no se sobrescriben con código de `master` sin versionar.

## Build

```bash
cargo build --release
```

El binario queda como `target/release/obsidian`.

Antes de enviar cambios, ejecuta las mismas comprobaciones principales del CI:

```bash
cargo fmt --all -- --check
cargo clippy --locked --all-targets --all-features -- -D warnings
cargo test --locked --all-features
```

El workflow valida además Linux y Windows, la sintaxis de los scripts de instalación y el target AArch64 usado por Termux con el NDK. Para reproducir este último check desde Windows, usa `./scripts/build-android-cross.ps1` después de instalar el NDK como se indica abajo.

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

La CLI intenta descubrir vaults desde `obsidian.json` **y también escanea rutas típicas de Documents en Android/Termux** (por ejemplo `~/storage/shared/Documents`, `/storage/emulated/0/Documents`) para que funcione globalmente sin estar parado dentro del vault. Guarda su estado en:

```text
$XDG_CONFIG_HOME/obsidian-termux-cli
```

Si quieres forzar otra ubicación de configuración de Obsidian:

```bash
export OBSIDIAN_CONFIG_DIR=/ruta/a/obsidian
```
Si quieres añadir una carpeta extra para auto-descubrir vaults:

```bash
export OBSIDIAN_VAULTS_DIR=/ruta/a/Documents
```


Notas:

- `rust-toolchain.toml` fija `stable`, el target `aarch64-linux-android` y el componente `rustfmt` para que `cargo fmt` esté disponible en entornos con `rustup`.
- En este workspace se verificó `cargo check --target aarch64-linux-android`.
- En este workspace sí se generó un binario Android AArch64 usando el NDK oficial en `.tooling/android-ndk-r29`.


## Licencia

Este proyecto se distribuye bajo la licencia MIT. Consulta el archivo `LICENSE`.
