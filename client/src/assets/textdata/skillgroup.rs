//! Parser for `server_dep/silkroad/textdata/skillgroup.txt` (Media.pk2).
//!
//! Idea: this table is the authoritative display order of a mastery's skill
//! branches (the "series" rows in the skill window). Each row is one branch of
//! one mastery, with a **Row** column (col 4) giving its top-to-bottom order
//! and an icon (col 6, `skillgroup\<race>\pack_<concept>.ddj`) whose `concept`
//! stem is the only link back to the skills — skilldata carries no branch id.
//!
//! Skills join a branch by that concept: the branch concept (e.g. `sword_smash`,
//! `earth`) appears as a substring of the skill's `basic_group` codename
//! (`SKILL_CH_SWORD_SMASH_A`, `SKILL_EU_WIZARD_EARTHA_AREA_A`). Matching the
//! **longest** such concept within the mastery resolves the overlaps
//! (`sword_shield` vs `sword_shieldpd`, `stealth` vs `stealth_expert`).
//!
//! Tab-separated, UTF-16LE with BOM like the other row tables; `//` comments.

use bevy::prelude::Resource;
use std::collections::HashMap;

/// One branch of one mastery from skillgroup.txt.
#[derive(Debug, Clone)]
pub struct SkillGroupBranch {
    /// Lowercased icon stem (`pack_` and `.ddj` stripped) — the join key.
    pub concept: String,
    /// Display order within the mastery (col 4).
    pub row: u32,
    /// Branch icon asset path, `icon/skillgroup/<race>/pack_<concept>.ddj`
    /// (col 6, backslashes normalized).
    pub icon: String,
}

/// Parsed skillgroup.txt: per mastery, its branches in file order.
#[derive(Resource, Debug, Default, Clone)]
pub struct SkillGroupTable {
    by_mastery: HashMap<u32, Vec<SkillGroupBranch>>,
}

/// The icon-stem concept of an icon path, e.g.
/// `skillgroup\china\pack_sword_smash.ddj` → `sword_smash`. `None` for the
/// empty/`xxx` placeholders.
fn icon_concept(icon: &str) -> Option<String> {
    let base = icon.rsplit(['\\', '/']).next().unwrap_or(icon);
    let stem = base.strip_suffix(".ddj").unwrap_or(base);
    let concept = stem.strip_prefix("pack_").unwrap_or(stem).to_lowercase();
    (!concept.is_empty() && concept != "xxx").then_some(concept)
}

/// The series token of a skill's `basic_group`: the part that names the
/// branch, lowercased. `SKILL_EU_BARD_BATTLAA_ROOT_A` → `battlaa`,
/// `SKILL_CH_SWORD_SMASH_A` → `sword`. `None` for a codename that is not
/// `SKILL_<race>_<class>_<series>_…`.
///
/// The CH form puts the weapon/element first and the concept second
/// (`sword_smash`), so its token is only the first part — which is why the CH
/// rows match through the substring pass and never need this one.
fn series_token(basic_group: &str) -> Option<String> {
    let body = basic_group
        .strip_prefix("SKILL_CH_")
        .or_else(|| basic_group.strip_prefix("SKILL_EU_"))?;
    let mut parts = body.split('_');
    let first = parts.next()?;
    let token = if basic_group.starts_with("SKILL_EU_") {
        parts.next()?
    } else {
        first
    };
    (!token.is_empty()).then(|| token.to_lowercase())
}

impl SkillGroupTable {
    pub fn parse(content: &str) -> Self {
        let mut by_mastery: HashMap<u32, Vec<SkillGroupBranch>> = HashMap::new();
        for line in content.lines() {
            let line = line
                .trim_end_matches(['\r', '\t'])
                .trim_start_matches('\u{feff}');
            if line.is_empty() || line.starts_with("//") {
                continue;
            }
            let row: Vec<&str> = line.split('\t').collect();
            let field = |i: usize| row.get(i).map(|s| s.trim()).unwrap_or("");
            if field(0) != "1" {
                continue;
            }
            let (Ok(mastery), Ok(order)) = (field(2).parse::<u32>(), field(4).parse::<u32>())
            else {
                continue;
            };
            let Some(concept) = icon_concept(field(6)) else {
                continue;
            };
            let icon = format!("icon/{}", field(6).replace('\\', "/"));
            by_mastery
                .entry(mastery)
                .or_default()
                .push(SkillGroupBranch {
                    concept,
                    row: order,
                    icon,
                });
        }
        Self { by_mastery }
    }

    /// The branch a skill series belongs to, matched by the longest branch
    /// concept that occurs in the series' `basic_group` (case-insensitive).
    /// `None` when no branch matches (rare oddball series).
    ///
    /// A substring test alone leaves branches the data does spell out on the
    /// table, because two codenames state the concept *differently* rather
    /// than not at all. Measured over the player's own
    /// `skilldata_*.txt`/`skillgroup.txt` (480 branch roots the window shows):
    /// 100 found no branch, and **13** of those are these two shapes —
    ///
    /// * `SKILL_EU_ROG_POIS_DAGP_*` against concept `poison`: the codename
    ///   **abbreviates** it, so the concept is longer than what the codename
    ///   holds (4 skills).
    /// * `SKILL_EU_BARD_BATTLAA_*` against concept `battle`: the codename
    ///   **misspells** it (`battla` + the active marker), so neither string
    ///   contains the other (9 skills).
    ///
    /// Both are a common prefix of the series token, which is why
    /// [`Self::prefix_branch`] is the second pass. The other 87 have no group
    /// icon in the archive at all (`heal`, `psychic`, `rebirth`, `confusion`,
    /// …) — for those the caller's dimmed skill icon is the honest answer and
    /// stays.
    pub fn branch_for(&self, mastery: u32, basic_group: &str) -> Option<&SkillGroupBranch> {
        let key = basic_group.to_lowercase();
        let branches = self.by_mastery.get(&mastery)?;
        branches
            .iter()
            .filter(|branch| key.contains(branch.concept.as_str()))
            .max_by_key(|branch| branch.concept.len())
            .or_else(|| Self::prefix_branch(branches, basic_group))
    }

    /// How many leading characters a concept and a series token must share.
    ///
    /// Four, and both bounds are measured rather than picked. Below it the
    /// test starts guessing: `bow`/`bowa` would reach `blood` at three.
    /// Above it the two real cases are lost — `pois` is four characters long.
    /// At four, **none** of the 87 series without a group icon finds a
    /// candidate, so the pass cannot invent a branch where the data has none.
    const PREFIX_MATCH: usize = 4;

    /// Second pass for [`Self::branch_for`]: the series token against the
    /// mastery's concepts by common prefix, and only when exactly one concept
    /// is in reach — two candidates cannot be told apart this way, and
    /// picking the first would be a guess dressed as a lookup.
    fn prefix_branch<'a>(
        branches: &'a [SkillGroupBranch],
        basic_group: &str,
    ) -> Option<&'a SkillGroupBranch> {
        let token = series_token(basic_group)?;
        let mut hit = None;
        for branch in branches {
            let shared = branch
                .concept
                .chars()
                .zip(token.chars())
                .take_while(|(a, b)| a == b)
                .count();
            if shared < Self::PREFIX_MATCH {
                continue;
            }
            if hit.is_some() {
                return None;
            }
            hit = Some(branch);
        }
        hit
    }

    /// Display row (col 4) of a skill series; `None` sorts it last.
    pub fn row_for(&self, mastery: u32, basic_group: &str) -> Option<u32> {
        self.branch_for(mastery, basic_group).map(|b| b.row)
    }

    pub fn len(&self) -> usize {
        self.by_mastery.values().map(Vec::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.by_mastery.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> SkillGroupTable {
        // mastery 257 (sword): the shield/shieldpd overlap; mastery 514
        // (wizard): element-only concepts with a class-prefixed basic_group.
        let content = "\u{feff}//\tSkillName\tSkillID\tPriority\tRefName\tRow\tUIIT\tIcon\n\
            1\t\t257\tref\t0\tU_VI_0\tskillgroup\\china\\pack_sword_smash.ddj\n\
            1\t\t257\tref\t2\tU_VI_2\tskillgroup\\china\\pack_sword_shield.ddj\n\
            1\t\t257\tref\t7\tU_VI_7\tskillgroup\\china\\pack_sword_shieldpd.ddj\n\
            1\t\t514\tref\t2\tU_EA_2\tskillgroup\\europe\\pack_earth.ddj\n\
            0\t\t257\tref\t9\tU_VI_9\tskillgroup\\china\\pack_sword_disabled.ddj\n";
        SkillGroupTable::parse(content)
    }

    #[test]
    fn maps_series_to_row() {
        let t = sample();
        assert_eq!(t.row_for(257, "SKILL_CH_SWORD_SMASH_A"), Some(0));
        // longest match wins: shieldpd must not collapse onto shield (row 2)
        assert_eq!(t.row_for(257, "SKILL_CH_SWORD_SHIELD_A"), Some(2));
        assert_eq!(t.row_for(257, "SKILL_CH_SWORD_SHIELDPD_A"), Some(7));
        // EU: element-only concept inside a class-prefixed codename
        assert_eq!(t.row_for(514, "SKILL_EU_WIZARD_EARTHA_AREA_A"), Some(2));
        // disabled row (col 0 != 1) skipped; unmatched series → None
        assert_eq!(t.row_for(257, "SKILL_CH_SWORD_KNOCKDOWN_A"), None);
        // authoritative icon path (col 6, backslashes normalized) of the match
        assert_eq!(
            t.branch_for(257, "SKILL_CH_SWORD_SHIELDPD_A")
                .map(|b| b.icon.as_str()),
            Some("icon/skillgroup/china/pack_sword_shieldpd.ddj")
        );
    }

    /// The two shapes the substring pass leaves on the table, both measured in
    /// the player's own data: an **abbreviated** concept (`pois` for `poison`,
    /// 4 skills) and a **misspelled** one (`battlaa` for `battle`, 9 skills).
    /// Before the prefix pass both drew the skill's own icon in the branch
    /// socket, which is what the bug report called "skill icons instead of
    /// group icon".
    #[test]
    fn an_abbreviated_or_misspelled_series_still_finds_its_branch() {
        let table = SkillGroupTable {
            by_mastery: HashMap::from([
                (
                    515,
                    vec![branch("poison", 2), branch("blood", 3), branch("dagger", 4)],
                ),
                (517, vec![branch("battle", 1), branch("beautiful", 2)]),
            ]),
        };

        assert_eq!(
            table
                .branch_for(515, "SKILL_EU_ROG_POIS_DAGP_DAMAGE_A")
                .map(|b| b.concept.as_str()),
            Some("poison"),
            "the codename abbreviates the concept"
        );
        assert_eq!(
            table
                .branch_for(517, "SKILL_EU_BARD_BATTLAA_ROOT_A")
                .map(|b| b.concept.as_str()),
            Some("battle"),
            "the codename misspells the concept"
        );
    }

    /// The bound that keeps the second pass from inventing a branch: a series
    /// with no concept in reach stays unmatched, so the caller keeps drawing
    /// the dimmed skill icon. `heala` against mastery 518's real concept list
    /// is one of the 87 measured cases — there is no `heal` icon in the
    /// archive at all.
    #[test]
    fn a_series_without_a_concept_in_reach_stays_unmatched() {
        let table = SkillGroupTable {
            by_mastery: HashMap::from([(
                518,
                vec![branch("holy", 1), branch("recover", 2), branch("bless", 3)],
            )]),
        };
        assert!(table
            .branch_for(518, "SKILL_EU_CLERIC_HEALA_CYCLE_A")
            .is_none());
        // three shared characters is not enough: `bow`/`blood` must not meet
        assert!(table
            .branch_for(518, "SKILL_EU_CLERIC_BOWA_SHOT_A")
            .is_none());
    }

    /// Two concepts in reach of the same token cannot be told apart by prefix,
    /// so the pass refuses rather than taking the first row.
    #[test]
    fn two_concepts_in_reach_resolve_to_nothing() {
        let table = SkillGroupTable {
            by_mastery: HashMap::from([(515, vec![branch("poison", 1), branch("poisgas", 2)])]),
        };
        assert!(table
            .branch_for(515, "SKILL_EU_ROG_POIS_DAGP_DAMAGE_A")
            .is_none());
    }

    /// The token itself, both races. CH names the weapon first and the concept
    /// second, which is why its rows resolve through the substring pass.
    #[test]
    fn the_series_token_is_the_branch_naming_part() {
        assert_eq!(
            series_token("SKILL_EU_BARD_BATTLAA_ROOT_A").as_deref(),
            Some("battlaa")
        );
        assert_eq!(
            series_token("SKILL_CH_SWORD_SMASH_A").as_deref(),
            Some("sword")
        );
        assert_eq!(series_token("SKILL_GM_WHATEVER"), None);
    }

    fn branch(concept: &str, row: u32) -> SkillGroupBranch {
        SkillGroupBranch {
            concept: concept.to_string(),
            row,
            icon: format!("icon/skillgroup/europe/pack_{concept}.ddj"),
        }
    }
}
