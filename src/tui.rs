use std::time::{Duration, Instant};

use epbs_sim::{Config, SlotOutcome, SlotRecord, Summary, run, summarize};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph, Sparkline};
use ratatui::{DefaultTerminal, Frame};

const WINDOW: usize = 32;
const SPEEDS: [usize; 5] = [1, 4, 16, 64, 256];

fn outcome_color(o: SlotOutcome) -> Color {
    match o {
        SlotOutcome::Full => Color::Green,
        SlotOutcome::Empty => Color::Yellow,
        SlotOutcome::Orphaned => Color::Red,
        SlotOutcome::Missed => Color::DarkGray,
    }
}

pub struct App {
    scenarios: Vec<Config>,
    selected: usize,
    seed: u64,
    slots: u64,
    records: Vec<SlotRecord>,
    /// Slots revealed so far by playback.
    shown: usize,
    cursor: usize,
    playing: bool,
    speed: usize,
    quit: bool,
}

impl App {
    pub fn new(scenarios: Vec<Config>, selected: usize, seed: u64, slots: u64) -> Self {
        let mut app = Self {
            scenarios,
            selected,
            seed,
            slots,
            records: Vec::new(),
            shown: 0,
            cursor: 0,
            playing: true,
            speed: 1,
            quit: false,
        };
        app.rerun();
        app
    }

    fn config(&self) -> Config {
        let mut cfg = self.scenarios[self.selected].clone();
        cfg.seed = self.seed;
        cfg.slots = self.slots;
        cfg
    }

    fn rerun(&mut self) {
        self.records = run(&self.config());
        self.shown = 0;
        self.cursor = 0;
        self.playing = true;
    }

    fn summary(&self) -> Summary {
        summarize(&self.records[..self.shown])
    }

    pub fn tick(&mut self) {
        if self.playing {
            self.shown = (self.shown + SPEEDS[self.speed]).min(self.records.len());
            self.cursor = self.shown.saturating_sub(1);
            if self.shown == self.records.len() {
                self.playing = false;
            }
        }
    }

    pub fn key(&mut self, key: KeyEvent) {
        let n = self.scenarios.len();
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => self.quit = true,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = (self.selected + n - 1) % n;
                self.rerun();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1) % n;
                self.rerun();
            }
            KeyCode::Left | KeyCode::Char('h') => {
                self.playing = false;
                self.cursor = self.cursor.saturating_sub(1);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.playing = false;
                self.cursor = (self.cursor + 1).min(self.shown.saturating_sub(1));
            }
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => {
                self.playing = false;
                self.shown = self.records.len();
                self.cursor = self.shown.saturating_sub(1);
            }
            KeyCode::Char(' ') => {
                if self.shown == self.records.len() {
                    self.rerun();
                } else {
                    self.playing = !self.playing;
                }
            }
            KeyCode::Char('+') | KeyCode::Char('=') => self.speed = (self.speed + 1).min(SPEEDS.len() - 1),
            KeyCode::Char('-') => self.speed = self.speed.saturating_sub(1),
            KeyCode::Char('s') => {
                self.seed += 1;
                self.rerun();
            }
            KeyCode::Char('S') => {
                self.seed = self.seed.saturating_sub(1).max(1);
                self.rerun();
            }
            KeyCode::Char('r') => self.rerun(),
            _ => {}
        }
    }

    pub fn draw(&self, f: &mut Frame) {
        let [header, body, footer] =
            Layout::vertical([Constraint::Length(1), Constraint::Min(10), Constraint::Length(1)])
                .areas(f.area());
        self.draw_header(f, header);
        let [left, right] = Layout::horizontal([Constraint::Length(26), Constraint::Min(40)]).areas(body);
        self.draw_scenarios(f, left);
        let [strip, middle, trends] =
            Layout::vertical([Constraint::Min(5), Constraint::Length(11), Constraint::Length(8)])
                .areas(right);
        self.draw_strip(f, strip);
        let [summary, detail] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(middle);
        self.draw_summary(f, summary);
        self.draw_detail(f, detail);
        self.draw_trends(f, trends);
        f.render_widget(
            Line::from(
                " ↑↓ scenario  ←→ slot  space play/pause  +/- speed  s/S seed  End skip  r rerun  q quit",
            )
            .dark_gray(),
            footer,
        );
    }

    fn draw_header(&self, f: &mut Frame, area: Rect) {
        let status = if self.playing {
            Span::styled(format!("▶ playing ×{}", SPEEDS[self.speed]), Style::new().green())
        } else {
            Span::styled("⏸ paused", Style::new().yellow())
        };
        let line = Line::from(vec![
            Span::styled(" epbs-sim ", Style::new().bold().black().on_cyan()),
            Span::raw("  "),
            Span::styled(self.scenarios[self.selected].name.clone(), Style::new().bold()),
            Span::raw(format!("   seed {}   slot {}/{}   ", self.seed, self.shown, self.records.len())),
            status,
        ]);
        f.render_widget(line, area);
    }

    fn draw_scenarios(&self, f: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self.scenarios.iter().map(|c| ListItem::new(c.name.clone())).collect();
        let list = List::new(items)
            .block(Block::bordered().title(" Scenarios "))
            .highlight_style(Style::new().bold().black().on_cyan())
            .highlight_symbol("▸ ");
        let mut state = ListState::default().with_selected(Some(self.selected));
        f.render_stateful_widget(list, area, &mut state);
    }

    fn draw_strip(&self, f: &mut Frame, area: Rect) {
        let block = Block::bordered().title(Line::from(vec![
            Span::raw(" Slots  "),
            Span::styled("■ full ", Style::new().green()),
            Span::styled("■ empty ", Style::new().yellow()),
            Span::styled("■ orphaned ", Style::new().red()),
            Span::styled("■ missed ", Style::new().dark_gray()),
        ]));
        let inner = block.inner(area);
        f.render_widget(block, area);
        let cols = (inner.width as usize / 2).max(1);
        let rows = inner.height.max(1) as usize;
        let first_row = (self.cursor / cols).saturating_sub(rows - 1);
        let lines: Vec<Line> = (first_row..first_row + rows)
            .map(|row| {
                let start = row * cols;
                let end = (start + cols).min(self.shown);
                Line::from(
                    (start..end.max(start))
                        .map(|i| {
                            let mut style = Style::new().fg(outcome_color(self.records[i].outcome));
                            if i == self.cursor {
                                style = style.add_modifier(Modifier::REVERSED);
                            }
                            Span::styled("■ ", style)
                        })
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        f.render_widget(Paragraph::new(lines), inner);
    }

    fn draw_summary(&self, f: &mut Frame, area: Rect) {
        let s = self.summary();
        let block = Block::bordered().title(" Summary ");
        let inner = block.inner(area);
        f.render_widget(block, area);
        let rows = Layout::vertical([Constraint::Length(1); 9]).split(inner);
        let total = s.slots.max(1) as f64;
        let bar_w = inner.width.saturating_sub(24) as usize;
        for (i, (label, n, color)) in [
            ("full", s.full, Color::Green),
            ("empty", s.empty, Color::Yellow),
            ("orphaned", s.orphaned, Color::Red),
            ("missed", s.missed, Color::DarkGray),
        ]
        .into_iter()
        .enumerate()
        {
            let ratio = n as f64 / total;
            let filled = (ratio * bar_w as f64).round() as usize;
            f.render_widget(
                Line::from(vec![
                    Span::raw(format!("{label:<9}{n:>5} {:>6.1}% ", ratio * 100.0)),
                    Span::styled("█".repeat(filled), Style::new().fg(color)),
                    Span::styled("░".repeat(bar_w - filled), Style::new().dark_gray()),
                ]),
                rows[i],
            );
        }
        let text = [
            format!("revealed     {:>6}   lost (honest) {}", s.revealed, s.honest_reveal_lost),
            format!(
                "paid w/o payload  {:>4}   unpaid canonical {}",
                s.paid_without_payload, s.unpaid_canonical
            ),
            format!(
                "bids {:.3} ETH, paid {:.3} ETH",
                s.total_bids_gwei as f64 / 1e9,
                s.total_paid_gwei as f64 / 1e9
            ),
            format!("mean PTC present  {:.1} / 512", s.mean_ptc_present),
        ];
        for (i, t) in text.into_iter().enumerate() {
            f.render_widget(Line::from(t), rows[5 + i]);
        }
    }

    fn draw_detail(&self, f: &mut Frame, area: Rect) {
        let block = Block::bordered().title(" Slot detail ");
        let Some(r) = self.records.get(self.cursor).filter(|_| self.shown > 0) else {
            f.render_widget(Paragraph::new("waiting for the first slot…").block(block), area);
            return;
        };
        let kv = |k: &str, v: String| {
            Line::from(vec![Span::styled(format!("{k:<20}"), Style::new().dark_gray()), Span::raw(v)])
        };
        let builder = match (r.self_built, r.builder, r.strategy) {
            (true, _, _) => "self-built".to_string(),
            (_, Some(b), Some(s)) => format!("#{b} ({s:?})").to_lowercase(),
            _ => "-".to_string(),
        };
        let lines = vec![
            Line::from(vec![
                Span::styled(format!("slot {}  ", r.slot), Style::new().bold()),
                Span::styled(
                    format!("{:?}", r.outcome).to_uppercase(),
                    Style::new().bold().fg(outcome_color(r.outcome)),
                ),
                Span::raw(if r.paid { "  paid" } else { "" }),
            ]),
            kv("proposal", format!("{:?}", r.proposal).to_lowercase()),
            kv("builder", builder),
            kv("bid", format!("{:.4} ETH", r.bid_gwei as f64 / 1e9)),
            kv("parent", if r.built_on_full_parent { "full".into() } else { "empty".into() }),
            kv("revealed", format!("{} (saw timely block: {})", r.revealed, r.builder_saw_timely_block)),
            kv("timely attesters", r.timely_attesters.to_string()),
            kv("same-slot votes", r.same_slot_votes.to_string()),
            kv(
                "PTC present/absent",
                format!("{} / {}  (available {})", r.ptc_present, r.ptc_absent, r.ptc_available),
            ),
        ];
        f.render_widget(Paragraph::new(lines).block(block), area);
    }

    fn draw_trends(&self, f: &mut Frame, area: Rect) {
        let [full, ptc] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)]).areas(area);
        let width = full.width.saturating_sub(2) as usize;
        let shown = &self.records[..self.shown];
        let rolling: Vec<u64> = (0..shown.len())
            .map(|i| {
                let w = &shown[i.saturating_sub(WINDOW - 1)..=i];
                100 * w.iter().filter(|r| r.outcome == SlotOutcome::Full).count() as u64 / w.len() as u64
            })
            .collect();
        let tail = |v: &[u64]| v[v.len().saturating_sub(width)..].to_vec();
        f.render_widget(
            Sparkline::default()
                .block(Block::bordered().title(format!(" Full slots, rolling {WINDOW} (%) ")))
                .data(tail(&rolling))
                .max(100)
                .style(Style::new().green()),
            full,
        );
        let ptc_data: Vec<u64> = shown.iter().map(|r| r.ptc_present as u64).collect();
        f.render_widget(
            Sparkline::default()
                .block(Block::bordered().title(" PTC 'present' votes per slot "))
                .data(tail(&ptc_data))
                .max(512)
                .style(Style::new().cyan()),
            ptc,
        );
    }
}

pub fn run_tui(app: &mut App) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, app);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App) -> std::io::Result<()> {
    let frame = Duration::from_millis(50);
    let mut last = Instant::now();
    while !app.quit {
        terminal.draw(|f| app.draw(f))?;
        let timeout = frame.saturating_sub(last.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.key(key);
                }
            }
        }
        if last.elapsed() >= frame {
            app.tick();
            last = Instant::now();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use epbs_sim::presets;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn app() -> App {
        App::new(presets(), 0, 1, 64)
    }

    fn press(app: &mut App, code: KeyCode) {
        app.key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn screen(app: &App) -> String {
        let mut t = Terminal::new(TestBackend::new(140, 40)).unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        t.backend().buffer().content().iter().map(|c| c.symbol()).collect()
    }

    #[test]
    fn playback_reveals_slots_and_stops() {
        let mut a = app();
        assert_eq!(a.shown, 0);
        a.tick();
        assert_eq!(a.shown, SPEEDS[1]);
        press(&mut a, KeyCode::End);
        assert_eq!((a.shown, a.cursor, a.playing), (64, 63, false));
        a.tick();
        assert_eq!(a.shown, 64);
    }

    #[test]
    fn keys_switch_scenario_seed_and_cursor() {
        let mut a = app();
        press(&mut a, KeyCode::Up);
        assert_eq!(a.selected, a.scenarios.len() - 1);
        press(&mut a, KeyCode::Down);
        assert_eq!(a.selected, 0);
        press(&mut a, KeyCode::Char('s'));
        assert_eq!(a.seed, 2);
        press(&mut a, KeyCode::End);
        press(&mut a, KeyCode::Left);
        assert_eq!(a.cursor, 62);
        press(&mut a, KeyCode::Right);
        press(&mut a, KeyCode::Right);
        assert_eq!(a.cursor, 63);
        press(&mut a, KeyCode::Char('+'));
        assert_eq!(SPEEDS[a.speed], 16);
        press(&mut a, KeyCode::Char('q'));
        assert!(a.quit);
    }

    #[test]
    fn renders_every_panel() {
        let mut a = app();
        assert!(screen(&a).contains("waiting for the first slot"));
        press(&mut a, KeyCode::End);
        let out = screen(&a);
        for want in [
            "epbs-sim",
            "baseline",
            "Scenarios",
            "Slots",
            "Summary",
            "Slot detail",
            "slot 64",
            "FULL",
            "100.0%",
        ] {
            assert!(out.contains(want), "missing {want:?}");
        }
    }
}
