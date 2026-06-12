use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use dirs::home_dir;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Cell, HighlightSpacing, Padding, Paragraph, Row, Scrollbar,
    ScrollbarOrientation, ScrollbarState, Table, TableState, Wrap,
};

use crate::model::{Agent, Inventory, LinkStatus, Skill, link_path};
use crate::ops;

const ACCENT: Color = Color::Indexed(110);
const BADGE_TEXT: Color = Color::Indexed(234);
const GREEN: Color = Color::Indexed(114);
const YELLOW: Color = Color::Indexed(179);
const RED: Color = Color::Indexed(174);
const BRIGHT: Color = Color::Indexed(252);
const MUTED: Color = Color::Indexed(245);
const FAINT: Color = Color::Indexed(238);
const SELECTION: Color = Color::Indexed(237);

const HINTS: [(&str, &str); 8] = [
    ("j/k", "move"),
    ("1", "claude"),
    ("2", "codex"),
    ("3", "cursor"),
    ("a", "all"),
    ("f", "fix"),
    ("r", "reload"),
    ("q", "quit"),
];

pub fn run(sources: Vec<PathBuf>) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::load(sources);
    let result = run_loop(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

#[derive(Clone, Copy)]
enum Feedback {
    Info,
    Success,
    Warn,
    Error,
}

/// One visible table row: either a group heading or an index into
/// `Inventory::skills`. Group headings are not selectable.
enum RowEntry {
    Group(String),
    Skill(usize),
}

/// Skills arrive sorted by (group, name); emit a heading row whenever a
/// non-empty group starts. Top-level skills sort first and get no heading.
fn build_rows(inventory: &Inventory) -> Vec<RowEntry> {
    let mut rows = Vec::new();
    let mut current_group = None;
    for (index, skill) in inventory.skills.iter().enumerate() {
        if !skill.group.is_empty() && current_group != Some(skill.group.as_str()) {
            rows.push(RowEntry::Group(skill.group.clone()));
        }
        current_group = Some(skill.group.as_str());
        rows.push(RowEntry::Skill(index));
    }
    rows
}

/// Nearest selectable (skill) row at or after `from`, else the nearest before.
fn nearest_skill_row(rows: &[RowEntry], from: usize) -> Option<usize> {
    if rows.is_empty() {
        return None;
    }
    let from = from.min(rows.len() - 1);
    rows[from..]
        .iter()
        .position(|row| matches!(row, RowEntry::Skill(_)))
        .map(|offset| from + offset)
        .or_else(|| {
            rows[..from]
                .iter()
                .rposition(|row| matches!(row, RowEntry::Skill(_)))
        })
}

struct App {
    sources: Vec<PathBuf>,
    inventory: Inventory,
    rows: Vec<RowEntry>,
    table_state: TableState,
    message: String,
    feedback: Feedback,
}

impl App {
    fn load(sources: Vec<PathBuf>) -> Self {
        let inventory = Inventory::load(&sources);
        let rows = build_rows(&inventory);
        let mut table_state = TableState::default();
        table_state.select(nearest_skill_row(&rows, 0));
        let mut app = Self {
            sources,
            inventory,
            rows,
            table_state,
            message: String::new(),
            feedback: Feedback::Info,
        };
        app.announce_load();
        app
    }

    fn announce_load(&mut self) {
        let count = self.inventory.skills.len();
        let noun = if count == 1 { "skill" } else { "skills" };
        if self.inventory.warnings.is_empty() {
            self.set_message(Feedback::Info, format!("{count} {noun} loaded"));
        } else {
            self.set_message(
                Feedback::Warn,
                format!(
                    "{count} {noun} loaded · {} warning(s) · {}",
                    self.inventory.warnings.len(),
                    self.inventory.warnings[0]
                ),
            );
        }
    }

    fn set_message(&mut self, feedback: Feedback, message: impl Into<String>) {
        let mut message = message.into();
        if let Some(home) = home_dir() {
            message = message.replace(&home.display().to_string(), "~");
        }
        self.message = message;
        self.feedback = feedback;
    }

    fn reload(&mut self) {
        let selected_path = self.selected_skill().map(|skill| skill.path.clone());
        let previous_row = self.table_state.selected().unwrap_or(0);
        self.inventory = Inventory::load(&self.sources);
        self.rows = build_rows(&self.inventory);
        let selection = selected_path
            .and_then(|path| {
                self.rows.iter().position(|row| {
                    matches!(row, RowEntry::Skill(index) if self.inventory.skills[*index].path == path)
                })
            })
            .or_else(|| nearest_skill_row(&self.rows, previous_row));
        self.table_state.select(selection);
    }

    fn selected_skill(&self) -> Option<&Skill> {
        match self.rows.get(self.table_state.selected()?)? {
            RowEntry::Skill(index) => self.inventory.skills.get(*index),
            RowEntry::Group(_) => None,
        }
    }

    fn move_down(&mut self) {
        let selected = self.table_state.selected().unwrap_or(0);
        let next = self.rows[selected.saturating_add(1).min(self.rows.len())..]
            .iter()
            .position(|row| matches!(row, RowEntry::Skill(_)))
            .map(|offset| selected + 1 + offset);
        if let Some(next) = next {
            self.table_state.select(Some(next));
        }
    }

    fn move_up(&mut self) {
        let selected = self.table_state.selected().unwrap_or(0);
        let previous = self.rows[..selected.min(self.rows.len())]
            .iter()
            .rposition(|row| matches!(row, RowEntry::Skill(_)));
        if let Some(previous) = previous {
            self.table_state.select(Some(previous));
        }
    }

    fn toggle(&mut self, agent: Agent) {
        let Some(skill) = self.selected_skill().cloned() else {
            self.set_message(Feedback::Warn, "no skill selected");
            return;
        };

        match ops::toggle_skill(&skill, agent) {
            Ok(result) => self.set_message(
                Feedback::Success,
                format!("{} {}: {}", agent.label(), skill.name, result.message),
            ),
            Err(error) => self.set_message(
                Feedback::Error,
                format!("{} {}: {error:#}", agent.label(), skill.name),
            ),
        }
        self.reload();
    }

    fn toggle_all(&mut self) {
        let Some(skill) = self.selected_skill().cloned() else {
            self.set_message(Feedback::Warn, "no skill selected");
            return;
        };

        let statuses: Vec<(Agent, LinkStatus)> = Agent::ALL
            .iter()
            .map(|agent| (*agent, self.inventory.status(&skill, *agent)))
            .collect();
        let any_missing = statuses
            .iter()
            .any(|(_, status)| *status == LinkStatus::Missing);

        let mut messages = Vec::new();
        let mut failed = false;
        for (agent, status) in statuses {
            let result = match (status, any_missing) {
                (LinkStatus::Missing, true) => ops::link_skill(&skill, agent),
                (LinkStatus::Linked, false) => ops::unlink_skill(&skill, agent),
                _ => continue,
            };
            match result {
                Ok(result) => messages.push(format!("{}: {}", agent.label(), result.message)),
                Err(error) => {
                    failed = true;
                    messages.push(format!("{}: {error:#}", agent.label()));
                }
            }
        }

        if messages.is_empty() {
            self.set_message(
                Feedback::Info,
                format!(
                    "{}: nothing to toggle (wrong/blocked links need f)",
                    skill.name
                ),
            );
        } else {
            let feedback = if failed {
                Feedback::Error
            } else {
                Feedback::Success
            };
            self.set_message(feedback, messages.join(" · "));
        }
        self.reload();
    }

    fn fix_wrong(&mut self) {
        let Some(skill) = self.selected_skill().cloned() else {
            self.set_message(Feedback::Warn, "no skill selected");
            return;
        };

        let mut messages = Vec::new();
        let mut failed = false;
        for agent in Agent::ALL {
            if let LinkStatus::WrongTarget(_) = self.inventory.status(&skill, agent) {
                match ops::fix_skill(&skill, agent) {
                    Ok(result) => messages.push(format!("{}: {}", agent.label(), result.message)),
                    Err(error) => {
                        failed = true;
                        messages.push(format!("{}: {error:#}", agent.label()));
                    }
                }
            }
        }

        if messages.is_empty() {
            self.set_message(Feedback::Info, format!("{} has no wrong links", skill.name));
        } else {
            let feedback = if failed {
                Feedback::Error
            } else {
                Feedback::Success
            };
            self.set_message(feedback, messages.join(" · "));
        }
        self.reload();
    }
}

fn run_loop(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>, app: &mut App) -> Result<()> {
    loop {
        terminal.draw(|frame| draw(frame, app))?;

        if !event::poll(Duration::from_millis(250))? {
            continue;
        }

        let Event::Key(key) = event::read()? else {
            continue;
        };

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(()),
            KeyCode::Down | KeyCode::Char('j') => app.move_down(),
            KeyCode::Up | KeyCode::Char('k') => app.move_up(),
            KeyCode::Char('1') => app.toggle(Agent::Claude),
            KeyCode::Char('2') => app.toggle(Agent::Codex),
            KeyCode::Char('3') => app.toggle(Agent::Cursor),
            KeyCode::Char('a') => app.toggle_all(),
            KeyCode::Char('f') => app.fix_wrong(),
            KeyCode::Char('r') => {
                app.reload();
                app.announce_load();
            }
            _ => {}
        }
    }
}

fn draw(frame: &mut ratatui::Frame<'_>, app: &mut App) {
    let [
        header_area,
        table_area,
        detail_area,
        message_area,
        hints_area,
    ] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(5),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    draw_header(frame, header_area, app);
    draw_table(frame, table_area, app);
    draw_detail(frame, detail_area, app);
    draw_message(frame, message_area, app);
    draw_hints(frame, hints_area);
}

fn draw_header(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let sources = app
        .sources
        .iter()
        .map(|source| tilde(source))
        .collect::<Vec<_>>()
        .join(" · ");
    let left = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            " symskill ",
            Style::new()
                .bg(ACCENT)
                .fg(BADGE_TEXT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            concat!(" v", env!("CARGO_PKG_VERSION")),
            Style::new().fg(MUTED),
        ),
        Span::raw("  "),
        Span::styled(sources, Style::new().fg(MUTED)),
    ]);

    let (linked, wrong, blocked) = link_counts(&app.inventory);
    let mut right_spans = vec![Span::styled(format!("● {linked}"), Style::new().fg(GREEN))];
    if wrong > 0 {
        right_spans.push(Span::styled(
            format!("  ▲ {wrong}"),
            Style::new().fg(YELLOW),
        ));
    }
    if blocked > 0 {
        right_spans.push(Span::styled(format!("  ✗ {blocked}"), Style::new().fg(RED)));
    }
    right_spans.push(Span::raw(" "));
    let right = Line::from(right_spans);

    let [left_area, right_area] = Layout::horizontal([
        Constraint::Fill(1),
        Constraint::Length(right.width() as u16),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(left), left_area);
    frame.render_widget(Paragraph::new(right), right_area);
}

fn link_counts(inventory: &Inventory) -> (usize, usize, usize) {
    let mut linked = 0;
    let mut wrong = 0;
    let mut blocked = 0;
    for status in inventory.statuses.values() {
        match status {
            LinkStatus::Linked => linked += 1,
            LinkStatus::WrongTarget(_) => wrong += 1,
            LinkStatus::Occupied => blocked += 1,
            LinkStatus::Missing => {}
        }
    }
    (linked, wrong, blocked)
}

fn draw_table(frame: &mut ratatui::Frame<'_>, area: Rect, app: &mut App) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(FAINT))
        .padding(Padding::horizontal(1))
        .title(Line::from(vec![
            Span::styled(
                " Skills ",
                Style::new().fg(BRIGHT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("· {} ", app.inventory.skills.len()),
                Style::new().fg(MUTED),
            ),
        ]));

    if app.inventory.skills.is_empty() {
        let inner = block.inner(area);
        frame.render_widget(block, area);
        draw_empty(frame, inner, app);
        return;
    }

    let rows: Vec<Row> = app
        .rows
        .iter()
        .map(|entry| match entry {
            RowEntry::Group(group) => Row::new(vec![Cell::from(Span::styled(
                format!("{group}/"),
                Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
            ))]),
            RowEntry::Skill(index) => {
                let skill = &app.inventory.skills[*index];
                let name = if skill.group.is_empty() {
                    skill.name.clone()
                } else {
                    format!("  {}", skill.name)
                };
                Row::new(vec![
                    Cell::from(name),
                    agent_status_cell(&app.inventory, skill, Agent::Claude),
                    agent_status_cell(&app.inventory, skill, Agent::Codex),
                    agent_status_cell(&app.inventory, skill, Agent::Cursor),
                    Cell::from(Span::styled(
                        squash(&skill.description),
                        Style::new().fg(MUTED),
                    )),
                ])
            }
        })
        .collect();

    let name_width = app
        .rows
        .iter()
        .map(|entry| match entry {
            RowEntry::Group(group) => group.len() + 1,
            RowEntry::Skill(index) => {
                let skill = &app.inventory.skills[*index];
                let indent = if skill.group.is_empty() { 0 } else { 2 };
                skill.name.len() + indent
            }
        })
        .max()
        .unwrap_or(0)
        .clamp(20, 40) as u16;

    let table = Table::new(
        rows,
        [
            Constraint::Length(name_width),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Length(8),
            Constraint::Fill(1),
        ],
    )
    .header(
        Row::new(vec![
            Cell::from("NAME"),
            Cell::from(Line::from("CLAUDE").centered()),
            Cell::from(Line::from("CODEX").centered()),
            Cell::from(Line::from("CURSOR").centered()),
            Cell::from("DESCRIPTION"),
        ])
        .style(Style::new().fg(MUTED).add_modifier(Modifier::BOLD))
        .bottom_margin(1),
    )
    .block(block)
    .column_spacing(2)
    .row_highlight_style(
        Style::new()
            .bg(SELECTION)
            .fg(BRIGHT)
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(Line::from(Span::styled("▌ ", Style::new().fg(ACCENT))))
    .highlight_spacing(HighlightSpacing::Always);

    frame.render_stateful_widget(table, area, &mut app.table_state);

    // 4 = top/bottom borders + header row + header margin
    let visible = area.height.saturating_sub(4) as usize;
    if app.rows.len() > visible {
        let mut scrollbar_state =
            ScrollbarState::new(app.rows.len()).position(app.table_state.offset());
        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_style(Style::new().fg(FAINT))
                .thumb_style(Style::new().fg(MUTED)),
            area.inner(Margin {
                horizontal: 0,
                vertical: 1,
            }),
            &mut scrollbar_state,
        );
    }
}

fn draw_empty(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let sources = app
        .sources
        .iter()
        .map(|source| tilde(source))
        .collect::<Vec<_>>()
        .join(", ");
    let lines = vec![
        Line::from(Span::styled(
            "No SKILL.md files found",
            Style::new().fg(BRIGHT).add_modifier(Modifier::BOLD),
        ))
        .centered(),
        Line::from(Span::styled(
            format!("searched: {sources}"),
            Style::new().fg(MUTED),
        ))
        .centered(),
        Line::from(Span::styled(
            "run with --source <path> to use a different skills root",
            Style::new().fg(MUTED),
        ))
        .centered(),
    ];
    let [_, middle, _] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(3),
        Constraint::Fill(1),
    ])
    .areas(area);
    frame.render_widget(Paragraph::new(lines), middle);
}

fn draw_detail(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(FAINT))
        .padding(Padding::horizontal(1));

    let Some(skill) = app.selected_skill() else {
        let block = block.title(Span::styled(" Details ", Style::new().fg(MUTED)));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(Span::styled("select a skill", Style::new().fg(MUTED))),
            inner,
        );
        return;
    };

    let block = block
        .title(Span::styled(
            format!(" {} ", skill.qualified_name()),
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ))
        .title(
            Line::from(Span::styled(
                format!(" {} ", tilde(&skill.path)),
                Style::new().fg(MUTED),
            ))
            .right_aligned(),
        );

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let [description_area, links_area] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(inner);

    let description = if skill.description.is_empty() {
        Span::styled("(no description)", Style::new().fg(FAINT))
    } else {
        Span::raw(squash(&skill.description))
    };
    frame.render_widget(
        Paragraph::new(Line::from(description)).wrap(Wrap { trim: true }),
        description_area,
    );

    let mut spans = Vec::new();
    for agent in Agent::ALL {
        spans.push(Span::styled(
            format!("{} ", agent.label()),
            Style::new().fg(MUTED),
        ));
        if let Some(source) = app.inventory.inherited_via(skill, agent) {
            spans.push(Span::styled(
                format!("○ via {}", source.label()),
                Style::new().fg(GREEN),
            ));
            spans.push(Span::raw("    "));
            continue;
        }
        let status = app.inventory.status(skill, agent);
        let (symbol, word, color) = status_view(&status);
        spans.push(Span::styled(
            format!("{symbol} {word}"),
            Style::new().fg(color),
        ));
        match &status {
            LinkStatus::WrongTarget(target) => spans.push(Span::styled(
                format!(" → {}", tilde(target)),
                Style::new().fg(MUTED),
            )),
            LinkStatus::Occupied => spans.push(Span::styled(
                format!(" at {}", tilde(&link_path(skill, agent))),
                Style::new().fg(MUTED),
            )),
            _ => {}
        }
        spans.push(Span::raw("    "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), links_area);
}

fn draw_message(frame: &mut ratatui::Frame<'_>, area: Rect, app: &App) {
    if app.message.is_empty() {
        return;
    }
    let (icon, color) = match app.feedback {
        Feedback::Info => ("›", ACCENT),
        Feedback::Success => ("✓", GREEN),
        Feedback::Warn => ("▲", YELLOW),
        Feedback::Error => ("✗", RED),
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {icon} "),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled(app.message.clone(), Style::new().fg(color)),
        ])),
        area,
    );
}

fn draw_hints(frame: &mut ratatui::Frame<'_>, area: Rect) {
    let mut spans = vec![Span::raw(" ")];
    for (index, (key, label)) in HINTS.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("   "));
        }
        spans.push(Span::styled(
            *key,
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {label}"), Style::new().fg(MUTED)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn status_view(status: &LinkStatus) -> (&'static str, &'static str, Color) {
    match status {
        LinkStatus::Linked => ("●", "linked", GREEN),
        LinkStatus::Missing => ("·", "not linked", MUTED),
        LinkStatus::WrongTarget(_) => ("▲", "wrong target", YELLOW),
        LinkStatus::Occupied => ("✗", "blocked", RED),
    }
}

fn agent_status_cell(inventory: &Inventory, skill: &Skill, agent: Agent) -> Cell<'static> {
    if inventory.inherited_via(skill, agent).is_some() {
        return Cell::from(Line::from(Span::styled("○", Style::new().fg(GREEN))).centered());
    }
    let (symbol, _, color) = status_view(&inventory.status(skill, agent));
    Cell::from(
        Line::from(Span::styled(
            symbol,
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        ))
        .centered(),
    )
}

fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tilde(path: &Path) -> String {
    if let Some(home) = home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        if rest.as_os_str().is_empty() {
            return "~".to_string();
        }
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn inventory(skills: &[(&str, &str)]) -> Inventory {
        Inventory {
            skills: skills
                .iter()
                .map(|(name, group)| Skill {
                    name: name.to_string(),
                    description: String::new(),
                    path: PathBuf::from(format!("/src/{group}/{name}")),
                    group: group.to_string(),
                })
                .collect(),
            statuses: BTreeMap::new(),
            warnings: Vec::new(),
        }
    }

    fn app(inventory: Inventory) -> App {
        let rows = build_rows(&inventory);
        let mut table_state = TableState::default();
        table_state.select(nearest_skill_row(&rows, 0));
        App {
            sources: Vec::new(),
            inventory,
            rows,
            table_state,
            message: String::new(),
            feedback: Feedback::Info,
        }
    }

    #[test]
    fn build_rows_inserts_heading_when_group_starts() {
        let inventory = inventory(&[("alpha", ""), ("beta", "ops"), ("gamma", "ops")]);
        let rows = build_rows(&inventory);
        assert!(matches!(rows[0], RowEntry::Skill(0)));
        assert!(matches!(&rows[1], RowEntry::Group(group) if group == "ops"));
        assert!(matches!(rows[2], RowEntry::Skill(1)));
        assert!(matches!(rows[3], RowEntry::Skill(2)));
    }

    #[test]
    fn movement_skips_group_headings() {
        let mut app = app(inventory(&[("alpha", ""), ("beta", "ops")]));
        assert_eq!(app.table_state.selected(), Some(0));

        app.move_down();
        assert_eq!(app.table_state.selected(), Some(2), "skips the heading row");
        assert_eq!(app.selected_skill().unwrap().name, "beta");

        app.move_down();
        assert_eq!(app.table_state.selected(), Some(2), "stays at the end");

        app.move_up();
        assert_eq!(app.table_state.selected(), Some(0));
        app.move_up();
        assert_eq!(app.table_state.selected(), Some(0), "stays at the start");
    }

    #[test]
    fn initial_selection_lands_on_first_skill_row() {
        let app = app(inventory(&[("beta", "ops")]));
        assert_eq!(app.table_state.selected(), Some(1), "row 0 is the heading");
        assert_eq!(app.selected_skill().unwrap().name, "beta");
    }
}
