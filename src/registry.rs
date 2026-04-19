use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportLevel {
    Local,
    Hybrid,
    BridgeOnly,
}

impl SupportLevel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Hybrid => "hybrid",
            Self::BridgeOnly => "bridge",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct CommandSpec {
    pub name: &'static str,
    pub category: &'static str,
    pub summary: &'static str,
    pub support: SupportLevel,
}

pub const COMMANDS: &[CommandSpec] = &[
    spec(
        "help",
        "General",
        "Muestra ayuda general o de un comando",
        SupportLevel::Local,
    ),
    spec(
        "version",
        "General",
        "Versión del binario y perfil de compatibilidad",
        SupportLevel::Local,
    ),
    spec(
        "update",
        "General",
        "Ejecuta actualización manual del binario",
        SupportLevel::Local,
    ),
    spec(
        "language",
        "General",
        "Consulta o cambia el idioma de la CLI/TUI",
        SupportLevel::Local,
    ),
    spec(
        "reload",
        "General",
        "Recarga la app de Obsidian",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "restart",
        "General",
        "Reinicia la app de Obsidian",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "vault",
        "Vault",
        "Información del vault actual",
        SupportLevel::Local,
    ),
    spec(
        "vaults",
        "Vault",
        "Lista vaults conocidos",
        SupportLevel::Local,
    ),
    spec(
        "vault:open",
        "Vault",
        "Cambia el vault activo",
        SupportLevel::Local,
    ),
    spec(
        "vault:init",
        "Vault",
        "Inicializa un vault creando `.obsidian`",
        SupportLevel::Local,
    ),
    spec(
        "file",
        "Archivos",
        "Información del archivo objetivo",
        SupportLevel::Local,
    ),
    spec(
        "files",
        "Archivos",
        "Lista archivos del vault",
        SupportLevel::Local,
    ),
    spec(
        "folder",
        "Archivos",
        "Información de una carpeta",
        SupportLevel::Local,
    ),
    spec(
        "folders",
        "Archivos",
        "Lista carpetas del vault",
        SupportLevel::Local,
    ),
    spec(
        "open",
        "Archivos",
        "Marca o abre un archivo como activo",
        SupportLevel::Local,
    ),
    spec("create", "Archivos", "Crea un archivo", SupportLevel::Local),
    spec("read", "Archivos", "Lee un archivo", SupportLevel::Local),
    spec(
        "append",
        "Archivos",
        "Agrega contenido al final",
        SupportLevel::Local,
    ),
    spec(
        "prepend",
        "Archivos",
        "Agrega contenido al inicio útil del archivo",
        SupportLevel::Local,
    ),
    spec(
        "move",
        "Archivos",
        "Mueve o renombra un archivo",
        SupportLevel::Local,
    ),
    spec(
        "rename",
        "Archivos",
        "Renombra un archivo",
        SupportLevel::Local,
    ),
    spec(
        "delete",
        "Archivos",
        "Elimina o manda a trash un archivo",
        SupportLevel::Local,
    ),
    spec(
        "links",
        "Enlaces",
        "Lista links salientes del archivo",
        SupportLevel::Local,
    ),
    spec(
        "backlinks",
        "Enlaces",
        "Lista backlinks del archivo",
        SupportLevel::Local,
    ),
    spec(
        "unresolved",
        "Enlaces",
        "Lista links no resueltos",
        SupportLevel::Local,
    ),
    spec(
        "orphans",
        "Enlaces",
        "Lista notas sin backlinks",
        SupportLevel::Local,
    ),
    spec(
        "deadends",
        "Enlaces",
        "Lista notas sin links salientes",
        SupportLevel::Local,
    ),
    spec(
        "outline",
        "Enlaces",
        "Muestra headings del archivo",
        SupportLevel::Local,
    ),
    spec(
        "daily",
        "Diario",
        "Abre o crea la daily note de hoy",
        SupportLevel::Local,
    ),
    spec(
        "daily:path",
        "Diario",
        "Devuelve el path esperado de la daily note",
        SupportLevel::Local,
    ),
    spec(
        "daily:read",
        "Diario",
        "Lee la daily note de hoy",
        SupportLevel::Local,
    ),
    spec(
        "daily:append",
        "Diario",
        "Agrega contenido a la daily note",
        SupportLevel::Local,
    ),
    spec(
        "daily:prepend",
        "Diario",
        "Prepend a la daily note",
        SupportLevel::Local,
    ),
    spec(
        "search",
        "Búsqueda",
        "Busca texto dentro del vault",
        SupportLevel::Local,
    ),
    spec(
        "search:context",
        "Búsqueda",
        "Busca texto y devuelve contexto tipo grep",
        SupportLevel::Local,
    ),
    spec(
        "search:open",
        "Búsqueda",
        "Abre la vista Search en la app",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "tags",
        "Metadatos",
        "Lista tags del vault o archivo",
        SupportLevel::Local,
    ),
    spec(
        "tag",
        "Metadatos",
        "Información de un tag",
        SupportLevel::Local,
    ),
    spec("tasks", "Metadatos", "Lista tareas", SupportLevel::Local),
    spec(
        "task",
        "Metadatos",
        "Muestra o actualiza una tarea",
        SupportLevel::Local,
    ),
    spec("aliases", "Metadatos", "Lista aliases", SupportLevel::Local),
    spec(
        "properties",
        "Metadatos",
        "Lista propiedades",
        SupportLevel::Local,
    ),
    spec(
        "property:set",
        "Metadatos",
        "Establece una propiedad",
        SupportLevel::Local,
    ),
    spec(
        "property:remove",
        "Metadatos",
        "Elimina una propiedad",
        SupportLevel::Local,
    ),
    spec(
        "property:read",
        "Metadatos",
        "Lee una propiedad",
        SupportLevel::Local,
    ),
    spec(
        "templates",
        "Plantillas",
        "Lista templates",
        SupportLevel::Local,
    ),
    spec(
        "template:read",
        "Plantillas",
        "Lee un template",
        SupportLevel::Local,
    ),
    spec(
        "template:insert",
        "Plantillas",
        "Inserta un template en el archivo activo",
        SupportLevel::Local,
    ),
    spec(
        "bases",
        "Bases",
        "Lista archivos .base",
        SupportLevel::Local,
    ),
    spec(
        "base:views",
        "Bases",
        "Lista vistas declaradas en una base",
        SupportLevel::Hybrid,
    ),
    spec(
        "base:create",
        "Bases",
        "Crea un item en una base",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "base:query",
        "Bases",
        "Consulta una base",
        SupportLevel::Hybrid,
    ),
    spec(
        "bookmarks",
        "Bookmarks",
        "Lista bookmarks",
        SupportLevel::Hybrid,
    ),
    spec(
        "bookmark",
        "Bookmarks",
        "Agrega un bookmark",
        SupportLevel::Hybrid,
    ),
    spec(
        "plugins",
        "Plugins",
        "Lista plugins instalados",
        SupportLevel::Hybrid,
    ),
    spec(
        "plugins:enabled",
        "Plugins",
        "Lista plugins habilitados",
        SupportLevel::Hybrid,
    ),
    spec(
        "plugins:restrict",
        "Plugins",
        "Cambia restricted mode",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "plugin",
        "Plugins",
        "Información de un plugin",
        SupportLevel::Hybrid,
    ),
    spec(
        "plugin:enable",
        "Plugins",
        "Habilita un plugin",
        SupportLevel::Hybrid,
    ),
    spec(
        "plugin:disable",
        "Plugins",
        "Deshabilita un plugin",
        SupportLevel::Hybrid,
    ),
    spec(
        "plugin:install",
        "Plugins",
        "Instala un community plugin",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "plugin:uninstall",
        "Plugins",
        "Desinstala un plugin",
        SupportLevel::Hybrid,
    ),
    spec(
        "plugin:reload",
        "Plugins",
        "Recarga un plugin",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "themes",
        "Apariencia",
        "Lista temas instalados",
        SupportLevel::Hybrid,
    ),
    spec(
        "theme",
        "Apariencia",
        "Información del tema activo o uno concreto",
        SupportLevel::Hybrid,
    ),
    spec(
        "theme:set",
        "Apariencia",
        "Activa un tema",
        SupportLevel::Hybrid,
    ),
    spec(
        "theme:install",
        "Apariencia",
        "Instala un tema",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "theme:uninstall",
        "Apariencia",
        "Desinstala un tema",
        SupportLevel::Hybrid,
    ),
    spec(
        "snippets",
        "Apariencia",
        "Lista snippets",
        SupportLevel::Hybrid,
    ),
    spec(
        "snippets:enabled",
        "Apariencia",
        "Lista snippets habilitados",
        SupportLevel::Hybrid,
    ),
    spec(
        "snippet:enable",
        "Apariencia",
        "Habilita un snippet",
        SupportLevel::Hybrid,
    ),
    spec(
        "snippet:disable",
        "Apariencia",
        "Deshabilita un snippet",
        SupportLevel::Hybrid,
    ),
    spec(
        "random",
        "Utilidades",
        "Selecciona una nota aleatoria",
        SupportLevel::Local,
    ),
    spec(
        "random:read",
        "Utilidades",
        "Lee una nota aleatoria",
        SupportLevel::Local,
    ),
    spec(
        "diff",
        "Historial",
        "Compara versiones",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "history",
        "Historial",
        "Lista versiones de file recovery",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "history:list",
        "Historial",
        "Lista versiones de file recovery",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "history:read",
        "Historial",
        "Lee una versión de file recovery",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "history:restore",
        "Historial",
        "Restaura una versión de file recovery",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "history:open",
        "Historial",
        "Abre una versión histórica",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "sync",
        "Sync",
        "Pausa o reanuda Obsidian Sync",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "sync:status",
        "Sync",
        "Estado de Sync",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "sync:history",
        "Sync",
        "Lista historial de Sync",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "sync:read",
        "Sync",
        "Lee una versión desde Sync",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "sync:restore",
        "Sync",
        "Restaura una versión desde Sync",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "sync:open",
        "Sync",
        "Abre una versión desde Sync",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "sync:deleted",
        "Sync",
        "Lista borrados en Sync",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "publish:site",
        "Publish",
        "Información del sitio Publish",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "publish:list",
        "Publish",
        "Lista archivos publicados",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "publish:status",
        "Publish",
        "Muestra cambios pendientes de Publish",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "publish:add",
        "Publish",
        "Publica un archivo",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "publish:remove",
        "Publish",
        "Despublica un archivo",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "publish:open",
        "Publish",
        "Abre un archivo publicado",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "workspace",
        "Espacio de trabajo",
        "Árbol del layout actual",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "workspaces",
        "Espacio de trabajo",
        "Lista workspaces guardados",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "workspace:save",
        "Espacio de trabajo",
        "Guarda un workspace",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "workspace:load",
        "Espacio de trabajo",
        "Carga un workspace",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "workspace:delete",
        "Espacio de trabajo",
        "Elimina un workspace",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "tabs",
        "Espacio de trabajo",
        "Lista tabs abiertos",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "tab:open",
        "Espacio de trabajo",
        "Abre una tab",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "recents",
        "Espacio de trabajo",
        "Lista recientes registrados por la CLI",
        SupportLevel::Local,
    ),
    spec(
        "web",
        "Espacio de trabajo",
        "Abre una URL en el viewer",
        SupportLevel::Hybrid,
    ),
    spec(
        "wordcount",
        "Utilidades",
        "Cuenta palabras y caracteres",
        SupportLevel::Local,
    ),
    spec(
        "devtools",
        "Developer",
        "Toggle de devtools",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "dev:debug",
        "Developer",
        "Attach o detach del debugger",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "dev:cdp",
        "Developer",
        "Ejecuta un método CDP",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "dev:errors",
        "Developer",
        "Errores JS capturados",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "dev:screenshot",
        "Developer",
        "Captura pantalla",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "dev:console",
        "Developer",
        "Mensajes de consola",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "dev:css",
        "Developer",
        "Inspección de CSS",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "dev:dom",
        "Developer",
        "Query del DOM",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "dev:mobile",
        "Developer",
        "Toggle de mobile emulation",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "eval",
        "Developer",
        "Ejecuta JavaScript en la app",
        SupportLevel::BridgeOnly,
    ),
];

pub fn find(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS.iter().find(|spec| spec.name == name)
}

pub fn overview(language: &str) -> String {
    let mut grouped: BTreeMap<&str, Vec<&CommandSpec>> = BTreeMap::new();
    for spec in COMMANDS {
        grouped.entry(spec.category).or_default().push(spec);
    }

    let mut out = String::new();
    if language == "en" {
        out.push_str("Obsidian CLI for Termux (Rust)\n");
        out.push_str(
            "Syntax compatibility: `vault=<name>` + command + `param=value` + boolean flags.\n",
        );
        out.push_str("Support: [local] works without app, [hybrid] mixes files/config and future bridge, [bridge] requires an Obsidian plugin/bridge.\n\n");
    } else {
        out.push_str("Obsidian CLI para Termux (Rust)\n");
        out.push_str(
            "Compatibilidad de sintaxis: `vault=<name>` + comando + `param=value` + flags booleanos.\n",
        );
        out.push_str("Soporte: [local] funciona sin app, [hybrid] mezcla archivos/config y futuro bridge, [bridge] requiere plugin/bridge en Obsidian.\n\n");
    }

    for (category, specs) in grouped {
        out.push_str(&localize_category(category, language));
        out.push('\n');
        for spec in specs {
            out.push_str("  ");
            out.push_str(spec.name);
            out.push_str(" [");
            out.push_str(spec.support.label());
            out.push_str("] ");
            out.push_str(spec.summary);
            out.push('\n');
        }
        out.push('\n');
    }

    out.trim_end().to_string()
}

pub fn command_help(name: &str, language: &str) -> Option<String> {
    let spec = find(name)?;
    Some(format!(
        "{name}\n{}: {}\n{}: [{}]\n{}: {}",
        if language == "en" {
            "category"
        } else {
            "categoría"
        },
        localize_category(spec.category, language),
        if language == "en" {
            "support"
        } else {
            "soporte"
        },
        spec.support.label(),
        if language == "en" {
            "summary"
        } else {
            "resumen"
        },
        spec.summary
    ))
}

pub fn localize_category(category: &str, language: &str) -> String {
    if language != "en" {
        return category.to_string();
    }
    match category {
        "General" => "General",
        "Vault" => "Vault",
        "Archivos" => "Files",
        "Enlaces" => "Links",
        "Diario" => "Daily",
        "Búsqueda" => "Search",
        "Metadatos" => "Metadata",
        "Plantillas" => "Templates",
        "Bases" => "Bases",
        "Bookmarks" => "Bookmarks",
        "Plugins" => "Plugins",
        "Apariencia" => "Appearance",
        "Utilidades" => "Utilities",
        "Historial" => "History",
        "Sync" => "Sync",
        "Publish" => "Publish",
        "Espacio de trabajo" => "Workspace",
        "Developer" => "Developer",
        _ => category,
    }
    .to_string()
}

const fn spec(
    name: &'static str,
    category: &'static str,
    summary: &'static str,
    support: SupportLevel,
) -> CommandSpec {
    CommandSpec {
        name,
        category,
        summary,
        support,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_support_level_label() {
        assert_eq!(SupportLevel::Local.label(), "local");
        assert_eq!(SupportLevel::Hybrid.label(), "hybrid");
        assert_eq!(SupportLevel::BridgeOnly.label(), "bridge");
    }

    #[test]
    fn test_find_existing_command() {
        let cmd = find("help").expect("Should find 'help' command");
        assert_eq!(cmd.name, "help");
        assert_eq!(cmd.category, "General");
        assert_eq!(cmd.support, SupportLevel::Local);
    }

    #[test]
    fn test_find_non_existing_command() {
        assert!(find("unknown_command_123").is_none());
    }

    #[test]
    fn test_overview_structure() {
        let out = overview("es");

        // Assert header is present
        assert!(out.contains("Obsidian CLI para Termux (Rust)"));
        assert!(out.contains("Compatibilidad de sintaxis"));

        // Assert some categories are present
        assert!(out.contains("\nGeneral\n"));
        assert!(out.contains("\nArchivos\n"));

        // Assert some specific commands are present
        assert!(out.contains("  help [local] "));
        assert!(out.contains("  publish:site [bridge] "));
    }

    #[test]
    fn test_command_help_existing() {
        let help = command_help("help", "es").expect("Should return help for 'help'");
        assert!(help.contains("help\n"));
        assert!(help.contains("categoría: General\n"));
        assert!(help.contains("soporte: [local]\n"));
        assert!(help.contains("resumen: Muestra ayuda general o de un comando"));
    }

    #[test]
    fn test_command_help_non_existing() {
        assert!(command_help("unknown_command_123", "es").is_none());
    }
}
