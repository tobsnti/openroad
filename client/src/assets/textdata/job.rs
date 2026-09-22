//! Job types and job rank names — the pure, data-only half of the job system.
//!
//! **The idea:** the client stores no rank table of its own. A job rank is a
//! `(job, rank 1..=7)` pair that the UI turns into a *string key*
//! (`UIIT_STT_CLASS_{EU_}<JOB>_<n>`) and looks up in `textuisystem.txt` like
//! any other label. So the whole feature is a key builder plus the existing UI
//! string path — no new file format, no new loader.
//!
//! Sources (the original client, read as data):
//! - the client formats `UIIT_STT_CLASS_%s_%d` / `UIIT_STT_CLASS_EU_%s_%d`
//!   and picks the variant from a byte at world-object `+0x9c` (0 = Chinese
//!   names, 1 = European names). Job tokens inside the client itself are
//!   `1 = "MERCHANT"`, `2 = "THIEF"`, `3 = "HUNTER"` `[V]`.
//! - the 43 `UIIT_STT_CLASS_*` rows in the shipped
//!   `textuisystem.txt` are exactly (CH + EU) × 3 jobs × 7 ranks + the extra
//!   `MERCHANT_6_NEW`. Seven ranks per job, cross-checked against
//!   `leveldata.txt` columns 6/7/8 (JL1…JL7, `-1` from row 8 on).
//!
//! Deliberately not here: the job-suit predicate. It needs the spawn packet,
//! and `assets/textdata/**` is a closed surface whose `use crate::` may point
//! at `assets` only, never at `net`/`plugins` (see `client/src/lib.rs:39`). It
//! belongs on the `net` side and is not written yet.
//!
//! Deliberately **not** here: anything that needs a server (the player's own
//! job, rank, points, prices, transport). This module is what can be decided
//! from data alone.

use crate::assets::textdata::uisystem::UiSystemText;

/// The three jobs, numbered as the wire numbers them.
///
/// `[V]` `0x30E6`/`JobInfo.job_type` and the client's own token table
/// 1 merchant/trader, 2 thief, 3 hunter. The same order is what the `0xB0E6`
/// reader implies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum JobType {
    Trader = 1,
    Thief = 2,
    Hunter = 3,
}

/// Highest job rank. `[V]` two data sources agree on 7: the 7
/// `UIIT_STT_CLASS_*_1..7` keys per job and `leveldata.txt` columns 6/7/8
/// (7 values, then `-1`).
pub const MAX_JOB_RANK: u8 = 7;

impl JobType {
    /// From the wire's `job_type` byte (`JobInfo.job_type`, `0x30E6`).
    /// `0` = no job, and anything else is unknown → `None`.
    pub fn from_wire(job_type: u8) -> Option<Self> {
        match job_type {
            1 => Some(Self::Trader),
            2 => Some(Self::Thief),
            3 => Some(Self::Hunter),
            _ => None,
        }
    }

    /// The token the client itself splices into `UIIT_STT_CLASS_%s_%d`.
    pub fn ui_token(self) -> &'static str {
        match self {
            Self::Trader => "MERCHANT",
            Self::Thief => "THIEF",
            Self::Hunter => "HUNTER",
        }
    }
}

/// Which of the two rank-name variants to use. The original picks it per world
/// object (0 → the plain keys, 1 → the `_EU_` keys). Which state feeds that
/// choice — character race or build region — is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RankNameSet {
    Chinese,
    European,
}

/// The `textuisystem.txt` key of a job rank's name, or `None` outside
/// `1..=`[`MAX_JOB_RANK`] — rank 0 (no job) has no key: the table starts at 1.
pub fn rank_name_key(job: JobType, rank: u8, set: RankNameSet) -> Option<String> {
    if rank == 0 || rank > MAX_JOB_RANK {
        return None;
    }
    let prefix = match set {
        RankNameSet::Chinese => "UIIT_STT_CLASS_",
        RankNameSet::European => "UIIT_STT_CLASS_EU_",
    };
    Some(format!("{prefix}{}_{rank}", job.ui_token()))
}

/// The display name of a job rank, resolved through the loaded UI strings.
/// `None` when the rank is out of range or the table lacks the key.
pub fn rank_name(strings: &UiSystemText, job: JobType, rank: u8, set: RankNameSet) -> Option<&str> {
    strings.get(&rank_name_key(job, rank, set)?)
}

#[cfg(test)]
mod test {
    use super::*;

    /// All 43 `UIIT_STT_CLASS_*` rows of
    /// `Media/server_dep/silkroad/textdata/textuisystem.txt` (English = last
    /// non-empty column). No PK2 is available in a test run, so the rows live
    /// constants instead of whatever archive a machine happens to have.
    const CLASS_ROWS: &[(&str, &str)] = &[
        ("UIIT_STT_CLASS_EU_HUNTER_1", "Hunter Beginner"),
        ("UIIT_STT_CLASS_EU_HUNTER_2", "Trader"),
        ("UIIT_STT_CLASS_EU_HUNTER_3", "Bodyguard"),
        ("UIIT_STT_CLASS_EU_HUNTER_4", "Hunter Leader"),
        ("UIIT_STT_CLASS_EU_HUNTER_5", "Guardian"),
        ("UIIT_STT_CLASS_EU_HUNTER_6", "Expert Hunter"),
        ("UIIT_STT_CLASS_EU_HUNTER_7", "Great Guardian"),
        ("UIIT_STT_CLASS_EU_MERCHANT_1", "Trade Beginner"),
        ("UIIT_STT_CLASS_EU_MERCHANT_2", "Merchant"),
        ("UIIT_STT_CLASS_EU_MERCHANT_3", "Dealer"),
        ("UIIT_STT_CLASS_EU_MERCHANT_4", "Trader"),
        ("UIIT_STT_CLASS_EU_MERCHANT_5", "Rich Trader"),
        ("UIIT_STT_CLASS_EU_MERCHANT_6", "Silk Caravan"),
        ("UIIT_STT_CLASS_EU_MERCHANT_7", "Great Merchant"),
        ("UIIT_STT_CLASS_EU_THIEF_1", "Thief Beginner"),
        ("UIIT_STT_CLASS_EU_THIEF_2", "Bandit"),
        ("UIIT_STT_CLASS_EU_THIEF_3", "Outlaw"),
        ("UIIT_STT_CLASS_EU_THIEF_4", "Blood Brigand"),
        ("UIIT_STT_CLASS_EU_THIEF_5", "Sharp Bandit"),
        ("UIIT_STT_CLASS_EU_THIEF_6", "Expert Bandit"),
        ("UIIT_STT_CLASS_EU_THIEF_7", "Great Thief"),
        ("UIIT_STT_CLASS_HUNTER_1", "Amateur Hunter"),
        ("UIIT_STT_CLASS_HUNTER_2", "Novice Hunter"),
        ("UIIT_STT_CLASS_HUNTER_3", "Hunter"),
        ("UIIT_STT_CLASS_HUNTER_4", "Silkroad Hunter"),
        ("UIIT_STT_CLASS_HUNTER_5", "Expert Hunter"),
        ("UIIT_STT_CLASS_HUNTER_6", "Elite Hunter"),
        ("UIIT_STT_CLASS_HUNTER_7", "Master Hunter"),
        ("UIIT_STT_CLASS_MERCHANT_1", "Amateur Trader"),
        ("UIIT_STT_CLASS_MERCHANT_2", "Novice Trader"),
        ("UIIT_STT_CLASS_MERCHANT_3", "Exchange Merchant"),
        ("UIIT_STT_CLASS_MERCHANT_4", "Trade Merchant"),
        ("UIIT_STT_CLASS_MERCHANT_5", "Expert Merchant"),
        ("UIIT_STT_CLASS_MERCHANT_6", "Silkroad Merchant"),
        ("UIIT_STT_CLASS_MERCHANT_6_NEW", "Elite Merchant"),
        ("UIIT_STT_CLASS_MERCHANT_7", "Master Merchant"),
        ("UIIT_STT_CLASS_THIEF_1", "Amateur Thief"),
        ("UIIT_STT_CLASS_THIEF_2", "Novice Thief"),
        ("UIIT_STT_CLASS_THIEF_3", "Thief"),
        ("UIIT_STT_CLASS_THIEF_4", "Silkroad Thief"),
        ("UIIT_STT_CLASS_THIEF_5", "Expert Thief"),
        ("UIIT_STT_CLASS_THIEF_6", "Elite Thief"),
        ("UIIT_STT_CLASS_THIEF_7", "Master Thief"),
    ];

    fn strings() -> UiSystemText {
        let content: String = CLASS_ROWS
            .iter()
            .map(|(key, value)| format!("1\t{key}\t\t\t\t\t\t\t{value}\r\n"))
            .collect();
        UiSystemText::parse(&content)
    }

    #[test]
    fn all_43_class_rows_are_reachable_through_the_key_builder() {
        let strings = strings();
        assert_eq!(strings.0.len(), 43);
        let mut hit = 0;
        for job in [JobType::Trader, JobType::Thief, JobType::Hunter] {
            for set in [RankNameSet::Chinese, RankNameSet::European] {
                for rank in 1..=MAX_JOB_RANK {
                    assert!(
                        rank_name(&strings, job, rank, set).is_some(),
                        "{job:?} rank {rank} {set:?} has no string"
                    );
                    hit += 1;
                }
            }
        }
        // 42 of the 43 rows; the 43rd is the MERCHANT_6_NEW variant below.
        assert_eq!(hit, 42);
    }

    #[test]
    fn rank_names_are_the_shipped_strings() {
        let strings = strings();
        let ch = |job, rank| rank_name(&strings, job, rank, RankNameSet::Chinese);
        let eu = |job, rank| rank_name(&strings, job, rank, RankNameSet::European);
        assert_eq!(ch(JobType::Trader, 1), Some("Amateur Trader"));
        assert_eq!(ch(JobType::Trader, 6), Some("Silkroad Merchant"));
        assert_eq!(ch(JobType::Trader, 7), Some("Master Merchant"));
        assert_eq!(ch(JobType::Hunter, 4), Some("Silkroad Hunter"));
        assert_eq!(ch(JobType::Thief, 3), Some("Thief"));
        assert_eq!(eu(JobType::Trader, 2), Some("Merchant"));
        assert_eq!(eu(JobType::Hunter, 5), Some("Guardian"));
        assert_eq!(eu(JobType::Thief, 7), Some("Great Thief"));
        // The EU set is genuinely a different table, not a fallback: the
        // hunter's second rank is called "Trader" there.
        assert_eq!(eu(JobType::Hunter, 2), Some("Trader"));
        assert_ne!(ch(JobType::Hunter, 2), eu(JobType::Hunter, 2));
    }

    /// `UIIT_STT_CLASS_MERCHANT_6_NEW` = "Elite Merchant" exists next to
    /// `_MERCHANT_6` = "Silkroad Merchant". Which one the client actually
    /// shows is unresolved (order 063 / 061), so the key builder deliberately
    /// produces only the numbered form — no guess baked in.
    #[test]
    fn merchant_6_new_is_not_synthesized() {
        assert_eq!(
            rank_name_key(JobType::Trader, 6, RankNameSet::Chinese).as_deref(),
            Some("UIIT_STT_CLASS_MERCHANT_6")
        );
        assert_eq!(
            CLASS_ROWS
                .iter()
                .find(|(k, _)| *k == "UIIT_STT_CLASS_MERCHANT_6_NEW")
                .map(|(_, v)| *v),
            Some("Elite Merchant"),
            "the variant is in the data, it is just not what we key on"
        );
    }

    #[test]
    fn rank_zero_and_overflow_have_no_key() {
        for set in [RankNameSet::Chinese, RankNameSet::European] {
            assert_eq!(rank_name_key(JobType::Trader, 0, set), None);
            assert_eq!(rank_name_key(JobType::Trader, 8, set), None);
            assert_eq!(rank_name_key(JobType::Trader, 255, set), None);
        }
        assert_eq!(
            rank_name(&strings(), JobType::Hunter, 0, RankNameSet::Chinese),
            None
        );
    }

    #[test]
    fn wire_job_type_numbers() {
        assert_eq!(JobType::from_wire(0), None, "0 = no job");
        assert_eq!(JobType::from_wire(1), Some(JobType::Trader));
        assert_eq!(JobType::from_wire(2), Some(JobType::Thief));
        assert_eq!(JobType::from_wire(3), Some(JobType::Hunter));
        assert_eq!(JobType::from_wire(4), None);
        assert_eq!(JobType::Hunter as u8, 3);
    }
}
