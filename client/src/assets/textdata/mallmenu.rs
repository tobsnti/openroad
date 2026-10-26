//! The Item Mall's menu table (`mallitemmenulistdata.txt`).
//!
//! Idea: the mall's left-hand tabs are **not** a layout fact. `ifitemmall*.txt`
//! authors the plates, but which tabs exist, in what order, with which string
//! key and which icon, is this table's two sections:
//!
//! ```text
//! #section  ITEMMALL_MENU_LIST
//! 1  1  MALL_CONSUME  UIIT_STT_SILKMALL_CONSUMPTION  …mall_item_tab.ddj  …mall_consum_icon.ddj  0  1  1  1
//! #section  ITEMMALL_HOTITEM_LIST
//! 1  추천상품  4  8  UILM_STT_DIVISION_RECOMMENDATION
//! ```
//!
//! `ITEMMALL_MENU_LIST` columns: service flag · order · code name · text key ·
//! tab art · icon art · four flags. `ITEMMALL_HOTITEM_LIST` columns: service
//! flag · Korean name · display order · **bit value** · text key — the bits run
//! 1, 2, 4, 8, 16, 32, which is what makes the second section a *division mask*
//! rather than a list.
//!
//! The division half is why this module exists at all beyond the tabs: it is
//! where the article-list captions come from, including `Recommend`
//! (`UILM_STT_DIVISION_RECOMMENDATION`, bit 8) — a key no `SILKMALL` grep finds,
//! because it is the only one of the six that is not spelled `UIIT_STT_SILKMALL_*`.

/// One row of `ITEMMALL_MENU_LIST`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MallTab {
    /// Column 1: the table's own service flag. `MALL_EVENT` ships with `0`.
    pub active: bool,
    /// Column 2, the order the tabs are drawn in.
    pub order: u8,
    /// Column 3. `xxx` for the two rows the table does not name (the HOT shop
    /// and the basket), so it is a hint, never a key.
    pub code: String,
    /// Column 4, the label's textuisystem key.
    pub text_key: String,
    /// Column 6, media-relative with separators normalized.
    pub icon: String,
}

/// One row of `ITEMMALL_HOTITEM_LIST`: a caption bound to a bit of the
/// article's division mask.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MallDivision {
    pub active: bool,
    /// Column 4 — a power of two, not an index.
    pub bit: u32,
    pub text_key: String,
}

#[derive(Debug, Clone, Default)]
pub struct MallMenuTable {
    pub tabs: Vec<MallTab>,
    pub divisions: Vec<MallDivision>,
}

impl MallMenuTable {
    pub fn parse(content: &str) -> Self {
        let mut table = Self::default();
        let mut section = String::new();
        for line in content.lines() {
            let fields: Vec<&str> = line
                .trim_end_matches('\r')
                .split('\t')
                .map(str::trim)
                .collect();
            let first = fields[0].trim_start_matches('\u{feff}');
            if first == "#section" {
                section = fields.get(1).copied().unwrap_or_default().to_string();
                continue;
            }
            match section.as_str() {
                "ITEMMALL_MENU_LIST" if fields.len() >= 6 => {
                    let Ok(order) = fields[1].parse::<u8>() else {
                        continue;
                    };
                    table.tabs.push(MallTab {
                        active: first == "1",
                        order,
                        code: fields[2].to_string(),
                        text_key: fields[3].to_string(),
                        icon: fields[5].replace('\\', "/"),
                    });
                }
                "ITEMMALL_HOTITEM_LIST" if fields.len() >= 5 => {
                    let Ok(bit) = fields[3].parse::<u32>() else {
                        continue;
                    };
                    table.divisions.push(MallDivision {
                        active: first == "1",
                        bit,
                        text_key: fields[4].to_string(),
                    });
                }
                _ => {}
            }
        }
        table.tabs.sort_by_key(|tab| tab.order);
        table
    }

    /// The tabs the table puts in service, in their authored order.
    pub fn active_tabs(&self) -> impl Iterator<Item = &MallTab> {
        self.tabs.iter().filter(|tab| tab.active)
    }

    /// The caption key for a division bit, when the table has one in service.
    pub fn division(&self, bit: u32) -> Option<&str> {
        self.divisions
            .iter()
            .find(|division| division.active && division.bit == bit)
            .map(|division| division.text_key.as_str())
    }
}

#[cfg(test)]
mod test {
    use super::*;

    /// Real rows, byte-for-byte from `Media/server_dep/silkroad/textdata/
    /// mallitemmenulistdata.txt` (decoded from UTF-16), including the
    /// out-of-service `MALL_EVENT` and both section headers.
    const FIXTURE: &str = "\u{feff}#section\tITEMMALL_MENU_LIST\r\n\
        1\t1\tMALL_CONSUME\tUIIT_STT_SILKMALL_CONSUMPTION\tinterface\\mall\\mall_item_tab.ddj\tinterface\\mall\\mall_consum_icon.ddj\t0\t1\t1\t1\r\n\
        1\t7\tMALL_PACKAGE\tUIIT_STT_SILKMALL_PACKAGE\tinterface\\mall\\mall_item_tab.ddj\tinterface\\mall\\mall_bundle_icon.ddj\t0\t1\t1\t1\r\n\
        1\t8\txxx\tUIIT_STT_SILKMALL_SHOPPING_BASKET_LIST\tinterface\\mall\\mall_item_tab.ddj\tinterface\\mall\\mall_choicelist_icon.ddj\t0\t1\t1\t0\r\n\
        0\t9\tMALL_EVENT\tUIIT_STT_SILKMALL_SPECIAL_COAST\tinterface\\mall\\mall_item_tab.ddj\tinterface\\mall\\mall_event_icon.ddj\t0\t1\t1\t1\r\n\
        #section\tITEMMALL_HOTITEM_LIST\r\n\
        1\t최신상품\t2\t1\tUIIT_STT_SILKMALL_UPDATE_ITEM\r\n\
        0\t할인상품\t5\t4\tUIIT_STT_SILKMALL_SALE_ITEM\r\n\
        1\t추천상품\t4\t8\tUILM_STT_DIVISION_RECOMMENDATION\r\n";

    #[test]
    fn the_sections_are_read_apart_and_the_service_flag_is_kept() {
        let table = MallMenuTable::parse(FIXTURE);
        assert_eq!(table.tabs.len(), 4);
        assert_eq!(table.active_tabs().count(), 3, "MALL_EVENT is out of service");

        let first = &table.tabs[0];
        assert_eq!(first.text_key, "UIIT_STT_SILKMALL_CONSUMPTION");
        // separators normalized, so the path can be joined to media://
        assert_eq!(first.icon, "interface/mall/mall_consum_icon.ddj");
        // the basket is one of the two rows the table does not name
        assert_eq!(table.tabs[2].code, "xxx");
    }

    /// The division column is a bit, not an index — the whole point of the
    /// second section, and what binds `Recommend` to an article.
    #[test]
    fn divisions_are_bits_and_recommend_is_one_of_them() {
        let table = MallMenuTable::parse(FIXTURE);
        assert_eq!(table.division(8), Some("UILM_STT_DIVISION_RECOMMENDATION"));
        assert_eq!(table.division(1), Some("UIIT_STT_SILKMALL_UPDATE_ITEM"));
        // out of service, so it is not offered as a caption
        assert_eq!(table.division(4), None);
        assert_eq!(table.division(3), None, "3 is not a bit this table carries");
    }
}
