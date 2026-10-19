//! Typed app settings over the `settings` table (F5.5 grows this). Missing or unreadable values
//! fall back to defaults, so a bad value never breaks startup.

use crate::library::db::{Db, Result};

const MINI_ON_MINIMIZE: &str = "mini_on_minimize";

/// Show the mini player when the main window is minimized (default: yes).
pub fn mini_on_minimize(db: &Db) -> bool {
    db.setting_json::<bool>(MINI_ON_MINIMIZE).ok().flatten().unwrap_or(true)
}

pub fn set_mini_on_minimize(db: &Db, on: bool) -> Result<()> {
    db.set_setting_json(MINI_ON_MINIMIZE, &on)
}

const VISUALIZER_ON: &str = "visualizer_on";
const VISUALIZER_STYLE: &str = "visualizer_style";
pub const VISUALIZER_STYLES: [&str; 3] = ["bars", "mirrored", "line"];

/// Show the visualizer in the mini player (default: yes).
pub fn visualizer_on(db: &Db) -> bool {
    db.setting_json::<bool>(VISUALIZER_ON).ok().flatten().unwrap_or(true)
}

pub fn set_visualizer_on(db: &Db, on: bool) -> Result<()> {
    db.set_setting_json(VISUALIZER_ON, &on)
}

/// One of `VISUALIZER_STYLES` (default and fallback: "bars").
pub fn visualizer_style(db: &Db) -> String {
    db.setting_json::<String>(VISUALIZER_STYLE)
        .ok()
        .flatten()
        .filter(|s| VISUALIZER_STYLES.contains(&s.as_str()))
        .unwrap_or_else(|| "bars".into())
}

pub fn set_visualizer_style(db: &Db, style: &str) -> std::result::Result<(), String> {
    if !VISUALIZER_STYLES.contains(&style) {
        return Err(format!("unknown visualizer style {style:?}"));
    }
    db.set_setting_json(VISUALIZER_STYLE, &style).map_err(|e| e.to_string())
}

const MINI_ON_TOP: &str = "mini_on_top";

/// Keep the mini player above other windows (default: yes — the pin in the mini player).
pub fn mini_on_top(db: &Db) -> bool {
    db.setting_json::<bool>(MINI_ON_TOP).ok().flatten().unwrap_or(true)
}

pub fn set_mini_on_top(db: &Db, on: bool) -> Result<()> {
    db.set_setting_json(MINI_ON_TOP, &on)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mini_player_opens_on_minimize_by_default() {
        assert!(mini_on_minimize(&Db::open_in_memory().unwrap()));
    }

    #[test]
    fn the_choice_is_remembered() {
        let db = Db::open_in_memory().unwrap();
        set_mini_on_minimize(&db, false).unwrap();
        assert!(!mini_on_minimize(&db));
        set_mini_on_minimize(&db, true).unwrap();
        assert!(mini_on_minimize(&db));
    }

    #[test]
    fn a_garbled_stored_value_falls_back_to_the_default() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting("mini_on_minimize", "not json").unwrap();
        assert!(mini_on_minimize(&db));
    }
}

#[cfg(test)]
mod on_top_tests {
    use super::*;

    #[test]
    fn mini_player_stays_on_top_by_default() {
        assert!(mini_on_top(&Db::open_in_memory().unwrap()));
    }

    #[test]
    fn unpinning_is_remembered() {
        let db = Db::open_in_memory().unwrap();
        set_mini_on_top(&db, false).unwrap();
        assert!(!mini_on_top(&db));
        set_mini_on_top(&db, true).unwrap();
        assert!(mini_on_top(&db));
    }

    #[test]
    fn the_two_mini_settings_are_independent() {
        let db = Db::open_in_memory().unwrap();
        set_mini_on_top(&db, false).unwrap();
        assert!(mini_on_minimize(&db));
    }
}

#[cfg(test)]
mod visualizer_tests {
    use super::*;

    #[test]
    fn visualizer_is_on_with_bars_by_default() {
        let db = Db::open_in_memory().unwrap();
        assert!(visualizer_on(&db));
        assert_eq!(visualizer_style(&db), "bars");
    }

    #[test]
    fn visualizer_choices_are_remembered() {
        let db = Db::open_in_memory().unwrap();
        set_visualizer_on(&db, false).unwrap();
        set_visualizer_style(&db, "mirrored").unwrap();
        assert!(!visualizer_on(&db));
        assert_eq!(visualizer_style(&db), "mirrored");
    }

    #[test]
    fn unknown_styles_are_refused_and_fall_back_to_bars() {
        let db = Db::open_in_memory().unwrap();
        assert!(set_visualizer_style(&db, "lasers").is_err());
        db.set_setting_json("visualizer_style", &"lasers").unwrap();
        assert_eq!(visualizer_style(&db), "bars");
    }
}
