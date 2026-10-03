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

/// Trim leading whitespace and clip a hit line around its match (byte offset `col`).
pub fn hit_text(s: &str, col: usize) -> String {
    let t = s.trim_start();
    clip(t, col.saturating_sub(s.len() - t.len()))
}

/// Cap a line at 200 chars, keeping the window around byte offset `col`, so a
/// minified file costs a few hundred bytes per hit instead of the whole line.
pub fn clip(t: &str, col: usize) -> String {
    const MAX: usize = 200;
    if t.chars().nth(MAX).is_none() {
        return t.to_owned();
    }
    let at = t.get(..col).map_or(0, |p| p.chars().count());
    let start = at.saturating_sub(60);
    let body: String = t.chars().skip(start).take(MAX).collect();
    let rest = t.chars().count() - start - body.chars().count();
    let head = if start > 0 { "…" } else { "" };
    let tail = if rest > 0 {
        format!("… (+{rest} chars)")
    } else {
        String::new()
    };
    format!("{head}{body}{tail}")
}
