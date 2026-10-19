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

const EQ: &str = "eq";
const EQ_USER_PRESETS: &str = "eq_user_presets";

/// Current equalizer settings (default: on and flat, which is bit-transparent).
pub fn eq_settings(db: &Db) -> crate::audio::eq::EqSettings {
    db.setting_json(EQ).ok().flatten().unwrap_or_default()
}

pub fn set_eq_settings(db: &Db, s: &crate::audio::eq::EqSettings) -> Result<()> {
    db.set_setting_json(EQ, &s.clamped())
}

/// The user's own presets, sorted by name.
pub fn user_presets(db: &Db) -> Vec<crate::audio::presets::Preset> {
    let mut list: Vec<crate::audio::presets::Preset> = db.setting_json(EQ_USER_PRESETS).ok().flatten().unwrap_or_default();
    list.sort_by_key(|p| p.name.to_lowercase());
    list
}

/// Save (or overwrite, ignoring case) a user preset. Built-in names are reserved.
pub fn save_user_preset(db: &Db, name: &str, s: &crate::audio::eq::EqSettings) -> std::result::Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("give the preset a name".into());
    }
    if crate::audio::presets::built_in().iter().any(|p| p.name.eq_ignore_ascii_case(name)) {
        return Err(format!("\"{name}\" is a built-in preset"));
    }
    let s = s.clamped();
    let mut list = user_presets(db);
    list.retain(|p| !p.name.eq_ignore_ascii_case(name));
    list.push(crate::audio::presets::Preset { name: name.to_string(), preamp_db: s.preamp_db, gains_db: s.gains_db });
    db.set_setting_json(EQ_USER_PRESETS, &list).map_err(|e| e.to_string())
}

pub fn delete_user_preset(db: &Db, name: &str) -> Result<()> {
    let mut list = user_presets(db);
    list.retain(|p| !p.name.eq_ignore_ascii_case(name));
    db.set_setting_json(EQ_USER_PRESETS, &list)
}

const OUTPUT_DEVICE: &str = "output_device";

/// Name of the chosen output device; `None` = follow the Windows default.
pub fn output_device(db: &Db) -> Option<String> {
    db.setting_json::<Option<String>>(OUTPUT_DEVICE).ok().flatten().flatten()
}

pub fn set_output_device(db: &Db, name: Option<&str>) -> Result<()> {
    db.set_setting_json(OUTPUT_DEVICE, &name)
}

const EXCLUSIVE_OUTPUT: &str = "exclusive_output";

/// Take the device in exclusive (bit-perfect) mode (default: no, share it with other apps).
pub fn exclusive_output(db: &Db) -> bool {
    db.setting_json::<bool>(EXCLUSIVE_OUTPUT).ok().flatten().unwrap_or(false)
}

pub fn set_exclusive_output(db: &Db, on: bool) -> Result<()> {
    db.set_setting_json(EXCLUSIVE_OUTPUT, &on)
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

#[cfg(test)]
mod eq_tests {
    use super::*;
    use crate::audio::eq::EqSettings;

    fn custom() -> EqSettings {
        EqSettings { enabled: true, preamp_db: -2.0, gains_db: [2.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0] }
    }

    #[test]
    fn eq_starts_flat_and_on() {
        assert_eq!(eq_settings(&Db::open_in_memory().unwrap()), EqSettings::default());
    }

    #[test]
    fn eq_settings_are_remembered() {
        let db = Db::open_in_memory().unwrap();
        set_eq_settings(&db, &custom()).unwrap();
        assert_eq!(eq_settings(&db), custom());
    }

    #[test]
    fn user_presets_save_overwrite_and_delete() {
        let db = Db::open_in_memory().unwrap();
        save_user_preset(&db, "Car", &custom()).unwrap();
        save_user_preset(&db, "Bedroom", &EqSettings::default()).unwrap();
        save_user_preset(&db, "car", &EqSettings::default()).unwrap(); // same name, different case: overwrite
        let names: Vec<String> = user_presets(&db).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Bedroom", "car"]);
        delete_user_preset(&db, "Bedroom").unwrap();
        assert_eq!(user_presets(&db).len(), 1);
    }

    #[test]
    fn preset_names_must_be_new_and_not_empty() {
        let db = Db::open_in_memory().unwrap();
        assert!(save_user_preset(&db, "  ", &custom()).is_err());
        assert!(save_user_preset(&db, "bass boost", &custom()).is_err(), "built-in name");
    }
}

#[cfg(test)]
mod output_tests {
    use super::*;

    #[test]
    fn output_follows_the_system_default_until_a_device_is_chosen() {
        assert_eq!(output_device(&Db::open_in_memory().unwrap()), None);
    }

    #[test]
    fn a_chosen_device_is_remembered_and_can_be_cleared() {
        let db = Db::open_in_memory().unwrap();
        set_output_device(&db, Some("Speakers (Realtek(R) Audio)")).unwrap();
        assert_eq!(output_device(&db).as_deref(), Some("Speakers (Realtek(R) Audio)"));
        set_output_device(&db, None).unwrap();
        assert_eq!(output_device(&db), None);
    }
}

#[cfg(test)]
mod exclusive_tests {
    use super::*;

    #[test]
    fn shared_mode_is_the_default() {
        assert!(!exclusive_output(&Db::open_in_memory().unwrap()));
    }

    #[test]
    fn exclusive_mode_is_remembered() {
        let db = Db::open_in_memory().unwrap();
        set_exclusive_output(&db, true).unwrap();
        assert!(exclusive_output(&db));
        set_exclusive_output(&db, false).unwrap();
        assert!(!exclusive_output(&db));
    }
}
