use anstyle::{AnsiColor, Style};
use std::io::IsTerminal;

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Format {
    Agent,
    Json,
    Human,
}

pub struct Out {
    pub format: Format,
    color: bool,
}

impl Out {
    pub fn new(format: Format) -> Self {
        Out {
            format,
            color: format == Format::Human && std::io::stdout().is_terminal(),
        }
    }

    fn paint(&self, style: Style, s: &str) -> String {
        if self.color {
            format!("{style}{s}{style:#}")
        } else {
            s.to_owned()
        }
    }

    pub fn header(&self, s: &str) -> String {
        self.paint(
            Style::new().bold().fg_color(Some(AnsiColor::Cyan.into())),
            s,
        )
    }

    pub fn dim(&self, s: &str) -> String {
        self.paint(Style::new().dimmed(), s)
    }

    /// `n\tline`, or `n>\tline` for matching lines — the shape agents know from `cat -n`.
    pub fn numbered(&self, n: usize, line: &str, mark: bool) -> String {
        format!("{n}{}\t{line}", if mark { ">" } else { "" })
    }
}

/// Trim leading whitespace and cap a hit line at 200 chars.
pub fn hit_text(s: &str) -> String {
    let t = s.trim_start();
    match t.char_indices().nth(200) {
        Some((i, _)) => format!("{}…", &t[..i]),
        None => t.to_owned(),
    }
}
