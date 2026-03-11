use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, bail};

#[derive(Debug, Clone, Default)]
pub struct GlobalOptions {
    pub vault: Option<String>,
    pub copy: bool,
}

#[derive(Debug, Clone)]
pub struct Invocation {
    pub command: String,
    pub params: BTreeMap<String, String>,
    pub flags: BTreeSet<String>,
    pub positionals: Vec<String>,
    pub global: GlobalOptions,
}

impl Invocation {
    pub fn param(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(String::as_str)
    }

    pub fn has_flag(&self, key: &str) -> bool {
        self.flags.contains(key)
    }
}

#[derive(Debug, Clone)]
pub enum Request {
    Interactive,
    Invocation(Invocation),
}

pub fn parse(args: &[String]) -> Result<Request> {
    if args.is_empty() {
        return Ok(Request::Interactive);
    }

    let mut global = GlobalOptions::default();
    let mut params = BTreeMap::new();
    let mut flags = BTreeSet::new();
    let mut positionals = Vec::new();
    let mut command = None;
    for token in args {
        if token == "--copy" {
            global.copy = true;
            continue;
        }

        if command.is_none() {
            if let Some((key, value)) = split_param(token) {
                let value = decode_value(value);
                if key == "vault" {
                    global.vault = Some(value);
                } else {
                    params.insert(key.to_string(), value);
                }
                continue;
            }

            if let Some(flag) = token.strip_prefix("--") {
                flags.insert(flag.to_string());
                continue;
            }

            command = Some(token.to_string());
            continue;
        }

        if let Some((key, value)) = split_param(token) {
            params.insert(key.to_string(), decode_value(value));
            continue;
        }

        if let Some(flag) = token.strip_prefix("--") {
            flags.insert(flag.to_string());
            continue;
        }

        positionals.push(token.to_string());
        flags.insert(token.to_string());
    }

    let Some(command) = command else {
        if !params.is_empty() || !flags.is_empty() {
            bail!("faltó el comando después de los parámetros globales");
        }
        return Ok(Request::Interactive);
    };

    Ok(Request::Invocation(Invocation {
        command,
        params,
        flags,
        positionals,
        global,
    }))
}

pub fn parse_line(line: &str) -> Result<Request> {
    let parts = shlex::split(line)
        .ok_or_else(|| anyhow::anyhow!("línea inválida: comillas desbalanceadas"))?;
    parse(&parts)
}

fn split_param(token: &str) -> Option<(&str, &str)> {
    let (key, value) = token.split_once('=')?;
    if key.is_empty() {
        return None;
    }
    Some((key, value))
}

fn decode_value(value: &str) -> String {
    let mut decoded = String::with_capacity(value.len());
    let mut chars = value.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            decoded.push(ch);
            continue;
        }

        match chars.next() {
            Some('n') => decoded.push('\n'),
            Some('r') => decoded.push('\r'),
            Some('t') => decoded.push('\t'),
            Some('\\') => decoded.push('\\'),
            Some('"') => decoded.push('"'),
            Some('\'') => decoded.push('\''),
            Some(other) => {
                decoded.push('\\');
                decoded.push(other);
            }
            None => decoded.push('\\'),
        }
    }

    decoded
}

#[cfg(test)]
mod tests {
    use super::{Request, parse, parse_line};

    #[test]
    fn parses_command_with_params_and_flags() {
        let args = vec![
            "vault=Main".to_string(),
            "append".to_string(),
            "file=Inbox".to_string(),
            "content=hola\\n2".to_string(),
            "inline".to_string(),
            "--copy".to_string(),
        ];

        let Request::Invocation(inv) = parse(&args).unwrap() else {
            panic!("expected invocation");
        };

        assert_eq!(inv.global.vault.as_deref(), Some("Main"));
        assert_eq!(inv.command, "append");
        assert_eq!(inv.param("file"), Some("Inbox"));
        assert_eq!(inv.param("content"), Some("hola\n2"));
        assert!(inv.positionals.contains(&"inline".to_string()));
        assert!(inv.has_flag("inline"));
        assert!(inv.global.copy);
    }

    #[test]
    fn empty_is_interactive() {
        assert!(matches!(parse(&[]).unwrap(), Request::Interactive));
    }

    #[test]
    fn parse_line_with_quotes() {
        let line =
            "vault=\"My Vault\" append file=\"Inbox\" content=\"hello\\nworld\" inline --copy";

        let Request::Invocation(inv) = parse_line(line).unwrap() else {
            panic!("expected invocation");
        };

        assert_eq!(inv.global.vault.as_deref(), Some("My Vault"));
        assert_eq!(inv.command, "append");
        assert_eq!(inv.param("file"), Some("Inbox"));
        assert_eq!(inv.param("content"), Some("hello\nworld"));
        assert!(inv.positionals.contains(&"inline".to_string()));
        assert!(inv.has_flag("inline"));
        assert!(inv.global.copy);
    }

    #[test]
    fn parse_line_empty() {
        assert!(matches!(parse_line("").unwrap(), Request::Interactive));
        assert!(matches!(parse_line("   ").unwrap(), Request::Interactive));
    }

    #[test]
    fn parse_line_unbalanced_quotes() {
        let err = parse_line("vault=\"Main").unwrap_err();
        assert_eq!(err.to_string(), "línea inválida: comillas desbalanceadas");
    }

    #[test]
    fn parse_line_missing_command() {
        let err = parse_line("vault=Main --copy someparam=somevalue").unwrap_err();
        assert_eq!(
            err.to_string(),
            "faltó el comando después de los parámetros globales"
        );
    }
}
