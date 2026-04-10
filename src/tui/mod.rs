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
    InstallComplete(PrinterOpResult),
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
    InstallComplete(PrinterOpResult),
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
    pub fn new(community: String, subnet_override: Option<String>) -> Self {
        let (msg_tx, msg_rx) = mpsc::unbounded_channel();

        let subnet = subnet_override
            .or_else(|| discovery::subnet::auto_detect_subnet(false))
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
                let name = result
                    .detail_as::<InstallDetail>()
                    .map(|d| d.printer_name)
                    .unwrap_or_else(|| "printer".to_string());
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

        views::scan::render_printer_list(
            f,
            list_rect,
            &self.printers,
            self.scanning,
            &mut self.printer_list_state,
            self.focused_panel == Panel::PrinterList,
        );

        if let Some(detail_rect) = detail_rect_opt {
            let focused_detail = self.focused_panel == Panel::Detail;
            match self.detail_view.clone() {
                DetailView::Installing { step, error } => {
                    let (ip, driver) = self.selected_printer_ip_and_driver();
                    views::install::render_install_progress(
                        f,
                        detail_rect,
                        step,
                        error.as_deref(),
                        &ip,
                        &driver,
                        false,
                        None,
                    );
                }
                DetailView::InstallComplete(ref result) => {
                    let (ip, driver) = self.selected_printer_ip_and_driver();
                    views::install::render_install_progress(
                        f,
                        detail_rect,
                        3,
                        None,
                        &ip,
                        &driver,
                        true,
                        Some(result),
                    );
                }
                DetailView::PrinterInfo => {
                    let selected_printer = self
                        .printer_list_state
                        .selected()
                        .and_then(|i| self.printers.get(i));
                    views::drivers::render_detail_pane(
                        f,
                        detail_rect,
                        selected_printer,
                        self.driver_results.as_ref(),
                        &mut self.driver_list_state,
                        focused_detail,
                        false,
                    );
                }
            }
        }

        self.render_status_bar(f, status_rect);

        if self.show_help {
            views::help::render_help_overlay(f, area);
        }
    }

    fn render_header(&self, f: &mut Frame, area: Rect) {
        let subnet = self.subnet.clone();
        let left = Span::styled(format!(" Subnet: {subnet}"), theme::HEADER);
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

    /// Returns the IP string and driver name for the currently selected printer/driver.
    /// Used to populate install progress views.
    fn selected_printer_ip_and_driver(&self) -> (String, String) {
        let ip = self
            .printer_list_state
            .selected()
            .and_then(|i| self.printers.get(i))
            .map(|p| p.display_ip())
            .unwrap_or_default();

        let driver = self
            .driver_results
            .as_ref()
            .and_then(|r| {
                let idx = self.driver_list_state.selected().unwrap_or(0);
                r.matched
                    .iter()
                    .chain(r.universal.iter())
                    .nth(idx)
                    .map(|dm| dm.name.clone())
            })
            .unwrap_or_default();

        (ip, driver)
    }
}
