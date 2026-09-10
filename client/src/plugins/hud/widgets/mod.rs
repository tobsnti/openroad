//! Shared HUD widgets — the composite controls the original factors out of the
//! individual windows and several of ours re-invent per site.
//!
//! # The idea
//!
//! The vanilla UI has a small vocabulary of composite widgets whose parts come
//! from one global prototype file rather than from the window that uses them
//! (`resinfo` prototype files). A window's own tree then only
//! carries the *instance* — a name, an id and a rect — and is silent about the
//! art, which is why a per-window transcription of such a control always ends
//! up either empty or invented. This module is where that vocabulary lives once
//! so a window can spend a rect and get the original's widget back.
//!
//! Two members so far, both chosen because more than one window needs them
//! (`shared-input-widgets.md` §9 "consumer census"):
//!
//! * [`page_manager`] — `CIFPageManager`, the Prev/Next strip under a paged
//!   list (item mall, stall network).
//! * [`combo_box`] — `CIFComboBox`, the drop-down field, closed and open
//!   (options/video, party matching, stall network, item mall, guild-war
//!   request, …).
//!
//! **These are displays, not controllers.** Neither widget fetches, owns or
//! derives its content: the page strip renders the page numbers it is handed
//! and the combo field renders the caption and the rows it is handed. Paging
//! policy and option lists belong to the window, which is the only place that
//! knows where the rows come from — and for the combo that is a **rule**, not a
//! convenience: the original's open list shows four of the six period lines the
//! text file ships in one block (`combo_box`, §31).

pub mod combo_box;
pub mod page_manager;
