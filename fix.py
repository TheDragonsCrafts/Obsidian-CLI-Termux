import sys

with open("src/parser.rs", "r") as f:
    content = f.read()

content = content.replace(
    "<<<<<<< HEAD\n    use anyhow::Result;\n\n    use super::{Request, parse};\n=======\n    use super::{Request, parse, parse_line};\n>>>>>>> origin/jules-stuff",
    "    use anyhow::Result;\n    use super::{Request, parse, parse_line};"
)

content = content.replace(
    """    #[test]
    fn parse_line_with_quotes() {
        let line = "vault=\\"My Vault\\" append file=\\"Inbox\\" content=\\"hello\\nworld\\" inline --copy";

        let Request::Invocation(inv) = parse_line(line).unwrap() else {
            panic!("expected invocation");
        };

        assert_eq!(inv.global.vault.as_deref(), Some("My Vault"));
        assert_eq!(inv.command, "append");
        assert_eq!(inv.param("file"), Some("Inbox"));
        assert_eq!(inv.param("content"), Some("hello\\nworld"));
        assert!(inv.positionals.contains(&"inline".to_string()));
        assert!(inv.has_flag("inline"));
        assert!(inv.global.copy);
    }""",
    """    #[test]
    fn parse_line_with_quotes() -> Result<()> {
        let line = "vault=\\"My Vault\\" append file=\\"Inbox\\" content=\\"hello\\\\nworld\\" inline --copy";

        let Request::Invocation(inv) = parse_line(line)? else {
            panic!("expected invocation");
        };

        assert_eq!(inv.global.vault.as_deref(), Some("My Vault"));
        assert_eq!(inv.command, "append");
        assert_eq!(inv.param("file"), Some("Inbox"));
        assert_eq!(inv.param("content"), Some("hello\\nworld"));
        assert!(inv.positionals.contains(&"inline".to_string()));
        assert!(inv.has_flag("inline"));
        assert!(inv.global.copy);
        Ok(())
    }"""
)

content = content.replace(
    """    #[test]
    fn parse_line_empty() {
        assert!(matches!(parse_line("").unwrap(), Request::Interactive));
        assert!(matches!(parse_line("   ").unwrap(), Request::Interactive));
    }""",
    """    #[test]
    fn parse_line_empty() -> Result<()> {
        assert!(matches!(parse_line("")?, Request::Interactive));
        assert!(matches!(parse_line("   ")?, Request::Interactive));
        Ok(())
    }"""
)

content = content.replace(
    """    #[test]
    fn parse_line_unbalanced_quotes() {
        // shlex::split will return None for this, leading to unwrap_or_default() returning an empty vec.
        // Thus parse will return Request::Interactive.
        assert!(matches!(parse_line("vault=\\"Main").unwrap(), Request::Interactive));
    }""",
    """    #[test]
    fn parse_line_unbalanced_quotes() -> Result<()> {
        // shlex::split will return None for this, leading to unwrap_or_default() returning an empty vec.
        // Thus parse will return Request::Interactive.
        assert!(matches!(parse_line("vault=\\"Main")?, Request::Interactive));
        Ok(())
    }"""
)

with open("src/parser.rs", "w") as f:
    f.write(content)
