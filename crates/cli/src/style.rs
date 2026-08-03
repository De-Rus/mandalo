use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Style {
    colored: bool,
}

impl Style {
    pub fn plain() -> Self {
        Style { colored: false }
    }

    pub fn for_stdout() -> Self {
        Style {
            colored: std::io::stdout().is_terminal()
                && std::env::var("NO_COLOR").is_err()
                && std::env::var("TERM").as_deref() != Ok("dumb"),
        }
    }

    fn paint(&self, code: &str, text: &str) -> String {
        if self.colored {
            format!("\u{1b}[{code}m{text}\u{1b}[0m")
        } else {
            text.to_string()
        }
    }

    pub fn pass(&self, text: &str) -> String {
        self.paint("32", text)
    }

    pub fn fail(&self, text: &str) -> String {
        self.paint("31", text)
    }

    pub fn warn(&self, text: &str) -> String {
        self.paint("33", text)
    }

    pub fn dim(&self, text: &str) -> String {
        self.paint("2", text)
    }

    pub fn bold(&self, text: &str) -> String {
        self.paint("1", text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_style_never_emits_escape_codes() {
        let s = Style::plain();
        assert_eq!(s.pass("ok"), "ok");
        assert_eq!(s.fail("no"), "no");
        assert_eq!(s.dim("x"), "x");
        assert_eq!(s.bold("x"), "x");
        assert_eq!(s.warn("x"), "x");
    }

    #[test]
    fn colored_style_wraps_in_ansi() {
        let s = Style { colored: true };
        assert_eq!(s.pass("ok"), "\u{1b}[32mok\u{1b}[0m");
        assert_eq!(s.fail("no"), "\u{1b}[31mno\u{1b}[0m");
    }
}
