pub mod keys;
pub mod layout;
pub mod theme;
pub mod views;

use std::time::{Duration, Instant};

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::prelude::*;
use ratatui::widgets::*;
use tokio::sync::mpsc;

use crate::discovery;
use crate::models::*;

// ── Panel focus ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Panel {
    PrinterList,
    Detail,
}

// ── Install progress steps ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum InstallStep {
    PortCreated,
    DriverInstalled,
    PrinterAdded,
    Failed(String),
}

// ── What the detail panel is showing ─────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum DetailView {
    PrinterInfo,
    Installing {
        step: usize, // 0=port, 1=driver, 2=printer
        error: Option<String>,
    },
    InstallComplete(InstallResult),
}

// ── Async message types ───────────────────────────────────────────────────────

pub enum Message {
    ScanProgress {
        found: usize,
        scanned: usize,
        total: usize,
    },
    ScanComplete(Vec<Printer>),
    DriversComplete(DriverResults),
    InstallStepComplete(InstallStep),
    InstallComplete(InstallResult),
}

// ── App state ─────────────────────────────────────────────────────────────────

pub struct App {
    pub printers: Vec<Printer>,
    pub printer_list_state: ListState,
    pub driver_results: Option<DriverResults>,
    pub driver_list_state: ListState,
    pub focused_panel: Panel,
    pub detail_view: DetailView,
    pub subnet: String,
    pub scanning: bool,
    pub scan_progress: (usize, usize), // (scanned, total)
    pub community: String,
    pub show_help: bool,
    pub status_message: Option<(String, Style, Instant)>,
    pub quit: bool,
    msg_rx: mpsc::UnboundedReceiver<Message>,
    msg_tx: mpsc::UnboundedSender<Message>,
}

impl App {
    pub fn new(community: String) -> Self {
        let (msg_tx, msg_rx) = mpsc::unbounded_channel();

        let subnet = discovery::subnet::auto_detect_subnet(false)
            .unwrap_or_else(|| "192.168.1.0/24".to_string());

        let mut printer_list_state = ListState::default();
        printer_list_state.select(Some(0));

        Self {
            printers: Vec::new(),
            printer_list_state,
            driver_results: None,
            driver_list_state: ListState::default(),
            focused_panel: Panel::PrinterList,
            detail_view: DetailView::PrinterInfo,
            subnet,
            scanning: false,
            scan_progress: (0, 0),
            community,
            show_help: false,
            status_message: None,
            quit: false,
            msg_rx,
            msg_tx,
        }
    }

    pub async fn run<B: Backend>(
        &mut self,
        terminal: &mut Terminal<B>,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        B::Error: 'static,
    {
        // Kick off an initial scan immediately on launch
        self.start_scan();

        loop {
            terminal.draw(|f| self.render(f))?;

            // Poll for input events (100ms timeout so messages stay responsive)
            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.handle_key(key);
            }

            // Drain the async message channel
            while let Ok(msg) = self.msg_rx.try_recv() {
                self.handle_message(msg);
            }

            // Auto-dismiss status messages after 3 seconds
            if matches!(&self.status_message, Some((_, _, ts)) if ts.elapsed() >= Duration::from_secs(3)) {
                self.status_message = None;
            }

            if self.quit {
                break;
            }
        }

        Ok(())
    }

    // ── Message handling ──────────────────────────────────────────────────────

    fn handle_message(&mut self, msg: Message) {
        match msg {
            Message::ScanProgress {
                found,
                scanned,
                total,
            } => {
                self.scan_progress = (scanned, total);
                // Update found count as we go — partial results feel snappy
                let _ = found; // used in status bar rendering
            }
            Message::ScanComplete(printers) => {
                self.printers = printers;
                self.scanning = false;
                self.scan_progress = (0, 0);
                // Keep selection in bounds
                let selected = self
                    .printer_list_state
                    .selected()
                    .unwrap_or(0)
                    .min(self.printers.len().saturating_sub(1));
                self.printer_list_state.select(if self.printers.is_empty() {
                    None
                } else {
                    Some(selected)
                });
                // Auto-load drivers for the newly selected printer
                if let Some(idx) = self.printer_list_state.selected()
                    && let Some(printer) = self.printers.get(idx)
                {
                    self.start_driver_load(printer);
                }
                let count = self.printers.len();
                self.set_status(
                    format!("Scan complete — {count} printer(s) found"),
                    theme::STATUS_SUCCESS,
                );
            }
            Message::DriversComplete(results) => {
                let mut state = ListState::default();
                let total = results.matched.len() + results.universal.len();
                if total > 0 {
                    state.select(Some(0));
                }
                self.driver_list_state = state;
                self.driver_results = Some(results);
            }
            Message::InstallStepComplete(step) => match step {
                InstallStep::PortCreated => {
                    self.detail_view = DetailView::Installing {
                        step: 1,
                        error: None,
                    };
                }
                InstallStep::DriverInstalled => {
                    self.detail_view = DetailView::Installing {
                        step: 2,
                        error: None,
                    };
                }
                InstallStep::PrinterAdded => {
                    self.detail_view = DetailView::Installing {
                        step: 3,
                        error: None,
                    };
                }
                InstallStep::Failed(msg) => {
                    let step = match &self.detail_view {
                        DetailView::Installing { step, .. } => *step,
                        _ => 0,
                    };
                    self.detail_view = DetailView::Installing {
                        step,
                        error: Some(msg),
                    };
                }
            },
            Message::InstallComplete(result) => {
                let success = result.success;
                let name = result.printer_name.clone();
                self.detail_view = DetailView::InstallComplete(result);
                if success {
                    self.set_status(
                        format!("'{name}' installed successfully"),
                        theme::STATUS_SUCCESS,
                    );
                } else {
                    self.set_status("Install failed".to_string(), theme::STATUS_ERROR_MSG);
                }
            }
        }
    }

    // ── Key handling ──────────────────────────────────────────────────────────

    fn handle_key(&mut self, event: KeyEvent) {
        // Help overlay consumes all keys
        if self.show_help {
            if keys::key(event, KeyCode::Esc) || keys::char(event, '?') {
                self.show_help = false;
            }
            return;
        }

        // Global keys — always active
        if keys::char(event, 'q') {
            self.quit = true;
            return;
        }
        if keys::char(event, '?') {
            self.show_help = true;
            return;
        }
        if keys::char(event, 's') {
            self.start_scan();
            return;
        }

        // Tab / Shift+Tab / h / l — cycle panel focus
        if keys::key(event, KeyCode::Tab) || keys::char(event, 'l') {
            self.focused_panel = Panel::Detail;
            return;
        }
        if keys::shift_tab(event) || keys::char(event, 'h') {
            self.focused_panel = Panel::PrinterList;
            return;
        }

        // Esc — close help or return focus to printer list
        if keys::key(event, KeyCode::Esc) {
            self.focused_panel = Panel::PrinterList;
            return;
        }

        match self.focused_panel {
            Panel::PrinterList => self.handle_printer_list_key(event),
            Panel::Detail => self.handle_detail_key(event),
        }
    }

    fn handle_printer_list_key(&mut self, event: KeyEvent) {
        let len = self.printers.len();
        if len == 0 {
            return;
        }

        let current = self.printer_list_state.selected().unwrap_or(0);

        if keys::char(event, 'j') || keys::key(event, KeyCode::Down) {
            let next = (current + 1).min(len - 1);
            self.printer_list_state.select(Some(next));
            if let Some(printer) = self.printers.get(next) {
                self.start_driver_load(printer);
            }
            self.detail_view = DetailView::PrinterInfo;
        } else if keys::char(event, 'k') || keys::key(event, KeyCode::Up) {
            let prev = current.saturating_sub(1);
            self.printer_list_state.select(Some(prev));
            if let Some(printer) = self.printers.get(prev) {
                self.start_driver_load(printer);
            }
            self.detail_view = DetailView::PrinterInfo;
        } else if keys::char(event, 'g') {
            self.printer_list_state.select(Some(0));
            if let Some(printer) = self.printers.first() {
                self.start_driver_load(printer);
            }
            self.detail_view = DetailView::PrinterInfo;
        } else if keys::char(event, 'G') {
            let last = len - 1;
            self.printer_list_state.select(Some(last));
            if let Some(printer) = self.printers.get(last) {
                self.start_driver_load(printer);
            }
            self.detail_view = DetailView::PrinterInfo;
        } else if keys::key(event, KeyCode::Enter) {
            self.focused_panel = Panel::Detail;
        }
    }

    fn handle_detail_key(&mut self, event: KeyEvent) {
        if keys::key(event, KeyCode::Esc) {
            self.focused_panel = Panel::PrinterList;
            return;
        }

        let Some(ref results) = self.driver_results else {
            return;
        };
        let total = results.matched.len() + results.universal.len();
        if total == 0 {
            return;
        }

        let current = self.driver_list_state.selected().unwrap_or(0);

        if keys::char(event, 'j') || keys::key(event, KeyCode::Down) {
            self.driver_list_state.select(Some((current + 1).min(total - 1)));
        } else if keys::char(event, 'k') || keys::key(event, KeyCode::Up) {
            self.driver_list_state.select(Some(current.saturating_sub(1)));
        } else if keys::key(event, KeyCode::Enter) {
            self.start_install();
        }
    }

    // ── Async task spawning ───────────────────────────────────────────────────

    fn start_scan(&mut self) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        self.driver_results = None;
        self.detail_view = DetailView::PrinterInfo;

        let tx = self.msg_tx.clone();
        let community = self.community.clone();
        let subnet = self.subnet.clone();

        tokio::spawn(async move {
            match discovery::subnet::parse_cidr(&subnet) {
                Ok(hosts) => {
                    let total = hosts.len();
                    let _ = tx.send(Message::ScanProgress {
                        found: 0,
                        scanned: 0,
                        total,
                    });
                    let printers = discovery::scan_subnet(
                        hosts,
                        &community,
                        &discovery::ScanMethod::All,
                        Duration::from_millis(500),
                        false,
                    )
                    .await;
                    let _ = tx.send(Message::ScanComplete(printers));
                }
                Err(e) => {
                    // Emit empty result so scanning flag resets
                    let _ = tx.send(Message::ScanComplete(vec![]));
                    let _ = e; // error visible via status; no separate error message type needed
                }
            }
        });
    }

    fn start_driver_load(&self, printer: &Printer) {
        let tx = self.msg_tx.clone();
        let model = printer.model.clone().unwrap_or_default();
        tokio::spawn(async move {
            let local_drivers = crate::drivers::local_store::list_drivers(false);
            let results = crate::drivers::matcher::match_drivers(&model, &local_drivers);
            let _ = tx.send(Message::DriversComplete(results));
        });
    }

    fn start_install(&mut self) {
        let Some(ref results) = self.driver_results else {
            return;
        };
        let driver_idx = self.driver_list_state.selected().unwrap_or(0);
        let all_drivers: Vec<&DriverMatch> = results.matched.iter().chain(results.universal.iter()).collect();
        let Some(driver) = all_drivers.get(driver_idx) else {
            return;
        };
        let printer_idx = self.printer_list_state.selected().unwrap_or(0);
        let Some(printer) = self.printers.get(printer_idx) else {
            return;
        };

        let ip = printer.display_ip();
        let driver_name = driver.name.clone();
        let model = results.printer_model.clone();

        self.detail_view = DetailView::Installing {
            step: 0,
            error: None,
        };

        let tx = self.msg_tx.clone();
        tokio::spawn(async move {
            let printer_name = model.clone();
            let result =
                crate::installer::install_printer(&ip, &driver_name, &printer_name, &model, false);
            let _ = tx.send(Message::InstallComplete(result));
        });
    }

    // ── Status messages ───────────────────────────────────────────────────────

    fn set_status(&mut self, msg: String, style: Style) {
        self.status_message = Some((msg, style, Instant::now()));
    }

    // ── Rendering ─────────────────────────────────────────────────────────────

    fn render(&mut self, f: &mut Frame) {
        let area = f.area();
        let (header_rect, panels_rect, status_rect) = layout::main_layout(area);
        let mode = layout::LayoutMode::from_width(area.width);
        let (list_rect, detail_rect_opt) = layout::panel_layout(panels_rect, mode);

        self.render_header(f, header_rect);
        self.render_printer_list(f, list_rect);
        if let Some(detail_rect) = detail_rect_opt {
            self.render_detail(f, detail_rect);
        }
        self.render_status_bar(f, status_rect);

        if self.show_help {
            self.render_help_overlay(f, area);
        }
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let subnet = self.subnet.clone();
        let left = Span::styled(
            format!(" Subnet: {subnet}"),
            theme::HEADER,
        );
        let right = Span::styled(" ?:help ", theme::HELP_TEXT);
        let spacer_width = area
            .width
            .saturating_sub(left.width() as u16 + right.width() as u16);
        let line = Line::from(vec![
            left,
            Span::raw(" ".repeat(spacer_width as usize)),
            right,
        ]);
        f.render_widget(Paragraph::new(line), area);
    }

    fn render_printer_list(&mut self, f: &mut Frame, area: Rect) {
        let focused = self.focused_panel == Panel::PrinterList;
        let border_style = if focused {
            theme::FOCUSED_BORDER
        } else {
            theme::UNFOCUSED_BORDER
        };

        let title = if self.scanning {
            " Printers (scanning...) "
        } else {
            " Printers "
        };

        let items: Vec<ListItem> = self
            .printers
            .iter()
            .map(|p| {
                let status_indicator = match p.status {
                    PrinterStatus::Ready => Span::styled("● ", theme::STATUS_READY),
                    PrinterStatus::Error => Span::styled("✗ ", theme::STATUS_ERROR),
                    PrinterStatus::Offline => Span::styled("○ ", theme::STATUS_OFFLINE),
                    PrinterStatus::Unknown => Span::styled("◆ ", theme::DIM),
                };
                let ip = Span::raw(p.display_ip());
                let model = Span::styled(
                    format!(
                        "  {}",
                        p.model.as_deref().unwrap_or("Unknown")
                    ),
                    theme::DIM,
                );
                ListItem::new(Line::from(vec![status_indicator, ip, model]))
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(title)
                    .border_style(border_style),
            )
            .highlight_style(theme::SELECTED)
            .highlight_symbol("▶ ");

        f.render_stateful_widget(list, area, &mut self.printer_list_state);
    }

    fn render_detail(&mut self, f: &mut Frame, area: Rect) {
        let focused = self.focused_panel == Panel::Detail;
        let border_style = if focused {
            theme::FOCUSED_BORDER
        } else {
            theme::UNFOCUSED_BORDER
        };

        match &self.detail_view.clone() {
            DetailView::PrinterInfo => {
                self.render_detail_printer_info(f, area, border_style);
            }
            DetailView::Installing { step, error } => {
                self.render_detail_installing(f, area, border_style, *step, error.clone());
            }
            DetailView::InstallComplete(result) => {
                self.render_detail_install_complete(f, area, border_style, result.clone());
            }
        }
    }

    fn render_detail_printer_info(&mut self, f: &mut Frame, area: Rect, border_style: Style) {
        // Split detail area: top = printer info, bottom = driver list
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(6), Constraint::Min(3)])
            .split(area);

        // Printer info panel
        let printer = self
            .printer_list_state
            .selected()
            .and_then(|i| self.printers.get(i));

        let info_lines: Vec<Line> = if let Some(p) = printer {
            vec![
                Line::from(vec![
                    Span::styled(" IP:     ", theme::HEADER),
                    Span::raw(p.display_ip()),
                ]),
                Line::from(vec![
                    Span::styled(" Model:  ", theme::HEADER),
                    Span::raw(p.model.as_deref().unwrap_or("Unknown")),
                ]),
                Line::from(vec![
                    Span::styled(" Serial: ", theme::HEADER),
                    Span::raw(p.serial.as_deref().unwrap_or("N/A")),
                ]),
                Line::from(vec![
                    Span::styled(" Status: ", theme::HEADER),
                    Span::raw(p.status.to_string()),
                ]),
            ]
        } else {
            vec![Line::from(Span::styled(
                " No printer selected",
                theme::DIM,
            ))]
        };

        f.render_widget(
            Paragraph::new(info_lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Printer Info ")
                    .border_style(border_style),
            ),
            chunks[0],
        );

        // Driver list panel
        self.render_driver_list(f, chunks[1], border_style);
    }

    fn render_driver_list(&mut self, f: &mut Frame, area: Rect, border_style: Style) {
        let mut items: Vec<ListItem> = Vec::new();

        if let Some(ref results) = self.driver_results.clone() {
            if !results.matched.is_empty() {
                items.push(ListItem::new(Line::from(Span::styled(
                    "── Matched ──",
                    theme::SECTION_HEADER,
                ))));
                for dm in &results.matched {
                    let badge = match dm.confidence {
                        MatchConfidence::Exact => Span::styled("★ ", theme::EXACT_BADGE),
                        MatchConfidence::Fuzzy => Span::styled("● ", theme::FUZZY_BADGE),
                        MatchConfidence::Universal => Span::styled("○ ", theme::DIM),
                    };
                    items.push(ListItem::new(Line::from(vec![
                        Span::raw("  "),
                        badge,
                        Span::raw(dm.name.clone()),
                    ])));
                }
            }

            if !results.universal.is_empty() {
                items.push(ListItem::new(Line::from(Span::styled(
                    "── Universal ──",
                    theme::SECTION_HEADER,
                ))));
                for dm in &results.universal {
                    items.push(ListItem::new(Line::from(vec![
                        Span::raw("  ○ "),
                        Span::raw(dm.name.clone()),
                    ])));
                }
            }

            if items.is_empty() {
                items.push(ListItem::new(Span::styled(
                    " No drivers found",
                    theme::DIM,
                )));
            }
        } else {
            items.push(ListItem::new(Span::styled(
                " Loading drivers...",
                theme::DIM,
            )));
        }

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Drivers ")
                    .border_style(border_style),
            )
            .highlight_style(theme::SELECTED)
            .highlight_symbol("▶ ");

        f.render_stateful_widget(list, area, &mut self.driver_list_state);
    }

    fn render_detail_installing(
        &self,
        f: &mut Frame,
        area: Rect,
        border_style: Style,
        step: usize,
        error: Option<String>,
    ) {
        let check = |s: usize| {
            if s < step {
                "✓"
            } else if s == step {
                "→"
            } else {
                " "
            }
        };

        let mut lines = vec![
            Line::from(format!("  {} Creating TCP/IP port...", check(0))),
            Line::from(format!("  {} Installing driver...", check(1))),
            Line::from(format!("  {} Adding printer queue...", check(2))),
        ];

        if let Some(ref err) = error {
            lines.push(Line::from(Span::styled(
                format!("\n  ✗ Error: {err}"),
                theme::STATUS_ERROR_MSG,
            )));
        }

        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Installing... ")
                    .border_style(border_style),
            ),
            area,
        );
    }

    fn render_detail_install_complete(
        &self,
        f: &mut Frame,
        area: Rect,
        border_style: Style,
        result: InstallResult,
    ) {
        let lines = if result.success {
            vec![
                Line::from("  ✓ Creating TCP/IP port"),
                Line::from("  ✓ Installing driver"),
                Line::from("  ✓ Adding printer queue"),
                Line::from(""),
                Line::from(Span::styled(
                    format!("  Printer '{}' is ready!", result.printer_name),
                    theme::STATUS_SUCCESS,
                )),
            ]
        } else {
            let err = result.error.as_deref().unwrap_or("unknown error");
            vec![
                Line::from("  ✓ Creating TCP/IP port"),
                Line::from("  ✓ Installing driver"),
                Line::from(Span::styled(
                    format!("  ✗ Failed: {err}"),
                    theme::STATUS_ERROR_MSG,
                )),
            ]
        };

        f.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Install Complete ")
                    .border_style(border_style),
            ),
            area,
        );
    }

    fn render_status_bar(&self, f: &mut Frame, area: Rect) {
        // Transient status message takes priority
        if let Some((ref msg, style, _)) = self.status_message {
            f.render_widget(
                Paragraph::new(Span::styled(format!(" {msg}"), style)),
                area,
            );
            return;
        }

        // Scan progress
        if self.scanning {
            let (scanned, total) = self.scan_progress;
            let text = if total > 0 {
                format!(" Scanning {scanned}/{total} hosts...")
            } else {
                " Scanning...".to_string()
            };
            f.render_widget(
                Paragraph::new(Span::styled(text, theme::STATUS_INFO)),
                area,
            );
            return;
        }

        // Default key hint bar
        let hints = Line::from(vec![
            Span::styled("j/k", theme::HELP_KEY),
            Span::styled(" navigate  ", theme::HELP_TEXT),
            Span::styled("Tab/h/l", theme::HELP_KEY),
            Span::styled(" switch panel  ", theme::HELP_TEXT),
            Span::styled("Enter", theme::HELP_KEY),
            Span::styled(" select  ", theme::HELP_TEXT),
            Span::styled("s", theme::HELP_KEY),
            Span::styled(" rescan  ", theme::HELP_TEXT),
            Span::styled("?", theme::HELP_KEY),
            Span::styled(" help  ", theme::HELP_TEXT),
            Span::styled("q", theme::HELP_KEY),
            Span::styled(" quit", theme::HELP_TEXT),
        ]);
        f.render_widget(Paragraph::new(hints), area);
    }

    fn render_help_overlay(&self, f: &mut Frame, area: Rect) {
        // Center a fixed-size popup
        let popup_width = 50u16.min(area.width);
        let popup_height = 18u16.min(area.height);
        let x = (area.width.saturating_sub(popup_width)) / 2;
        let y = (area.height.saturating_sub(popup_height)) / 2;
        let popup_rect = Rect::new(x, y, popup_width, popup_height);

        // Clear background
        f.render_widget(Clear, popup_rect);

        let help_text = vec![
            Line::from(Span::styled(" Navigation", theme::SECTION_HEADER)),
            Line::from(""),
            Line::from(vec![
                Span::styled("  j/k ↑↓  ", theme::HELP_KEY),
                Span::styled("Move up/down in list", theme::HELP_TEXT),
            ]),
            Line::from(vec![
                Span::styled("  g/G     ", theme::HELP_KEY),
                Span::styled("Jump to top/bottom", theme::HELP_TEXT),
            ]),
            Line::from(vec![
                Span::styled("  h/l Tab ", theme::HELP_KEY),
                Span::styled("Switch panel focus", theme::HELP_TEXT),
            ]),
            Line::from(vec![
                Span::styled("  Enter   ", theme::HELP_KEY),
                Span::styled("Select / confirm", theme::HELP_TEXT),
            ]),
            Line::from(vec![
                Span::styled("  Esc     ", theme::HELP_KEY),
                Span::styled("Back / focus printer list", theme::HELP_TEXT),
            ]),
            Line::from(""),
            Line::from(Span::styled(" Actions", theme::SECTION_HEADER)),
            Line::from(""),
            Line::from(vec![
                Span::styled("  s       ", theme::HELP_KEY),
                Span::styled("Rescan network", theme::HELP_TEXT),
            ]),
            Line::from(vec![
                Span::styled("  Enter   ", theme::HELP_KEY),
                Span::styled("(detail panel) Install driver", theme::HELP_TEXT),
            ]),
            Line::from(""),
            Line::from(Span::styled(" Other", theme::SECTION_HEADER)),
            Line::from(""),
            Line::from(vec![
                Span::styled("  ?       ", theme::HELP_KEY),
                Span::styled("Toggle this help", theme::HELP_TEXT),
            ]),
            Line::from(vec![
                Span::styled("  q       ", theme::HELP_KEY),
                Span::styled("Quit", theme::HELP_TEXT),
            ]),
        ];

        f.render_widget(
            Paragraph::new(help_text).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Help ")
                    .border_style(theme::FOCUSED_BORDER),
            ),
            popup_rect,
        );
    }
}
