//! Portable global shortcut syntax; native registration belongs to the adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalShortcut {
    pub control: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: ShortcutKey,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutKey {
    Character(char),
    Function(u8),
    Space,
}
impl GlobalShortcut {
    pub fn parse(value: &str) -> Result<Self, String> {
        if value.len() > 64 {
            return Err("Shortcut is too long".into());
        }
        let (mut control, mut alt, mut shift, mut key) = (false, false, false, None);
        for token in value.split('+').map(str::trim) {
            let token = token.to_ascii_uppercase();
            let modifier = match token.as_str() {
                "CTRL" | "CONTROL" => Some(&mut control),
                "ALT" => Some(&mut alt),
                "SHIFT" => Some(&mut shift),
                "WIN" | "WINDOWS" | "META" => {
                    return Err("Windows-key shortcuts are reserved; use Ctrl or Alt".into())
                }
                _ => None,
            };
            if let Some(modifier) = modifier {
                if *modifier {
                    return Err("A shortcut modifier is repeated".into());
                }
                *modifier = true;
                continue;
            }
            let parsed = if token == "SPACE" {
                ShortcutKey::Space
            } else if token.len() == 1 && token.as_bytes()[0].is_ascii_alphanumeric() {
                ShortcutKey::Character(token.chars().next().unwrap())
            } else if let Some(n) = token.strip_prefix('F').and_then(|v| v.parse::<u8>().ok()) {
                if n == 12 {
                    return Err("F12 is reserved for the Windows debugger".into());
                }
                if !(1..=24).contains(&n) {
                    return Err("Use F1-F24, except F12".into());
                }
                ShortcutKey::Function(n)
            } else {
                return Err(
                    "Use Ctrl and/or Alt, optional Shift, plus A-Z, 0-9, Space or a function key"
                        .into(),
                );
            };
            if key.replace(parsed).is_some() {
                return Err("A shortcut must have exactly one non-modifier key".into());
            }
        }
        if !control && !alt {
            return Err("A global shortcut must include Ctrl or Alt".into());
        }
        let shortcut = Self {
            control,
            alt,
            shift,
            key: key.ok_or("A shortcut needs a non-modifier key")?,
        };
        if shortcut.control
            && !shortcut.alt
            && !shortcut.shift
            && shortcut.key == ShortcutKey::Character('V')
        {
            return Err("Ctrl+V would recursively trigger Echo's injected paste".into());
        }
        if shortcut.alt
            && !shortcut.control
            && matches!(shortcut.key, ShortcutKey::Function(4) | ShortcutKey::Space)
        {
            return Err("Alt+F4 and Alt+Space are reserved Windows window controls".into());
        }
        Ok(shortcut)
    }
    pub fn canonical(&self) -> String {
        let mut parts = Vec::new();
        if self.control {
            parts.push("Ctrl".to_owned());
        }
        if self.alt {
            parts.push("Alt".to_owned());
        }
        if self.shift {
            parts.push("Shift".to_owned());
        }
        parts.push(match self.key {
            ShortcutKey::Character(c) => c.to_string(),
            ShortcutKey::Function(n) => format!("F{n}"),
            ShortcutKey::Space => "Space".into(),
        });
        parts.join("+")
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_shortcuts_are_case_and_order_independent() {
        for (input, expected) in [
            ("alt + v", "Alt+V"),
            ("shift+v+control", "Ctrl+Shift+V"),
            ("CTRL+ALT+F24", "Ctrl+Alt+F24"),
            ("ctrl+alt+space", "Ctrl+Alt+Space"),
        ] {
            let key = GlobalShortcut::parse(input).unwrap();
            assert_eq!(key.canonical(), expected);
            assert_eq!(GlobalShortcut::parse(&key.canonical()).unwrap(), key);
        }
    }
    #[test]
    fn unsafe_or_ambiguous_shortcuts_are_rejected() {
        for input in [
            "",
            "V",
            "Shift+V",
            "Alt",
            "Alt+",
            "Alt+V+X",
            "Alt+Alt+V",
            "Win+V",
            "Ctrl+F12",
            "Alt+F0",
            "Alt+F25",
            "Alt+Enter",
            "Ctrl+V",
            "Alt+F4",
            "Alt+Space",
        ] {
            assert!(GlobalShortcut::parse(input).is_err(), "{input}");
        }
    }

    #[test]
    fn useful_variants_of_reserved_combinations_remain_available() {
        for input in [
            "Ctrl+Shift+V",
            "Ctrl+Alt+V",
            "Ctrl+Alt+F4",
            "Ctrl+Alt+Space",
        ] {
            assert!(GlobalShortcut::parse(input).is_ok(), "{input}");
        }
    }

    #[test]
    fn parsing_uses_ascii_key_identity_only() {
        for input in ["Ctrl+é", "Alt+中", "Ctrl+💾"] {
            assert!(GlobalShortcut::parse(input).is_err(), "{input}");
        }
    }
}
