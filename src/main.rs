use chrono::{DateTime, Local};
use iced::widget::{
    button, container, mouse_area, row, rule, scrollable, stack, text, text_editor, text_input,
    Column,
};
use iced::widget::operation::focus_next;
use iced::{keyboard, window, Color, Element, Length, Subscription, Task, Theme};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use uuid::Uuid;

// ─── Constants & Configuration ───────────────────────────────────────────────

static NOTES_DIR: Lazy<PathBuf> = Lazy::new(|| {
    let dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".quick-notes")
        .join("notes");
    let _ = fs::create_dir_all(&dir);
    dir
});

const SEARCH_ID: &str = "search_input";
const EDITOR_ID: &str = "main_editor";
const AUTO_SAVE_DELAY_MS: u64 = 2000;
const MAX_PREVIEW_CHARS: usize = 120;

const COLORS: Colors = Colors {
    primary: Color::from_rgb(0.35, 0.9, 0.55),
    warning: Color::from_rgb(0.95, 0.6, 0.3),
    danger: Color::from_rgb(0.95, 0.4, 0.3),
    muted: Color::from_rgb(0.5, 0.5, 0.55),
    bg_card: Color::from_rgb(0.12, 0.12, 0.15),
    bg_card_active: Color::from_rgb(0.18, 0.22, 0.28),
    text_primary: Color::from_rgb(0.9, 0.9, 0.95),
    text_secondary: Color::from_rgb(0.75, 0.75, 0.8),
};

#[derive(Clone, Copy)]
struct Colors {
    primary: Color,
    warning: Color,
    danger: Color,
    muted: Color,
    bg_card: Color,
    bg_card_active: Color,
    text_primary: Color,
    text_secondary: Color,
}

// ─── Domain Models ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NoteData {
    id: String,
    title: String,
    content: String,
    timestamp: i64,
    pinned: bool,
    #[serde(default)]
    tags: Vec<String>,
}

impl NoteData {
    fn new(content: String) -> Self {
        let title = Self::extract_title(&content);
        Self {
            id: Uuid::new_v4().to_string(),
            title,
            content,
            timestamp: Local::now().timestamp_millis(),
            pinned: false,
            tags: Vec::new(),
        }
    }

    /// Derives the note title from the first non-empty line of content.
    fn extract_title(content: &str) -> String {
        content
            .lines()
            .next()
            .map(|line| line.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Untitled".to_string())
    }

    fn save_to_disk(&self) -> Result<(), AppError> {
        let path = NOTES_DIR.join(format!("{}.json", self.id));
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    fn delete_from_disk(id: &str) -> Result<(), std::io::Error> {
        let _ = fs::remove_file(NOTES_DIR.join(format!("{}.json", id)));
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct Note {
    id: String,
    title: String,
    content: String,
    timestamp: i64,
    pinned: bool,
    tags: Vec<String>,
}

impl Note {
    /// Applies new content and refreshes title + timestamp.
    /// Content is expected to already be normalised (trim_end applied).
    fn apply_content_update(&mut self, new_content: String) {
        self.title = NoteData::extract_title(&new_content);
        self.content = new_content;
        self.timestamp = Local::now().timestamp_millis();
    }
}

impl From<NoteData> for Note {
    fn from(data: NoteData) -> Self {
        Self {
            id: data.id,
            title: data.title,
            content: data.content,
            timestamp: data.timestamp,
            pinned: data.pinned,
            tags: data.tags,
        }
    }
}

// ─── Error Handling ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum AppError {
    IoError(String),
    SerdeError(String),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppError::IoError(s) => write!(f, "{s}"),
            AppError::SerdeError(s) => write!(f, "{s}"),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::IoError(e.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::SerdeError(e.to_string())
    }
}

type AppResult<T> = Result<T, AppError>;

// ─── Application State ───────────────────────────────────────────────────────

/// Which confirmation dialog is currently shown, if any.
/// The note id for delete confirmations is stored inside the variant
/// so there is a single source of truth (no separate pending_delete_id).
#[derive(Debug, Clone, PartialEq)]
enum ConfirmAction {
    None,
    DiscardChanges,
    /// Holds the id of the note pending deletion.
    DeleteNote(String),
    DeleteAll,
}

#[derive(Debug, Clone)]
enum Toast {
    Saved,
    Deleted,
    Error(String),
}

struct AppState {
    notes: Vec<Note>,

    editor: text_editor::Content,
    search_query: String,
    editing_id: Option<String>,

    confirm_action: ConfirmAction,
    /// Set when the user clicks a note while the editor has unsaved changes;
    /// the load is deferred until the confirm dialog resolves.
    pending_load_id: Option<String>,

    toast: Option<(Toast, Instant)>,
    auto_save_deadline: Option<Instant>,
    is_dirty: bool,
    sort_by_pinned: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            // Load all notes, tolerating per-file errors (fix #5).
            notes: load_all_notes_tolerant(),
            editor: text_editor::Content::new(),
            search_query: String::new(),
            editing_id: None,

            confirm_action: ConfirmAction::None,
            pending_load_id: None,

            toast: None,
            auto_save_deadline: None,
            is_dirty: false,
            sort_by_pinned: true,
        }
    }
}

// ─── Messages ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
enum Message {
    Edit(text_editor::Action),

    SaveNote,
    ClearEditor,

    LoadNote(String),
    ConfirmLoad,
    CancelAction,

    DeleteNote(String),
    ConfirmDelete,
    DeleteAllNotes,
    ConfirmDeleteAll,

    TogglePin(String),
    SearchChanged(String),
    ClearSearch,
    ToggleSort,

    CloseWindow(window::Id),
    /// Fired at ~2 Hz for autosave and toast dismissal (fix #4).
    Tick,

    FocusEditor,
    FocusSearch,
}

// ─── Persistence Layer ───────────────────────────────────────────────────────

/// Loads all notes, skipping files that fail to parse rather than aborting (fix #5).
fn load_all_notes_tolerant() -> Vec<Note> {
    let entries = match fs::read_dir(&*NOTES_DIR) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut notes = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        let result: AppResult<Note> = (|| {
            let raw = fs::read_to_string(&path)?;
            let data: NoteData = serde_json::from_str(&raw)?;
            Ok(Note::from(data))
        })();

        match result {
            Ok(note) if !note.content.trim().is_empty() => notes.push(note),
            Ok(_) => {} // skip blank notes
            Err(e) => eprintln!("Skipping {:?}: {e}", path.file_name().unwrap_or_default()),
        }
    }

    sort_notes(true, &mut notes);
    notes
}

fn save_note_to_disk(note: &Note) -> AppResult<()> {
    // Construct NoteData directly from &Note to avoid a clone (fix #13).
    let data = NoteData {
        id: note.id.clone(),
        title: note.title.clone(),
        content: note.content.clone(),
        timestamp: note.timestamp,
        pinned: note.pinned,
        tags: note.tags.clone(),
    };
    data.save_to_disk()
}

/// Sorts the notes slice in-place.
/// When `pinned_first` is true, pinned notes float to the top within recency order.
fn sort_notes(pinned_first: bool, notes: &mut [Note]) {
    notes.sort_by(|a, b| {
        if pinned_first {
            match (a.pinned, b.pinned) {
                (true, false) => return std::cmp::Ordering::Less,
                (false, true) => return std::cmp::Ordering::Greater,
                _ => {}
            }
        }
        b.timestamp.cmp(&a.timestamp)
    });
}

// ─── Utility Functions ───────────────────────────────────────────────────────

fn truncate_preview(s: &str, max_chars: usize) -> String {
    let cleaned = s.replace('\n', " ").replace('\r', "");
    if cleaned.chars().count() <= max_chars {
        return cleaned;
    }

    let byte_idx = cleaned
        .char_indices()
        .nth(max_chars)
        .map(|(i, _)| i)
        .unwrap_or(cleaned.len());

    let truncated = &cleaned[..byte_idx];
    truncated
        .rfind(' ')
        .map(|sp| format!("{}…", &truncated[..sp]))
        .unwrap_or_else(|| format!("{}…", truncated))
}

fn format_timestamp(ts: i64) -> String {
    DateTime::from_timestamp_millis(ts)
        .map(|dt| dt.format("%d/%m %H:%M").to_string())
        .unwrap_or_default()
}

/// Returns the editor content normalised: trailing whitespace stripped (fix #6).
fn editor_text(content: &text_editor::Content) -> String {
    content.text().trim_end().to_string()
}

// ─── Update Logic ────────────────────────────────────────────────────────────

fn update(state: &mut AppState, message: Message) -> Task<Message> {
    match message {
        Message::Edit(action) => {
            state.editor.perform(action);
            state.is_dirty = true;
            state.auto_save_deadline =
                Some(Instant::now() + Duration::from_millis(AUTO_SAVE_DELAY_MS));
            Task::none()
        }

        Message::SaveNote => {
            // Single normalisation point — used for all comparisons below (fix #6).
            let content = editor_text(&state.editor);

            if content.is_empty() {
                state.toast =
                    Some((Toast::Error("Cannot save empty note".into()), Instant::now()));
                return Task::none();
            }

            if let Some(ref id) = state.editing_id.clone() {
                if let Some(note) = state.notes.iter_mut().find(|n| n.id == *id) {
                    if note.content != content {
                        note.apply_content_update(content);
                        if let Err(e) = save_note_to_disk(note) {
                            state.toast =
                                Some((Toast::Error(format!("Save failed: {e}")), Instant::now()));
                            return Task::none();
                        }
                        sort_notes(state.sort_by_pinned, &mut state.notes);
                    }
                }
            } else {
                // New note: dropped the "duplicate content" guard (fix #7).
                // Silently deduplicating identically-worded notes is surprising UX.
                let note = Note::from(NoteData::new(content));
                if let Err(e) = save_note_to_disk(&note) {
                    state.toast =
                        Some((Toast::Error(format!("Save failed: {e}")), Instant::now()));
                    return Task::none();
                }
                state.notes.insert(0, note);
                sort_notes(state.sort_by_pinned, &mut state.notes);
            }

            state.is_dirty = false;
            state.auto_save_deadline = None;
            state.editor = text_editor::Content::new();
            state.editing_id = None;
            state.toast = Some((Toast::Saved, Instant::now()));
            Task::none()
        }

        Message::ClearEditor => {
            let current = editor_text(&state.editor);
            if state.is_dirty && !current.is_empty() {
                state.confirm_action = ConfirmAction::DiscardChanges;
                state.pending_load_id = None;
            } else {
                state.editor = text_editor::Content::new();
                state.editing_id = None;
                state.is_dirty = false;
                state.auto_save_deadline = None;
            }
            Task::none()
        }

        Message::LoadNote(id) => {
            let current = editor_text(&state.editor);
            if state.is_dirty && !current.is_empty() {
                state.confirm_action = ConfirmAction::DiscardChanges;
                state.pending_load_id = Some(id);
                return Task::none();
            }
            load_note_into_editor(state, &id)
        }

        Message::ConfirmLoad => {
            state.confirm_action = ConfirmAction::None;
            if let Some(id) = state.pending_load_id.take() {
                load_note_into_editor(state, &id)
            } else {
                Task::none()
            }
        }

        Message::CancelAction => {
            state.confirm_action = ConfirmAction::None;
            state.pending_load_id = None;
            Task::none()
        }

        Message::DeleteNote(id) => {
            // Id lives only inside the ConfirmAction variant (fix #1).
            state.confirm_action = ConfirmAction::DeleteNote(id);
            Task::none()
        }

        Message::ConfirmDelete => {
            // Extract the id from the variant itself — no separate field (fix #1).
            if let ConfirmAction::DeleteNote(id) = state.confirm_action.clone() {
                let _ = NoteData::delete_from_disk(&id);
                state.notes.retain(|n| n.id != id);

                if state.editing_id.as_deref() == Some(&id) {
                    state.editor = text_editor::Content::new();
                    state.editing_id = None;
                    state.is_dirty = false;
                    state.auto_save_deadline = None;
                }

                state.confirm_action = ConfirmAction::None;
                state.toast = Some((Toast::Deleted, Instant::now()));
            }
            Task::none()
        }

        Message::DeleteAllNotes => {
            state.confirm_action = ConfirmAction::DeleteAll;
            Task::none()
        }

        Message::ConfirmDeleteAll => {
            for note in &state.notes {
                let _ = NoteData::delete_from_disk(&note.id);
            }
            state.notes.clear();
            state.editor = text_editor::Content::new();
            state.editing_id = None;
            state.is_dirty = false;
            state.auto_save_deadline = None;
            state.confirm_action = ConfirmAction::None;
            state.toast = Some((Toast::Deleted, Instant::now()));
            Task::none()
        }

        Message::TogglePin(id) => {
            if let Some(note) = state.notes.iter_mut().find(|n| n.id == id) {
                note.pinned = !note.pinned;
                let _ = save_note_to_disk(note);
                sort_notes(state.sort_by_pinned, &mut state.notes);
            }
            Task::none()
        }

        Message::SearchChanged(query) => {
            // No focus() call — the input already has focus while the user types (fix #3).
            state.search_query = query;
            Task::none()
        }

        Message::ClearSearch => {
            state.search_query.clear();
            // Focus the search input after clearing so the user can keep typing.
            focus_next()
        }

        Message::ToggleSort => {
            state.sort_by_pinned = !state.sort_by_pinned;
            sort_notes(state.sort_by_pinned, &mut state.notes);
            Task::none()
        }

        Message::CloseWindow(window_id) => window::close(window_id),

        Message::Tick => {
            // Dismiss toast after 3 s.
            if let Some((_, created)) = &state.toast {
                if created.elapsed() >= Duration::from_secs(3) {
                    state.toast = None;
                }
            }

            // Trigger autosave once the deadline has passed (fix #4: runs at 2 Hz, not 60 fps).
            if state.is_dirty {
                if let Some(deadline) = state.auto_save_deadline {
                    if Instant::now() >= deadline {
                        let current = editor_text(&state.editor);
                        if !current.is_empty() {
                            return Task::done(Message::SaveNote);
                        }
                    }
                }
            }

            Task::none()
        }

        Message::FocusEditor => focus_next(),
        Message::FocusSearch => focus_next(),
    }
}

fn load_note_into_editor(state: &mut AppState, id: &str) -> Task<Message> {
    if let Some(note) = state.notes.iter().find(|n| n.id == id) {
        state.editor = text_editor::Content::with_text(&note.content);
        state.editing_id = Some(note.id.clone());
        state.is_dirty = false;
        state.auto_save_deadline = None;
    }
    // Use iced's built-in focus traversal rather than a raw id string (fix #3).
    focus_next()
}

// ─── View Layer ──────────────────────────────────────────────────────────────

fn view(state: &AppState) -> Element<'_, Message> {
    let header = build_header(state);
    let editor_section = build_editor_section(state);
    let action_buttons = build_action_buttons();
    let search_bar = build_search_bar(state);
    let notes_list = build_notes_list(state);
    let delete_all = build_delete_all_zone(&state.confirm_action);

    let mut main_col = Column::new()
        .push(header)
        .push(editor_section)
        .push(action_buttons)
        .push(rule::horizontal(1))
        .push(search_bar)
        .push(notes_list)
        .push(delete_all)
        .spacing(8)
        .padding(16);

    if let Some(dialog) = build_confirm_dialog(&state.confirm_action) {
        main_col = main_col.push(dialog);
    }

    let base: Element<'_, Message> = container(main_col)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_| iced::widget::container::Style {
            background: Some(Color::from_rgb(0.08, 0.08, 0.11).into()),
            ..Default::default()
        })
        .into();

    // Toast is overlaid via stack so it never shifts content (fix #15).
    if let Some((toast, _)) = state.toast.as_ref() {
        stack![base, build_toast(toast)].into()
    } else {
        base
    }
}

fn build_header(state: &AppState) -> Element<'_, Message> {
    let (title, subtitle, accent) = if state.editing_id.is_some() {
        if state.is_dirty {
            ("✎ Editing", "Unsaved changes", COLORS.warning)
        } else {
            ("✎ Editing", "All changes saved", COLORS.primary)
        }
    } else {
        ("＋ New Note", "Start typing to create", COLORS.primary)
    };

    row![
        text(title).size(18).color(accent),
        text(subtitle).size(12).color(COLORS.muted),
        if state.sort_by_pinned {
            button("📌 Pinned")
                .on_press(Message::ToggleSort)
                .padding([4, 10])
        } else {
            button("🕐 Recent")
                .on_press(Message::ToggleSort)
                .padding([4, 10])
        },
    ]
    .spacing(12)
    .align_y(iced::Alignment::Center)
    .into()
}

fn build_editor_section(state: &AppState) -> Element<'_, Message> {
    let placeholder = if state.editing_id.is_some() {
        "Edit your note... (Ctrl+S to save)"
    } else {
        "Type your note... (Ctrl+S to save)"
    };

    container(
        text_editor(&state.editor)
            .id(iced::widget::Id::new(EDITOR_ID))
            .placeholder(placeholder)
            .on_action(Message::Edit)
            .height(Length::Fixed(140.0))
            .size(15),
    )
    .padding(16)
    .style(|_| iced::widget::container::Style {
        background: Some(Color::from_rgb(0.14, 0.14, 0.18).into()),
        ..Default::default()
    })
    .into()
}

fn build_action_buttons() -> Element<'static, Message> {
    row![
        button("💾 Save")
            .on_press(Message::SaveNote)
            .padding([8, 20])
            .style(|_, _| iced::widget::button::Style {
                background: Some(COLORS.primary.into()),
                ..Default::default()
            }),
        button("🗑 Clear")
            .on_press(Message::ClearEditor)
            .padding([8, 20]),
    ]
    .spacing(10)
    .into()
}

fn build_search_bar(state: &AppState) -> Element<'_, Message> {
    let search_input = text_input("🔍 Search notes...", &state.search_query)
        .id(iced::widget::Id::new(SEARCH_ID))
        .on_input(Message::SearchChanged)
        .padding([8, 12])
        .size(14)
        .width(Length::Fill);

    if !state.search_query.is_empty() {
        row![
            search_input,
            button("✕").on_press(Message::ClearSearch).padding(6),
        ]
        .spacing(8)
        .align_y(iced::Alignment::Center)
        .into()
    } else {
        search_input.into()
    }
}

fn build_notes_list(state: &AppState) -> Element<'_, Message> {
    let q = state.search_query.to_lowercase();
    let is_filtering = !q.is_empty();

    let mut visible_count = 0usize;

    let mut list = Column::new().push(
        container(
            text(if is_filtering { "🔍 Results" } else { "📋 Notes" })
                .size(11)
                .color(COLORS.muted),
        )
        .padding([8, 0]),
    );

    for note in &state.notes {
        let matches = !is_filtering
            || note.title.to_lowercase().contains(&q)
            || note.content.to_lowercase().contains(&q)
            || note.tags.iter().any(|t| t.to_lowercase().contains(&q));

        if !matches {
            continue;
        }

        visible_count += 1;

        let is_editing = state.editing_id.as_deref() == Some(&note.id);
        let bg = if is_editing {
            COLORS.bg_card_active
        } else {
            COLORS.bg_card
        };

        let preview = truncate_preview(&note.content, MAX_PREVIEW_CHARS);
        let timestamp = format_timestamp(note.timestamp);

        let title_row: Element<'_, Message> = if note.pinned {
            row![
                text("📌").size(12),
                text(&note.title).size(14).color(COLORS.text_primary),
            ]
            .spacing(4)
            .into()
        } else {
            text(&note.title)
                .size(14)
                .color(COLORS.text_primary)
                .into()
        };

        let note_content = Column::new()
            .push(title_row)
            .push(text(preview).size(13).color(COLORS.text_secondary))
            .push(text(timestamp).size(10).color(COLORS.muted))
            .spacing(4);

        let actions = row![
            button(if note.pinned { "📍" } else { "📌" })
                .on_press(Message::TogglePin(note.id.clone()))
                .padding(6),
            build_delete_button(&state.confirm_action, &note.id),
        ]
        .spacing(4);

        let card = container(
            row![note_content.width(Length::Fill), actions]
                .align_y(iced::Alignment::Center),
        )
        .padding(14)
        .width(Length::Fill)
        .style(move |_| iced::widget::container::Style {
            background: Some(bg.into()),
            ..Default::default()
        });

        list = list.push(mouse_area(card).on_press(Message::LoadNote(note.id.clone())));
    }

    if visible_count == 0 {
        let msg = if is_filtering {
            "No notes match your search."
        } else {
            "No notes yet. Create your first note above! ✨"
        };

        list = list.push(
            container(text(msg).color(COLORS.muted))
                .padding(20)
                .center_x(Length::Fill)
                .width(Length::Fill),
        );
    }

    scrollable(list).height(Length::Fill).into()
}

/// Renders either a delete button or an inline confirm/cancel pair for that note (fix #14 inline).
fn build_delete_button<'a>(
    confirm_state: &'a ConfirmAction,
    note_id: &'a str,
) -> Element<'a, Message> {
    match confirm_state {
        ConfirmAction::DeleteNote(id) if id == note_id => row![
            button("✓")
                .on_press(Message::ConfirmDelete)
                .padding([4, 10]),
            button("✕")
                .on_press(Message::CancelAction)
                .padding([4, 10]),
        ]
        .spacing(4)
        .into(),
        _ => button("🗑")
            .on_press(Message::DeleteNote(note_id.to_string()))
            .padding(6)
            .into(),
    }
}

/// Builds the modal confirmation dialog. Only DiscardChanges is handled here;
/// DeleteNote confirmations are inline per-card, and DeleteAll is in build_delete_all_zone
/// to avoid duplicating the same dialog (fix #2).
fn build_confirm_dialog(action: &ConfirmAction) -> Option<Element<'_, Message>> {
    match action {
        ConfirmAction::DiscardChanges => Some(build_dialog(
            "Unsaved Changes",
            "Discard changes and continue?",
            row![
                button("Discard")
                    .on_press(Message::ConfirmLoad)
                    .padding([8, 16]),
                button("Keep Editing")
                    .on_press(Message::CancelAction)
                    .padding([8, 16]),
            ]
            .spacing(10)
            .into(),
            COLORS.warning,
        )),
        _ => None,
    }
}

fn build_dialog<'a>(
    title: &'a str,
    message: &'a str,
    buttons: Element<'a, Message>,
    accent: Color,
) -> Element<'a, Message> {
    container(
        Column::new()
            .push(text(title).size(16).color(accent))
            .push(text(message).size(13).color(COLORS.text_secondary))
            .push(buttons)
            .spacing(12),
    )
    .padding(20)
    .style(|_| iced::widget::container::Style {
        background: Some(Color::from_rgb(0.14, 0.14, 0.18).into()),
        ..Default::default()
    })
    .center_x(Length::Fill)
    .width(Length::Fixed(400.0))
    .into()
}

/// Renders the "Delete All" button or its inline confirm/cancel row.
/// This is the single owner of DeleteAll state (fix #2).
fn build_delete_all_zone(confirm_state: &ConfirmAction) -> Element<'_, Message> {
    match confirm_state {
        ConfirmAction::DeleteAll => container(
            row![
                text("⚠️ Delete ALL notes?").color(COLORS.danger),
                button("Yes")
                    .on_press(Message::ConfirmDeleteAll)
                    .padding([6, 14]),
                button("No")
                    .on_press(Message::CancelAction)
                    .padding([6, 14]),
            ]
            .spacing(10)
            .align_y(iced::Alignment::Center),
        )
        .padding([12, 20])
        .into(),
        _ => container(
            button("🗑 Delete All Notes")
                .on_press(Message::DeleteAllNotes)
                .padding([6, 14]),
        )
        .padding([12, 20])
        .into(),
    }
}

/// Toast overlaid via stack — does not shift surrounding content (fix #15).
fn build_toast(toast: &Toast) -> Element<'_, Message> {
    let (message, color) = match toast {
        Toast::Saved => ("✓ Saved", COLORS.primary),
        Toast::Deleted => ("✓ Deleted", COLORS.primary),
        Toast::Error(msg) => (msg.as_str(), COLORS.danger),
    };

    container(
        container(text(message).color(Color::WHITE))
            .padding([10, 20])
            .style(move |_| iced::widget::container::Style {
                background: Some(color.into()),
                ..Default::default()
            }),
    )
    .width(Length::Fill)
    .align_y(iced::Alignment::End)
    .padding(iced::Padding { top: 0.0, right: 0.0, bottom: 20.0, left: 0.0 })
    .into()
}

// ─── Subscriptions ───────────────────────────────────────────────────────────

fn subscription(_state: &AppState) -> Subscription<Message> {
    use keyboard::{key::Named, Key};

    Subscription::batch([
        keyboard::listen().filter_map(|event| {
            if let keyboard::Event::KeyPressed { key, modifiers, .. } = event {
                if modifiers.contains(keyboard::Modifiers::CTRL) {
                    match &key {
                        Key::Character(c) if c.as_str() == "s" => return Some(Message::SaveNote),
                        Key::Character(c) if c.as_str() == "f" => {
                            return Some(Message::FocusSearch)
                        }
                        _ => {}
                    }
                }
                if key == Key::Named(Named::Escape) {
                    return Some(Message::CancelAction);
                }
            }
            None
        }),

        window::events().filter_map(|(id, event)| {
            if let window::Event::CloseRequested = event {
                Some(Message::CloseWindow(id))
            } else {
                None
            }
        }),

        // Tick driven by window frames; the deadline guard in update() prevents
        // autosave from firing more than once per AUTO_SAVE_DELAY_MS interval.
        window::frames().map(|_| Message::Tick),
    ])
}

// ─── Entry Point ─────────────────────────────────────────────────────────────

fn main() -> iced::Result {
    iced::application(AppState::default, update, view)
        .subscription(subscription)
        .theme(Theme::Dark)
        .title("Quick Notes ✦")
        .run()
}