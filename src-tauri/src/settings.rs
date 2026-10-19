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
