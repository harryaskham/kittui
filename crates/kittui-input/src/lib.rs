//! kittui-input
//!
//! Parses kitty's pointer (SGR mouse mode 1006, motion 1003, focus 1004) and
//! key-reporting CSI escapes into typed events. The parser is byte-level and
//! does not allocate beyond the input slice; it is suitable for hot pointer
//! loops inside a WM compositor.
//!
//! The supported subset matches what kitty/Ghostty/WezTerm emit when SGR
//! mouse + motion + focus reporting are enabled:
//!
//! ```text
//! \x1b[<{button};{col};{row}M       press
//! \x1b[<{button};{col};{row}m       release
//! \x1b[<{button+32};{col};{row}M    motion (with optional button held)
//! \x1b[I / \x1b[O                   focus in / out
//! \x1b[{code}~                      function keys (legacy)
//! \x1b[1;{mods}{letter}             modified arrows / function keys
//! ```
//!
//! Out of scope for v1: bracketed paste, IME composition.

#![forbid(unsafe_code)]
#![warn(missing_docs, rust_2018_idioms)]

use serde::{Deserialize, Serialize};

/// Mouse button identifiers, matching kitty's SGR low-bit encoding.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    /// Primary (left).
    Left,
    /// Middle.
    Middle,
    /// Secondary (right).
    Right,
    /// Scroll wheel up.
    ScrollUp,
    /// Scroll wheel down.
    ScrollDown,
    /// Any extra button (kitty reports button index).
    Other(u16),
    /// Motion without a held button.
    None,
}

/// Modifier flags carried alongside a mouse or key event.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Modifiers {
    /// Shift held.
    pub shift: bool,
    /// Alt/Meta held.
    pub alt: bool,
    /// Control held.
    pub ctrl: bool,
}

impl Modifiers {
    fn from_sgr(bits: u32) -> Self {
        Self {
            shift: bits & 0b0000_0100 != 0,
            alt: bits & 0b0000_1000 != 0,
            ctrl: bits & 0b0001_0000 != 0,
        }
    }
}

/// One parsed input event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InputEvent {
    /// Mouse button press at `(col, row)` (1-indexed, kitty's convention).
    MousePress {
        /// Button.
        button: MouseButton,
        /// 1-indexed column.
        col: u16,
        /// 1-indexed row.
        row: u16,
        /// Modifiers.
        mods: Modifiers,
    },
    /// Mouse button release.
    MouseRelease {
        /// Button.
        button: MouseButton,
        /// 1-indexed column.
        col: u16,
        /// 1-indexed row.
        row: u16,
        /// Modifiers.
        mods: Modifiers,
    },
    /// Pointer motion (with optional button held).
    MouseMove {
        /// Button held during motion (or `None`).
        button: MouseButton,
        /// 1-indexed column.
        col: u16,
        /// 1-indexed row.
        row: u16,
        /// Modifiers.
        mods: Modifiers,
    },
    /// Terminal gained focus.
    FocusIn,
    /// Terminal lost focus.
    FocusOut,
    /// A printable character (UTF-8 decoded).
    Char {
        /// The character.
        ch: char,
        /// Modifiers.
        mods: Modifiers,
    },
    /// A named key (Enter, Backspace, arrows, function keys).
    Key {
        /// Key.
        key: Key,
        /// Modifiers.
        mods: Modifiers,
    },
}

/// Named keys we surface from CSI sequences.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Insert.
    Insert,
    /// Delete.
    Delete,
    /// Function key F1..F12.
    F(u8),
    /// Tab.
    Tab,
    /// Backspace.
    Backspace,
    /// Enter.
    Enter,
    /// Escape.
    Escape,
}

/// Parse a single byte slice into one event plus the number of bytes
/// consumed. Returns `None` if the slice doesn't yet contain a complete
/// event (the caller should buffer more bytes and retry).
pub fn parse(buf: &[u8]) -> Option<(InputEvent, usize)> {
    if buf.is_empty() {
        return None;
    }
    // CSI sequences begin with ESC [.
    if buf[0] == 0x1b {
        if buf.len() < 2 {
            return None;
        }
        if buf[1] == b'[' {
            return parse_csi(&buf[2..]).map(|(ev, n)| (ev, n + 2));
        }
        if buf[1] == 0x1b {
            // Bare double-ESC: treat as Escape.
            return Some((
                InputEvent::Key {
                    key: Key::Escape,
                    mods: Modifiers::default(),
                },
                1,
            ));
        }
        // Bare ESC (no following byte yet) → caller should buffer.
        return None;
    }
    // Single byte: control or UTF-8 char.
    if let Some(key) = special_byte(buf[0]) {
        return Some((
            InputEvent::Key {
                key,
                mods: Modifiers::default(),
            },
            1,
        ));
    }
    // UTF-8 decode of one character.
    let s = match std::str::from_utf8(buf) {
        Ok(s) => s,
        Err(e) if e.valid_up_to() > 0 => std::str::from_utf8(&buf[..e.valid_up_to()]).unwrap(),
        Err(_) => return None,
    };
    let ch = s.chars().next()?;
    Some((
        InputEvent::Char {
            ch,
            mods: Modifiers::default(),
        },
        ch.len_utf8(),
    ))
}

fn special_byte(b: u8) -> Option<Key> {
    match b {
        b'\r' | b'\n' => Some(Key::Enter),
        b'\t' => Some(Key::Tab),
        0x7f | 0x08 => Some(Key::Backspace),
        0x1b => Some(Key::Escape),
        _ => None,
    }
}

fn parse_csi(buf: &[u8]) -> Option<(InputEvent, usize)> {
    // SGR mouse: \x1b[<{b};{c};{r}M  or  m
    if let Some(rest) = buf.strip_prefix(b"<") {
        let term = rest.iter().position(|&b| b == b'M' || b == b'm')?;
        let payload = std::str::from_utf8(&rest[..term]).ok()?;
        let mut parts = payload.split(';');
        let bits: u32 = parts.next()?.parse().ok()?;
        let col: u16 = parts.next()?.parse().ok()?;
        let row: u16 = parts.next()?.parse().ok()?;
        let mods = Modifiers::from_sgr(bits);
        let is_press = rest[term] == b'M';
        let is_motion = bits & 32 != 0;
        let is_scroll = bits & 64 != 0;
        let button_bits = bits & 0b11;
        let button = if is_scroll {
            match button_bits {
                0 => MouseButton::ScrollUp,
                1 => MouseButton::ScrollDown,
                other => MouseButton::Other(other as u16),
            }
        } else {
            match button_bits {
                0 => MouseButton::Left,
                1 => MouseButton::Middle,
                2 => MouseButton::Right,
                3 => MouseButton::None,
                other => MouseButton::Other(other as u16),
            }
        };
        let n = 1 + term + 1; // skip '<' + payload + terminator
        let ev = if is_motion {
            InputEvent::MouseMove {
                button,
                col,
                row,
                mods,
            }
        } else if is_press {
            InputEvent::MousePress {
                button,
                col,
                row,
                mods,
            }
        } else {
            InputEvent::MouseRelease {
                button,
                col,
                row,
                mods,
            }
        };
        return Some((ev, n));
    }
    // Focus reporting: \x1b[I or \x1b[O
    if buf.first() == Some(&b'I') {
        return Some((InputEvent::FocusIn, 1));
    }
    if buf.first() == Some(&b'O') {
        return Some((InputEvent::FocusOut, 1));
    }
    // Arrow / named keys: \x1b[A..D or with modifiers \x1b[1;<mods>{A..D}
    let term = buf.iter().position(|&b| b.is_ascii_alphabetic() || b == b'~')?;
    let payload = std::str::from_utf8(&buf[..term]).ok()?;
    let letter = buf[term] as char;
    let mods = if let Some((_, m)) = payload.split_once(';') {
        let bits: u32 = m.parse().ok().unwrap_or(1);
        Modifiers {
            shift: (bits.saturating_sub(1)) & 1 != 0,
            alt: (bits.saturating_sub(1)) & 2 != 0,
            ctrl: (bits.saturating_sub(1)) & 4 != 0,
        }
    } else {
        Modifiers::default()
    };
    let key = match letter {
        'A' => Key::Up,
        'B' => Key::Down,
        'C' => Key::Right,
        'D' => Key::Left,
        'H' => Key::Home,
        'F' => Key::End,
        '~' => {
            let code: u32 = payload.split(';').next()?.parse().ok()?;
            match code {
                2 => Key::Insert,
                3 => Key::Delete,
                5 => Key::PageUp,
                6 => Key::PageDown,
                11 => Key::F(1),
                12 => Key::F(2),
                13 => Key::F(3),
                14 => Key::F(4),
                15 => Key::F(5),
                17 => Key::F(6),
                18 => Key::F(7),
                19 => Key::F(8),
                20 => Key::F(9),
                21 => Key::F(10),
                23 => Key::F(11),
                24 => Key::F(12),
                _ => return None,
            }
        }
        _ => return None,
    };
    Some((InputEvent::Key { key, mods }, term + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sgr_left_press() {
        let bytes = b"\x1b[<0;10;5M";
        let (ev, n) = parse(bytes).unwrap();
        assert_eq!(n, bytes.len());
        assert_eq!(
            ev,
            InputEvent::MousePress {
                button: MouseButton::Left,
                col: 10,
                row: 5,
                mods: Modifiers::default()
            }
        );
    }

    #[test]
    fn parses_sgr_motion_with_held_button_and_modifiers() {
        // bits = 0 (left) | 32 (motion) | 4 (shift) = 36
        let bytes = b"\x1b[<36;42;7M";
        let (ev, _) = parse(bytes).unwrap();
        match ev {
            InputEvent::MouseMove {
                button,
                mods,
                col,
                row,
            } => {
                assert_eq!(button, MouseButton::Left);
                assert!(mods.shift);
                assert_eq!((col, row), (42, 7));
            }
            other => panic!("expected motion, got {other:?}"),
        }
    }

    #[test]
    fn parses_sgr_release() {
        let bytes = b"\x1b[<0;3;3m";
        let (ev, _) = parse(bytes).unwrap();
        assert!(matches!(ev, InputEvent::MouseRelease { .. }));
    }

    #[test]
    fn parses_scroll_up() {
        // 64 = scroll, 0 = up
        let bytes = b"\x1b[<64;1;1M";
        let (ev, _) = parse(bytes).unwrap();
        assert!(matches!(
            ev,
            InputEvent::MousePress {
                button: MouseButton::ScrollUp,
                ..
            }
        ));
    }

    #[test]
    fn parses_focus_in_and_out() {
        let (a, _) = parse(b"\x1b[I").unwrap();
        let (b, _) = parse(b"\x1b[O").unwrap();
        assert_eq!(a, InputEvent::FocusIn);
        assert_eq!(b, InputEvent::FocusOut);
    }

    #[test]
    fn parses_arrow_keys_and_modifiers() {
        let (up, _) = parse(b"\x1b[A").unwrap();
        assert_eq!(
            up,
            InputEvent::Key {
                key: Key::Up,
                mods: Modifiers::default()
            }
        );
        let (mod_up, _) = parse(b"\x1b[1;5A").unwrap();
        match mod_up {
            InputEvent::Key { key, mods } => {
                assert_eq!(key, Key::Up);
                assert!(mods.ctrl);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parses_function_keys_and_delete() {
        let (f5, _) = parse(b"\x1b[15~").unwrap();
        assert_eq!(
            f5,
            InputEvent::Key {
                key: Key::F(5),
                mods: Modifiers::default()
            }
        );
        let (del, _) = parse(b"\x1b[3~").unwrap();
        assert_eq!(
            del,
            InputEvent::Key {
                key: Key::Delete,
                mods: Modifiers::default()
            }
        );
    }

    #[test]
    fn parses_plain_chars_and_control_keys() {
        let (a, n) = parse(b"a").unwrap();
        assert_eq!(n, 1);
        assert_eq!(
            a,
            InputEvent::Char {
                ch: 'a',
                mods: Modifiers::default()
            }
        );
        let (enter, _) = parse(b"\r").unwrap();
        assert_eq!(
            enter,
            InputEvent::Key {
                key: Key::Enter,
                mods: Modifiers::default()
            }
        );
    }

    #[test]
    fn empty_or_partial_buffer_returns_none() {
        assert!(parse(b"").is_none());
        assert!(parse(b"\x1b").is_none());
        assert!(parse(b"\x1b[").is_none());
        assert!(parse(b"\x1b[<0;10").is_none()); // not yet terminated
    }
}
