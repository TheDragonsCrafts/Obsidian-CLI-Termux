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
        "file",
        "Files",
        "Información del archivo objetivo",
        SupportLevel::Local,
    ),
    spec(
        "files",
        "Files",
        "Lista archivos del vault",
        SupportLevel::Local,
    ),
    spec(
        "folder",
        "Files",
        "Información de una carpeta",
        SupportLevel::Local,
    ),
    spec(
        "folders",
        "Files",
        "Lista carpetas del vault",
        SupportLevel::Local,
    ),
    spec(
        "open",
        "Files",
        "Marca o abre un archivo como activo",
        SupportLevel::Local,
    ),
    spec("create", "Files", "Crea un archivo", SupportLevel::Local),
    spec("read", "Files", "Lee un archivo", SupportLevel::Local),
    spec(
        "append",
        "Files",
        "Agrega contenido al final",
        SupportLevel::Local,
    ),
    spec(
        "prepend",
        "Files",
        "Agrega contenido al inicio útil del archivo",
        SupportLevel::Local,
    ),
    spec(
        "move",
        "Files",
        "Mueve o renombra un archivo",
        SupportLevel::Local,
    ),
    spec(
        "rename",
        "Files",
        "Renombra un archivo",
        SupportLevel::Local,
    ),
    spec(
        "delete",
        "Files",
        "Elimina o manda a trash un archivo",
        SupportLevel::Local,
    ),
    spec(
        "links",
        "Links",
        "Lista links salientes del archivo",
        SupportLevel::Local,
    ),
    spec(
        "backlinks",
        "Links",
        "Lista backlinks del archivo",
        SupportLevel::Local,
    ),
    spec(
        "unresolved",
        "Links",
        "Lista links no resueltos",
        SupportLevel::Local,
    ),
    spec(
        "orphans",
        "Links",
        "Lista notas sin backlinks",
        SupportLevel::Local,
    ),
    spec(
        "deadends",
        "Links",
        "Lista notas sin links salientes",
        SupportLevel::Local,
    ),
    spec(
        "outline",
        "Links",
        "Muestra headings del archivo",
        SupportLevel::Local,
    ),
    spec(
        "daily",
        "Daily",
        "Abre o crea la daily note de hoy",
        SupportLevel::Local,
    ),
    spec(
        "daily:path",
        "Daily",
        "Devuelve el path esperado de la daily note",
        SupportLevel::Local,
    ),
    spec(
        "daily:read",
        "Daily",
        "Lee la daily note de hoy",
        SupportLevel::Local,
    ),
    spec(
        "daily:append",
        "Daily",
        "Agrega contenido a la daily note",
        SupportLevel::Local,
    ),
    spec(
        "daily:prepend",
        "Daily",
        "Prepend a la daily note",
        SupportLevel::Local,
    ),
    spec(
        "search",
        "Search",
        "Busca texto dentro del vault",
        SupportLevel::Local,
    ),
    spec(
        "search:context",
        "Search",
        "Busca texto y devuelve contexto tipo grep",
        SupportLevel::Local,
    ),
    spec(
        "search:open",
        "Search",
        "Abre la vista Search en la app",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "tags",
        "Metadata",
        "Lista tags del vault o archivo",
        SupportLevel::Local,
    ),
    spec(
        "tag",
        "Metadata",
        "Información de un tag",
        SupportLevel::Local,
    ),
    spec("tasks", "Metadata", "Lista tareas", SupportLevel::Local),
    spec(
        "task",
        "Metadata",
        "Muestra o actualiza una tarea",
        SupportLevel::Local,
    ),
    spec("aliases", "Metadata", "Lista aliases", SupportLevel::Local),
    spec(
        "properties",
        "Metadata",
        "Lista propiedades",
        SupportLevel::Local,
    ),
    spec(
        "property:set",
        "Metadata",
        "Establece una propiedad",
        SupportLevel::Local,
    ),
    spec(
        "property:remove",
        "Metadata",
        "Elimina una propiedad",
        SupportLevel::Local,
    ),
    spec(
        "property:read",
        "Metadata",
        "Lee una propiedad",
        SupportLevel::Local,
    ),
    spec(
        "templates",
        "Templates",
        "Lista templates",
        SupportLevel::Local,
    ),
    spec(
        "template:read",
        "Templates",
        "Lee un template",
        SupportLevel::Local,
    ),
    spec(
        "template:insert",
        "Templates",
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
        "Appearance",
        "Lista temas instalados",
        SupportLevel::Hybrid,
    ),
    spec(
        "theme",
        "Appearance",
        "Información del tema activo o uno concreto",
        SupportLevel::Hybrid,
    ),
    spec(
        "theme:set",
        "Appearance",
        "Activa un tema",
        SupportLevel::Hybrid,
    ),
    spec(
        "theme:install",
        "Appearance",
        "Instala un tema",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "theme:uninstall",
        "Appearance",
        "Desinstala un tema",
        SupportLevel::Hybrid,
    ),
    spec(
        "snippets",
        "Appearance",
        "Lista snippets",
        SupportLevel::Hybrid,
    ),
    spec(
        "snippets:enabled",
        "Appearance",
        "Lista snippets habilitados",
        SupportLevel::Hybrid,
    ),
    spec(
        "snippet:enable",
        "Appearance",
        "Habilita un snippet",
        SupportLevel::Hybrid,
    ),
    spec(
        "snippet:disable",
        "Appearance",
        "Deshabilita un snippet",
        SupportLevel::Hybrid,
    ),
    spec(
        "random",
        "Utilities",
        "Selecciona una nota aleatoria",
        SupportLevel::Local,
    ),
    spec(
        "random:read",
        "Utilities",
        "Lee una nota aleatoria",
        SupportLevel::Local,
    ),
    spec(
        "diff",
        "History",
        "Compara versiones",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "history",
        "History",
        "Lista versiones de file recovery",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "history:list",
        "History",
        "Lista versiones de file recovery",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "history:read",
        "History",
        "Lee una versión de file recovery",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "history:restore",
        "History",
        "Restaura una versión de file recovery",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "history:open",
        "History",
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
        "Workspace",
        "Árbol del layout actual",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "workspaces",
        "Workspace",
        "Lista workspaces guardados",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "workspace:save",
        "Workspace",
        "Guarda un workspace",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "workspace:load",
        "Workspace",
        "Carga un workspace",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "workspace:delete",
        "Workspace",
        "Elimina un workspace",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "tabs",
        "Workspace",
        "Lista tabs abiertos",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "tab:open",
        "Workspace",
        "Abre una tab",
        SupportLevel::BridgeOnly,
    ),
    spec(
        "recents",
        "Workspace",
        "Lista recientes registrados por la CLI",
        SupportLevel::Local,
    ),
    spec(
        "web",
        "Workspace",
        "Abre una URL en el viewer",
        SupportLevel::Hybrid,
    ),
    spec(
        "wordcount",
        "Utilities",
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

pub fn overview() -> String {
    let mut grouped: BTreeMap<&str, Vec<&CommandSpec>> = BTreeMap::new();
    for spec in COMMANDS {
        grouped.entry(spec.category).or_default().push(spec);
    }

    let mut out = String::new();
    out.push_str("Obsidian CLI for Termux (Rust)\n");
    out.push_str(
        "Compatibilidad de sintaxis: `vault=<name>` + comando + `param=value` + flags booleanos.\n",
    );
    out.push_str("Soporte: [local] funciona sin app, [hybrid] mezcla archivos/config y futuro bridge, [bridge] requiere plugin/bridge en Obsidian.\n\n");

    for (category, specs) in grouped {
        out.push_str(category);
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

pub fn command_help(name: &str) -> Option<String> {
    let spec = find(name)?;
    Some(format!(
        "{name}\ncategory: {}\nsupport: [{}]\nsummary: {}",
        spec.category,
        spec.support.label(),
        spec.summary
    ))
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
        let out = overview();

        // Assert header is present
        assert!(out.contains("Obsidian CLI for Termux (Rust)"));
        assert!(out.contains("Compatibilidad de sintaxis"));

        // Assert some categories are present
        assert!(out.contains("\nGeneral\n"));
        assert!(out.contains("\nFiles\n"));

        // Assert some specific commands are present
        assert!(out.contains("  help [local] "));
        assert!(out.contains("  publish:site [bridge] "));
    }

    #[test]
    fn test_command_help_existing() {
        let help = command_help("help").expect("Should return help for 'help'");
        assert!(help.contains("help\n"));
        assert!(help.contains("category: General\n"));
        assert!(help.contains("support: [local]\n"));
        assert!(help.contains("summary: Muestra ayuda general o de un comando"));
    }

    #[test]
    fn test_command_help_non_existing() {
        assert!(command_help("unknown_command_123").is_none());
    }
}
