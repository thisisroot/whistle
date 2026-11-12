//! Terminal output: columns that fit the window, and a little style when
//! it's a terminal (none when piped, or with NO_COLOR set).

use std::io::IsTerminal;

pub struct Term {
    /// Columns to fit rows into; `None` when not a terminal, so nothing is cut.
    width: Option<usize>,
    color: bool,
}

/// One line of a list: a name, how it installs, and a description.
pub struct Row {
    pub name: String,
    pub tag: String,
    pub text: String,
}

/// The widest a name column gets; longer names are shortened.
const MAX_NAME: usize = 28;

impl Term {
    pub fn detect() -> Self {
        let tty = std::io::stdout().is_terminal();
        Self {
            width: tty.then(|| width().unwrap_or(100)),
            color: tty && std::env::var_os("NO_COLOR").is_none(),
        }
    }

    pub fn bold(&self, text: &str) -> String {
        self.style("1", text)
    }

    pub fn dim(&self, text: &str) -> String {
        self.style("2", text)
    }

    fn style(&self, code: &str, text: &str) -> String {
        if self.color && !text.is_empty() {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_owned()
        }
    }

    /// Rows in aligned columns, numbered from 1 when `numbered`.
    pub fn rows(&self, rows: &[Row], numbered: bool) {
        let name_width = rows
            .iter()
            .map(|r| r.name.chars().count())
            .max()
            .unwrap_or(0)
            .min(MAX_NAME);
        let tag_width = rows
            .iter()
            .map(|r| r.tag.chars().count())
            .max()
            .unwrap_or(0);
        for (n, row) in rows.iter().enumerate() {
            let number = if numbered {
                format!("{:>4}  ", n + 1)
            } else {
                String::new()
            };
            let name = fit(&row.name, name_width);
            let used = number.chars().count() + name_width + 2 + tag_width + 2;
            let text = match self.width {
                Some(width) if width > used + 10 => fit(&row.text, width - used),
                Some(_) => String::new(),
                None => row.text.clone(),
            };
            println!(
                "{number}{}{}  {:<tag_width$}  {}",
                self.bold(&name),
                " ".repeat(name_width - name.chars().count()),
                row.tag,
                self.dim(&text)
            );
        }
    }
}

/// Shorten `text` to `width` characters, marking the cut with an ellipsis.
pub fn fit(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_owned();
    }
    let mut short: String = text.chars().take(width.saturating_sub(1)).collect();
    short.push('…');
    short
}

/// The terminal's width in columns.
fn width() -> Option<usize> {
    #[cfg(unix)]
    {
        // SAFETY: TIOCGWINSZ only fills in the winsize it's given.
        let mut size: libc::winsize = unsafe { std::mem::zeroed() };
        if unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut size) } == 0
            && size.ws_col > 0
        {
            return Some(usize::from(size.ws_col));
        }
    }
    std::env::var("COLUMNS").ok()?.parse().ok()
}
