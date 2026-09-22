//! The KeyMap side of [`GameOptions`]: which action each key triggers.
//!
//! Idea: `OptionSet.csv` names 32 rebindable shortcut actions (ids 3001-3035
//! with three gaps) and the option stream stores each as a raw **Win32 VK code**
//! (`docs/formats/sroptionset.md`). Bevy speaks [`KeyCode`], so this module owns
//! the one translation table between the two and resolves "which key is action
//! N bound to" for the gameplay systems. The stored value always wins; a
//! [`KeyAction::default_key`] is only the fallback when nothing is bound.
//!
//! **On defaults.** A stored binding always wins — a real `SROptionSet.dat`
//! overrides everything below, and importing a `.dat` is the only way to learn
//! what *that* player uses.
//!
//! The defaults below are the shipped ones: every binding carries its
//! `SROptionSet.dat` id and VK code, and the six actions that stay `None` are
//! stored there as VK `0x00`, i.e. they ship unbound.
//!
//! A second source agrees with the option stream:
//! `Media/server_dep/silkroad/textdata/textuisystem.txt` prints fourteen of the
//! bindings in the label text itself, at L2250-2274 (the file is **UTF-16LE**,
//! which is why an ASCII grep finds nothing there). `Character ( C )`,
//! `Inventory ( I )` and `Skill ( S )` agree with what openroad binds, and the
//! neighbouring `UIIT_STT_TOGGLE_CONTENTS_MENU_*` family (L2275+) writes the same
//! labels with `%s` where the letter goes — the same UI showing a literal in one
//! place and a live keymap lookup in the other, which is what makes the
//! bracketed letters read as the shipped bindings rather than decoration.
//!
//! Eight further actions take their default from that text, each carrying its
//! textuisystem line in a comment; the `.dat` confirms every one of them.
//!
//! * **`Guild ( U )` (L2252) and `Community ( U )` (L2263) claim the same
//!   letter.** Nothing in the data decides it — plausibly the original really
//!   ships both on `U` (the guild page lives inside the community window), or one
//!   label is stale. `OptionSet.csv` has no `KeyGuild` action at all, so the only
//!   one of the pair that is bindable here is `KeyCommunity`, and it is the only
//!   one bound (`.dat` id 3007 = `0x55`). The collision is recorded, not resolved.
//! * **`UIIT_STT_TOGGLE_WORLD_MAP` (L2258) carries no letter** — its text is
//!   "Whole area map". `KeyWorldMap`'s `M` comes from the option stream instead:
//!   `.dat` id 3008 stores `0x4D` = `M`.
//! * `Option ( ESC )` (L2265), `System ( Esc )` (L2274) and `Stall network ( F )`
//!   (L2272) name no `OptionSet.csv` action, so there is nothing to bind them to.
//! * Six actions (3029-3032 `KeyTarget*`, 3034/3035 `KeyHide*`) ship unbound: the
//!   `.dat` stores VK `0x00` for them.
//!
//! **Three COS defaults come from the same kind of text**: the original prints
//! its own bindings inside the button captions in `textuisystem.txt` —
//! `UIIT_STT_COS_DISEMBARK` = "Dismount (Home)", `UIIT_STT_COS_CLEAN` =
//! "Terminated (PgUp)", `UIIT_STT_COS_AGGRESSIVE` / `_DEFENSIVE` =
//! "Offensive (PgDn)" / "Defensive (PgDn)" — all three confirmed by the `.dat`
//! (3017 `0x24`, 3018 `0x21`, 3021 `0x22`). No caption names a key for
//! `KeyCOSFollow`, but the option stream does: `.dat` id 3019 = `0x2E` =
//! `VK_DELETE`, and 3020 `KeyCOSAttack` = `0x23` = `VK_END`, 3025
//! `KeyCOSSelection` = `0x57` = `W`. A caption is simply not the only place the
//! original states a binding.

use bevy::input_focus::InputFocus;
use bevy::prelude::*;
use bevy::text::EditableText;

use super::options::GameOptions;

/// Whether a text field currently holds keyboard focus, i.e. the player is
/// typing rather than issuing shortcuts.
///
/// Every keybind toggle already refuses to fire while the *chat* input is open
/// (`ChatState::input_open`), but chat is not the only text field any more: the
/// party-match register dialog has a title box, and typing a name like
/// "Uigur run" into it would otherwise fire Inventory, Skill and the match
/// board itself as the letters went by. This is the general form of that
/// guard — any focused [`EditableText`] — and it composes as a run condition:
/// `.run_if(not(text_field_focused))`.
/// `InputFocus` is optional because it comes from bevy's `InputFocusPlugin`,
/// which the headless test apps do not build — and a run condition panics on a
/// missing `Res` exactly as a system does. No focus resource means nothing is
/// focused, so the keybind fires.
pub fn text_field_focused(
    focus: Option<Res<InputFocus>>,
    fields: Query<(), With<EditableText>>,
) -> bool {
    focus
        .and_then(|focus| focus.get())
        .is_some_and(|entity| fields.contains(entity))
}

/// One rebindable shortcut, as `OptionSet.csv` names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyAction {
    /// `OptionSet.csv` id (the option-stream id, 3001..=3035).
    pub id: u16,
    /// The CSV's `Name` column, verbatim.
    pub name: &'static str,
    /// openroad's current binding, where one exists — see the module note.
    pub default_key: Option<KeyCode>,
}

/// The 32 KeyMap actions from `OptionSet.csv`, in id order.
///
/// Ids **3010, 3022 and 3028 do not exist** — the 32-record count is what makes
/// the 681-byte `SROptionSet.dat` arithmetic land, so the gaps are real and not a
/// transcription slip (`docs/formats/sroptionset.md`).
pub const KEY_ACTIONS: [KeyAction; 32] = [
    // Every default below is a byte out of `SROptionSet.dat`, cited per entry as
    // `id NNNN = 0xVV`. Fifteen of them also appear in the textuisystem
    // L2250-2274 literals, so those citations are kept alongside. The six
    // actions with no `default_key` store VK `0x00`: shipped unbound.
    KeyAction {
        id: 3001,
        name: "KeyCharacter",
        // SROptionSet.dat id 3001 = 0x43 C; textuisystem L2259 "Character ( C )"
        default_key: Some(KeyCode::KeyC),
    },
    KeyAction {
        id: 3002,
        name: "KeyInventory",
        // SROptionSet.dat id 3002 = 0x49 I; textuisystem L2260 "Inventory ( I )"
        default_key: Some(KeyCode::KeyI),
    },
    KeyAction {
        id: 3003,
        name: "KeySkill",
        // SROptionSet.dat id 3003 = 0x53 S; textuisystem L2261 "Skill ( S )"
        default_key: Some(KeyCode::KeyS),
    },
    KeyAction {
        id: 3004,
        name: "KeyAction",
        // SROptionSet.dat id 3004 = 0x41 A; textuisystem L2254 "Action ( A )"
        default_key: Some(KeyCode::KeyA),
    },
    KeyAction {
        id: 3005,
        name: "KeyParty",
        // SROptionSet.dat id 3005 = 0x50 P; textuisystem L2250 "Party ( P )"
        default_key: Some(KeyCode::KeyP),
    },
    KeyAction {
        id: 3006,
        name: "KeyQuest",
        // SROptionSet.dat id 3006 = 0x51 Q; textuisystem L2262 "Quest ( Q )"
        default_key: Some(KeyCode::KeyQ),
    },
    KeyAction {
        id: 3007,
        name: "KeyCommunity",
        // SROptionSet.dat id 3007 = 0x55 U; textuisystem L2263
        // `UIIT_STT_TOGGLE_COMMUNITY` "Community ( U )". L2252 `Guild ( U )`
        // claims the same letter and OptionSet.csv has no KeyGuild action, so
        // this is the only bindable half of that collision — and the option
        // file confirms U belongs to this half.
        default_key: Some(KeyCode::KeyU),
    },
    KeyAction {
        id: 3008,
        name: "KeyWorldMap",
        // SROptionSet.dat id 3008 = 0x4D. L2258 "Whole area map" prints no
        // letter, so the option stream is the only source for this one.
        default_key: Some(KeyCode::KeyM),
    },
    KeyAction {
        id: 3009,
        name: "KeyBerserkerMode",
        // SROptionSet.dat id 3009 = 0x09
        default_key: Some(KeyCode::Tab),
    },
    KeyAction {
        id: 3011,
        name: "KeyHelp",
        // SROptionSet.dat id 3011 = 0x48
        default_key: Some(KeyCode::KeyH),
    },
    KeyAction {
        id: 3012,
        name: "KeyViewDropItem",
        // SROptionSet.dat (two real v1.188-era files, byte-identical): id 3012 = 0x5A
        default_key: Some(KeyCode::KeyZ),
    },
    KeyAction {
        id: 3013,
        name: "KeyMouseQuickSlot",
        // SROptionSet.dat id 3013 = 0x58
        default_key: Some(KeyCode::KeyX),
    },
    KeyAction {
        id: 3014,
        name: "KeySitStand",
        // SROptionSet.dat id 3014 = 0x4E
        default_key: Some(KeyCode::KeyN),
    },
    KeyAction {
        id: 3015,
        name: "KeyAutoPickup",
        // SROptionSet.dat id 3015 = 0x47
        default_key: Some(KeyCode::KeyG),
    },
    KeyAction {
        id: 3016,
        name: "KeyCOSInfo",
        // SROptionSet.dat id 3016 = 0x2D
        default_key: Some(KeyCode::Insert),
    },
    // Board/dismount is one toggle in the original (there is no separate
    // dismount action); SROptionSet.dat id 3017 = 0x24 VK_HOME, and the caption
    // names the same key: "Dismount (Home)".
    KeyAction {
        id: 3017,
        name: "KeyCOSRide",
        default_key: Some(KeyCode::Home),
    },
    // SROptionSet.dat id 3018 = 0x21 VK_PRIOR; caption "Terminated (PgUp)".
    KeyAction {
        id: 3018,
        name: "KeyCOSRelease",
        default_key: Some(KeyCode::PageUp),
    },
    KeyAction {
        id: 3019,
        name: "KeyCOSFollow",
        // SROptionSet.dat id 3019 = 0x2E
        default_key: Some(KeyCode::Delete),
    },
    KeyAction {
        id: 3020,
        name: "KeyCOSAttack",
        // SROptionSet.dat id 3020 = 0x23
        default_key: Some(KeyCode::End),
    },
    // Offensive/defensive share one toggle; SROptionSet.dat id 3021 = 0x22
    // VK_NEXT, and both captions name "(PgDn)".
    KeyAction {
        id: 3021,
        name: "KeyCOSAIType",
        default_key: Some(KeyCode::PageDown),
    },
    KeyAction {
        id: 3023,
        name: "KeyReplyWhisper",
        // SROptionSet.dat id 3023 = 0x52
        default_key: Some(KeyCode::KeyR),
    },
    KeyAction {
        id: 3024,
        name: "KeyAutoPotion",
        // SROptionSet.dat id 3024 = 0x54 T; textuisystem L2271 "Auto Potion (T)"
        default_key: Some(KeyCode::KeyT),
    },
    KeyAction {
        id: 3025,
        name: "KeyCOSSelection",
        // SROptionSet.dat id 3025 = 0x57
        default_key: Some(KeyCode::KeyW),
    },
    KeyAction {
        id: 3026,
        name: "KeyPartyMatch",
        // SROptionSet.dat id 3026 = 0x45 E; textuisystem L2251 "Party Matching(E)"
        default_key: Some(KeyCode::KeyE),
    },
    KeyAction {
        id: 3027,
        name: "KeyAlchemy",
        // SROptionSet.dat id 3027 = 0x59 Y; textuisystem L2273 "Alchemy ( Y )"
        default_key: Some(KeyCode::KeyY),
    },
    KeyAction {
        id: 3029,
        name: "KeyTargetEnemy",
        // SROptionSet.dat id 3029 = 0x00 = unbound.
        default_key: None,
    },
    KeyAction {
        id: 3030,
        name: "KeyTargetRecent",
        // SROptionSet.dat id 3030 = 0x00 = unbound.
        default_key: None,
    },
    KeyAction {
        id: 3031,
        name: "KeyTargetSupport",
        // SROptionSet.dat id 3031 = 0x00 = unbound.
        default_key: None,
    },
    KeyAction {
        id: 3032,
        name: "KeyTargetSee",
        // SROptionSet.dat id 3032 = 0x00 = unbound.
        default_key: None,
    },
    KeyAction {
        id: 3033,
        name: "KeyAcademy",
        // SROptionSet.dat id 3033 = 0x4C L; textuisystem L2253 "Academy ( L )"
        default_key: Some(KeyCode::KeyL),
    },
    KeyAction {
        id: 3034,
        name: "KeyHideFriends",
        // SROptionSet.dat id 3034 = 0x00 = unbound.
        default_key: None,
    },
    KeyAction {
        id: 3035,
        name: "KeyHideEnemies",
        // SROptionSet.dat id 3035 = 0x00 = unbound.
        default_key: None,
    },
];

/// The option ids openroad actually consumes today. Kept next to the migrated
/// call sites' ids so a rename cannot silently unbind a window.
pub const KEY_CHARACTER: u16 = 3001;
/// Opens the COS/companion window.
pub const KEY_COS_INFO: u16 = 3016;
pub const KEY_INVENTORY: u16 = 3002;
pub const KEY_SKILL: u16 = 3003;
/// Opens the party roster page.
pub const KEY_PARTY: u16 = 3005;
/// Opens the party-matching board.
pub const KEY_PARTY_MATCH: u16 = 3026;
pub const KEY_WORLD_MAP: u16 = 3008;
pub const KEY_ALCHEMY: u16 = 3027;
/// Held (not tapped) while the player wants every dropped item in range
/// labelled.
pub const KEY_VIEW_DROP_ITEM: u16 = 3012;
/// Opens the auto-potion configuration window.
pub const KEY_AUTO_POTION: u16 = 3024;
/// Board/dismount toggle (the COS command bar's first cell).
pub const KEY_COS_RIDE: u16 = 3017;
/// Dismiss the summon ("Terminated").
pub const KEY_COS_RELEASE: u16 = 3018;
/// Order the COS to follow. `SROptionSet.dat` id 3019 = `0x2E` = `VK_DELETE`,
/// bound by default even though no caption names a key for it.
pub const KEY_COS_FOLLOW: u16 = 3019;
/// Send the attack pet at the selected target. Unbound by default, and that is
/// the evidence-correct choice: `UIIT_STT_COS_ATTACK` is plain "Attack", where
/// the three bound COS commands all name their key in the caption itself
/// ("Terminated (PgUp)", "Offensive (PgDn)", "Dismount (Home)").
pub const KEY_COS_ATTACK: u16 = 3020;
/// Offensive/defensive toggle.
pub const KEY_COS_AI_TYPE: u16 = 3021;

/// Win32 VK code ↔ [`KeyCode`]. One table, both directions, so they cannot drift.
///
/// Letters and digits are their ASCII uppercase values (`VK_A == 0x41`), which is
/// what the option stream stores. Only keys a player could plausibly bind are
/// listed; a press outside this table cannot be persisted and is refused at the
/// capture site rather than stored as a value we could not read back.
const VK_TABLE: &[(u32, KeyCode)] = &[
    (0x08, KeyCode::Backspace),
    (0x09, KeyCode::Tab),
    (0x0D, KeyCode::Enter),
    (0x13, KeyCode::Pause),
    (0x14, KeyCode::CapsLock),
    (0x20, KeyCode::Space),
    (0x21, KeyCode::PageUp),
    (0x22, KeyCode::PageDown),
    (0x23, KeyCode::End),
    (0x24, KeyCode::Home),
    (0x25, KeyCode::ArrowLeft),
    (0x26, KeyCode::ArrowUp),
    (0x27, KeyCode::ArrowRight),
    (0x28, KeyCode::ArrowDown),
    (0x2D, KeyCode::Insert),
    (0x2E, KeyCode::Delete),
    (0x30, KeyCode::Digit0),
    (0x31, KeyCode::Digit1),
    (0x32, KeyCode::Digit2),
    (0x33, KeyCode::Digit3),
    (0x34, KeyCode::Digit4),
    (0x35, KeyCode::Digit5),
    (0x36, KeyCode::Digit6),
    (0x37, KeyCode::Digit7),
    (0x38, KeyCode::Digit8),
    (0x39, KeyCode::Digit9),
    (0x41, KeyCode::KeyA),
    (0x42, KeyCode::KeyB),
    (0x43, KeyCode::KeyC),
    (0x44, KeyCode::KeyD),
    (0x45, KeyCode::KeyE),
    (0x46, KeyCode::KeyF),
    (0x47, KeyCode::KeyG),
    (0x48, KeyCode::KeyH),
    (0x49, KeyCode::KeyI),
    (0x4A, KeyCode::KeyJ),
    (0x4B, KeyCode::KeyK),
    (0x4C, KeyCode::KeyL),
    (0x4D, KeyCode::KeyM),
    (0x4E, KeyCode::KeyN),
    (0x4F, KeyCode::KeyO),
    (0x50, KeyCode::KeyP),
    (0x51, KeyCode::KeyQ),
    (0x52, KeyCode::KeyR),
    (0x53, KeyCode::KeyS),
    (0x54, KeyCode::KeyT),
    (0x55, KeyCode::KeyU),
    (0x56, KeyCode::KeyV),
    (0x57, KeyCode::KeyW),
    (0x58, KeyCode::KeyX),
    (0x59, KeyCode::KeyY),
    (0x5A, KeyCode::KeyZ),
    (0x60, KeyCode::Numpad0),
    (0x61, KeyCode::Numpad1),
    (0x62, KeyCode::Numpad2),
    (0x63, KeyCode::Numpad3),
    (0x64, KeyCode::Numpad4),
    (0x65, KeyCode::Numpad5),
    (0x66, KeyCode::Numpad6),
    (0x67, KeyCode::Numpad7),
    (0x68, KeyCode::Numpad8),
    (0x69, KeyCode::Numpad9),
    (0x6A, KeyCode::NumpadMultiply),
    (0x6B, KeyCode::NumpadAdd),
    (0x6D, KeyCode::NumpadSubtract),
    (0x6E, KeyCode::NumpadDecimal),
    (0x6F, KeyCode::NumpadDivide),
    (0x70, KeyCode::F1),
    (0x71, KeyCode::F2),
    (0x72, KeyCode::F3),
    (0x73, KeyCode::F4),
    (0x74, KeyCode::F5),
    (0x75, KeyCode::F6),
    (0x76, KeyCode::F7),
    (0x77, KeyCode::F8),
    (0x78, KeyCode::F9),
    (0x79, KeyCode::F10),
    (0x7A, KeyCode::F11),
    (0x7B, KeyCode::F12),
    (0xBA, KeyCode::Semicolon),
    (0xBB, KeyCode::Equal),
    (0xBC, KeyCode::Comma),
    (0xBD, KeyCode::Minus),
    (0xBE, KeyCode::Period),
    (0xBF, KeyCode::Slash),
    (0xC0, KeyCode::Backquote),
    (0xDB, KeyCode::BracketLeft),
    (0xDC, KeyCode::Backslash),
    (0xDD, KeyCode::BracketRight),
    (0xDE, KeyCode::Quote),
];

/// A stored VK code as a Bevy key, or `None` if it is outside [`VK_TABLE`].
pub fn vk_to_keycode(vk: u32) -> Option<KeyCode> {
    VK_TABLE
        .iter()
        .find_map(|&(code, key)| (code == vk).then_some(key))
}

/// A Bevy key as the VK code the option stream stores, or `None` if it has no
/// representation there — such a key cannot be persisted, so it is not accepted.
pub fn keycode_to_vk(key: KeyCode) -> Option<u32> {
    VK_TABLE
        .iter()
        .find_map(|&(code, k)| (k == key).then_some(code))
}

/// Metadata for one action id.
pub fn action(id: u16) -> Option<&'static KeyAction> {
    KEY_ACTIONS.iter().find(|a| a.id == id)
}

impl GameOptions {
    /// Which key currently triggers action `id`: the stored binding if there is a
    /// readable one, else the action's default, else unbound.
    ///
    /// A stored VK code outside [`VK_TABLE`] falls back to the default rather than
    /// leaving the action dead — an unreadable binding is a data problem, not a
    /// reason to lose the shortcut.
    pub fn key_for(&self, id: u16) -> Option<KeyCode> {
        self.keymap
            .bindings
            .get(&id)
            .copied()
            .and_then(vk_to_keycode)
            .or_else(|| action(id).and_then(|a| a.default_key))
    }

    /// Bind `key` to action `id`. Returns `false` (and changes nothing) when the
    /// key has no VK representation, so the caller can reject the capture.
    pub fn bind_key(&mut self, id: u16, key: KeyCode) -> bool {
        match keycode_to_vk(key) {
            Some(vk) => {
                self.keymap.bindings.insert(id, vk);
                true
            }
            None => false,
        }
    }

    /// Drop the stored binding for `id`, so [`Self::key_for`] falls back to the
    /// action's default.
    pub fn reset_key(&mut self, id: u16) {
        self.keymap.bindings.remove(&id);
    }

    /// Every other action currently resolving to the same key as `id`.
    ///
    /// Two actions on one key is a real state the option stream can hold (and the
    /// UI must surface), not something to silently repair.
    pub fn key_conflicts(&self, id: u16) -> Vec<u16> {
        let Some(key) = self.key_for(id) else {
            return Vec::new();
        };
        KEY_ACTIONS
            .iter()
            .filter(|a| a.id != id && self.key_for(a.id) == Some(key))
            .map(|a| a.id)
            .collect()
    }
}

#[cfg(test)]
mod tests {

    /// A key the options pane offers must actually do something — or be
    /// listed here as knowingly unwired.
    ///
    /// The Key Map tab renders every [`KEY_ACTIONS`] entry with its default
    /// key, so a player sees "Action ( A )" and presses A. If no system reads
    /// the id, nothing happens and the UI has lied. Wiring one is a few lines;
    /// the point of this test is that *forgetting* it cannot be silent — a new
    /// window either consumes its id or says out loud that it does not yet.
    #[test]
    fn every_bound_key_is_either_consumed_or_declared_unwired() {
        /// Ids whose consumer does not exist yet. Every entry is a row the
        /// pane draws without effect; removing one means wiring it.
        const NOT_YET_WIRED: [(u16, &str); 12] = [
            (3004, "KeyAction — no action window yet"),
            (3006, "KeyQuest — no quest journal yet"),
            (3007, "KeyCommunity — no community window yet"),
            (3009, "KeyBerserkerMode — no berserk trigger yet"),
            (3011, "KeyHelp — no help window yet"),
            (3013, "KeyMouseQuickSlot — no mouse quick-slot mode yet"),
            (3014, "KeySitStand — no sit/stand chain yet"),
            (3015, "KeyAutoPickup — pickup is click/loot driven"),
            (3023, "KeyReplyWhisper — no whisper-reply shortcut yet"),
            (
                3025,
                "KeyCOSSelection — bound (0x57) since the .dat was read; \
                    there is no COS cycling/selection action yet",
            ),
            (3026, "KeyPartyMatch — no party-matching window yet"),
            (3033, "KeyAcademy — no academy panel yet"),
        ];

        let sources = client_sources();
        let consumed = |id: u16| {
            let needle = format!("key_for({id}");
            // A call site may name the constant with any path prefix
            // (`KEY_COMMUNITY`, `keymap::KEY_COMMUNITY`, the full path), so the
            // constant's *name* is what is searched for, outside this file.
            let by_const = KEY_CONSTS
                .iter()
                .find(|(const_id, _)| *const_id == id)
                .map(|(_, name)| *name);
            sources.iter().any(|text| {
                if text.contains("pub const KEY_ACTIONS") {
                    return false; // this file defines them; it does not consume them
                }
                text.contains(&needle) || by_const.is_some_and(|name| text.contains(name))
            })
        };

        let mut dead: Vec<String> = Vec::new();
        for action in KEY_ACTIONS.iter() {
            if action.default_key.is_none() {
                continue;
            }
            if NOT_YET_WIRED.iter().any(|(id, _)| *id == action.id) {
                continue;
            }
            if !consumed(action.id) {
                dead.push(format!("{} ({})", action.name, action.id));
            }
        }
        assert!(
            dead.is_empty(),
            "the Key Map tab offers these bindings and no system reads them:\n  {}",
            dead.join("\n  ")
        );

        // The excuse list must not outlive the excuse. `NOT_YET_WIRED` is a
        // *skip* list, so an id that gained its window would keep silencing the
        // check above. A listed id that is consumed is a failure, not a shrug.
        let stale: Vec<&str> = NOT_YET_WIRED
            .iter()
            .filter(|(id, _)| consumed(*id))
            .map(|(_, why)| *why)
            .collect();
        assert!(
            stale.is_empty(),
            "these ids are consumed and must leave NOT_YET_WIRED:\n  {}",
            stale.join("\n  ")
        );

        // Positive control: the scan can actually find a consumed id, so a
        // broken scan fails loudly instead of passing everything.
        assert!(
            consumed(3002),
            "KeyInventory is consumed by inventory/model.rs — the scan is broken"
        );
    }

    /// The `pub const KEY_*` names a call site may use instead of a literal id.
    /// A missing entry is not cosmetic: `consumed()` falls back to searching for
    /// `key_for(<id>`, which a site calling `pressed(KEY_COS_RIDE)` never
    /// matches.
    const KEY_CONSTS: [(u16, &str); 14] = [
        (3001, "KEY_CHARACTER"),
        (3002, "KEY_INVENTORY"),
        (3003, "KEY_SKILL"),
        (3005, "KEY_PARTY"),
        (3008, "KEY_WORLD_MAP"),
        (3012, "KEY_VIEW_DROP_ITEM"),
        (3016, "KEY_COS_INFO"),
        (3017, "KEY_COS_RIDE"),
        (3018, "KEY_COS_RELEASE"),
        (3019, "KEY_COS_FOLLOW"),
        (3020, "KEY_COS_ATTACK"),
        (3021, "KEY_COS_AI_TYPE"),
        (3024, "KEY_AUTO_POTION"),
        (3027, "KEY_ALCHEMY"),
    ];

    /// Every `.rs` file of the client crate.
    fn client_sources() -> Vec<String> {
        fn walk(dir: &std::path::Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("src is readable").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(std::fs::read_to_string(&path).expect("source is readable"));
                }
            }
        }
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut out = Vec::new();
        walk(&root, &mut out);
        out
    }
    use super::*;

    /// The trap that came with the party-title field: typing a name like
    /// "Uigur run" would otherwise fire Inventory, Skill and the match board
    /// itself as the letters went by, because every toggle gated only on the
    /// *chat* input being open.
    #[test]
    fn a_focused_text_field_blocks_the_keybinds_and_nothing_else_does() {
        let mut app = App::new();
        app.init_resource::<InputFocus>();

        let field = app.world_mut().spawn(EditableText::new("")).id();
        let plain = app.world_mut().spawn_empty().id();

        let focused = |app: &mut App| {
            app.world_mut()
                .run_system_cached(text_field_focused)
                .expect("the condition must be callable")
        };

        // nothing focused
        assert!(!focused(&mut app));

        // a focused entity that is not a text field is not typing
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(plain, bevy::input_focus::FocusCause::Navigated);
        assert!(!focused(&mut app));

        // ...and one that is, is
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(field, bevy::input_focus::FocusCause::Navigated);
        assert!(focused(&mut app));

        app.world_mut().resource_mut::<InputFocus>().clear();
        assert!(!focused(&mut app));
    }

    /// The headless apps do not build bevy's `InputFocusPlugin`, and a run
    /// condition panics on a missing `Res` exactly as a system does. No focus
    /// resource means nothing is focused, so the keybind fires.
    #[test]
    fn a_world_without_the_focus_resource_does_not_block_keybinds() {
        let mut app = App::new();
        assert!(!app
            .world_mut()
            .run_system_cached(text_field_focused)
            .expect("the condition must survive a missing InputFocus"));
    }

    /// The gaps are load-bearing: 32 records is what makes the documented
    /// 681-byte option-stream arithmetic work.
    #[test]
    fn the_action_table_has_32_entries_and_skips_the_three_absent_ids() {
        assert_eq!(KEY_ACTIONS.len(), 32);
        for missing in [3010u16, 3022, 3028] {
            assert!(action(missing).is_none(), "{missing} should not exist");
        }
        // ids are unique and ascending
        let ids: Vec<u16> = KEY_ACTIONS.iter().map(|a| a.id).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(ids, sorted);
    }

    /// Letters and digits are ASCII uppercase in the option stream — the same
    /// assumption `options.rs`'s own round-trip test encodes with `0x41`.
    #[test]
    fn vk_translation_round_trips_both_ways() {
        assert_eq!(vk_to_keycode(0x41), Some(KeyCode::KeyA));
        assert_eq!(keycode_to_vk(KeyCode::KeyA), Some(0x41));
        for &(vk, key) in VK_TABLE {
            assert_eq!(vk_to_keycode(vk), Some(key));
            assert_eq!(keycode_to_vk(key), Some(vk));
        }
    }

    /// The table must be injective in both directions, or a rebind could resolve
    /// to a different key than it stored.
    #[test]
    fn the_vk_table_has_no_duplicate_entries() {
        let mut vks: Vec<u32> = VK_TABLE.iter().map(|&(vk, _)| vk).collect();
        let before = vks.len();
        vks.sort_unstable();
        vks.dedup();
        assert_eq!(vks.len(), before, "duplicate VK code");

        let mut keys: Vec<String> = VK_TABLE.iter().map(|&(_, k)| format!("{k:?}")).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), before, "duplicate KeyCode");
    }

    /// Every action openroad consumes must resolve out of the box, so migrating
    /// the hardcoded call sites cannot silently disable a window.
    #[test]
    fn the_four_consumed_actions_default_to_todays_keys() {
        let opts = GameOptions::default();
        assert_eq!(opts.key_for(KEY_CHARACTER), Some(KeyCode::KeyC));
        assert_eq!(opts.key_for(KEY_INVENTORY), Some(KeyCode::KeyI));
        assert_eq!(opts.key_for(KEY_SKILL), Some(KeyCode::KeyS));
        assert_eq!(opts.key_for(KEY_WORLD_MAP), Some(KeyCode::KeyM));
    }

    /// The COS keys, cross-confirmed twice over: the captions print three of
    /// them ("Dismount (Home)", "Terminated (PgUp)", "Offensive/Defensive
    /// (PgDn)") and `SROptionSet.dat` carries the same three VK codes plus the
    /// two no caption names — follow = `0x2E` VK_DELETE, attack = `0x23`
    /// two no caption names — follow = `0x2E` VK_DELETE, attack = `0x23`
    /// VK_END.
    #[test]
    fn the_cos_keys_named_by_vanilla_captions_are_bound() {
        let opts = GameOptions::default();
        assert_eq!(opts.key_for(KEY_COS_RIDE), Some(KeyCode::Home));
        assert_eq!(opts.key_for(KEY_COS_RELEASE), Some(KeyCode::PageUp));
        assert_eq!(opts.key_for(KEY_COS_AI_TYPE), Some(KeyCode::PageDown));
        // Follow names no caption, but the option stream does:
        // SROptionSet.dat id 3019 = 0x2E = VK_DELETE.
        assert_eq!(opts.key_for(KEY_COS_FOLLOW), Some(KeyCode::Delete));
    }

    /// The shipped keymap: 26 of the 32 actions carry a binding and exactly 6
    /// do not. Both halves come from the `SROptionSet.dat` KeyMap block — the
    /// unbound six are stored there as VK `0x00`, so "unbound" is a value the
    /// file states, not a gap we left.
    #[test]
    fn the_keymap_binds_26_actions_and_leaves_6_unbound() {
        let opts = GameOptions::default();
        let bound = KEY_ACTIONS
            .iter()
            .filter(|a| opts.key_for(a.id).is_some())
            .count();
        assert_eq!(bound, 26);
        assert_eq!(KEY_ACTIONS.len() - bound, 6);

        // The six the file stores as 0 — the four target keys, hide-friends,
        // hide-enemies. These are the data saying "unbound".
        for id in [3029u16, 3030, 3031, 3032, 3034, 3035] {
            assert_eq!(opts.key_for(id), None, "{id} is 0 in SROptionSet.dat");
        }
    }

    /// Eleven defaults that come from the option stream alone, each against the
    /// VK code `SROptionSet.dat` stores for that id. Two independent installs
    /// carry a byte-identical keymap block (see the module note).
    #[test]
    fn the_eleven_option_file_defaults_match_their_vk_codes() {
        let opts = GameOptions::default();
        for (id, vk, key) in [
            (3009u16, 0x09u32, KeyCode::Tab),
            (3011, 0x48, KeyCode::KeyH),
            (3012, 0x5A, KeyCode::KeyZ),
            (3013, 0x58, KeyCode::KeyX),
            (3014, 0x4E, KeyCode::KeyN),
            (3015, 0x47, KeyCode::KeyG),
            (3016, 0x2D, KeyCode::Insert),
            (3019, 0x2E, KeyCode::Delete),
            (3020, 0x23, KeyCode::End),
            (3023, 0x52, KeyCode::KeyR),
            (3025, 0x57, KeyCode::KeyW),
        ] {
            assert_eq!(opts.key_for(id), Some(key), "default of {id}");
            // and the translation is the one an imported .dat would take
            assert_eq!(vk_to_keycode(vk), Some(key), "VK {vk:#04x} of {id}");
        }
    }

    /// The shipped set must not collide with itself: 26 bound actions, 26
    /// distinct keys. A silent duplicate would make two windows fight over one
    /// key straight out of the box.
    #[test]
    fn no_two_shipped_defaults_share_a_key() {
        let opts = GameOptions::default();
        let mut keys: Vec<String> = KEY_ACTIONS
            .iter()
            .filter_map(|a| opts.key_for(a.id))
            .map(|k| format!("{k:?}"))
            .collect();
        let before = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), before, "two actions ship on the same key");
        assert_eq!(before, 26);
    }

    /// The defaults the string table names in its own label text, pinned against
    /// their lines (`textuisystem.txt`, UTF-16LE). This is the second source that
    /// lets the `SROptionSet.dat` keymap block be read as the shipped defaults
    /// rather than one player's rebind: all fourteen string-named keys agree with
    /// the file.
    #[test]
    fn data_sourced_defaults_match_their_textuisystem_lines() {
        let opts = GameOptions::default();
        // L2250 "Party ( P )", L2251 "Party Matching(E)", L2254 "Action ( A )"
        assert_eq!(opts.key_for(3005), Some(KeyCode::KeyP));
        assert_eq!(opts.key_for(3026), Some(KeyCode::KeyE));
        assert_eq!(opts.key_for(3004), Some(KeyCode::KeyA));
        // L2262 "Quest ( Q )", L2271 "Auto Potion (T)", L2273 "Alchemy ( Y )"
        assert_eq!(opts.key_for(3006), Some(KeyCode::KeyQ));
        assert_eq!(opts.key_for(3024), Some(KeyCode::KeyT));
        assert_eq!(opts.key_for(KEY_ALCHEMY), Some(KeyCode::KeyY));
        // L2253 "Academy ( L )"
        assert_eq!(opts.key_for(3033), Some(KeyCode::KeyL));
    }

    /// `Guild ( U )` (L2252) and `Community ( U )` (L2263) claim the same
    /// letter and the strings alone cannot decide it, so at most one action may
    /// hold `U`. The option file settles which: id 3007 (`KeyCommunity`, the
    /// only half `OptionSet.csv` makes bindable) is `0x55`. The world map's
    /// label carries no letter at all (L2258 "Whole area map") — its `M` is
    /// id 3008 = `0x4D` in the same file, not an openroad invention any more.
    #[test]
    fn the_u_collision_binds_exactly_the_community_half_and_the_map_keeps_its_m() {
        let opts = GameOptions::default();
        let on_u: Vec<u16> = KEY_ACTIONS
            .iter()
            .filter(|a| opts.key_for(a.id) == Some(KeyCode::KeyU))
            .map(|a| a.id)
            .collect();
        assert_eq!(on_u, vec![3007], "U belongs to KeyCommunity, id 3007");
        assert_eq!(opts.key_for(KEY_WORLD_MAP), Some(KeyCode::KeyM));
    }

    /// Eleven bindings that come from the option stream alone, nailed to the id
    /// and the raw VK byte the file stores.
    ///
    /// Source for every row: `setting\SROptionSet.dat`, KeyMap block. **No
    /// fixture is copied into the repo**: the bytes are the constants below, and
    /// each one is checked twice — once as the VK the file stores, once as the
    /// `KeyCode` it must resolve to — so a slip in `VK_TABLE` cannot hide behind
    /// a slip here.
    #[test]
    fn the_eleven_dat_sourced_bindings_match_their_stored_vk_bytes() {
        /// (`OptionSet.csv` id, raw VK byte in the `.dat`, expected key)
        const SHIPPED_BINDINGS: [(u16, u32, KeyCode); 11] = [
            (3009, 0x09, KeyCode::Tab),    // KeyBerserkerMode
            (3011, 0x48, KeyCode::KeyH),   // KeyHelp
            (3012, 0x5A, KeyCode::KeyZ),   // KeyViewDropItem
            (3013, 0x58, KeyCode::KeyX),   // KeyMouseQuickSlot
            (3014, 0x4E, KeyCode::KeyN),   // KeySitStand
            (3015, 0x47, KeyCode::KeyG),   // KeyAutoPickup
            (3016, 0x2D, KeyCode::Insert), // KeyCOSInfo
            (3019, 0x2E, KeyCode::Delete), // KeyCOSFollow
            (3020, 0x23, KeyCode::End),    // KeyCOSAttack
            (3023, 0x52, KeyCode::KeyR),   // KeyReplyWhisper
            (3025, 0x57, KeyCode::KeyW),   // KeyCOSSelection
        ];

        let opts = GameOptions::default();
        for (id, vk, key) in SHIPPED_BINDINGS {
            assert_eq!(
                vk_to_keycode(vk),
                Some(key),
                "VK {vk:#04X} (id {id}) must translate to {key:?}"
            );
            assert_eq!(
                opts.key_for(id),
                Some(key),
                "id {id} ships bound to VK {vk:#04X} in both SROptionSet.dat"
            );
            assert_eq!(
                keycode_to_vk(key),
                Some(vk),
                "rebinding id {id} back to its shipped key must store {vk:#04X} again"
            );
        }
    }

    /// The six actions the original itself leaves unbound. Same two files, same
    /// read: their stored VK is `0x00`, which is not a key — so openroad must
    /// not invent one either.
    ///
    /// Positive control on the same read path: id 3033 `KeyAcademy` comes out
    /// of that same `.dat` block with `0x4C`, so a table that had lost its
    /// bindings wholesale would fail here instead of passing this test.
    #[test]
    fn the_six_actions_with_vk_zero_stay_unbound() {
        const UNBOUND: [(u16, &str); 6] = [
            (3029, "KeyTargetEnemy"),
            (3030, "KeyTargetRecent"),
            (3031, "KeyTargetSupport"),
            (3032, "KeyTargetSee"),
            (3034, "KeyHideFriends"),
            (3035, "KeyHideEnemies"),
        ];

        let opts = GameOptions::default();
        for (id, name) in UNBOUND {
            assert_eq!(
                action(id).map(|a| a.name),
                Some(name),
                "id {id} must still be {name} in OptionSet.csv order"
            );
            assert_eq!(
                opts.key_for(id),
                None,
                "{name} ({id}) stores VK 0x00 in both SROptionSet.dat"
            );
        }

        assert_eq!(
            opts.key_for(3033),
            Some(KeyCode::KeyL),
            "positive control: id 3033 is 0x4C in the same block"
        );
    }

    #[test]
    fn a_stored_binding_overrides_the_default_and_reset_restores_it() {
        let mut opts = GameOptions::default();
        assert!(opts.bind_key(KEY_INVENTORY, KeyCode::F5));
        assert_eq!(opts.key_for(KEY_INVENTORY), Some(KeyCode::F5));

        opts.reset_key(KEY_INVENTORY);
        assert_eq!(opts.key_for(KEY_INVENTORY), Some(KeyCode::KeyI));
    }

    /// A key with no VK code cannot be stored, so the bind is refused outright
    /// rather than writing a value we could not read back.
    #[test]
    fn a_key_outside_the_vk_table_is_refused() {
        let mut opts = GameOptions::default();
        assert!(!opts.bind_key(KEY_INVENTORY, KeyCode::ContextMenu));
        assert!(opts.keymap.bindings.get(&KEY_INVENTORY).is_none());
        assert_eq!(opts.key_for(KEY_INVENTORY), Some(KeyCode::KeyI));
    }

    /// An unreadable stored VK falls back to the default instead of killing the
    /// shortcut.
    #[test]
    fn an_unreadable_stored_vk_falls_back_to_the_default() {
        let mut opts = GameOptions::default();
        opts.keymap.bindings.insert(KEY_SKILL, 0xFFFF);

        assert_eq!(opts.key_for(KEY_SKILL), Some(KeyCode::KeyS));
    }

    #[test]
    fn binding_two_actions_to_one_key_is_reported_as_a_conflict() {
        let mut opts = GameOptions::default();
        assert!(opts.key_conflicts(KEY_INVENTORY).is_empty());

        assert!(opts.bind_key(KEY_INVENTORY, KeyCode::KeyS));

        assert_eq!(opts.key_conflicts(KEY_INVENTORY), vec![KEY_SKILL]);
        assert_eq!(opts.key_conflicts(KEY_SKILL), vec![KEY_INVENTORY]);
    }

    /// Unbound actions all resolve to `None`; that must not read as 28 mutual
    /// conflicts.
    #[test]
    fn unbound_actions_do_not_conflict_with_each_other() {
        let opts = GameOptions::default();

        assert!(opts.key_conflicts(3004).is_empty());
        assert!(opts.key_conflicts(3035).is_empty());
    }
}
