//! Show an app's icon inline, in terminals that can display images.

use std::io::{IsTerminal, Write};

use whistle_core::fetch;

enum Protocol {
    /// Kitty's graphics protocol (Kitty, Ghostty, ...).
    Kitty,
    /// iTerm2's inline images (iTerm2, WezTerm, ...).
    Iterm,
}

fn protocol() -> Option<Protocol> {
    if std::env::var_os("WHISTLE_NO_IMAGES").is_some() || !std::io::stdout().is_terminal() {
        return None;
    }
    let var = |name: &str| std::env::var(name).unwrap_or_default();
    if std::env::var_os("KITTY_WINDOW_ID").is_some()
        || var("TERM") == "xterm-kitty"
        || var("TERM") == "xterm-ghostty"
    {
        return Some(Protocol::Kitty);
    }
    if ["iTerm.app", "WezTerm"].contains(&var("TERM_PROGRAM").as_str())
        || var("LC_TERMINAL") == "iTerm2"
    {
        return Some(Protocol::Iterm);
    }
    None
}

/// Print the PNG at `url` about 8 columns wide, if the terminal can show it.
/// Silent on any failure: the icon is a nicety.
pub fn show_icon(url: &str) {
    let Some(protocol) = protocol() else {
        return;
    };
    if !url.ends_with(".png") {
        return;
    }
    let Ok(png) = fetch::download_bytes(url, 512 * 1024) else {
        return;
    };
    let data = base64(&png);
    let mut out = std::io::stdout().lock();
    match protocol {
        Protocol::Kitty => {
            // Sent in chunks of at most 4096 bytes; m=1 means more follow.
            let chunks: Vec<&[u8]> = data.as_bytes().chunks(4096).collect();
            for (n, chunk) in chunks.iter().enumerate() {
                let more = u8::from(n + 1 < chunks.len());
                let head = if n == 0 { "a=T,f=100,c=8,r=4," } else { "" };
                let _ = write!(
                    out,
                    "\x1b_G{head}m={more};{}\x1b\\",
                    String::from_utf8_lossy(chunk)
                );
            }
        }
        Protocol::Iterm => {
            let _ = write!(
                out,
                "\x1b]1337;File=inline=1;width=8;height=4;preserveAspectRatio=1:{data}\x07"
            );
        }
    }
    let _ = writeln!(out);
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i)) as usize & 63] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::base64;

    #[test]
    fn encodes_base64() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }
}
