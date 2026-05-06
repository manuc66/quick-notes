use chrono::{DateTime, Local};
use iced::widget::{button, container, mouse_area, row, scrollable, text, text_input, Column};
use iced::widget::operation::focus;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use once_cell::sync::Lazy;
use uuid::Uuid;
use iced::widget::text_editor::Action;

static NOTES_DIR: Lazy<PathBuf> = Lazy::new(|| {
    let dir = dirs::home_dir().unwrap().join(".quick-notes").join("notes");
    fs::create_dir_all(&dir).ok();
    dir
});

// ─── Persistence layer ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NoteFile {
    id: String,
    content: String,
    timestamp: i64,
}

impl NoteFile {
    fn new(content: String) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            content,
            timestamp: Local::now().timestamp_millis(),
        }
    }

    fn save(&self) -> Result<(), std::io::Error> {
        let path = NOTES_DIR.join(format!("{}.json", self.id));
        fs::write(path, serde_json::to_string(self).unwrap())
    }

    fn delete(id: &str) {
        let _ = fs::remove_file(NOTES_DIR.join(format!("{}.json", id)));
        let _ = fs::remove_file(NOTES_DIR.join(format!("{}.txt", id)));
    }
}

// ─── In-memory note ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Note {
    id: String,
    content: String,
    timestamp: i64,
}

impl From<NoteFile> for Note {
    fn from(f: NoteFile) -> Self {
        Note { id: f.id, content: f.content, timestamp: f.timestamp }
    }
}

// ─── Confirm state ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum ConfirmState {
    None,
    DeleteOne(String),
    DeleteAll,
    /// Carries the ID of the note the user wants to load
    UnsavedWorkBeforeLoad(String),
    UnsavedWorkBeforeClear,
}

// ─── Messages ─────────────────────────────────────────────────────────────────

// Stable widget ID for the search input so we can re-focus it after each keystroke
const SEARCH_ID: &str = "search_input";

#[derive(Debug, Clone)]
enum Message {
    Edit(Action),
    Save,
    Clear,
    /// Click on a note row (or Load button) — triggers load with dirty check
    LoadNote(String),
    ConfirmLoad(String),
    CancelLoad,
    DeleteNote(String),
    ConfirmDeleteNote(String),
    DeleteAllNotes,
    ConfirmDeleteAll,
    CancelConfirm,
    SearchChanged(String),
    ClearSearch,
    CloseWindow(iced::window::Id),
}

// ─── Application state ────────────────────────────────────────────────────────

struct AppState {
    notes: Vec<Note>,
    editor: iced::widget::text_editor::Content,
    search_query: String,
    editing_id: Option<String>,
    confirm_state: ConfirmState,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            notes: load_all_notes(),
            editor: iced::widget::text_editor::Content::new(),
            search_query: String::new(),
            editing_id: None,
            confirm_state: ConfirmState::None,
        }
    }
}

// ─── Persistence helpers ──────────────────────────────────────────────────────

fn load_all_notes() -> Vec<Note> {
    let mut notes: Vec<Note> = fs::read_dir(&*NOTES_DIR)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            match path.extension().and_then(|x| x.to_str()) {
                Some("json") => {
                    let raw = fs::read_to_string(&path).ok()?;
                    let f: NoteFile = serde_json::from_str(&raw).ok()?;
                    if f.content.trim().is_empty() { return None; }
                    Some(Note::from(f))
                }
                Some("txt") => {
                    // Migrate legacy files
                    let content = fs::read_to_string(&path).ok()?;
                    if content.trim().is_empty() { return None; }
                    let f = NoteFile::new(content);
                    f.save().ok();
                    let _ = fs::remove_file(&path);
                    Some(Note::from(f))
                }
                _ => None,
            }
        })
        .collect();
    notes.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    notes
}

fn editor_text(content: &iced::widget::text_editor::Content) -> String {
    content.text().trim_end_matches('\n').to_string()
}

/// Unicode-aware word-boundary truncation.
fn truncate_at_word(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let byte_end = s.char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let cut = &s[..byte_end];
    match cut.rfind(' ') {
        Some(sp) => format!("{}...", &cut[..sp]),
        None => format!("{}...", cut),
    }
}

// ─── Update ───────────────────────────────────────────────────────────────────

fn new_state() -> AppState {
    AppState::default()
}

fn update(state: &mut AppState, message: Message) -> iced::Task<Message> {
    match message {
        Message::Edit(action) => {
            state.editor.perform(action);
            iced::Task::none()
        }

        Message::Save => {
            let content = editor_text(&state.editor);
            if content.trim().is_empty() {
                return iced::Task::none();
            }
            if let Some(ref eid) = state.editing_id.clone() {
                if let Some(note) = state.notes.iter_mut().find(|n| n.id == *eid) {
                    if note.content != content {
                        note.content = content.clone();
                        note.timestamp = Local::now().timestamp_millis();
                        let file = NoteFile { id: note.id.clone(), content, timestamp: note.timestamp };
                        file.save().ok();
                        state.notes.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
                    }
                }
                state.editor = iced::widget::text_editor::Content::new();
                state.editing_id = None;
            } else {
                let is_dup = state.notes.iter().any(|n| n.content.trim() == content.trim());
                if !is_dup {
                    let file = NoteFile::new(content);
                    file.save().ok();
                    state.notes.insert(0, Note::from(file));
                }
                state.editor = iced::widget::text_editor::Content::new();
            }
            iced::Task::none()
        }

        Message::Clear => {
            if let Some(ref eid) = state.editing_id {
                let content = editor_text(&state.editor);
                let original = state.notes.iter()
                    .find(|n| n.id == *eid)
                    .map(|n| n.content.as_str())
                    .unwrap_or("");
                if content != original && !content.trim().is_empty() {
                    state.confirm_state = ConfirmState::UnsavedWorkBeforeClear;
                    return iced::Task::none();
                }
            }
            state.editor = iced::widget::text_editor::Content::new();
            state.editing_id = None;
            state.confirm_state = ConfirmState::None;
            iced::Task::none()
        }

        Message::LoadNote(id) => {
            let content = editor_text(&state.editor);
            let has_unsaved_new = !content.trim().is_empty() && state.editing_id.is_none();
            let has_unsaved_edit = state.editing_id.as_ref().map(|eid| {
                state.notes.iter()
                    .find(|n| n.id == *eid)
                    .map(|n| n.content != content)
                    .unwrap_or(false)
            }).unwrap_or(false);

            if has_unsaved_new || has_unsaved_edit {
                state.confirm_state = ConfirmState::UnsavedWorkBeforeLoad(id);
                return iced::Task::none();
            }

            if let Some(note) = state.notes.iter().find(|n| n.id == id) {
                state.editor = iced::widget::text_editor::Content::with_text(&note.content);
                state.editing_id = Some(note.id.clone());
            }
            state.confirm_state = ConfirmState::None;
            iced::Task::none()
        }

        Message::ConfirmLoad(id) => {
            if let Some(note) = state.notes.iter().find(|n| n.id == id) {
                state.editor = iced::widget::text_editor::Content::with_text(&note.content);
                state.editing_id = Some(note.id.clone());
            }
            state.confirm_state = ConfirmState::None;
            iced::Task::none()
        }

        Message::CancelLoad | Message::CancelConfirm => {
            state.confirm_state = ConfirmState::None;
            iced::Task::none()
        }

        Message::DeleteNote(id) => {
            state.confirm_state = ConfirmState::DeleteOne(id);
            iced::Task::none()
        }

        Message::ConfirmDeleteNote(id) => {
            NoteFile::delete(&id);
            state.notes.retain(|n| n.id != id);
            if state.editing_id.as_deref() == Some(id.as_str()) {
                state.editor = iced::widget::text_editor::Content::new();
                state.editing_id = None;
            }
            state.confirm_state = ConfirmState::None;
            iced::Task::none()
        }

        Message::DeleteAllNotes => {
            state.confirm_state = ConfirmState::DeleteAll;
            iced::Task::none()
        }

        Message::ConfirmDeleteAll => {
            for note in &state.notes {
                NoteFile::delete(&note.id);
            }
            state.notes.clear();
            state.editor = iced::widget::text_editor::Content::new();
            state.editing_id = None;
            state.confirm_state = ConfirmState::None;
            iced::Task::none()
        }

        // FIX: re-focus the search field after every keystroke so the user
        // never has to re-click between characters.
        Message::SearchChanged(q) => {
            state.search_query = q;
            focus(iced::widget::Id::new(SEARCH_ID))
        }

        Message::ClearSearch => {
            state.search_query.clear();
            focus(iced::widget::Id::new(SEARCH_ID))
        }

        Message::CloseWindow(window_id) => {
            let content = editor_text(&state.editor);
            if !content.trim().is_empty() {
                if let Some(ref eid) = state.editing_id.clone() {
                    if let Some(note) = state.notes.iter_mut().find(|n| n.id == *eid) {
                        if note.content != content {
                            note.content = content.clone();
                            note.timestamp = Local::now().timestamp_millis();
                            let file = NoteFile { id: note.id.clone(), content, timestamp: note.timestamp };
                            file.save().ok();
                        }
                    }
                } else {
                    let is_dup = state.notes.iter().any(|n| n.content.trim() == content.trim());
                    if !is_dup {
                        let file = NoteFile::new(content);
                        file.save().ok();
                    }
                }
            }
            iced::window::close(window_id)
        }
    }
}

// ─── View ─────────────────────────────────────────────────────────────────────

fn view(state: &AppState) -> iced::Element<'_, Message> {
    let is_filtering = !state.search_query.is_empty();

    let filtered: Vec<&Note> = state.notes.iter()
        .filter(|n| {
            !is_filtering
                || n.content.to_lowercase().contains(&state.search_query.to_lowercase())
        })
        .collect();

    // ── Editor header label ───────────────────────────────────────────────────
    let (header_text, header_color) = if let Some(ref eid) = state.editing_id {
        match state.notes.iter().find(|n| n.id == *eid) {
            Some(note) if editor_text(&state.editor) != note.content =>
                ("Editing (unsaved)", iced::Color::from_rgb(0.95, 0.6, 0.3)),
            Some(_) =>
                ("Editing", iced::Color::from_rgb(0.95, 0.75, 0.3)),
            None =>
                ("New Note", iced::Color::from_rgb(0.35, 0.9, 0.55)),
        }
    } else {
        ("New Note", iced::Color::from_rgb(0.35, 0.9, 0.55))
    };

    // ── Confirmation dialog ───────────────────────────────────────────────────
    let confirm_dialog: Option<iced::Element<Message>> = match &state.confirm_state {
        ConfirmState::UnsavedWorkBeforeLoad(pid) => {
            let pid = pid.clone();
            Some(container(
                Column::new()
                    .push(text("Unsaved changes will be lost.")
                        .size(14).color(iced::Color::from_rgb(0.95, 0.5, 0.3)))
                    .push(text("Discard and load the selected note?")
                        .size(12).color(iced::Color::from_rgb(0.7, 0.7, 0.75)))
                    .push(row![
                        button("Discard & Load").on_press(Message::ConfirmLoad(pid)).padding(8),
                        button("Keep editing").on_press(Message::CancelLoad).padding(8),
                    ].spacing(8))
                    .spacing(8)
            ).padding(12).into())
        }
        ConfirmState::UnsavedWorkBeforeClear => {
            Some(container(
                Column::new()
                    .push(text("Unsaved changes will be lost.")
                        .size(14).color(iced::Color::from_rgb(0.95, 0.5, 0.3)))
                    .push(text("Discard changes and clear the editor?")
                        .size(12).color(iced::Color::from_rgb(0.7, 0.7, 0.75)))
                    .push(row![
                        button("Discard").on_press(Message::CancelConfirm).padding(8),
                        button("Keep editing").on_press(Message::CancelConfirm).padding(8),
                    ].spacing(8))
                    .spacing(8)
            ).padding(12).into())
        }
        _ => None,
    };

    // ── Multi-line text editor ────────────────────────────────────────────────
    let editor_widget = iced::widget::text_editor(&state.editor)
        .on_action(Message::Edit)
        .placeholder("Type your note here...")
        .padding(12)
        .size(15)
        .height(iced::Length::Fixed(120.0));

    // ── Save / Clear (Delete All is now at the bottom, away from these) ───────
    let save_label = if state.editing_id.is_some() { "Update (Ctrl+S)" } else { "Save (Ctrl+S)" };
    let buttons = row![
        button(save_label).on_press(Message::Save).padding(10),
        button("Clear").on_press(Message::Clear).padding(10),
    ].spacing(12);

    // ── Search field ──────────────────────────────────────────────────────────
    // FIX: stable Id enables text_input::focus() after SearchChanged so focus
    // is never lost between keystrokes.
    // FIX: amber border when filtering gives a persistent visual cue.
    let search_field = text_input("Search notes...", &state.search_query)
        .id(iced::widget::Id::new(SEARCH_ID))
        .on_input(Message::SearchChanged)
        .size(14)
        .padding(8)
        .width(iced::Length::Fill)
        .style(move |theme, status| {
            let mut s = text_input::default(theme, status);
            if is_filtering {
                s.border.color = iced::Color::from_rgb(0.9, 0.7, 0.2);
                s.border.width = 1.5;
            }
            s
        });

    // FIX: show live match count and clear button when a filter is active.
    let search_row: iced::Element<Message> = if is_filtering {
        row![
            search_field,
            text(format!("{} match", filtered.len()))
                .size(12)
                .color(iced::Color::from_rgb(0.9, 0.7, 0.2)),
            button("✕").on_press(Message::ClearSearch).padding(6),
        ].spacing(8).align_y(iced::Alignment::Center).into()
    } else {
        search_field.into()
    };

    // ── Status bar ────────────────────────────────────────────────────────────
    let status = format!(
        "{} chars • {} notes",
        editor_text(&state.editor).len(),
        state.notes.len()
    );

    // ── Section label changes colour + text when filtering ────────────────────
    let section_label: iced::Element<Message> = if is_filtering {
        text(format!("FILTERED — {} of {} notes", filtered.len(), state.notes.len()))
            .size(10)
            .color(iced::Color::from_rgb(0.9, 0.7, 0.2))
            .into()
    } else {
        text("ALL NOTES")
            .size(10)
            .color(iced::Color::from_rgb(0.4, 0.4, 0.45))
            .into()
    };

    // ── Notes list ────────────────────────────────────────────────────────────
    let mut notes_col = Column::new().spacing(0);

    if filtered.is_empty() {
        let msg = if is_filtering {
            "No notes match your search."
        } else {
            "No saved notes yet. Type above and press Ctrl+S to save."
        };
        notes_col = notes_col.push(
            container(
                text(msg).size(13).color(iced::Color::from_rgb(0.4, 0.4, 0.45))
            ).padding(12)
        );
    } else {
        for note in &filtered {
            let date = DateTime::from_timestamp_millis(note.timestamp)
                .map(|dt| dt.format("%d/%m/%y %H:%M").to_string())
                .unwrap_or_default();

            let preview = truncate_at_word(&note.content.replace('\n', " ↵ "), 100);
            let is_editing = state.editing_id.as_deref() == Some(note.id.as_str());

            // FIX: blue-tinted background on the row currently being edited
            // so the user always knows which note is open in the editor.
            let row_bg = if is_editing {
                iced::Color::from_rgb(0.18, 0.22, 0.28)
            } else {
                iced::Color::from_rgb(0.12, 0.12, 0.15)
            };

            // Delete action — only button on the row now
            let delete_btn: iced::Element<Message> = match &state.confirm_state {
                ConfirmState::DeleteOne(id) if id == &note.id => row![
                    button("Delete")
                        .on_press(Message::ConfirmDeleteNote(note.id.clone()))
                        .padding(8),
                    button("Cancel")
                        .on_press(Message::CancelConfirm)
                        .padding(8),
                ].spacing(6).into(),
                _ => button("✕")
                    .on_press(Message::DeleteNote(note.id.clone()))
                    .padding(10)
                    .into(),
            };

            // FIX: "← editing" label under the timestamp when this note is
            // loaded in the editor — visible even without looking at the header.
            let mut note_col = Column::new()
                .push(text(preview).size(14).color(iced::Color::from_rgb(0.9, 0.9, 0.95)))
                .push(text(date.clone()).size(11).color(iced::Color::from_rgb(0.75, 0.75, 0.8)))
                .spacing(4);

            if is_editing {
                note_col = note_col.push(
                    text("← editing")
                        .size(10)
                        .color(iced::Color::from_rgb(0.95, 0.75, 0.3))
                );
            }

            // FIX: mouse_area wraps the entire card — clicking anywhere on
            // the row loads the note, no separate Load button required.
            let card = container(
                row![
                    note_col.width(iced::Length::Fill),
                    delete_btn,
                ].align_y(iced::Alignment::Center)
            )
            .width(iced::Length::Fill)
            .padding(12)
            .style(move |_| iced::widget::container::Style {
                background: Some(row_bg.into()),
                ..Default::default()
            });

            let clickable_card = mouse_area(card)
                .on_press(Message::LoadNote(note.id.clone()));

            notes_col = notes_col.push(clickable_card);
            notes_col = notes_col.push(
                container(text(""))
                    .height(iced::Length::Fixed(1.0))
                    .width(iced::Length::Fill)
                    .style(|_| iced::widget::container::Style {
                        background: Some(iced::Color::from_rgb(0.18, 0.18, 0.22).into()),
                        ..Default::default()
                    })
            );
        }
    }

    // FIX: Delete All moved to a danger zone anchored at the very bottom of
    // the window, spatially separated from Save/Clear.
    let delete_all_zone: iced::Element<Message> = match state.confirm_state {
        ConfirmState::DeleteAll => container(
            row![
                text("Delete ALL notes permanently?")
                    .size(12)
                    .color(iced::Color::from_rgb(0.95, 0.4, 0.3)),
                button("Yes, delete all")
                    .on_press(Message::ConfirmDeleteAll)
                    .padding(8),
                button("Cancel")
                    .on_press(Message::CancelConfirm)
                    .padding(8),
            ].spacing(10).align_y(iced::Alignment::Center)
        ).padding(iced::Padding::from([8, 20])).into(),
        _ => container(
            button("Delete All Notes")
                .on_press(Message::DeleteAllNotes)
                .padding(8)
        ).padding(iced::Padding::from([8, 20])).into(),
    };

    // ── Assemble main layout ─────────────────────────────────────────────────
    let input_section = container(
        Column::new()
            .push(text(header_text).size(14).color(header_color))
            .push(editor_widget)
            .spacing(8)
    ).padding(20);

    let mut main = Column::new()
        .push(input_section)
        .push(container(buttons).padding(iced::Padding::from([0, 20])))
        .push(container(
            text(status).size(11).color(iced::Color::from_rgb(0.5, 0.5, 0.55))
        ).padding(iced::Padding::from([0, 20])))
        .push(container(search_row).padding(iced::Padding::from([0, 20])))
        .push(container(section_label).padding(iced::Padding::from([4, 20])))
        .push(
            scrollable(notes_col)
                .id(iced::widget::Id::new("notes_scroll"))
                .height(iced::Length::Fill)
        )
        // Delete All sits below the scroll, anchored to the window bottom
        .push(delete_all_zone)
        .spacing(4);

    if let Some(dialog) = confirm_dialog {
        main = Column::new().push(dialog).push(main);
    }

    container(main)
        .width(iced::Length::Fill)
        .height(iced::Length::Fill)
        .padding(12)
        .into()
}

// ─── Subscriptions ────────────────────────────────────────────────────────────

fn subscription(_state: &AppState) -> iced::Subscription<Message> {
    use iced::keyboard::Key;
    use iced::window::Event;

    iced::Subscription::batch([
        iced::keyboard::listen().filter_map(|event| {
            if let iced::keyboard::Event::KeyPressed { key, modifiers, .. } = event {
                if modifiers.contains(iced::keyboard::Modifiers::CTRL) {
                    if let Key::Character(c) = &key {
                        if c.as_str() == "s" {
                            return Some(Message::Save);
                        }
                    }
                }
            }
            None
        }),
        iced::window::events().filter_map(|(id, event)| {
            if let Event::CloseRequested = event {
                Some(Message::CloseWindow(id))
            } else {
                None
            }
        }),
    ])
}

// ─── Entry point ──────────────────────────────────────────────────────────────

fn main() -> iced::Result {
    iced::application(new_state, update, view)
        .subscription(subscription)
        .title("Quick Notes")
        .exit_on_close_request(false)
        .run()
}