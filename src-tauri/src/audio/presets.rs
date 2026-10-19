//! EQ presets (F7.7): built-in ones (each with enough preamp cut that it cannot clip) and the
//! user's own. Settings that match no preset are shown as "Custom".

use serde::{Deserialize, Serialize};

use crate::audio::eq::EqSettings;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Preset {
    pub name: String,
    pub preamp_db: f32,
    pub gains_db: [f32; 10],
}

impl Preset {
    pub fn settings(&self) -> EqSettings {
        EqSettings { enabled: true, preamp_db: self.preamp_db, gains_db: self.gains_db }
    }
}

/// (name, preamp, gains 31 Hz … 16 kHz). Preamp = minus the largest boost, so none can clip.
const BUILT_IN: [(&str, f32, [f32; 10]); 9] = [
    ("Flat", 0.0, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Bass boost", -6.0, [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Vocal", -4.0, [-2.0, -1.0, 0.0, 2.0, 4.0, 4.0, 3.0, 1.0, 0.0, -1.0]),
    ("Rock", -5.0, [5.0, 3.0, 1.0, -1.0, -2.0, 0.0, 2.0, 3.0, 4.0, 4.0]),
    ("Classical", -3.0, [3.0, 2.0, 1.0, 0.0, 0.0, 0.0, -1.0, 1.0, 2.0, 3.0]),
    ("Jazz", -3.0, [3.0, 2.0, 1.0, 2.0, -1.0, -1.0, 0.0, 1.0, 2.0, 3.0]),
    ("Acoustic", -3.0, [3.0, 3.0, 2.0, 1.0, 1.0, 1.0, 2.0, 2.0, 2.0, 1.0]),
    ("Treble boost", -6.0, [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 4.0, 5.0, 6.0]),
    ("Loudness", -6.0, [6.0, 4.0, 1.0, 0.0, -1.0, 0.0, -1.0, 1.0, 4.0, 5.0]),
];

pub fn built_in() -> Vec<Preset> {
    BUILT_IN.iter().map(|(n, p, g)| Preset { name: n.to_string(), preamp_db: *p, gains_db: *g }).collect()
}

/// The boosted signal could exceed full scale: preamp + largest boost above 0 dB (EQ on).
pub fn clip_risk(s: &EqSettings) -> bool {
    let largest = s.gains_db.iter().copied().fold(0.0f32, f32::max);
    s.enabled && s.preamp_db + largest > 0.0
}

/// Name of the built-in or user preset these settings equal, if any.
pub fn match_preset(s: &EqSettings, user: &[Preset]) -> Option<String> {
    let same = |a: f32, b: f32| (a - b).abs() < 0.05;
    built_in()
        .into_iter()
        .chain(user.iter().cloned())
        .find(|p| same(p.preamp_db, s.preamp_db) && p.gains_db.iter().zip(s.gains_db).all(|(&a, b)| same(a, b)))
        .map(|p| p.name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on(preamp: f32, gains: [f32; 10]) -> EqSettings {
        EqSettings { enabled: true, preamp_db: preamp, gains_db: gains }
    }

    #[test]
    fn nine_built_in_presets_in_menu_order() {
        let presets = built_in();
        let names: Vec<&str> = presets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Flat", "Bass boost", "Vocal", "Rock", "Classical", "Jazz", "Acoustic", "Treble boost", "Loudness"]);
    }

    #[test]
    fn no_built_in_preset_can_clip() {
        for p in built_in() {
            assert!(!clip_risk(&p.settings()), "{} can clip", p.name);
        }
    }

    #[test]
    fn clipping_warning_when_preamp_plus_largest_boost_is_above_zero() {
        assert!(!clip_risk(&on(0.0, [0.0; 10])));
        let mut g = [0.0; 10];
        g[0] = 3.0;
        assert!(clip_risk(&on(0.0, g)));
        assert!(!clip_risk(&on(-3.0, g)));
        assert!(!clip_risk(&EqSettings { enabled: false, ..on(0.0, g) }), "off cannot clip");
        assert!(!clip_risk(&on(0.0, [-6.0; 10])), "cuts only");
    }

    #[test]
    fn settings_that_match_a_preset_show_its_name() {
        let bass = built_in().into_iter().find(|p| p.name == "Bass boost").unwrap();
        assert_eq!(match_preset(&bass.settings(), &[]).as_deref(), Some("Bass boost"));
        assert_eq!(match_preset(&on(0.0, [0.0; 10]), &[]).as_deref(), Some("Flat"));
    }

    #[test]
    fn edited_settings_are_custom() {
        let mut s = built_in()[1].settings();
        s.gains_db[4] += 1.0;
        assert_eq!(match_preset(&s, &[]), None);
    }

    #[test]
    fn user_presets_are_matched_too() {
        let mine = Preset { name: "Car".into(), preamp_db: -2.0, gains_db: [2.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0] };
        assert_eq!(match_preset(&mine.settings(), &[mine.clone()]).as_deref(), Some("Car"));
    }
}
