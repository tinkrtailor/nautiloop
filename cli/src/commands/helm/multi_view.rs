use std::collections::VecDeque;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};

use crate::api_types::LoopSummary;
use super::themes::Theme;

/// Render the multi-loop split view (FR-6).
/// Splits the area horizontally into N slots showing recent log lines per loop.
pub fn render(
    frame: &mut ratatui::Frame<'_>,
    loops: &[LoopSummary],
    all_logs: &std::collections::HashMap<uuid::Uuid, VecDeque<String>>,
    area: Rect,
    theme: &Theme,
) {
    // Pick N most recently active non-terminal loops, capped at 4
    let active_loops: Vec<&LoopSummary> = loops
        .iter()
        .filter(|l| !is_terminal(&l.state))
        .take(4)
        .collect();

    if active_loops.is_empty() {
        let p = Paragraph::new(Text::from(vec![Line::from(Span::styled(
            "No active loops to display",
            Style::default().fg(theme.muted),
        ))]))
        .block(
            Block::default()
                .title(Span::styled(
                    " multi-loop view ",
                    Style::default()
                        .fg(theme.text)
                        .add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(theme.border).bg(theme.surface))
                .style(Style::default().bg(theme.bg)),
        )
        .style(Style::default().fg(theme.text).bg(theme.bg));
        frame.render_widget(p, area);
        return;
    }

    let n = active_loops.len();
    let constraints: Vec<Constraint> = (0..n)
        .map(|_| Constraint::Ratio(1, n as u32))
        .collect();

    let slots = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    for (i, loop_item) in active_loops.iter().enumerate() {
        let spec_name = loop_item
            .spec_path
            .rsplit('/')
            .next()
            .unwrap_or(&loop_item.spec_path)
            .trim_end_matches(".md");
        let stage = loop_item.current_stage.as_deref().unwrap_or("-");
        let title = format!(" {} · {} r{} ", spec_name, stage, loop_item.round);

        let log_lines = all_logs.get(&loop_item.loop_id);
        let lines: Vec<Line<'static>> = match log_lines {
            Some(logs) if !logs.is_empty() => {
                let visible = slots[i].height.saturating_sub(2) as usize;
                logs.iter()
                    .rev()
                    .take(visible)
                    .rev()
                    .map(|line| Line::from(Span::styled(line.clone(), Style::default().fg(theme.text))))
                    .collect()
            }
            _ => vec![Line::from(Span::styled(
                "waiting for logs...",
                Style::default().fg(theme.muted),
            ))],
        };

        let p = Paragraph::new(Text::from(lines))
            .block(
                Block::default()
                    .title(Span::styled(
                        title,
                        Style::default()
                            .fg(theme.teal)
                            .add_modifier(Modifier::BOLD),
                    ))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.border).bg(theme.surface))
                    .style(Style::default().bg(theme.bg)),
            )
            .style(Style::default().fg(theme.text).bg(theme.bg))
            .wrap(Wrap { trim: false });

        frame.render_widget(p, slots[i]);
    }
}

fn is_terminal(state: &str) -> bool {
    matches!(state, "CONVERGED" | "FAILED" | "CANCELLED" | "HARDENED" | "SHIPPED")
}
