use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
};

#[derive(Clone, Copy, Debug)]
pub(super) struct ParsedTrigger {
    pub(super) register_modifiers: u32,
    pub(super) required_modifiers: u32,
    pub(super) key: u32,
}

pub(super) fn parse_trigger(trigger: &str) -> Result<ParsedTrigger, String> {
    let tokens = trigger
        .split('+')
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();
    if tokens.is_empty() {
        return Err("empty hotkey trigger".to_string());
    }

    let mut required_modifiers = 0u32;
    let mut key_token = None;

    for token in tokens {
        match token.to_ascii_lowercase().as_str() {
            "alt" => required_modifiers |= MOD_ALT,
            "ctrl" | "control" => required_modifiers |= MOD_CONTROL,
            "shift" => required_modifiers |= MOD_SHIFT,
            "win" | "windows" => required_modifiers |= MOD_WIN,
            _ => {
                if key_token.is_some() {
                    return Err(format!(
                        "hotkey trigger '{trigger}' contains more than one non-modifier token"
                    ));
                }
                key_token = Some(token.to_string());
            }
        }
    }

    let Some(key_token) = key_token else {
        return Err(format!("hotkey trigger '{trigger}' does not contain a key"));
    };

    Ok(ParsedTrigger {
        register_modifiers: required_modifiers | MOD_NOREPEAT,
        required_modifiers,
        key: resolve_virtual_key(&key_token)?,
    })
}

pub(super) fn resolve_virtual_key(token: &str) -> Result<u32, String> {
    let normalized = token.trim().to_ascii_uppercase();
    if normalized.len() == 1 {
        let value = normalized.as_bytes()[0];
        if value.is_ascii_uppercase() || value.is_ascii_digit() {
            return Ok(u32::from(value));
        }
    }

    match normalized.as_str() {
        "SPACE" => Ok(0x20),
        "TAB" => Ok(0x09),
        "ENTER" => Ok(0x0D),
        "ESC" | "ESCAPE" => Ok(0x1B),
        "BACKSPACE" => Ok(0x08),
        "DELETE" | "DEL" => Ok(0x2E),
        "HOME" => Ok(0x24),
        "END" => Ok(0x23),
        "PAGEUP" | "PGUP" => Ok(0x21),
        "PAGEDOWN" | "PGDN" => Ok(0x22),
        "LEFT" => Ok(0x25),
        "UP" => Ok(0x26),
        "RIGHT" => Ok(0x27),
        "DOWN" => Ok(0x28),
        _ if normalized.starts_with('F') => {
            let suffix = normalized.trim_start_matches('F');
            let number = suffix
                .parse::<u32>()
                .map_err(|_| format!("unsupported hotkey key token '{token}'"))?;
            if (1..=24).contains(&number) {
                Ok(0x70 + number - 1)
            } else {
                Err(format!("unsupported hotkey key token '{token}'"))
            }
        }
        _ => Err(format!("unsupported hotkey key token '{token}'")),
    }
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{MOD_CONTROL, MOD_NOREPEAT, MOD_WIN};

    use super::{parse_trigger, resolve_virtual_key};

    #[test]
    fn parses_super_control_hotkey() {
        let parsed = parse_trigger("Win+Ctrl+L").expect("trigger should parse");
        assert_eq!(parsed.required_modifiers, MOD_CONTROL | MOD_WIN);
        assert_eq!(
            parsed.register_modifiers,
            MOD_CONTROL | MOD_WIN | MOD_NOREPEAT
        );
        assert_eq!(parsed.key, u32::from(b'L'));
    }

    #[test]
    fn rejects_multiple_non_modifier_tokens() {
        let error = parse_trigger("Win+Ctrl+L+K").expect_err("trigger should fail");
        assert!(error.contains("more than one non-modifier"));
    }

    #[test]
    fn resolves_function_keys() {
        assert_eq!(resolve_virtual_key("F1").expect("F1 should parse"), 0x70);
        assert_eq!(resolve_virtual_key("F24").expect("F24 should parse"), 0x87);
    }
}
