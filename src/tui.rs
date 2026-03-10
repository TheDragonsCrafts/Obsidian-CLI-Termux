use std::fs;
use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::Result;
use chrono::Local;
use crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::prelude::{Alignment, Color, Frame, Line, Modifier, Position, Span, Style, Text};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::app::App;
use crate::parser::{Request, parse_line};
use crate::registry::{COMMANDS, CommandSpec, SupportLevel};

const POLL_INTERVAL_MS: u64 = 120;
const MAX_RUNS: usize = 24;
const COMMON_TOKENS: &[&str] = &[
    "vault=",
    "file=",
    "path=",
    "name=",
    "content=",
    "query=",
    "format=json",
    "format=tsv",
    "format=csv",
    "total",
    "verbose",
    "--copy",
];

pub fn run(app: &mut App) -> Result<()> {
    let history_file = app.workspace.runtime.history_file.clone();
    let history = load_history(&history_file);
    let mut state = DashboardState::new(history);

    let mut terminal = setup_terminal()?;
    let result = run_loop(app, &mut terminal, &mut state);
    let _ = save_history(&history_file, &state.history);
    restore_terminal(&mut terminal)?;
    result
}

fn run_loop(
    app: &mut App,
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &mut DashboardState,
) -> Result<()> {
    loop {
        let commands = state.filtered_commands();
        state.clamp_selection(commands.len());
        terminal.draw(|frame| draw(frame, app, state, &commands))?;

        if !event::poll(Duration::from_millis(POLL_INTERVAL_MS))? {
            continue;
        }

        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                if handle_key(app, state, key)? {
                    break;
                }
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollDown => state.scroll_output(3),
                MouseEventKind::ScrollUp => state.scroll_output(-3),
                _ => {}
            },
            Event::Resize(_, _) => {}
            _ => {}
        }
    }

    Ok(())
}

struct DashboardState {
    input: String,
    cursor: usize,
    selected_command: usize,
    output_scroll: u16,
    history: Vec<String>,
    history_index: Option<usize>,
    runs: Vec<RunRecord>,
    status: StatusLine,
}

struct RunRecord {
    timestamp: String,
    command: String,
    ok: bool,
    output: String,
}

struct StatusLine {
    level: StatusLevel,
    message: String,
}

#[derive(Clone, Copy)]
enum StatusLevel {
    Info,
    Success,
    Error,
}

impl DashboardState {
    fn new(history: Vec<String>) -> Self {
        Self {
            input: String::new(),
            cursor: 0,
            selected_command: 0,
            output_scroll: 0,
            history,
            history_index: None,
            runs: Vec::new(),
            status: StatusLine {
                level: StatusLevel::Info,
                message: "Type a command, use Tab to insert, Enter to run, q to exit.".to_string(),
            },
        }
    }

    fn filtered_commands(&self) -> Vec<&'static CommandSpec> {
        let token = self.current_token();
        if token.is_empty() {
            return COMMANDS.iter().collect();
        }

        let prefix = first_token(&self.input);
        let prefix = if prefix.contains('=') { token } else { prefix };
        let prefix_lower = prefix.to_ascii_lowercase();
        let mut commands = COMMANDS
            .iter()
            .filter(|spec| {
                spec.name.starts_with(prefix)
                    || spec.name.to_ascii_lowercase().contains(&prefix_lower)
                    || spec.summary.to_ascii_lowercase().contains(&prefix_lower)
            })
            .collect::<Vec<_>>();

        if commands.is_empty() {
            commands = COMMANDS.iter().collect();
        }

        commands
    }

    fn clamp_selection(&mut self, len: usize) {
        if len == 0 {
            self.selected_command = 0;
            return;
        }
        if self.selected_command >= len {
            self.selected_command = len.saturating_sub(1);
        }
    }

    fn move_selection(&mut self, delta: isize, len: usize) {
        if len == 0 {
            self.selected_command = 0;
            return;
        }
        let current = self.selected_command as isize;
        let next = (current + delta).clamp(0, len.saturating_sub(1) as isize);
        self.selected_command = next as usize;
    }

    fn current_token(&self) -> &str {
        let (start, end) = token_bounds(&self.input, self.cursor);
        &self.input[start..end]
    }

    fn push_run(&mut self, command: String, ok: bool, output: String) {
        self.runs.insert(
            0,
            RunRecord {
                timestamp: Local::now().format("%H:%M:%S").to_string(),
                command,
                ok,
                output,
            },
        );
        self.runs.truncate(MAX_RUNS);
        self.output_scroll = 0;
        self.history_index = None;
    }

    fn last_output(&self) -> (&str, bool, &str) {
        if let Some(record) = self.runs.first() {
            (&record.output, record.ok, &record.command)
        } else {
            (
                "No commands executed yet.\n\nUse the left panel to browse commands or type directly in the command bar.",
                true,
                "idle",
            )
        }
    }

    fn set_status(&mut self, level: StatusLevel, message: impl Into<String>) {
        self.status = StatusLine {
            level,
            message: message.into(),
        };
    }

    fn clear_input(&mut self) {
        self.input.clear();
        self.cursor = 0;
        self.history_index = None;
    }

    fn scroll_output(&mut self, delta: i16) {
        if delta.is_negative() {
            self.output_scroll = self.output_scroll.saturating_sub(delta.unsigned_abs());
        } else {
            self.output_scroll = self.output_scroll.saturating_add(delta as u16);
        }
    }
}

fn handle_key(app: &mut App, state: &mut DashboardState, key: KeyEvent) -> Result<bool> {
    match key.code {
        KeyCode::Char('q') if key.modifiers.is_empty() && state.input.is_empty() => {
            return Ok(true);
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.runs.clear();
            state.set_status(StatusLevel::Info, "Output cleared.");
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.clear_input();
        }
        KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            history_prev(state);
        }
        KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            history_next(state);
        }
        KeyCode::PageDown => state.scroll_output(10),
        KeyCode::PageUp => state.scroll_output(-10),
        KeyCode::Up => {
            let commands = state.filtered_commands();
            state.move_selection(-1, commands.len());
        }
        KeyCode::Down => {
            let commands = state.filtered_commands();
            state.move_selection(1, commands.len());
        }
        KeyCode::Left => move_cursor_left(state),
        KeyCode::Right => move_cursor_right(state),
        KeyCode::Home => state.cursor = 0,
        KeyCode::End => state.cursor = state.input.len(),
        KeyCode::Backspace => backspace(state),
        KeyCode::Delete => delete(state),
        KeyCode::Esc => {
            if state.input.is_empty() {
                state.set_status(StatusLevel::Info, "Input already empty.");
            } else {
                state.clear_input();
                state.set_status(StatusLevel::Info, "Input cleared.");
            }
        }
        KeyCode::Tab => insert_selected_suggestion(app, state),
        KeyCode::Enter => submit_or_fill(app, state)?,
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            insert_char(state, ch)
        }
        _ => {}
    }

    Ok(false)
}

fn draw(
    frame: &mut Frame<'_>,
    app: &App,
    state: &DashboardState,
    commands: &[&'static CommandSpec],
) {
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(color_bg())),
        area,
    );

    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(12),
            Constraint::Length(6),
        ])
        .split(area);

    draw_header(frame, app, state, vertical[0]);

    if area.width >= 120 {
        let main = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(32), Constraint::Percentage(68)])
            .split(vertical[1]);
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(10), Constraint::Length(9)])
            .split(main[1]);
        draw_command_browser(frame, state, commands, main[0]);
        draw_output(frame, state, right[0]);
        draw_runs(frame, state, right[1]);
    } else {
        let stacked = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(14),
                Constraint::Min(8),
                Constraint::Length(8),
            ])
            .split(vertical[1]);
        draw_command_browser(frame, state, commands, stacked[0]);
        draw_output(frame, state, stacked[1]);
        draw_runs(frame, state, stacked[2]);
    }

    draw_input(frame, app, state, vertical[2]);
}

fn draw_header(frame: &mut Frame<'_>, app: &App, state: &DashboardState, area: Rect) {
    let vault_name = app
        .workspace
        .resolve_vault(None)
        .ok()
        .map(|vault| vault.name)
        .unwrap_or_else(|| "no-vault".to_string());
    let active_file = app
        .workspace
        .state
        .active_file
        .clone()
        .unwrap_or_else(|| "none".to_string());

    let title = Line::from(vec![
        Span::styled(
            "Obsidian",
            Style::default()
                .fg(color_text())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" // ", Style::default().fg(color_muted())),
        Span::styled(
            "Termux TUI",
            Style::default()
                .fg(color_accent())
                .add_modifier(Modifier::BOLD),
        ),
    ]);

    let meta = Line::from(vec![
        badge("vault", color_accent_2()),
        Span::raw(" "),
        Span::styled(vault_name, Style::default().fg(color_text())),
        Span::raw("   "),
        badge("active", color_local()),
        Span::raw(" "),
        Span::styled(active_file, Style::default().fg(color_text())),
    ]);

    let status = Line::from(vec![
        badge(
            match state.status.level {
                StatusLevel::Info => "info",
                StatusLevel::Success => "ok",
                StatusLevel::Error => "error",
            },
            status_color(state.status.level),
        ),
        Span::raw(" "),
        Span::styled(
            state.status.message.as_str(),
            Style::default().fg(color_text()),
        ),
    ]);

    let widget = Paragraph::new(Text::from(vec![title, meta, status]))
        .block(panel_block("Session", color_panel_border()))
        .alignment(Alignment::Left);
    frame.render_widget(widget, area);
}

fn draw_command_browser(
    frame: &mut Frame<'_>,
    state: &DashboardState,
    commands: &[&'static CommandSpec],
    area: Rect,
) {
    let items = commands
        .iter()
        .map(|spec| {
            let support = match spec.support {
                SupportLevel::Local => "local",
                SupportLevel::Hybrid => "hybrid",
                SupportLevel::BridgeOnly => "bridge",
            };
            ListItem::new(vec![
                Line::from(vec![
                    Span::styled(
                        spec.name,
                        Style::default()
                            .fg(color_text())
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::styled(
                        support,
                        Style::default().fg(status_color_from_support(spec.support)),
                    ),
                ]),
                Line::from(Span::styled(
                    format!("{} / {}", spec.category, spec.summary),
                    Style::default().fg(color_muted()),
                )),
            ])
        })
        .collect::<Vec<_>>();

    let mut list_state = ListState::default();
    if !commands.is_empty() {
        list_state.select(Some(state.selected_command));
    }

    let list = List::new(items)
        .block(panel_block("Command Browser", color_accent()))
        .highlight_style(
            Style::default()
                .bg(color_highlight())
                .fg(color_text())
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol(">> ");
    frame.render_stateful_widget(list, area, &mut list_state);
}

fn draw_output(frame: &mut Frame<'_>, state: &DashboardState, area: Rect) {
    let (output, ok, command) = state.last_output();
    let title = if ok {
        format!("Last Output  [{}]", command)
    } else {
        format!("Last Error  [{}]", command)
    };
    let paragraph = Paragraph::new(output)
        .block(panel_block(
            &title,
            if ok { color_local() } else { color_error() },
        ))
        .style(Style::default().fg(color_text()).bg(color_surface()))
        .wrap(Wrap { trim: false })
        .scroll((state.output_scroll, 0));
    frame.render_widget(paragraph, area);
}

fn draw_runs(frame: &mut Frame<'_>, state: &DashboardState, area: Rect) {
    let items = if state.runs.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "No runs yet.",
            Style::default().fg(color_muted()),
        )))]
    } else {
        state
            .runs
            .iter()
            .take(6)
            .map(|run| {
                let preview = first_non_empty_line(&run.output);
                ListItem::new(vec![
                    Line::from(vec![
                        Span::styled(
                            if run.ok { "OK " } else { "ERR" },
                            Style::default()
                                .fg(if run.ok { color_local() } else { color_error() })
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(" "),
                        Span::styled(run.timestamp.as_str(), Style::default().fg(color_muted())),
                        Span::raw("  "),
                        Span::styled(run.command.as_str(), Style::default().fg(color_text())),
                    ]),
                    Line::from(Span::styled(preview, Style::default().fg(color_muted()))),
                ])
            })
            .collect::<Vec<_>>()
    };

    let list = List::new(items).block(panel_block("Recent Runs", color_accent_2()));
    frame.render_widget(list, area);
}

fn draw_input(frame: &mut Frame<'_>, app: &App, state: &DashboardState, area: Rect) {
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(1),
            Constraint::Length(2),
        ])
        .split(area);

    let input = Paragraph::new(state.input.as_str())
        .block(panel_block("Command Bar", color_accent()))
        .style(Style::default().fg(color_text()).bg(color_surface()));
    frame.render_widget(input, sections[0]);

    let suggestions = suggestion_tokens(app, state, &state.filtered_commands());
    let suggestions_line = if suggestions.is_empty() {
        Line::from(Span::styled(
            "Suggestions: type a command or press Up/Down to browse.",
            Style::default().fg(color_muted()),
        ))
    } else {
        let mut spans = vec![Span::styled(
            "Suggestions ",
            Style::default()
                .fg(color_muted())
                .add_modifier(Modifier::BOLD),
        )];
        for (index, suggestion) in suggestions.iter().take(5).enumerate() {
            if index > 0 {
                spans.push(Span::raw("  "));
            }
            spans.push(Span::styled(
                suggestion.clone(),
                Style::default().fg(color_accent_2()).bg(color_highlight()),
            ));
        }
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(suggestions_line), sections[1]);

    let shortcuts = Line::from(vec![
        Span::styled(
            "Enter",
            Style::default()
                .fg(color_text())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" run  ", Style::default().fg(color_muted())),
        Span::styled(
            "Tab",
            Style::default()
                .fg(color_text())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" insert  ", Style::default().fg(color_muted())),
        Span::styled(
            "Ctrl+P/N",
            Style::default()
                .fg(color_text())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" history  ", Style::default().fg(color_muted())),
        Span::styled(
            "PgUp/PgDn",
            Style::default()
                .fg(color_text())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" scroll  ", Style::default().fg(color_muted())),
        Span::styled(
            "q",
            Style::default()
                .fg(color_text())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" quit", Style::default().fg(color_muted())),
    ]);
    frame.render_widget(Paragraph::new(shortcuts), sections[2]);

    let cursor_col = visible_width(&state.input[..state.cursor]) as u16 + 1;
    let cursor = Position::new(sections[0].x + cursor_col, sections[0].y + 1);
    frame.set_cursor_position(cursor);
}

fn insert_selected_suggestion(app: &App, state: &mut DashboardState) {
    let commands = state.filtered_commands();
    let suggestion = if state.input.trim().is_empty() || !state.input.contains(' ') {
        commands
            .get(state.selected_command)
            .map(|spec| spec.name.to_string())
    } else {
        suggestion_tokens(app, state, &commands).into_iter().next()
    };

    let Some(suggestion) = suggestion else {
        return;
    };

    replace_current_token(
        &mut state.input,
        &mut state.cursor,
        &suggestion,
        !suggestion.ends_with('='),
    );
    state.set_status(StatusLevel::Info, format!("Inserted `{suggestion}`."));
}

fn submit_or_fill(app: &mut App, state: &mut DashboardState) -> Result<()> {
    if state.input.trim().is_empty() {
        let commands = state.filtered_commands();
        if let Some(command) = commands.get(state.selected_command) {
            state.input = format!("{} ", command.name);
            state.cursor = state.input.len();
            state.set_status(StatusLevel::Info, format!("Inserted `{}`.", command.name));
        }
        return Ok(());
    }

    let command = state.input.trim().to_string();
    let execution = match parse_line(&command)? {
        Request::Interactive => Ok(String::new()),
        Request::Invocation(invocation) => app.execute(invocation),
    };

    if !command.is_empty() && state.history.last() != Some(&command) {
        state.history.push(command.clone());
    }

    match execution {
        Ok(output) => {
            let rendered = if output.trim().is_empty() {
                "(no output)".to_string()
            } else {
                output
            };
            state.push_run(command.clone(), true, rendered);
            state.set_status(StatusLevel::Success, format!("Executed `{command}`."));
            state.clear_input();
        }
        Err(error) => {
            let message = format!("{error:#}");
            state.push_run(command.clone(), false, message.clone());
            state.set_status(StatusLevel::Error, format!("Command failed: `{command}`."));
            state.clear_input();
        }
    }

    Ok(())
}

fn insert_char(state: &mut DashboardState, ch: char) {
    state.input.insert(state.cursor, ch);
    state.cursor += ch.len_utf8();
}

fn backspace(state: &mut DashboardState) {
    if state.cursor == 0 {
        return;
    }
    let previous = prev_boundary(&state.input, state.cursor);
    state.input.drain(previous..state.cursor);
    state.cursor = previous;
}

fn delete(state: &mut DashboardState) {
    if state.cursor >= state.input.len() {
        return;
    }
    let next = next_boundary(&state.input, state.cursor);
    state.input.drain(state.cursor..next);
}

fn move_cursor_left(state: &mut DashboardState) {
    if state.cursor == 0 {
        return;
    }
    state.cursor = prev_boundary(&state.input, state.cursor);
}

fn move_cursor_right(state: &mut DashboardState) {
    if state.cursor >= state.input.len() {
        return;
    }
    state.cursor = next_boundary(&state.input, state.cursor);
}

fn history_prev(state: &mut DashboardState) {
    if state.history.is_empty() {
        return;
    }
    let next = match state.history_index {
        Some(index) if index > 0 => index - 1,
        Some(index) => index,
        None => state.history.len().saturating_sub(1),
    };
    state.history_index = Some(next);
    state.input = state.history[next].clone();
    state.cursor = state.input.len();
}

fn history_next(state: &mut DashboardState) {
    let Some(index) = state.history_index else {
        return;
    };
    let next = index + 1;
    if next >= state.history.len() {
        state.history_index = None;
        state.clear_input();
        return;
    }
    state.history_index = Some(next);
    state.input = state.history[next].clone();
    state.cursor = state.input.len();
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn load_history(path: &std::path::Path) -> Vec<String> {
    fs::read_to_string(path)
        .map(|content| {
            content
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn save_history(path: &std::path::Path, history: &[String]) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let body = history.join("\n");
    fs::write(path, body)?;
    Ok(())
}

fn suggestion_tokens(
    app: &App,
    state: &DashboardState,
    commands: &[&'static CommandSpec],
) -> Vec<String> {
    let token = state.current_token();
    if token.starts_with("vault=") {
        let selector = token.trim_start_matches("vault=");
        let mut vaults = app
            .workspace
            .known_vaults
            .iter()
            .map(|vault| format!("vault={}", vault.name))
            .filter(|value| value.starts_with(&format!("vault={selector}")))
            .collect::<Vec<_>>();
        vaults.sort();
        return vaults;
    }

    if state.input.trim().is_empty() || !state.input.contains(' ') {
        return commands
            .iter()
            .take(6)
            .map(|spec| spec.name.to_string())
            .collect();
    }

    COMMON_TOKENS
        .iter()
        .filter(|token_candidate| token.is_empty() || token_candidate.starts_with(token))
        .map(|token_candidate| (*token_candidate).to_string())
        .collect()
}

fn replace_current_token(
    input: &mut String,
    cursor: &mut usize,
    replacement: &str,
    add_space: bool,
) {
    let (start, end) = token_bounds(input, *cursor);
    let mut next = String::new();
    next.push_str(&input[..start]);
    next.push_str(replacement);
    if add_space && !replacement.ends_with(' ') {
        next.push(' ');
    }
    next.push_str(&input[end..]);
    *cursor = start + replacement.len() + usize::from(add_space && !replacement.ends_with(' '));
    *input = next;
}

fn token_bounds(input: &str, cursor: usize) -> (usize, usize) {
    let start = input[..cursor]
        .rfind(char::is_whitespace)
        .map(|index| index + 1)
        .unwrap_or(0);
    let end = input[cursor..]
        .find(char::is_whitespace)
        .map(|index| cursor + index)
        .unwrap_or(input.len());
    (start, end)
}

fn first_token(input: &str) -> &str {
    input.split_whitespace().next().unwrap_or_default()
}

fn prev_boundary(input: &str, cursor: usize) -> usize {
    input[..cursor]
        .char_indices()
        .last()
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn next_boundary(input: &str, cursor: usize) -> usize {
    input[cursor..]
        .chars()
        .next()
        .map(|ch| cursor + ch.len_utf8())
        .unwrap_or(cursor)
}

fn first_non_empty_line(output: &str) -> String {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("(empty)")
        .to_string()
}

fn visible_width(value: &str) -> usize {
    value.chars().count()
}

fn badge<'a>(label: &'a str, color: Color) -> Span<'a> {
    Span::styled(
        format!(" {label} "),
        Style::default()
            .fg(color_bg())
            .bg(color)
            .add_modifier(Modifier::BOLD),
    )
}

fn panel_block(title: &str, color: Color) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(color))
        .style(Style::default().bg(color_surface()))
        .title(Line::from(vec![
            Span::styled(" ", Style::default().bg(color_surface())),
            Span::styled(
                title,
                Style::default()
                    .fg(color_text())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", Style::default().bg(color_surface())),
        ]))
}

fn status_color(level: StatusLevel) -> Color {
    match level {
        StatusLevel::Info => color_accent(),
        StatusLevel::Success => color_local(),
        StatusLevel::Error => color_error(),
    }
}

fn status_color_from_support(support: SupportLevel) -> Color {
    match support {
        SupportLevel::Local => color_local(),
        SupportLevel::Hybrid => color_accent_2(),
        SupportLevel::BridgeOnly => color_error(),
    }
}

fn color_bg() -> Color {
    Color::Rgb(11, 15, 20)
}

fn color_surface() -> Color {
    Color::Rgb(17, 23, 31)
}

fn color_highlight() -> Color {
    Color::Rgb(28, 38, 49)
}

fn color_panel_border() -> Color {
    Color::Rgb(64, 94, 123)
}

fn color_text() -> Color {
    Color::Rgb(237, 244, 251)
}

fn color_muted() -> Color {
    Color::Rgb(132, 150, 170)
}

fn color_accent() -> Color {
    Color::Rgb(89, 198, 223)
}

fn color_accent_2() -> Color {
    Color::Rgb(255, 185, 92)
}

fn color_local() -> Color {
    Color::Rgb(114, 212, 156)
}

fn color_error() -> Color {
    Color::Rgb(255, 109, 122)
}

#[cfg(test)]
mod tests {
    use super::{replace_current_token, token_bounds};

    #[test]
    fn token_bounds_find_current_word() {
        let input = "append file=Inbox content=test";
        assert_eq!(token_bounds(input, 9), (7, 17));
        assert_eq!(token_bounds(input, input.len()), (18, input.len()));
    }

    #[test]
    fn replace_current_token_inserts_spacing() {
        let mut input = "rea".to_string();
        let mut cursor = input.len();
        replace_current_token(&mut input, &mut cursor, "read", true);
        assert_eq!(input, "read ");
        assert_eq!(cursor, 5);
    }
}
