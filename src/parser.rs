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
    let parts = shlex::split(line).unwrap_or_default();
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
    use anyhow::Result;

    use super::{Request, parse};

    #[test]
    fn parses_command_with_params_and_flags() -> Result<()> {
        let args = vec![
            "vault=Main".to_string(),
            "append".to_string(),
            "file=Inbox".to_string(),
            "content=hola\\n2".to_string(),
            "inline".to_string(),
            "--copy".to_string(),
        ];

        let Request::Invocation(inv) = parse(&args)? else {
            panic!("expected invocation");
        };

        assert_eq!(inv.global.vault.as_deref(), Some("Main"));
        assert_eq!(inv.command, "append");
        assert_eq!(inv.param("file"), Some("Inbox"));
        assert_eq!(inv.param("content"), Some("hola\n2"));
        assert!(inv.positionals.contains(&"inline".to_string()));
        assert!(inv.has_flag("inline"));
        assert!(inv.global.copy);

        Ok(())
    }

    #[test]
    fn empty_is_interactive() -> Result<()> {
        assert!(matches!(parse(&[])?, Request::Interactive));
        Ok(())
    }
}
