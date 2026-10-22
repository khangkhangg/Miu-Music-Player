//! Keyboard shortcuts that work anywhere in Windows (K.1): play/pause, next, previous, like,
//! dislike, mini player, restore the window, sync. This part is pure: the actions, their default
//! keys, and checking key combinations; `shortcuts_service` registers them with Windows.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    PlayPause,
    Next,
    Previous,
    Like,
    Dislike,
    MiniPlayer,
    Restore,
    Sync,
}

pub const ACTIONS: [Action; 8] = [
    Action::PlayPause,
    Action::Next,
    Action::Previous,
    Action::Like,
    Action::Dislike,
    Action::MiniPlayer,
    Action::Restore,
    Action::Sync,
];

impl Action {
    /// The default keys: all Ctrl+Alt, which few apps take for themselves.
    pub fn default_keys(self) -> &'static str {
        match self {
            Action::PlayPause => "Ctrl+Alt+P",
            Action::Next => "Ctrl+Alt+Right",
            Action::Previous => "Ctrl+Alt+Left",
            Action::Like => "Ctrl+Alt+Up",
            Action::Dislike => "Ctrl+Alt+Down",
            Action::MiniPlayer => "Ctrl+Alt+M",
            Action::Restore => "Ctrl+Alt+R",
            Action::Sync => "Ctrl+Alt+S",
        }
    }
}

/// A key combination in one spelling ("Ctrl+Alt+Space"): modifiers in the order Ctrl, Alt,
/// Shift, Win, then one key. At least one modifier is needed (a bare key would stop working in
/// every other app), except for F13–F24, which nothing else uses.
pub fn normalize(keys: &str) -> Result<String, String> {
    const ORDER: [&str; 4] = ["Ctrl", "Alt", "Shift", "Win"];
    let mut mods = [false; 4];
    let mut key: Option<String> = None;
    for part in keys.split('+').map(str::trim) {
        if part.is_empty() {
            return Err(format!("{keys:?} is not a key combination"));
        }
        let p = part.to_lowercase();
        let m = match p.as_str() {
            "ctrl" | "control" => Some(0),
            "alt" | "option" => Some(1),
            "shift" => Some(2),
            "win" | "super" | "meta" | "cmd" | "command" => Some(3),
            _ => None,
        };
        if let Some(i) = m {
            if mods[i] {
                return Err(format!("{} twice in {keys:?}", ORDER[i]));
            }
            mods[i] = true;
            continue;
        }
        if key.is_some() {
            return Err(format!("{keys:?} has more than one key"));
        }
        key = Some(key_name(&p).ok_or_else(|| format!("unknown key {part:?}"))?);
    }
    let key = key.ok_or_else(|| format!("{keys:?} has no key besides Ctrl, Alt, Shift or Win"))?;
    let lone_ok = key.strip_prefix('F').and_then(|n| n.parse::<u32>().ok()).is_some_and(|n| n >= 13);
    if !mods.iter().any(|&m| m) && !lone_ok {
        return Err(format!("{key} needs Ctrl, Alt, Shift or Win with it"));
    }
    let mut parts: Vec<String> = ORDER.iter().zip(mods).filter(|(_, on)| *on).map(|(m, _)| m.to_string()).collect();
    parts.push(key);
    Ok(parts.join("+"))
}

/// The one spelling of a (lower-cased) key name, if Miu accepts it.
fn key_name(p: &str) -> Option<String> {
    let named = match p {
        "space" | "spacebar" => "Space",
        "left" | "arrowleft" => "Left",
        "right" | "arrowright" => "Right",
        "up" | "arrowup" => "Up",
        "down" | "arrowdown" => "Down",
        "home" => "Home",
        "end" => "End",
        "pageup" => "PageUp",
        "pagedown" => "PageDown",
        "insert" => "Insert",
        "delete" => "Delete",
        _ => "",
    };
    if !named.is_empty() {
        return Some(named.into());
    }
    let mut chars = p.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_alphanumeric() {
            return Some(c.to_ascii_uppercase().to_string());
        }
    }
    let n: u32 = p.strip_prefix('f')?.parse().ok()?;
    (1..=24).contains(&n).then(|| format!("F{n}"))
}

/// Pairs of actions given the same keys.
pub fn duplicates(bindings: &[(Action, Option<String>)]) -> Vec<(Action, Action)> {
    let mut out = Vec::new();
    for (i, (a, ka)) in bindings.iter().enumerate() {
        for (b, kb) in &bindings[i + 1..] {
            if ka.is_some() && ka == kb {
                out.push((*a, *b));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_default_is_valid_and_different() {
        let all: Vec<(Action, Option<String>)> = ACTIONS.iter().map(|&a| (a, Some(normalize(a.default_keys()).unwrap()))).collect();
        for (a, k) in &all {
            assert_eq!(k.as_deref(), Some(a.default_keys()), "defaults are already in normal form");
        }
        assert!(duplicates(&all).is_empty());
    }

    #[test]
    fn spellings_become_one_form() {
        assert_eq!(normalize("ctrl+alt+space").unwrap(), "Ctrl+Alt+Space");
        assert_eq!(normalize("Alt + Control + p").unwrap(), "Ctrl+Alt+P");
        assert_eq!(normalize("shift+ctrl+ArrowRight").unwrap(), "Ctrl+Shift+Right");
        assert_eq!(normalize("Win+Shift+f5").unwrap(), "Shift+Win+F5");
        assert_eq!(normalize("Super+1").unwrap(), "Win+1");
        assert_eq!(normalize("ctrl+alt+pageup").unwrap(), "Ctrl+Alt+PageUp");
        assert_eq!(normalize("F13").unwrap(), "F13", "F13–F24 may stand alone");
    }

    #[test]
    fn combinations_that_cannot_work_are_refused() {
        assert!(normalize("Space").is_err(), "a bare key would break typing in every app");
        assert!(normalize("Ctrl+Alt").is_err(), "no key");
        assert!(normalize("Ctrl+A+B").is_err(), "two keys");
        assert!(normalize("Ctrl+Alt+Banana").is_err(), "unknown key");
        assert!(normalize("").is_err());
        assert!(normalize("Ctrl+Ctrl+X").is_err(), "a modifier twice");
    }

    #[test]
    fn the_same_keys_for_two_actions_are_found() {
        let b = vec![
            (Action::Next, Some("Ctrl+Alt+N".to_string())),
            (Action::Like, Some("Ctrl+Alt+N".to_string())),
            (Action::Sync, None),
            (Action::Restore, None),
        ];
        assert_eq!(duplicates(&b), vec![(Action::Next, Action::Like)], "turned-off actions never clash");
    }
}
