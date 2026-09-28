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
//! Two members, each kept to what this tree actually draws:
//!
//! * [`combo_box`] — `CIFComboBox`, the drop-down field as the autopotion
//!   panel spends it (`autopotion/ui.rs`).
//! * [`page_manager`] — `CIFPageManager`, the Prev/Next strip under a paged
//!   list. It landed with its first consumer, exactly as this note asked:
//!   `stall_network.rs` pages its result list and was drawing guessed text
//!   labels in the widget's band.
//!
//! **This is a display, not a controller.** The widget neither fetches nor owns
//! its content: it renders the caption it is handed. The option list belongs to
//! the window, which is the only place that knows where the rows come from.

pub mod combo_box;
pub mod page_manager;
