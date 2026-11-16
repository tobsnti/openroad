//! One way for a control openroad has not hooked up yet to say so.
//!
//! Idea: such controls answered a click with a log line only, which to the
//! player is a dead click. Each site now writes one [`Unavailable`] with its
//! own label, read from the data where the data has one, and the one reader
//! here turns it into a chat system line. The chat history doubles as the
//! memory, so each control is told once per session instead of filling the
//! chat. Chat rather than a toast for the same reason: the line stays to be
//! read again.

use bevy::prelude::*;

use crate::plugins::hud::chat::model::{ChatHistory, ChatLine};

/// Something the player reached that openroad does not do yet, by its label.
#[derive(Message, Clone, Debug)]
pub struct Unavailable(pub String);

/// "Once" is bound to the chat history: a line still in it is not said again,
/// the history is emptied on every entry into the world, and a line pushed out
/// of a full history (`CHAT_HISTORY_CAP`) is said again on the next click.
fn report_unavailable(mut reader: MessageReader<Unavailable>, mut history: ResMut<ChatHistory>) {
    for Unavailable(label) in reader.read() {
        // some labels are whole sentences (`UIIT_STT_FRPVP_ON`, an action slot)
        let label = label.trim_end_matches(['.', ' ']);
        info!("not hooked up in openroad yet: {label}");
        // Ours (ADR 0009; the data has no such sentence). It names the control,
        // not the feature, which can exist elsewhere (the Free PvP panel shows
        // itself in Free PvP mode).
        let line = format!("{label}: not hooked up in openroad yet.");
        if !history.iter().any(|told| told.text == line) {
            history.push(ChatLine::system(line));
        }
    }
}

pub struct UnavailablePlugin;

impl Plugin for UnavailablePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Unavailable>()
            .add_systems(Update, report_unavailable.run_if(super::hud_scenes));
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::plugins::hud::chat::model::ChatLineKind;

    fn say(app: &mut App, labels: &[&str]) -> Vec<(ChatLineKind, String)> {
        for label in labels {
            app.world_mut()
                .write_message(Unavailable(label.to_string()));
        }
        app.update();
        app.world()
            .resource::<ChatHistory>()
            .iter()
            .map(|line| (line.kind, line.text.clone()))
            .collect()
    }

    /// One message is one system line naming its label; the same label again,
    /// in the same frame or a later one, adds nothing, and another label does.
    /// A new session starts with an empty history and is told again.
    #[test]
    fn each_label_is_told_once_per_session() {
        let mut app = App::new();
        app.init_resource::<ChatHistory>()
            .add_message::<Unavailable>()
            .add_systems(Update, report_unavailable);

        let lines = say(&mut app, &["Craft"]);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].0, ChatLineKind::System);
        assert!(lines[0].1.contains("Craft"), "{lines:?}");
        assert_eq!(say(&mut app, &["Craft", "Craft"]), lines);
        assert_eq!(say(&mut app, &["Quest ( Q )"]).len(), 2);

        // what `chat::ui::spawn_chat_window` does when a session starts
        app.world_mut().resource_mut::<ChatHistory>().clear();
        assert_eq!(say(&mut app, &["Craft"]), lines);
    }
}
