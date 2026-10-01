//! The rects the UI actually occupies on screen, served over BRP.
//!
//! A window's authored layout is in its `.2dt` descriptor, and `twodt_dump`
//! prints it. Checking the drawn window against it has meant taking a
//! screenshot, measuring pixels by hand and doing the arithmetic in a comment —
//! which found a real error every time, and also carried one (absolute numbers
//! written where local ones belong). This hands back the measured side of that
//! comparison instead: name, position and size of each UI node, in the same
//! logical pixels the layout is authored in.
//!
//! Positions are reported as the node's **top-left corner**, not its centre.
//! Bevy stores the centre (`UiGlobalTransform::translation`), while a
//! descriptor's rect is a corner plus a size, so a raw centre would compare a
//! point against a corner and be wrong by half the node on each axis.

use bevy::prelude::{In, InheritedVisibility, Name, Query, Res};
use bevy::remote::error_codes::INVALID_PARAMS;
use bevy::remote::{BrpError, BrpResult};
use bevy::ui::{ComputedNode, UiGlobalTransform, UiStack};
use serde_json::{json, Value};

/// What a caller wants out of the node list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UiRectFilter {
    /// Case-insensitive substring of the node's `Name`; unnamed nodes are kept
    /// out when this is set, because an unnamed node cannot match a name.
    pub name: Option<String>,
    /// Keep only visible nodes. Off by default: "the window is missing" is
    /// usually answered by a node that exists and is invisible.
    pub visible_only: bool,
    /// Newest-first cut-off, 0 for everything.
    pub limit: usize,
}

/// Reads `{name, visible_only, limit}`.
pub fn ui_rect_filter_from_params(params: Option<&Value>) -> Result<UiRectFilter, String> {
    let mut filter = UiRectFilter::default();
    let Some(params) = params else {
        return Ok(filter);
    };
    if let Some(raw) = params.get("name") {
        let text = raw
            .as_str()
            .ok_or_else(|| format!("name must be a string, got {raw}"))?;
        let trimmed = text.trim();
        if trimmed.is_empty() {
            // An empty filter would match everything while reading like a
            // filter, so it is refused rather than ignored.
            return Err("name is empty: leave it out to list every node".to_string());
        }
        filter.name = Some(trimmed.to_ascii_lowercase());
    }
    if let Some(raw) = params.get("visible_only") {
        filter.visible_only = raw
            .as_bool()
            .ok_or_else(|| format!("visible_only must be true or false, got {raw}"))?;
    }
    if let Some(raw) = params.get("limit") {
        let limit = raw
            .as_u64()
            .ok_or_else(|| format!("limit must be a non-negative number, got {raw}"))?;
        filter.limit = usize::try_from(limit).unwrap_or(usize::MAX);
    }
    Ok(filter)
}

/// The top-left corner of a node whose centre and size are known.
pub fn top_left(center: (f32, f32), size: (f32, f32)) -> (f32, f32) {
    (center.0 - size.0 / 2.0, center.1 - size.1 / 2.0)
}

/// Whether one node passes the filter. Split out so the name rule is testable
/// without a world: an unnamed node cannot match a requested name.
pub fn matches(filter: &UiRectFilter, name: Option<&str>, visible: bool) -> bool {
    if filter.visible_only && !visible {
        return false;
    }
    match &filter.name {
        None => true,
        Some(wanted) => name
            .map(|name| name.to_ascii_lowercase().contains(wanted))
            .unwrap_or(false),
    }
}

/// `openroad/ui_rects`: every UI node's name, rect and visibility, in draw
/// order (back to front, the order the renderer uses).
pub fn brp_ui_rects(
    In(params): In<Option<Value>>,
    stack: Res<UiStack>,
    nodes: Query<(
        &ComputedNode,
        &UiGlobalTransform,
        Option<&Name>,
        &InheritedVisibility,
    )>,
) -> BrpResult {
    let filter = ui_rect_filter_from_params(params.as_ref()).map_err(|message| BrpError {
        code: INVALID_PARAMS,
        message,
        data: None,
    })?;
    let mut rects: Vec<Value> = Vec::new();
    for (order, entity) in stack.uinodes.iter().enumerate() {
        let Ok((node, transform, name, visible)) = nodes.get(*entity) else {
            continue;
        };
        let name = name.map(Name::as_str);
        if !matches(&filter, name, visible.get()) {
            continue;
        }
        let size = node.size();
        let center = transform.translation;
        let (x, y) = top_left((center.x, center.y), (size.x, size.y));
        rects.push(json!({
            "order": order,
            "name": name,
            "x": x,
            "y": y,
            "w": size.x,
            "h": size.y,
            "visible": visible.get(),
        }));
    }
    let total = rects.len();
    if filter.limit > 0 && rects.len() > filter.limit {
        rects.drain(..rects.len() - filter.limit);
    }
    Ok(json!({
        "nodes": rects,
        "matched": total,
        "stack_len": stack.uinodes.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Half the node on each axis is exactly the error a raw centre would
    /// make against a descriptor's corner.
    #[test]
    fn a_rect_is_reported_from_its_top_left_corner() {
        assert_eq!(top_left((100.0, 200.0), (40.0, 20.0)), (80.0, 190.0));
        // A zero-sized node is its own corner.
        assert_eq!(top_left((7.0, 9.0), (0.0, 0.0)), (7.0, 9.0));
    }

    #[test]
    fn a_name_filter_is_a_case_insensitive_substring() {
        let filter = ui_rect_filter_from_params(Some(&json!({ "name": "Inventory" })))
            .expect("a name is valid");
        assert!(matches(&filter, Some("inventory_window"), true));
        assert!(matches(&filter, Some("HUD/InventoryGrid"), true));
        assert!(!matches(&filter, Some("storage_window"), true));
    }

    /// An unnamed node cannot match a requested name — reporting it would
    /// answer a question about one window with a different node.
    #[test]
    fn an_unnamed_node_never_matches_a_name_filter() {
        let named = ui_rect_filter_from_params(Some(&json!({ "name": "x" }))).expect("valid");
        assert!(!matches(&named, None, true));
        let all = UiRectFilter::default();
        assert!(matches(&all, None, true), "without a filter it is listed");
    }

    /// Invisible nodes are listed by default: "the window is missing" is
    /// usually a node that exists and does not draw.
    #[test]
    fn invisible_nodes_are_listed_unless_asked_otherwise() {
        let all = UiRectFilter::default();
        assert!(matches(&all, Some("hidden"), false));
        let visible = ui_rect_filter_from_params(Some(&json!({ "visible_only": true })))
            .expect("a flag is valid");
        assert!(!matches(&visible, Some("hidden"), false));
        assert!(matches(&visible, Some("shown"), true));
    }

    /// Every malformed field is refused instead of quietly widening the list.
    #[test]
    fn a_malformed_filter_is_refused() {
        assert!(ui_rect_filter_from_params(Some(&json!({ "name": 3 }))).is_err());
        assert!(ui_rect_filter_from_params(Some(&json!({ "name": "  " }))).is_err());
        assert!(ui_rect_filter_from_params(Some(&json!({ "visible_only": "yes" }))).is_err());
        assert!(ui_rect_filter_from_params(Some(&json!({ "limit": -1 }))).is_err());
        let filter = ui_rect_filter_from_params(None).expect("no params lists everything");
        assert_eq!(filter, UiRectFilter::default());
        assert_eq!(filter.limit, 0);
    }
}
