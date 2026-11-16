//! What in a game data set this sim doesn't know. A game update can bring a skill condition keyword, an ability type,
//! or a value/duration calculation the sim has never seen: those are silently without effect (an unknown condition
//! never fires). It can also bring a course whose geometry files aren't in the data folder: a race there panics.
//! [`check`] lists all of them, using the same parsers and lists the sim itself uses.

use crate::api::RaceInput;
use crate::data::course::lane_path_name;
use crate::data::GameData;
use crate::sim::horse::Horse;
use crate::sim::race::Race;
use crate::sim::skill::trigger::kind::create_trigger;
use crate::sim::skill::trigger::parse_term;
use crate::sim::skill::value::{time_usage_known, value_usage_known};
use crate::sim::skill::{ability_known, SkillDetailRef};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    pub skills_checked: usize,
    pub terms_checked: usize,
    /// (skill id, the condition or precondition term) that doesn't parse or has an unknown keyword.
    pub unknown_conditions: Vec<(i32, String)>,
    /// (skill id, ability_type) the sim has never seen.
    pub unknown_abilities: Vec<(i32, i32)>,
    /// (skill id, ability_value_usage) the sim doesn't implement.
    pub unknown_value_usages: Vec<(i32, i32)>,
    /// (skill id, ability_time_usage) the sim doesn't implement.
    pub unknown_time_usages: Vec<(i32, i32)>,
    pub courses_checked: usize,
    /// (race_course_set id, missing file stem) for course sets that races use.
    pub missing_course_files: Vec<(i32, String)>,
}

impl Coverage {
    pub fn is_complete(&self) -> bool {
        self.unknown_conditions.is_empty()
            && self.unknown_abilities.is_empty()
            && self.unknown_value_usages.is_empty()
            && self.unknown_time_usages.is_empty()
            && self.missing_course_files.is_empty()
    }
}

/// Checks every era-effective skill of `data` and every course set a race of `data` uses.
pub fn check(data: &GameData) -> Coverage {
    let mut c = Coverage::default();

    // A two-horse race to build triggers in (some keywords read the field or draw from the RNG while building).
    let input = RaceInput::default();
    let mut race = Race::bare(data, &input, 1, vec![Horse::default(); 2]);
    race.info.num_race_horses = 2;
    race.info.horse_index_by_popularity = vec![0, 1];
    race.horses[0].skill_manager.skills.push(Default::default());
    let r = SkillDetailRef { horse: 0, skill: 0, detail: 0 };

    let mut ids: Vec<i32> = data.skills.keys().copied().collect();
    ids.sort_unstable();
    for id in ids {
        let s = &data.skills[&id];
        c.skills_checked += 1;
        for d in &s.details {
            for text in [&d.precondition, &d.condition] {
                for term in text.split(['@', '&']).filter(|t| !t.is_empty()) {
                    c.terms_checked += 1;
                    let built = parse_term(term).and_then(|(kw, op, rh)| create_trigger(&mut race, r, kw, op, rh));
                    if built.is_none() {
                        c.unknown_conditions.push((id, term.to_string()));
                    }
                }
            }
            if d.abilities.iter().any(|a| a.ability_type != 0) && !time_usage_known(d.ability_time_usage) {
                c.unknown_time_usages.push((id, d.ability_time_usage));
            }
            for a in d.abilities.iter().filter(|a| a.ability_type != 0) {
                if !ability_known(a.ability_type) {
                    c.unknown_abilities.push((id, a.ability_type));
                }
                if !value_usage_known(a.ability_value_usage) {
                    c.unknown_value_usages.push((id, a.ability_value_usage));
                }
            }
        }
        // Triggers pile up in the horse's arena; clear it so the check stays flat in memory.
        race.horses[0].skill_manager.triggers.clear();
    }
    for v in [&mut c.unknown_abilities, &mut c.unknown_value_usages, &mut c.unknown_time_usages] {
        v.dedup();
    }

    let mut sets: Vec<i32> = data.master.race.values().map(|r| r.course_set).collect();
    sets.sort_unstable();
    sets.dedup();
    for id in sets {
        let Some(cs) = data.master.race_course_set.get(&id) else { continue };
        c.courses_checked += 1;
        let stems = [
            id.to_string(),
            lane_path_name(cs.race_track_id, cs.distance, cs.ground, cs.inout, 0),
        ];
        let has = [data.course_event_params.has(&stems[0]), data.course_paths.has(&stems[1])];
        for (stem, ok) in stems.into_iter().zip(has) {
            if !ok {
                c.missing_course_files.push((id, stem));
            }
        }
        // The overrun path is optional (`load_course_lane_anims` takes it as an Option): not checked.
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_data_is_covered_but_for_known_gaps() {
        let dir = std::path::Path::new(crate::data::DATA_DIR);
        if !dir.exists() {
            return;
        }
        let g = GameData::load(dir, crate::data::DataEra::current()).unwrap();
        let c = check(&g);
        assert!(c.terms_checked > 1000, "only {} terms checked", c.terms_checked);
        assert!(c.courses_checked > 50, "only {} courses checked", c.courses_checked);
        let skills_only = Coverage { missing_course_files: vec![], ..c.clone() };
        assert!(skills_only.is_complete(), "{c:#?}");
        // Morioka 2000 m dirt (track 10105; JBC Classic, Mercury Cup: career races only): the data folder has its lane
        // path but no courseeventparam (2026-11-16). A race there panics in `load_course_params`.
        assert_eq!(c.missing_course_files, vec![(11504, "11504".to_string())], "{c:#?}");
    }

    #[test]
    fn unknown_things_are_listed() {
        let dir = std::path::Path::new(crate::data::DATA_DIR);
        if !dir.exists() {
            return;
        }
        let mut g = GameData::load(dir, crate::data::DataEra::current()).unwrap();
        let id = *g.skills.keys().min().unwrap();
        let s = g.skills.get_mut(&id).unwrap();
        s.details[0].condition = "brand_new_keyword==1&distance_rate>=50".into();
        s.details[0].abilities[0].ability_type = 9999;
        s.details[0].abilities[0].ability_value_usage = 99;
        let c = check(&g);
        assert_eq!(c.unknown_conditions, vec![(id, "brand_new_keyword==1".to_string())]);
        assert_eq!(c.unknown_abilities, vec![(id, 9999)]);
        assert_eq!(c.unknown_value_usages, vec![(id, 99)]);
        assert!(!c.is_complete());
    }
}
