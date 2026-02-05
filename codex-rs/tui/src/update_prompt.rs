#![cfg(not(debug_assertions))]

use crate::history_cell::padded_emoji;
use crate::key_hint;
use crate::render::Insets;
use crate::render::renderable::ColumnRenderable;
use crate::render::renderable::Renderable;
use crate::render::renderable::RenderableExt as _;
use crate::selection_list::selection_option_row;
use crate::tui::FrameRequester;
use crate::tui::Tui;
use crate::tui::TuiEvent;
use crate::update_action::UpdateAction;
use crate::updates;
use codex_core::config::Config;
use color_eyre::Result;
use crossterm::event::KeyCode;
use crossterm::event::KeyEvent;
use crossterm::event::KeyEventKind;
use crossterm::event::KeyModifiers;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::prelude::Widget;
use ratatui::style::Stylize as _;
use ratatui::text::Line;
use ratatui::widgets::Clear;
use ratatui::widgets::WidgetRef;
use tokio_stream::StreamExt;

pub(crate) enum UpdatePromptOutcome {
    Continue,
    RunUpdate(UpdateAction),
}

pub(crate) async fn run_update_prompt_if_needed(
    tui: &mut Tui,
    config: &Config,
) -> Result<UpdatePromptOutcome> {
    let Some(latest_version) = updates::get_upgrade_version_for_popup(config) else {
        return Ok(UpdatePromptOutcome::Continue);
    };
    let update_actions = crate::update_action::get_update_actions();
    if update_actions.is_empty() {
        return Ok(UpdatePromptOutcome::Continue);
    };

    let mut screen = UpdatePromptScreen::new(
        tui.frame_requester(),
        latest_version.clone(),
        update_actions,
    );
    tui.draw(u16::MAX, |frame| {
        frame.render_widget_ref(&screen, frame.area());
    })?;

    let events = tui.event_stream();
    tokio::pin!(events);

    while !screen.is_done() {
        if let Some(event) = events.next().await {
            match event {
                TuiEvent::Key(key_event) => screen.handle_key(key_event),
                TuiEvent::Paste(_) => {}
                TuiEvent::Draw => {
                    tui.draw(u16::MAX, |frame| {
                        frame.render_widget_ref(&screen, frame.area());
                    })?;
                }
            }
        } else {
            break;
        }
    }

    match screen.selection() {
        Some(UpdateSelection::UpdateNow(action)) => {
            tui.terminal.clear()?;
            Ok(UpdatePromptOutcome::RunUpdate(action))
        }
        Some(UpdateSelection::NotNow) | None => Ok(UpdatePromptOutcome::Continue),
        Some(UpdateSelection::DontRemind) => {
            if let Err(err) = updates::dismiss_version(config, screen.latest_version()).await {
                tracing::error!("Failed to persist update dismissal: {err}");
            }
            Ok(UpdatePromptOutcome::Continue)
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateSelection {
    UpdateNow(UpdateAction),
    NotNow,
    DontRemind,
}

struct UpdatePromptScreen {
    request_frame: FrameRequester,
    latest_version: String,
    current_version: String,
    options: Vec<UpdateSelection>,
    highlighted_idx: usize,
    selection: Option<UpdateSelection>,
}

impl UpdatePromptScreen {
    fn new(
        request_frame: FrameRequester,
        latest_version: String,
        update_actions: Vec<UpdateAction>,
    ) -> Self {
        let mut options: Vec<UpdateSelection> = update_actions
            .into_iter()
            .map(UpdateSelection::UpdateNow)
            .collect();
        options.push(UpdateSelection::NotNow);
        options.push(UpdateSelection::DontRemind);

        Self {
            request_frame,
            latest_version,
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            options,
            highlighted_idx: 0,
            selection: None,
        }
    }

    fn handle_key(&mut self, key_event: KeyEvent) {
        if key_event.kind == KeyEventKind::Release {
            return;
        }
        if key_event.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key_event.code, KeyCode::Char('c') | KeyCode::Char('d'))
        {
            self.select(UpdateSelection::NotNow);
            return;
        }
        match key_event.code {
            KeyCode::Up | KeyCode::Char('k') => self.highlight_prev(),
            KeyCode::Down | KeyCode::Char('j') => self.highlight_next(),
            KeyCode::Char(ch) if ch.is_ascii_digit() => self.select_by_number(ch),
            KeyCode::Enter => self.select(self.options[self.highlighted_idx]),
            KeyCode::Esc => self.select(UpdateSelection::NotNow),
            _ => {}
        }
    }

    fn highlight_prev(&mut self) {
        let new_idx = if self.highlighted_idx == 0 {
            self.options.len().saturating_sub(1)
        } else {
            self.highlighted_idx - 1
        };
        self.set_highlight(new_idx);
    }

    fn highlight_next(&mut self) {
        let new_idx = if self.options.is_empty() {
            0
        } else {
            (self.highlighted_idx + 1) % self.options.len()
        };
        self.set_highlight(new_idx);
    }

    fn set_highlight(&mut self, idx: usize) {
        if self.highlighted_idx != idx {
            self.highlighted_idx = idx;
            self.request_frame.schedule_frame();
        }
    }

    fn select_by_number(&mut self, ch: char) {
        let Some(digit) = ch.to_digit(10) else {
            return;
        };
        if digit == 0 {
            return;
        }
        let idx = (digit - 1) as usize;
        if let Some(selection) = self.options.get(idx).copied() {
            self.select(selection);
        }
    }

    fn select(&mut self, selection: UpdateSelection) {
        if let Some(idx) = self.options.iter().position(|opt| *opt == selection) {
            self.highlighted_idx = idx;
        }
        self.selection = Some(selection);
        self.request_frame.schedule_frame();
    }

    fn is_done(&self) -> bool {
        self.selection.is_some()
    }

    fn selection(&self) -> Option<UpdateSelection> {
        self.selection
    }

    fn latest_version(&self) -> &str {
        self.latest_version.as_str()
    }
}

impl UpdateSelection {
    fn label(self) -> String {
        match self {
            UpdateSelection::UpdateNow(action) => {
                format!("Update now (runs `{}`)", action.command_str())
            }
            UpdateSelection::NotNow => "Skip".to_string(),
            UpdateSelection::DontRemind => "Skip until next version".to_string(),
        }
    }
}

impl WidgetRef for &UpdatePromptScreen {
    fn render_ref(&self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);
        let mut column = ColumnRenderable::new();

        column.push("");
        column.push(Line::from(vec![
            padded_emoji("  ✨").bold().cyan(),
            "Update available!".bold(),
            " ".into(),
            format!(
                "{current} -> {latest}",
                current = self.current_version,
                latest = self.latest_version
            )
            .dim(),
        ]));
        column.push("");
        column.push(
            Line::from(vec![
                "Release notes: ".dim(),
                "https://github.com/wenwen12345/codex/releases/latest"
                    .dim()
                    .underlined(),
            ])
            .inset(Insets::tlbr(0, 2, 0, 0)),
        );
        column.push("");
        for (idx, opt) in self.options.iter().copied().enumerate() {
            column.push(selection_option_row(
                idx,
                opt.label(),
                self.highlighted_idx == idx,
            ));
        }
        column.push("");
        column.push(
            Line::from(vec![
                "Press ".dim(),
                key_hint::plain(KeyCode::Enter).into(),
                " to continue".dim(),
            ])
            .inset(Insets::tlbr(0, 2, 0, 0)),
        );
        column.render(area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_backend::VT100Backend;
    use crate::tui::FrameRequester;
    use crossterm::event::KeyCode;
    use crossterm::event::KeyEvent;
    use crossterm::event::KeyModifiers;
    use ratatui::Terminal;

    fn new_prompt() -> UpdatePromptScreen {
        UpdatePromptScreen::new(
            FrameRequester::test_dummy(),
            "9.9.9".into(),
            vec![UpdateAction::NpmGlobalLatest],
        )
    }

    #[test]
    fn update_prompt_snapshot() {
        let screen = new_prompt();
        let mut terminal = Terminal::new(VT100Backend::new(80, 12)).expect("terminal");
        terminal
            .draw(|frame| frame.render_widget_ref(&screen, frame.area()))
            .expect("render update prompt");
        insta::assert_snapshot!("update_prompt_modal", terminal.backend());
    }

    #[test]
    fn update_prompt_confirm_selects_update() {
        let mut screen = new_prompt();
        screen.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(screen.is_done());
        assert_eq!(screen.selection(), Some(UpdateSelection::UpdateNow));
    }

    #[test]
    fn update_prompt_dismiss_option_leaves_prompt_in_normal_state() {
        let mut screen = new_prompt();
        screen.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        screen.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(screen.is_done());
        assert_eq!(screen.selection(), Some(UpdateSelection::NotNow));
    }

    #[test]
    fn update_prompt_dont_remind_selects_dismissal() {
        let mut screen = new_prompt();
        screen.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        screen.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        screen.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(screen.is_done());
        assert_eq!(screen.selection(), Some(UpdateSelection::DontRemind));
    }

    #[test]
    fn update_prompt_ctrl_c_skips_update() {
        let mut screen = new_prompt();
        screen.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(screen.is_done());
        assert_eq!(screen.selection(), Some(UpdateSelection::NotNow));
    }

    #[test]
    fn update_prompt_navigation_wraps_between_entries() {
        let mut screen = new_prompt();
        screen.handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(screen.highlighted_idx, 2);
        screen.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(screen.highlighted_idx, 0);
    }

    #[test]
    fn update_prompt_supports_multiple_update_actions() {
        let mut screen = UpdatePromptScreen::new(
            FrameRequester::test_dummy(),
            "9.9.9".into(),
            vec![
                UpdateAction::NpmGlobalLatest,
                UpdateAction::PnpmGlobalLatest,
            ],
        );
        screen.handle_key(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        assert!(screen.is_done());
        assert_eq!(
            screen.selection(),
            Some(UpdateSelection::UpdateNow(UpdateAction::PnpmGlobalLatest))
        );
    }
}
