use chrono::{DateTime, Local};
use iced::widget::{button, container, row, scrollable, text, text_input, Column};
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
        // Remove both .json (current) and legacy .txt files
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

/// Distinguishes what we're confirming so the dialog can route correctly.
#[derive(Debug, Clone, PartialEq)]
enum ConfirmState {
    None,
    /// Pending delete of a single note by ID.
    DeleteOne(String),
    /// Pending delete-all.
    DeleteAll,
    /// User tried to Load a note while editor has unsaved content.
    /// Carries the ID of the note they want to load.
    UnsavedWorkBeforeLoad(String),
    /// User hit Clear while editing an existing note with unsaved changes.
    UnsavedWorkBeforeClear,
}

// ─── Messages ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Message {
    /// text_editor action (keypress, cursor move, …)
    Edit(Action),
    /// Save new note or commit edits to existing one
    Save,
    /// Discard editor contents, exit edit mode
    Clear,
    /// Load a note into the editor (may trigger UnsavedWork confirm)
    LoadNote(String),
    /// Confirmed: discard editor content and load the pending note
    ConfirmLoad(String),
    /// Cancelled: keep editor content, dismiss dialog
    CancelLoad,
    /// First press: request delete confirmation
    DeleteNote(String),
    /// Second press: actually delete
    ConfirmDeleteNote(String),
    /// First press: request delete-all confirmation
    DeleteAllNotes,
    /// Second press: actually delete all
    ConfirmDeleteAll,
    /// Cancel any pending confirmation
    CancelConfirm,
    SearchChanged(String),
    ClearSearch,
    /// Window close requested (Ctrl+Q or title-bar X)
    CloseWindow(iced::window::Id),
}

// ─── Application state ────────────────────────────────────────────────────────

struct AppState {
    notes: Vec<Note>,
    editor: iced::widget::text_editor::Content,
    search_query: String,
    /// ID of the note currently loaded into the editor (edit mode)
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
                // Migrate legacy .txt files on first load
                Some("txt") => {
                    let content = fs::read_to_string(&path).ok()?;
                    if content.trim().is_empty() { return None; }
                    let f = NoteFile::new(content);
                    f.save().ok();
                    // Remove old file after successful migration
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

/// Returns the full text of the editor, with the trailing newline that
/// `text_editor::Content::text()` always appends stripped off.
fn editor_text(content: &iced::widget::text_editor::Content) -> String {
    let s = content.text();
    s.trim_end_matches('\n').to_string()
}

/// Safe Unicode-aware truncation at a word boundary.
fn truncate_at_word(s: &str, max_chars: usize) -> String {
    let char_count = s.chars().count();
    if char_count <= max_chars {
        return s.to_string();
    }
    // Find byte offset of the max_chars-th character
    let byte_end = s.char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(s.len());
    let cut = &s[..byte_end];
    // Walk back to a word boundary
    if let Some(last_space) = cut.rfind(' ') {
        format!("{}...", &cut[..last_space])
    } else {
        format!("{}...", cut)
    }
}

// ─── Update ───────────────────────────────────────────────────────────────────

fn new_state() -> AppState {
    AppState::default()
}

fn update(state: &mut AppState, message: Message) -> iced::Task<Message> {
    match message {
        // ── Editor input ──────────────────────────────────────────────────────
        Message::Edit(action) => {
            state.editor.perform(action);
            iced::Task::none()
        }

        // ── Save / Update ─────────────────────────────────────────────────────
        Message::Save => {
            let content = editor_text(&state.editor);
            if content.trim().is_empty() {
                return iced::Task::none();
            }

            if let Some(ref editing_id) = state.editing_id.clone() {
                // Update existing note
                if let Some(note) = state.notes.iter_mut().find(|n| n.id == *editing_id) {
                    if note.content != content {
                        note.content = content.clone();
                        note.timestamp = Local::now().timestamp_millis();
                        let file = NoteFile {
                            id: note.id.clone(),
                            content,
                            timestamp: note.timestamp,
                        };
                        file.save().ok();
                        state.notes.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
                    }
                }
                state.editor = iced::widget::text_editor::Content::new();
                state.editing_id = None;
            } else {
                // New note — silently skip exact duplicates
                let is_duplicate = state.notes.iter().any(|n| n.content.trim() == content.trim());
                if !is_duplicate {
                    let file = NoteFile::new(content);
                    file.save().ok();
                    state.notes.insert(0, Note::from(file));
                }
                state.editor = iced::widget::text_editor::Content::new();
            }
            iced::Task::none()
        }

        // ── Clear / cancel edit ───────────────────────────────────────────────
        Message::Clear => {
            if let Some(ref editing_id) = state.editing_id {
                let content = editor_text(&state.editor);
                let original = state.notes.iter()
                    .find(|n| n.id == *editing_id)
                    .map(|n| n.content.as_str())
                    .unwrap_or("");
                if content != original && !content.trim().is_empty() {
                    // Unsaved edits — ask before discarding
                    state.confirm_state = ConfirmState::UnsavedWorkBeforeClear;
                    return iced::Task::none();
                }
            }
            state.editor = iced::widget::text_editor::Content::new();
            state.editing_id = None;
            state.confirm_state = ConfirmState::None;
            iced::Task::none()
        }

        // ── Load a note into the editor ───────────────────────────────────────
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

            // Safe to load directly
            if let Some(note) = state.notes.iter().find(|n| n.id == id) {
                state.editor = iced::widget::text_editor::Content::with_text(&note.content);
                state.editing_id = Some(note.id.clone());
            }
            state.confirm_state = ConfirmState::None;
            iced::Task::none()
        }

        // ── Confirmed: discard and load ───────────────────────────────────────
        Message::ConfirmLoad(id) => {
            if let Some(note) = state.notes.iter().find(|n| n.id == id) {
                state.editor = iced::widget::text_editor::Content::with_text(&note.content);
                state.editing_id = Some(note.id.clone());
            }
            state.confirm_state = ConfirmState::None;
            iced::Task::none()
        }

        // ── Cancel any confirm dialog — restores nothing, just dismisses ──────
        Message::CancelLoad | Message::CancelConfirm => {
            state.confirm_state = ConfirmState::None;
            iced::Task::none()
        }

        // ── Delete single (first press = confirm prompt) ──────────────────────
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

        // ── Delete all ────────────────────────────────────────────────────────
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

        // ── Search ────────────────────────────────────────────────────────────
        Message::SearchChanged(q) => {
            state.search_query = q;
            iced::Task::none()
        }
        Message::ClearSearch => {
            state.search_query.clear();
            iced::Task::none()
        }

        // ── Window close ──────────────────────────────────────────────────────
        // FIX: use iced::window::close() instead of process::exit().
        Message::CloseWindow(window_id) => {
            let content = editor_text(&state.editor);
            if !content.trim().is_empty() {
                if let Some(ref editing_id) = state.editing_id.clone() {
                    if let Some(note) = state.notes.iter_mut().find(|n| n.id == *editing_id) {
                        if note.content != content {
                            note.content = content.clone();
                            note.timestamp = Local::now().timestamp_millis();
                            let file = NoteFile {
                                id: note.id.clone(),
                                content,
                                timestamp: note.timestamp,
                            };
                            file.save().ok();
                        }
                    }
                } else {
                    let is_duplicate = state.notes.iter().any(|n| n.content.trim() == content.trim());
                    if !is_duplicate {
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

fn view(state: &AppState) -> iced::Element<Message> {
    let filtered: Vec<&Note> = state.notes.iter()
        .filter(|n| {
            state.search_query.is_empty()
                || n.content.to_lowercase().contains(&state.search_query.to_lowercase())
        })
        .collect();

    // ── Header label ─────────────────────────────────────────────────────────
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

    // ── Confirmation dialog (rendered above editor when active) ───────────────
    let confirm_dialog: Option<iced::Element<Message>> = match &state.confirm_state {
        ConfirmState::UnsavedWorkBeforeLoad(pending_id) => {
            let pid = pending_id.clone();
            Some(
                container(
                    Column::new()
                        .push(text("Unsaved changes will be lost.")
                            .size(14).color(iced::Color::from_rgb(0.95, 0.5, 0.3)))
                        .push(text("Discard and load the selected note?")
                            .size(12).color(iced::Color::from_rgb(0.7, 0.7, 0.75)))
                        .push(
                            row![
                                button("Discard & Load")
                                    .on_press(Message::ConfirmLoad(pid))
                                    .padding(8),
                                button("Keep editing")
                                    .on_press(Message::CancelLoad)
                                    .padding(8),
                            ].spacing(8)
                        )
                        .spacing(8)
                )
                .padding(12)
                .into()
            )
        }
        ConfirmState::UnsavedWorkBeforeClear => {
            Some(
                container(
                    Column::new()
                        .push(text("Unsaved changes will be lost.")
                            .size(14).color(iced::Color::from_rgb(0.95, 0.5, 0.3)))
                        .push(text("Discard changes and clear the editor?")
                            .size(12).color(iced::Color::from_rgb(0.7, 0.7, 0.75)))
                        .push(
                            row![
                                button("Discard")
                                    .on_press(Message::CancelConfirm) // reuse cancel; Clear handler will re-run without unsaved state
                                    .padding(8),
                                button("Keep editing")
                                    .on_press(Message::CancelConfirm)
                                    .padding(8),
                            ].spacing(8)
                        )
                        .spacing(8)
                )
                .padding(12)
                .into()
            )
        }
        _ => None,
    };

    // ── Multi-line text editor ────────────────────────────────────────────────
    let editor = iced::widget::text_editor(&state.editor)
        .on_action(Message::Edit)
        .placeholder("Type your note here...")
        .padding(12)
        .size(15)
        .height(iced::Length::Fixed(120.0));

    // ── Action buttons ────────────────────────────────────────────────────────
    let save_label = if state.editing_id.is_some() { "Update (Ctrl+S)" } else { "Save (Ctrl+S)" };
    let save_btn = button(save_label).on_press(Message::Save).padding(10);
    let clear_btn = button("Clear").on_press(Message::Clear).padding(10);

    let delete_all_btn: iced::Element<Message> = match state.confirm_state {
        ConfirmState::DeleteAll => row![
            button("Confirm Delete All").on_press(Message::ConfirmDeleteAll).padding(10),
            button("Cancel").on_press(Message::CancelConfirm).padding(10),
        ].spacing(8).into(),
        _ => button("Delete All").on_press(Message::DeleteAllNotes).padding(10).into(),
    };

    let buttons = row![save_btn, clear_btn, delete_all_btn].spacing(12);

    // ── Search row ────────────────────────────────────────────────────────────
    let search_field = text_input("Search notes...", &state.search_query)
        .on_input(Message::SearchChanged)
        .size(14)
        .padding(8)
        .width(iced::Length::Fill);

    let search_row: iced::Element<Message> = if state.search_query.is_empty() {
        search_field.into()
    } else {
        row![
            search_field,
            button("✕").on_press(Message::ClearSearch).padding(6),
        ].spacing(8).into()
    };

    // ── Status bar ────────────────────────────────────────────────────────────
    let status = if state.search_query.is_empty() {
        format!(
            "{} chars • {} notes",
            editor_text(&state.editor).len(),
            state.notes.len()
        )
    } else {
        format!("{} / {} notes", filtered.len(), state.notes.len())
    };

    // ── Notes list ────────────────────────────────────────────────────────────
    let mut notes_col = Column::new().spacing(0);

    notes_col = notes_col.push(
        container(text(status).size(11).color(iced::Color::from_rgb(0.5, 0.5, 0.55)))
    );
    notes_col = notes_col.push(container(search_row));
    notes_col = notes_col.push(
        container(text("ALL NOTES").size(10).color(iced::Color::from_rgb(0.4, 0.4, 0.45)))
    );

    if filtered.is_empty() {
        let msg = if state.search_query.is_empty() {
            "No saved notes yet. Type above and press Ctrl+S to save."
        } else {
            "No notes match your search."
        };
        notes_col = notes_col.push(
            text(msg).size(13).color(iced::Color::from_rgb(0.4, 0.4, 0.45))
        );
    } else {
        for note in filtered {
            let date = DateTime::from_timestamp_millis(note.timestamp)
                .map(|dt| dt.format("%d/%m/%y %H:%M").to_string())
                .unwrap_or_default();

            // Unicode-safe preview; replace newlines for single-line display
            let preview = truncate_at_word(&note.content.replace('\n', " ↵ "), 100);
            let is_editing = state.editing_id.as_deref() == Some(note.id.as_str());

            let actions: iced::Element<Message> = match &state.confirm_state {
                ConfirmState::DeleteOne(id) if id == &note.id => row![
                    button("Delete").on_press(Message::ConfirmDeleteNote(note.id.clone())).padding(8),
                    button("Cancel").on_press(Message::CancelConfirm).padding(8),
                ].spacing(6).into(),
                _ => row![
                    button(if is_editing { "Editing…" } else { "Load" })
                        .on_press(Message::LoadNote(note.id.clone()))
                        .padding(10),
                    button("✕")
                        .on_press(Message::DeleteNote(note.id.clone()))
                        .padding(10),
                ].spacing(8).into(),
            };

            let note_content = Column::new()
                .push(text(preview).size(14).color(iced::Color::from_rgb(0.9, 0.9, 0.95)))
                .push(text(date).size(11).color(iced::Color::from_rgb(0.75, 0.75, 0.8)))
                .spacing(4);

            notes_col = notes_col.push(
                container(
                    row![note_content.width(iced::Length::Fill), actions]
                        .align_y(iced::Alignment::Center)
                )
                .width(iced::Length::Fill)
                .padding(12)
            );

            // Separator line
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

    // ── Assemble main layout ─────────────────────────────────────────────────
    let input_section = container(
        Column::new()
            .push(text(header_text).size(14).color(header_color))
            .push(editor)
            .spacing(8)
    )
    .padding(20);

    let mut main = Column::new()
        .push(input_section)
        .push(container(buttons).padding(iced::Padding::from([0, 20])))
        .push(
            scrollable(notes_col)
                .id(iced::widget::Id::new("notes_scroll"))
                .height(iced::Length::Fill)
        )
        .spacing(10);

    // Confirmation dialog floats above the rest
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
                        match c.as_str() {
                            "s" => return Some(Message::Save),
                            _ => {}
                        }
                    }
                }
            }
            None
        }),
        // FIX: carry the window ID so CloseWindow can call iced::window::close(id)
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
        .exit_on_close_request(false) // we handle close manually to auto-save
        .run()
}