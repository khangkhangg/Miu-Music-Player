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

const SECOND_OUTPUT: &str = "second_output";
const DUAL_MODE: &str = "dual_mode";
const DUAL_DELAYS: &str = "dual_delays";

/// Name of the second output device (F7.9, experimental); `None` = play on one output.
pub fn second_output(db: &Db) -> Option<String> {
    db.setting_json::<Option<String>>(SECOND_OUTPUT).ok().flatten().flatten()
}

pub fn set_second_output(db: &Db, name: Option<&str>) -> Result<()> {
    db.set_setting_json(SECOND_OUTPUT, &name)
}

/// Same sound on both outputs (default) or stereo split.
pub fn dual_mode(db: &Db) -> crate::audio::dual::DualMode {
    db.setting_json(DUAL_MODE).ok().flatten().unwrap_or_default()
}

pub fn set_dual_mode(db: &Db, mode: crate::audio::dual::DualMode) -> Result<()> {
    db.set_setting_json(DUAL_MODE, &mode)
}

/// One key per pair of devices whichever is first; `true` when `first` sorts after `second`.
fn pair_key(first: &str, second: &str) -> (String, bool) {
    let swapped = first > second;
    let (a, b) = if swapped { (second, first) } else { (first, second) };
    (format!("{a}\u{1f}{b}"), swapped)
}

/// Delays (ms) of `first` and `second` when they play together, as last set for that pair.
pub fn dual_delays(db: &Db, first: &str, second: &str) -> [u32; 2] {
    let all: std::collections::HashMap<String, [u32; 2]> = db.setting_json(DUAL_DELAYS).ok().flatten().unwrap_or_default();
    let (key, swapped) = pair_key(first, second);
    let [a, b] = all.get(&key).copied().unwrap_or_default().map(|d| d.min(crate::audio::player::MAX_DELAY_MS));
    if swapped { [b, a] } else { [a, b] }
}

pub fn set_dual_delays(db: &Db, first: &str, second: &str, delays: [u32; 2]) -> Result<()> {
    let mut all: std::collections::HashMap<String, [u32; 2]> = db.setting_json(DUAL_DELAYS).ok().flatten().unwrap_or_default();
    let (key, swapped) = pair_key(first, second);
    let [a, b] = delays.map(|d| d.min(crate::audio::player::MAX_DELAY_MS));
    all.insert(key, if swapped { [b, a] } else { [a, b] });
    db.set_setting_json(DUAL_DELAYS, &all)
}

const DUAL_BALANCE: &str = "dual_balance";

/// Balance (dB) between `first` and `second` when they play together: positive turns the second
/// one down. Remembered per pair, like the delays.
pub fn dual_balance(db: &Db, first: &str, second: &str) -> f32 {
    let all: std::collections::HashMap<String, f32> = db.setting_json(DUAL_BALANCE).ok().flatten().unwrap_or_default();
    let (key, swapped) = pair_key(first, second);
    let v = all.get(&key).copied().unwrap_or(0.0);
    if swapped { -v } else { v }
}

pub fn set_dual_balance(db: &Db, first: &str, second: &str, balance_db: f32) -> Result<()> {
    let mut all: std::collections::HashMap<String, f32> = db.setting_json(DUAL_BALANCE).ok().flatten().unwrap_or_default();
    let (key, swapped) = pair_key(first, second);
    let max = crate::audio::dual::MAX_BALANCE_DB;
    let v = if balance_db.is_nan() { 0.0 } else { balance_db.clamp(-max, max) };
    all.insert(key, if swapped { -v } else { v });
    db.set_setting_json(DUAL_BALANCE, &all)
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
    pub podcast_cleanup_days: Option<u32>,
    pub auto_update: bool,
    /// Look lyrics up online (LRCLIB) when a song has none of its own.
    pub lyrics_online: bool,
    /// The lyrics view is open in the now-playing column.
    pub lyrics_shown: bool,
    /// Keyboard shortcuts work anywhere in Windows (K.1).
    pub shortcuts_enabled: bool,
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
        podcast_cleanup_days: podcast_cleanup_days(db),
        auto_update: auto_update(db),
        lyrics_online: lyrics_online(db),
        lyrics_shown: lyrics_shown(db),
        shortcuts_enabled: shortcuts_enabled(db),
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
        "podcastCleanupDays" => set_podcast_cleanup_days(db, get(value)?).map_err(db_err),
        "autoUpdate" => db.set_setting_json(AUTO_UPDATE, &get::<bool>(value)?).map_err(db_err),
        "lyricsOnline" => db.set_setting_json(LYRICS_ONLINE, &get::<bool>(value)?).map_err(db_err),
        "lyricsShown" => db.set_setting_json(LYRICS_SHOWN, &get::<bool>(value)?).map_err(db_err),
        "shortcutsEnabled" => db.set_setting_json(SHORTCUTS_ENABLED, &get::<bool>(value)?).map_err(db_err),
        _ => Err(format!("unknown setting {name:?}")),
    }
}

const SHORTCUTS_ENABLED: &str = "shortcuts_enabled";
const SHORTCUT_KEYS: &str = "shortcut_keys";

/// Keyboard shortcuts work anywhere in Windows (default: yes).
pub fn shortcuts_enabled(db: &Db) -> bool {
    db.setting_json::<bool>(SHORTCUTS_ENABLED).ok().flatten().unwrap_or(true)
}

/// The keys of every action (`shortcuts::ACTIONS` order): the saved ones, else the default;
/// `None` = turned off. A saved value that no longer reads as keys falls back to the default.
pub fn shortcut_bindings(db: &Db) -> Vec<(crate::shortcuts::Action, Option<String>)> {
    let saved: std::collections::HashMap<crate::shortcuts::Action, String> = db.setting_json(SHORTCUT_KEYS).ok().flatten().unwrap_or_default();
    crate::shortcuts::ACTIONS
        .iter()
        .map(|&a| {
            let keys = match saved.get(&a) {
                None => Some(a.default_keys().to_string()),
                Some(k) if k.is_empty() => None,
                Some(k) => Some(crate::shortcuts::normalize(k).unwrap_or_else(|_| a.default_keys().to_string())),
            };
            (a, keys)
        })
        .collect()
}

/// Give `action` new keys (`None` turns it off). Refused when another action has them.
pub fn set_shortcut(db: &Db, action: crate::shortcuts::Action, keys: Option<&str>) -> std::result::Result<(), String> {
    let keys = keys.map(str::trim).filter(|k| !k.is_empty()).map(crate::shortcuts::normalize).transpose()?;
    if let Some(k) = &keys {
        if let Some((other, _)) = shortcut_bindings(db).into_iter().find(|(a, b)| *a != action && b.as_deref() == Some(k.as_str())) {
            let name = serde_json::to_value(other).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
            return Err(format!("{k} is already used for {name}"));
        }
    }
    let mut saved: std::collections::HashMap<crate::shortcuts::Action, String> = db.setting_json(SHORTCUT_KEYS).ok().flatten().unwrap_or_default();
    saved.insert(action, keys.unwrap_or_default());
    db.set_setting_json(SHORTCUT_KEYS, &saved).map_err(|e| e.to_string())
}

/// Every action back to its default keys.
pub fn reset_shortcuts(db: &Db) -> Result<()> {
    db.set_setting_json(SHORTCUT_KEYS, &std::collections::HashMap::<crate::shortcuts::Action, String>::new())
}

const LYRICS_ONLINE: &str = "lyrics_online";
const LYRICS_SHOWN: &str = "lyrics_shown";

/// Look lyrics up on LRCLIB when a song has none of its own (default: yes).
pub fn lyrics_online(db: &Db) -> bool {
    db.setting_json::<bool>(LYRICS_ONLINE).ok().flatten().unwrap_or(true)
}

/// The lyrics view is open (default: no).
pub fn lyrics_shown(db: &Db) -> bool {
    db.setting_json::<bool>(LYRICS_SHOWN).ok().flatten().unwrap_or(false)
}

const PODCAST_CLEANUP_DAYS: &str = "podcast_cleanup_days";

/// Days after which a played episode's download is deleted; `None` = never (default: 7).
pub fn podcast_cleanup_days(db: &Db) -> Option<u32> {
    match db.setting_json::<Option<u32>>(PODCAST_CLEANUP_DAYS) {
        Ok(Some(choice)) => choice,
        _ => Some(7),
    }
}

pub fn set_podcast_cleanup_days(db: &Db, days: Option<u32>) -> Result<()> {
    db.set_setting_json(PODCAST_CLEANUP_DAYS, &days)
}

const AUTO_UPDATE: &str = "auto_update";
const LAST_UPDATE_CHECK: &str = "last_update_check";

/// Look for a new version of Miu in the background (default: yes).
pub fn auto_update(db: &Db) -> bool {
    db.setting_json::<bool>(AUTO_UPDATE).ok().flatten().unwrap_or(true)
}

/// Remember when updates were last looked for (unix seconds).
pub fn set_last_update_check(db: &Db, now: i64) -> Result<()> {
    db.set_setting_json(LAST_UPDATE_CHECK, &now)
}

/// An automatic check is due when it is on and the last one was a day ago or more (or in the future).
pub fn update_check_due(db: &Db, now: i64) -> bool {
    const DAY: i64 = 24 * 3600;
    let last = db.setting_json::<i64>(LAST_UPDATE_CHECK).ok().flatten();
    auto_update(db) && last.is_none_or(|t| now - t >= DAY || now < t)
}

const SHUFFLE: &str = "shuffle";
const REPEAT: &str = "repeat";

/// Play the queue in random order (default: no).
pub fn shuffle(db: &Db) -> bool {
    db.setting_json::<bool>(SHUFFLE).ok().flatten().unwrap_or(false)
}

pub fn set_shuffle(db: &Db, on: bool) -> Result<()> {
    db.set_setting_json(SHUFFLE, &on)
}

/// Repeat off, the whole queue, or one song (default: off).
pub fn repeat(db: &Db) -> crate::audio::order::Repeat {
    db.setting_json(REPEAT).ok().flatten().unwrap_or_default()
}

pub fn set_repeat(db: &Db, r: crate::audio::order::Repeat) -> Result<()> {
    db.set_setting_json(REPEAT, &r)
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
mod second_output_tests {
    use super::*;
    use crate::audio::dual::DualMode;

    #[test]
    fn there_is_no_second_output_until_one_is_chosen() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(second_output(&db), None);
        assert_eq!(dual_mode(&db), DualMode::Same);
    }

    #[test]
    fn the_second_output_and_its_mode_are_remembered() {
        let db = Db::open_in_memory().unwrap();
        set_second_output(&db, Some("JBL Flip 6")).unwrap();
        set_dual_mode(&db, DualMode::Split).unwrap();
        assert_eq!(second_output(&db).as_deref(), Some("JBL Flip 6"));
        assert_eq!(dual_mode(&db), DualMode::Split);
        set_second_output(&db, None).unwrap();
        assert_eq!(second_output(&db), None);
    }

    #[test]
    fn delays_are_remembered_per_pair_of_devices() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(dual_delays(&db, "Bose A", "JBL B"), [0, 0]);
        set_dual_delays(&db, "Bose A", "JBL B", [0, 180]).unwrap();
        set_dual_delays(&db, "Bose A", "Headphones", [40, 0]).unwrap();
        assert_eq!(dual_delays(&db, "Bose A", "JBL B"), [0, 180]);
        assert_eq!(dual_delays(&db, "Bose A", "Headphones"), [40, 0]);
        assert_eq!(dual_delays(&db, "JBL B", "Bose A"), [180, 0], "the same pair the other way round");
    }

    #[test]
    fn delays_are_kept_within_half_a_second() {
        let db = Db::open_in_memory().unwrap();
        set_dual_delays(&db, "A", "B", [9_000, 20]).unwrap();
        assert_eq!(dual_delays(&db, "A", "B"), [500, 20]);
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
            podcast_cleanup_days: Some(7),
            auto_update: true,
            lyrics_online: true,
            lyrics_shown: false,
            shortcuts_enabled: true,
        });
    }

    #[test]
    fn each_setting_can_be_changed_by_name() {
        let db = Db::open_in_memory().unwrap();
        change(&db, "theme", &serde_json::json!("dark")).unwrap();
        change(&db, "miniOnTop", &serde_json::json!(false)).unwrap();
        change(&db, "visualizerStyle", &serde_json::json!("line")).unwrap();
        change(&db, "lyricsOnline", &serde_json::json!(false)).unwrap();
        change(&db, "lyricsShown", &serde_json::json!(true)).unwrap();
        change(&db, "conversion", &serde_json::json!({ "hiRes": "alac16_44", "lossless": "aac256" })).unwrap();
        let s = all(&db);
        assert_eq!((s.theme.as_str(), s.mini_on_top, s.visualizer_style.as_str()), ("dark", false, "line"));
        assert_eq!(s.conversion, Policy { hi_res: HiRes::Alac16_44, lossless: Lossless::Aac256 });
        assert!(!s.lyrics_online && s.lyrics_shown);
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

#[cfg(test)]
mod play_order_tests {
    use super::*;
    use crate::audio::order::Repeat;

    #[test]
    fn songs_play_in_list_order_without_repeat_by_default() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!((shuffle(&db), repeat(&db)), (false, Repeat::Off));
    }

    #[test]
    fn shuffle_and_repeat_are_remembered() {
        let db = Db::open_in_memory().unwrap();
        set_shuffle(&db, true).unwrap();
        set_repeat(&db, Repeat::One).unwrap();
        assert_eq!((shuffle(&db), repeat(&db)), (true, Repeat::One));
    }
}

#[cfg(test)]
mod podcast_cleanup_tests {
    use super::*;

    #[test]
    fn played_downloads_are_cleared_after_a_week_by_default() {
        assert_eq!(podcast_cleanup_days(&Db::open_in_memory().unwrap()), Some(7));
    }

    #[test]
    fn the_choice_including_never_is_remembered_through_the_settings_page() {
        let db = Db::open_in_memory().unwrap();
        change(&db, "podcastCleanupDays", &serde_json::json!(30)).unwrap();
        assert_eq!(podcast_cleanup_days(&db), Some(30));
        assert_eq!(all(&db).podcast_cleanup_days, Some(30));
        change(&db, "podcastCleanupDays", &serde_json::json!(null)).unwrap();
        assert_eq!(podcast_cleanup_days(&db), None, "never");
        assert!(change(&db, "podcastCleanupDays", &serde_json::json!(-3)).is_err());
    }
}

#[cfg(test)]
mod balance_tests {
    use super::*;

    #[test]
    fn balance_is_remembered_per_pair_and_flips_with_the_order() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(dual_balance(&db, "Klipsch", "Bose"), 0.0);
        set_dual_balance(&db, "Klipsch", "Bose", 4.5).unwrap();
        assert_eq!(dual_balance(&db, "Klipsch", "Bose"), 4.5);
        assert_eq!(dual_balance(&db, "Bose", "Klipsch"), -4.5, "turning the second down = turning the first down the other way round");
        set_dual_balance(&db, "Klipsch", "Bose", 80.0).unwrap();
        assert_eq!(dual_balance(&db, "Klipsch", "Bose"), crate::audio::dual::MAX_BALANCE_DB);
    }
}

#[cfg(test)]
mod update_tests {
    use super::*;

    #[test]
    fn updates_are_checked_automatically_by_default_and_the_choice_is_remembered() {
        let db = Db::open_in_memory().unwrap();
        assert!(auto_update(&db));
        assert!(all(&db).auto_update);
        change(&db, "autoUpdate", &serde_json::json!(false)).unwrap();
        assert!(!auto_update(&db));
        assert!(!all(&db).auto_update);
    }

    #[test]
    fn an_automatic_check_is_due_at_most_once_a_day() {
        let db = Db::open_in_memory().unwrap();
        let now = 1_800_000_000;
        assert!(update_check_due(&db, now), "never checked");
        set_last_update_check(&db, now).unwrap();
        assert!(!update_check_due(&db, now + 3600));
        assert!(update_check_due(&db, now + 24 * 3600));
        assert!(update_check_due(&db, now - 3600), "a clock set back does not stop checks for good");
        change(&db, "autoUpdate", &serde_json::json!(false)).unwrap();
        assert!(!update_check_due(&db, now + 48 * 3600), "off means off");
    }
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;
    use crate::shortcuts::{Action, ACTIONS};

    #[test]
    fn shortcuts_are_on_with_their_default_keys_at_first() {
        let db = Db::open_in_memory().unwrap();
        assert!(shortcuts_enabled(&db));
        let b = shortcut_bindings(&db);
        assert_eq!(b.len(), ACTIONS.len());
        assert!(b.iter().all(|(a, k)| k.as_deref() == Some(a.default_keys())));
    }

    #[test]
    fn a_shortcut_can_be_changed_turned_off_and_reset() {
        let db = Db::open_in_memory().unwrap();
        set_shortcut(&db, Action::Sync, Some("shift+ctrl+y")).unwrap();
        set_shortcut(&db, Action::Like, None).unwrap();
        let b: std::collections::HashMap<_, _> = shortcut_bindings(&db).into_iter().collect();
        assert_eq!(b[&Action::Sync].as_deref(), Some("Ctrl+Shift+Y"), "saved in its one spelling");
        assert_eq!(b[&Action::Like], None);
        reset_shortcuts(&db).unwrap();
        assert!(shortcut_bindings(&db).iter().all(|(a, k)| k.as_deref() == Some(a.default_keys())));
    }

    #[test]
    fn keys_already_used_or_unusable_are_refused() {
        let db = Db::open_in_memory().unwrap();
        let err = set_shortcut(&db, Action::Sync, Some("Ctrl+Alt+P")).unwrap_err();
        assert!(err.contains("playPause"), "says which action has them: {err}");
        assert!(set_shortcut(&db, Action::Sync, Some("Space")).is_err());
        set_shortcut(&db, Action::PlayPause, Some("Ctrl+Alt+P")).unwrap();
        assert_eq!(shortcut_bindings(&db)[0].1.as_deref(), Some("Ctrl+Alt+P"), "giving an action its own keys again is fine");
    }

    #[test]
    fn shortcuts_can_be_turned_off_altogether() {
        let db = Db::open_in_memory().unwrap();
        change(&db, "shortcutsEnabled", &serde_json::json!(false)).unwrap();
        assert!(!shortcuts_enabled(&db));
        assert!(!all(&db).shortcuts_enabled);
    }
}
