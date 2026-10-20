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

const CONVERSION: &str = "conversion";

/// How songs are converted for the iPhone (default: ALAC, hi-res as ALAC 24/48).
pub fn conversion_policy(db: &Db) -> crate::library::convert::Policy {
    db.setting_json(CONVERSION).ok().flatten().unwrap_or_default()
}

pub fn set_conversion_policy(db: &Db, p: &crate::library::convert::Policy) -> Result<()> {
    db.set_setting_json(CONVERSION, p)
}

const UI_LANGUAGE: &str = "ui_language";
/// "auto" follows the system; the others have a `src/i18n/<code>.json`.
pub const UI_LANGUAGES: [&str; 3] = ["auto", "en", "vi"];

/// The interface language setting (default and fallback: "auto").
pub fn ui_language(db: &Db) -> String {
    db.setting_json::<String>(UI_LANGUAGE)
        .ok()
        .flatten()
        .filter(|s| UI_LANGUAGES.contains(&s.as_str()))
        .unwrap_or_else(|| "auto".into())
}

pub fn set_ui_language(db: &Db, code: &str) -> std::result::Result<(), String> {
    if !UI_LANGUAGES.contains(&code) {
        return Err(format!("unknown language {code:?}"));
    }
    db.set_setting_json(UI_LANGUAGE, &code).map_err(|e| e.to_string())
}

const THEME: &str = "theme";
pub const THEMES: [&str; 3] = ["system", "light", "dark"];

/// Light, dark or following Windows (default and fallback: "dark", the redesign's look).
pub fn theme(db: &Db) -> String {
    db.setting_json::<String>(THEME).ok().flatten().filter(|s| THEMES.contains(&s.as_str())).unwrap_or_else(|| "dark".into())
}

/// Every user-facing setting at once, for the settings page (F5.5).
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub theme: String,
    pub language: String,
    pub mini_on_minimize: bool,
    pub mini_on_top: bool,
    pub visualizer_on: bool,
    pub visualizer_style: String,
    pub conversion: crate::library::convert::Policy,
    pub exclusive_output: bool,
}

pub fn all(db: &Db) -> AppSettings {
    AppSettings {
        theme: theme(db),
        language: ui_language(db),
        mini_on_minimize: mini_on_minimize(db),
        mini_on_top: mini_on_top(db),
        visualizer_on: visualizer_on(db),
        visualizer_style: visualizer_style(db),
        conversion: conversion_policy(db),
        exclusive_output: exclusive_output(db),
    }
}

/// Change one setting by its camelCase name, checking the value first.
pub fn change(db: &Db, name: &str, value: &serde_json::Value) -> std::result::Result<(), String> {
    fn get<T: serde::de::DeserializeOwned>(v: &serde_json::Value) -> std::result::Result<T, String> {
        serde_json::from_value(v.clone()).map_err(|e| format!("bad value: {e}"))
    }
    let db_err = |e: crate::library::db::DbError| e.to_string();
    match name {
        "theme" => {
            let t: String = get(value)?;
            if !THEMES.contains(&t.as_str()) {
                return Err(format!("unknown theme {t:?}"));
            }
            db.set_setting_json(THEME, &t).map_err(db_err)
        }
        "language" => set_ui_language(db, &get::<String>(value)?),
        "miniOnMinimize" => set_mini_on_minimize(db, get(value)?).map_err(db_err),
        "miniOnTop" => set_mini_on_top(db, get(value)?).map_err(db_err),
        "visualizerOn" => set_visualizer_on(db, get(value)?).map_err(db_err),
        "visualizerStyle" => set_visualizer_style(db, &get::<String>(value)?),
        "conversion" => set_conversion_policy(db, &get(value)?).map_err(db_err),
        "exclusiveOutput" => set_exclusive_output(db, get(value)?).map_err(db_err),
        _ => Err(format!("unknown setting {name:?}")),
    }
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

#[cfg(test)]
mod conversion_tests {
    use super::*;
    use crate::library::convert::{HiRes, Lossless, Policy};

    #[test]
    fn conversion_defaults_to_alac_and_24_48_for_hi_res() {
        assert_eq!(conversion_policy(&Db::open_in_memory().unwrap()), Policy { hi_res: HiRes::Alac24_48, lossless: Lossless::Alac });
    }

    #[test]
    fn the_conversion_choice_is_remembered() {
        let db = Db::open_in_memory().unwrap();
        let p = Policy { hi_res: HiRes::Alac16_44, lossless: Lossless::Aac256 };
        set_conversion_policy(&db, &p).unwrap();
        assert_eq!(conversion_policy(&db), p);
    }

    #[test]
    fn a_garbled_conversion_setting_falls_back_to_the_default() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting("conversion", "{\"hiRes\":\"lasers\"}").unwrap();
        assert_eq!(conversion_policy(&db), Policy::default());
    }
}

#[cfg(test)]
mod language_tests {
    use super::*;

    #[test]
    fn the_language_follows_the_system_by_default() {
        assert_eq!(ui_language(&Db::open_in_memory().unwrap()), "auto");
    }

    #[test]
    fn a_chosen_language_is_remembered_and_unknown_ones_refused() {
        let db = Db::open_in_memory().unwrap();
        set_ui_language(&db, "vi").unwrap();
        assert_eq!(ui_language(&db), "vi");
        assert!(set_ui_language(&db, "klingon").is_err());
        assert_eq!(ui_language(&db), "vi");
        db.set_setting_json("ui_language", &"klingon").unwrap();
        assert_eq!(ui_language(&db), "auto", "a bad stored value falls back");
    }
}

#[cfg(test)]
mod schema_tests {
    use super::*;
    use crate::library::convert::{HiRes, Lossless, Policy};

    #[test]
    fn a_fresh_install_has_these_defaults() {
        let s = all(&Db::open_in_memory().unwrap());
        assert_eq!(s, AppSettings {
            theme: "dark".into(),
            language: "auto".into(),
            mini_on_minimize: true,
            mini_on_top: true,
            visualizer_on: true,
            visualizer_style: "bars".into(),
            conversion: Policy { hi_res: HiRes::Alac24_48, lossless: Lossless::Alac },
            exclusive_output: false,
        });
    }

    #[test]
    fn each_setting_can_be_changed_by_name() {
        let db = Db::open_in_memory().unwrap();
        change(&db, "theme", &serde_json::json!("dark")).unwrap();
        change(&db, "miniOnTop", &serde_json::json!(false)).unwrap();
        change(&db, "visualizerStyle", &serde_json::json!("line")).unwrap();
        change(&db, "conversion", &serde_json::json!({ "hiRes": "alac16_44", "lossless": "aac256" })).unwrap();
        let s = all(&db);
        assert_eq!((s.theme.as_str(), s.mini_on_top, s.visualizer_style.as_str()), ("dark", false, "line"));
        assert_eq!(s.conversion, Policy { hi_res: HiRes::Alac16_44, lossless: Lossless::Aac256 });
    }

    #[test]
    fn bad_names_and_values_are_refused_and_change_nothing() {
        let db = Db::open_in_memory().unwrap();
        assert!(change(&db, "colour", &serde_json::json!("red")).is_err(), "unknown setting");
        assert!(change(&db, "theme", &serde_json::json!("purple")).is_err());
        assert!(change(&db, "miniOnTop", &serde_json::json!("yes")).is_err(), "wrong type");
        assert!(change(&db, "conversion", &serde_json::json!({ "hiRes": "8k" })).is_err());
        assert!(change(&db, "language", &serde_json::json!("klingon")).is_err());
        assert_eq!(all(&db), all(&Db::open_in_memory().unwrap()));
    }
}
