pub mod theme;
pub mod views;

use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::prelude::*;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::discovery;
use crate::drivers;
use crate::installer;
use crate::models::*;
use views::install::InstallState;

enum Page {
    Scan,
    Identify,
    Drivers,
    Install,
}

enum Message {
    ScanComplete(Vec<Printer>),
    #[allow(dead_code)]
    IdentifyComplete(Option<Printer>),
    DriversComplete(DriverResults),
    InstallComplete(InstallResult),
    Error(String),
}

pub struct App {
    page: Page,
    printers: Vec<Printer>,
    selected_printer: usize,
    selected_driver: usize,
    current_printer: Option<Printer>,
    driver_results: DriverResults,
    install_state: InstallState,
    install_ip: String,
    install_driver: String,
    scanning: bool,
    loading: bool,
    community: String,
    tx: mpsc::UnboundedSender<Message>,
    rx: mpsc::UnboundedReceiver<Message>,
    should_quit: bool,
}

impl App {
    pub fn new(community: String) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            page: Page::Scan,
            printers: Vec::new(),
            selected_printer: 0,
            selected_driver: 0,
            current_printer: None,
            driver_results: DriverResults {
                printer_model: String::new(),
                matched: vec![],
                universal: vec![],
            },
            install_state: InstallState::CreatingPort,
            install_ip: String::new(),
            install_driver: String::new(),
            scanning: false,
            loading: false,
            community,
            tx,
            rx,
            should_quit: false,
        }
    }

    pub async fn run(&mut self, terminal: &mut ratatui::DefaultTerminal) -> color_eyre::Result<()> {
        while !self.should_quit {
            terminal.draw(|f| self.render(f))?;

            while let Ok(msg) = self.rx.try_recv() {
                self.handle_message(msg);
            }

            if event::poll(Duration::from_millis(100))?
                && let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
            {
                self.handle_key(key.code);
            }
        }

        Ok(())
    }

    fn handle_message(&mut self, msg: Message) {
        match msg {
            Message::ScanComplete(printers) => {
                self.printers = printers;
                self.scanning = false;
                self.selected_printer = 0;
            }
            Message::IdentifyComplete(printer) => {
                self.loading = false;
                if let Some(p) = printer {
                    self.current_printer = Some(p);
                    self.page = Page::Identify;
                }
            }
            Message::DriversComplete(results) => {
                self.loading = false;
                self.driver_results = results;
                self.selected_driver = 0;
                self.page = Page::Drivers;
            }
            Message::InstallComplete(result) => {
                self.install_state = InstallState::Complete(result);
            }
            Message::Error(_e) => {
                self.scanning = false;
                self.loading = false;
            }
        }
    }

    fn render(&self, f: &mut Frame) {
        match self.page {
            Page::Scan => views::scan::render_scan_view(
                f, f.area(), &self.printers, self.selected_printer, self.scanning,
            ),
            Page::Identify => {
                if let Some(ref printer) = self.current_printer {
                    views::identify::render_identify_view(f, f.area(), printer);
                }
            }
            Page::Drivers => views::drivers::render_drivers_view(
                f, f.area(), &self.driver_results, self.selected_driver, self.loading,
            ),
            Page::Install => views::install::render_install_view(
                f, f.area(), &self.install_state, &self.install_ip, &self.install_driver,
            ),
        }
    }

    fn handle_key(&mut self, key: KeyCode) {
        match self.page {
            Page::Scan => match key {
                KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.selected_printer > 0 { self.selected_printer -= 1; }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.selected_printer + 1 < self.printers.len() { self.selected_printer += 1; }
                }
                KeyCode::Char('s') => self.start_scan(),
                KeyCode::Char('i') => self.identify_selected(),
                KeyCode::Enter => self.drivers_for_selected(),
                _ => {}
            },
            Page::Identify => match key {
                KeyCode::Char('q') => self.should_quit = true,
                KeyCode::Esc => self.page = Page::Scan,
                KeyCode::Char('d') | KeyCode::Enter => self.drivers_for_current(),
                _ => {}
            },
            Page::Drivers => match key {
                KeyCode::Char('q') => self.should_quit = true,
                KeyCode::Esc => self.page = Page::Scan,
                KeyCode::Up | KeyCode::Char('k') => {
                    if self.selected_driver > 0 { self.selected_driver -= 1; }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let total = self.driver_results.matched.len() + self.driver_results.universal.len();
                    if self.selected_driver + 1 < total { self.selected_driver += 1; }
                }
                KeyCode::Enter => self.install_selected_driver(),
                _ => {}
            },
            Page::Install => match key {
                KeyCode::Char('q') => self.should_quit = true,
                KeyCode::Esc => self.page = Page::Drivers,
                _ => {}
            },
        }
    }

    fn start_scan(&mut self) {
        if self.scanning { return; }
        self.scanning = true;
        let tx = self.tx.clone();
        let community = self.community.clone();
        // TODO: prompt for subnet via input widget; for now uses common default
        tokio::spawn(async move {
            match discovery::subnet::parse_cidr("192.168.1.0/24") {
                Ok(hosts) => {
                    let printers = discovery::scan_subnet(hosts, &community).await;
                    let _ = tx.send(Message::ScanComplete(printers));
                }
                Err(e) => { let _ = tx.send(Message::Error(e)); }
            }
        });
    }

    fn identify_selected(&mut self) {
        if self.printers.is_empty() { return; }
        let printer = self.printers[self.selected_printer].clone();
        self.current_printer = Some(printer);
        self.page = Page::Identify;
    }

    fn drivers_for_selected(&mut self) {
        if self.printers.is_empty() { return; }
        let printer = self.printers[self.selected_printer].clone();
        self.loading = true;
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let model = printer.model.unwrap_or_default();
            let local_drivers = drivers::local_store::list_drivers(false);
            let results = drivers::matcher::match_drivers(&model, &local_drivers);
            let _ = tx.send(Message::DriversComplete(results));
        });
    }

    fn drivers_for_current(&mut self) {
        if let Some(ref printer) = self.current_printer {
            let model = printer.model.clone().unwrap_or_default();
            self.loading = true;
            let tx = self.tx.clone();
            tokio::spawn(async move {
                let local_drivers = drivers::local_store::list_drivers(false);
                let results = drivers::matcher::match_drivers(&model, &local_drivers);
                let _ = tx.send(Message::DriversComplete(results));
            });
        }
    }

    fn install_selected_driver(&mut self) {
        let all_drivers: Vec<&DriverMatch> = self.driver_results.matched.iter()
            .chain(self.driver_results.universal.iter())
            .collect();
        if all_drivers.is_empty() { return; }
        let driver = all_drivers[self.selected_driver].clone();
        let ip = self.current_printer.as_ref()
            .or(self.printers.get(self.selected_printer))
            .map(|p| p.display_ip())
            .unwrap_or_default();

        self.install_ip = ip.clone();
        self.install_driver = driver.name.clone();
        self.install_state = InstallState::CreatingPort;
        self.page = Page::Install;

        let tx = self.tx.clone();
        let driver_name = driver.name.clone();
        let model = self.driver_results.printer_model.clone();
        tokio::spawn(async move {
            let printer_name = model.clone();
            let result = installer::install_printer(&ip, &driver_name, &printer_name, &model, false);
            let _ = tx.send(Message::InstallComplete(result));
        });
    }
}
