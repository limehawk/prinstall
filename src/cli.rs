use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "prinstall",
    version,
    about = "Discover network printers, match drivers, and install them",
    long_about = "Prinstall discovers network printers via SNMP, finds matching drivers \
                  (from the local driver store or manufacturer downloads), and installs \
                  them on Windows. Run without arguments to launch the interactive TUI, \
                  or use subcommands for scripted/RMM usage.",
    after_help = "EXAMPLES:\n  \
        prinstall                              Launch interactive TUI\n  \
        prinstall scan                         Scan local subnet (all methods, incl. mDNS)\n  \
        prinstall scan 192.168.1.0/24          Scan a specific subnet\n  \
        prinstall scan --method mdns           mDNS-only multicast browse (no subnet needed)\n  \
        prinstall id 192.168.1.100             Identify a printer by IP\n  \
        prinstall drivers 192.168.1.100        Show matched drivers for a printer\n  \
        prinstall add 192.168.1.100            Install printer with best-match driver\n  \
        prinstall add 192.168.1.100 --driver \"HP Universal Print Driver PCL6\"\n  \
        prinstall remove 192.168.1.100         Remove printer and clean up driver/port\n  \
        prinstall list                         List locally installed printers\n  \
        prinstall list --json                  List printers as JSON (for scripting)\n\n\
        Each subcommand has detailed --help. Try: prinstall scan --help"
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Output results as JSON (for scripting)
    #[arg(long, global = true)]
    pub json: bool,

    /// Step-by-step diagnostic output
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// SNMP community string [default: public]
    #[arg(long, global = true, default_value = "public")]
    pub community: String,

    /// Force operations that would normally warn (e.g., large subnet scans)
    #[arg(long, global = true)]
    pub force: bool,

    /// Override auto-detected subnet for TUI launch (e.g., 192.168.1.0/24)
    #[arg(long, global = true)]
    pub subnet: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Scan the local network for printers via multiple discovery methods
    ///
    /// Probes every IP on the subnet using SNMP, TCP port checks, or both.
    /// Each discovered printer shows its IP, model, and status.
    ///
    /// Without a subnet argument, scans the local machine's subnet.
    /// Subnets larger than /24 require --force.
    #[command(
        after_help = "EXAMPLES:\n  \
            prinstall scan                          Auto-detect subnet, run all methods\n  \
            prinstall scan 192.168.1.0/24           Scan a specific subnet\n  \
            prinstall scan --method snmp            SNMP-only (no mDNS)\n  \
            prinstall scan --method port            TCP port-check only\n  \
            prinstall scan --method mdns            mDNS-only multicast browse (no subnet)\n  \
            prinstall scan --timeout 200            200ms per-host timeout\n  \
            prinstall scan 10.0.0.0/24 --community private\n\n\
            HOW IT WORKS:\n  \
            all (default): port probe + IPP + SNMP across the subnet,\n                 \
                           PLUS an mDNS multicast browse. Results merged.\n  \
            port: Checks TCP port 9100 (raw print) for responsive hosts.\n  \
            snmp: Sends SNMP v2c GET requests to each IP on UDP port 161.\n  \
            mdns: Browses _ipp/_ipps/_pdl-datastream/_printer._tcp.local.\n        \
                  Multicast-based — subnet arg is not needed or used.\n        \
                  Runs for 3 seconds.\n  \
            Max 64 concurrent probes per unicast method.\n\n\
            TROUBLESHOOTING:\n  \
            No results? Common causes:\n  \
            • SNMP disabled on printer — enable via printer web UI\n  \
            • Non-default community string — try --community <string>\n  \
            • Firewall blocking UDP 161 — check network rules\n  \
            • mDNS multicast blocked by the NIC or router"
    )]
    Scan {
        /// Subnet in CIDR notation (e.g., 192.168.1.0/24). Ignored when
        /// `--method mdns` is used — mDNS is multicast and doesn't care
        /// about the subnet. Auto-detected from the local NIC otherwise.
        subnet: Option<String>,

        /// Discovery method: all (default), snmp, port, mdns
        #[arg(long)]
        method: Option<String>,

        /// Per-host timeout in milliseconds [default: 500]
        #[arg(long)]
        timeout: Option<u64>,

        /// Skip USB enumeration, show only network-discovered printers
        #[arg(long, conflicts_with = "usb_only")]
        network_only: bool,

        /// Skip network scan, show only USB-attached printers
        #[arg(long)]
        usb_only: bool,
    },

    /// Identify a specific printer by IP address
    ///
    /// Queries a single printer via SNMP to retrieve its model,
    /// serial number, and current status.
    #[command(
        after_help = "EXAMPLES:\n  \
            prinstall id 192.168.1.100\n  \
            prinstall id 10.0.0.50 --community private\n  \
            prinstall id 192.168.1.100 --json\n\n\
            HOW IT WORKS:\n  \
            Sends SNMP GET requests to the specified IP on UDP 161.\n  \
            Retrieves device description, serial number, and device status.\n  \
            Times out after 2 seconds if the printer doesn't respond."
    )]
    Id {
        /// Printer IP address
        ip: String,
    },

    /// Show matched drivers for a printer
    ///
    /// Identifies the printer via SNMP (or --model), then searches for
    /// compatible drivers in the local driver store and curated database.
    /// Results are split into Matched Drivers (ranked by confidence) and
    /// Universal Drivers (always available for the manufacturer).
    #[command(
        alias = "driver",
        after_help = "EXAMPLES:\n  \
            prinstall drivers 192.168.1.100\n  \
            prinstall drivers 192.168.1.100 --json\n  \
            prinstall drivers 192.168.1.100 --model \"HP LaserJet Pro MFP M428fdw\"\n\n\
            HOW IT WORKS:\n  \
            1. Identifies printer model via SNMP (or uses --model)\n  \
            2. Checks local driver store (pnputil) for staged drivers\n  \
            3. Matches against curated driver database\n  \
            4. Shows universal drivers for the manufacturer\n\n\
            CONFIDENCE LEVELS:\n  \
            ★ exact  — curated match from known database\n  \
            ● fuzzy  — name similarity match\n  \
            ○ low    — partial match, verify before installing"
    )]
    Drivers {
        /// Printer IP address
        ip: String,

        /// Manually specify printer model (bypass SNMP discovery)
        #[arg(long)]
        model: Option<String>,
    },

    /// Add a printer (network or USB)
    ///
    /// For network printers: identifies via SNMP, auto-picks the best-matched
    /// driver, downloads and stages it if needed, then runs Add-PrinterPort →
    /// Add-PrinterDriver → Add-Printer. If the primary install fails and the
    /// printer speaks IPP (port 631), falls back to Microsoft's built-in IPP
    /// Class Driver with a clearly-marked warning.
    ///
    /// For USB printers: pass `--usb` and specify the existing printer queue
    /// name as the target. The command verifies the queue exists, finds the
    /// best driver for the model, stages it, and swaps it in via Set-Printer.
    /// No port creation — USB printers use the auto-detected USB port.
    #[command(
        after_help = "EXAMPLES:\n  \
            # Network printers (target = IP)\n  \
            prinstall add 192.168.1.100\n  \
            prinstall add 192.168.1.100 --driver \"HP Universal Print Driver PCL6\"\n  \
            prinstall add 192.168.1.100 --name \"Front Desk Printer\"\n\n  \
            # USB printers (target = printer queue name)\n  \
            prinstall add \"Brother MFC-L2750DW\" --usb\n  \
            prinstall add \"HP OfficeJet Pro\" --usb --driver \"HP Universal PCL6\"\n\n\
            HOW IT WORKS (network):\n  \
            1. Identifies printer (SNMP or --model)\n  \
            2. Finds best driver (or uses --driver)\n  \
            3. Downloads driver if not locally staged\n  \
            4. Runs: Add-PrinterPort → Add-PrinterDriver → Add-Printer\n  \
            5. Falls back to Microsoft IPP Class Driver if primary install\n     \
               fails and port 631 is open (with visible warning)\n\n\
            HOW IT WORKS (USB):\n  \
            1. Verifies the USB printer queue exists (prinstall list)\n  \
            2. Finds best driver for the model (from queue name or --model)\n  \
            3. Stages the driver if not already in the driver store\n  \
            4. Runs: Set-Printer -DriverName to swap the driver\n\n\
            REQUIRES:\n  \
            Administrator privileges (UAC prompt if not elevated).\n  \
            Existing ports/drivers are reused, not duplicated."
    )]
    Add {
        /// Printer IP address (network mode) or queue name (--usb mode)
        target: String,

        /// Specific driver name to install (skip auto-matching)
        #[arg(long)]
        driver: Option<String>,

        /// Display name for the printer (network mode only; ignored for --usb)
        #[arg(long)]
        name: Option<String>,

        /// Manually specify printer model (bypass SNMP discovery for network;
        /// override the queue name for USB driver matching)
        #[arg(long)]
        model: Option<String>,

        /// USB printer mode: target is a queue name, skip port creation,
        /// swap driver via Set-Printer instead of Add-Printer
        #[arg(long)]
        usb: bool,

        /// Disable the SDI (Snappy Driver Installer Origin) driver tier
        /// for this run. Falls through directly to Microsoft Update
        /// Catalog / IPP fallback.
        #[cfg(feature = "sdi")]
        #[arg(long)]
        no_sdi: bool,

        /// Disable the Microsoft Update Catalog driver tier for this
        /// run. Skips the catalog.update.microsoft.com HTTP scraper.
        #[arg(long)]
        no_catalog: bool,

        /// Allow auto-pick to trigger a first-run SDI pack download.
        /// Without this flag, uncached SDI packs are skipped with a
        /// visible warning. Use `prinstall sdi prefetch` to pre-cache
        /// instead if you prefer.
        #[cfg(feature = "sdi")]
        #[arg(long)]
        sdi_fetch: bool,
    },

    /// Remove a printer queue, with optional cleanup of driver and port
    ///
    /// Removes the target printer queue via Remove-Printer. If the driver
    /// is no longer used by any other printer, it's also removed from the
    /// driver store. Same for the TCP/IP port. Pass `--keep-driver` or
    /// `--keep-port` to skip those cleanup steps.
    ///
    /// The target can be either a printer IP (looked up via the `IP_<ip>`
    /// port name convention) or the printer queue name directly.
    #[command(
        after_help = "EXAMPLES:\n  \
            prinstall remove 192.168.1.100          Remove by IP (full cleanup)\n  \
            prinstall remove \"HP LaserJet Pro\"      Remove by queue name\n  \
            prinstall remove 192.168.1.100 --keep-driver\n  \
            prinstall remove 192.168.1.100 --keep-port --keep-driver\n\n\
            HOW IT WORKS:\n  \
            1. Resolves target → printer queue name\n  \
            2. Captures driver + port before removal\n  \
            3. Runs: Remove-Printer\n  \
            4. If no other printer uses the driver → Remove-PrinterDriver\n  \
            5. If no other printer uses the port → Remove-PrinterPort\n\n\
            Driver/port cleanup failures are non-fatal warnings. If the\n  \
            printer doesn't exist, the command succeeds (idempotent).\n\n\
            REQUIRES:\n  \
            Administrator privileges (UAC prompt if not elevated)."
    )]
    Remove {
        /// Printer IP address or queue name
        target: String,

        /// Don't remove the driver even if it's no longer used
        #[arg(long)]
        keep_driver: bool,

        /// Don't remove the TCP/IP port even if it's no longer used
        #[arg(long)]
        keep_port: bool,
    },

    /// List locally installed printers (USB, network, virtual)
    ///
    /// Enumerates every printer queue Windows already knows about via
    /// Get-Printer. Shows the queue name, driver, port, and the source
    /// (USB, network, or installed/virtual). This is the fastest way to
    /// audit what's on a machine before adding or removing anything.
    #[command(
        after_help = "EXAMPLES:\n  \
            prinstall list                  Show all installed printers\n  \
            prinstall list --json           Output as JSON (for scripting)\n  \
            prinstall list --verbose        Show full Get-Printer output\n\n\
            HOW IT WORKS:\n  \
            Runs Get-Printer via PowerShell, parses the structured output\n  \
            into the same Printer model scan uses, and prints:\n    \
            • Queue name            (e.g. \"Brother MFC-L2750DW series\")\n    \
            • Driver name           (e.g. \"Brother Laser Type1 Class Driver\")\n    \
            • Port name             (e.g. \"IP_192.168.1.50\" or \"USB001\")\n    \
            • Source                (network / USB / installed)\n\n\
            USE CASES:\n  \
            • RMM audit: pipe --json to a parser to check what's deployed\n  \
            • Pre-install check: see if a printer is already installed\n  \
            • Post-install verification: confirm a queue landed correctly\n  \
            • Troubleshooting: find a queue name for `prinstall remove`\n\n\
            NOTE:\n  \
            Unlike scan, list is local-only — it doesn't touch the network.\n  \
            No admin privileges required. Safe to run from any shell."
    )]
    List,

    /// Manage the SDI (Snappy Driver Installer Origin) driver cache
    ///
    /// SDI provides vendor-specific printer drivers for brands the
    /// Microsoft Update Catalog doesn't reliably carry (Brother, Canon,
    /// Epson, Ricoh, etc.). The SDI cache stores index files (.bin) and
    /// driver packs (.7z) fetched from the prinstall GitHub Releases
    /// mirror.
    #[cfg(feature = "sdi")]
    #[command(subcommand)]
    Sdi(SdiAction),
}

/// Actions for the `prinstall sdi` subcommand.
#[cfg(feature = "sdi")]
#[derive(Debug, Clone, clap::Subcommand)]
pub enum SdiAction {
    /// Show SDI cache status: indexes, cached packs, total size, mirror URL
    Status,
    /// Refresh SDI indexes from the configured mirror
    Refresh,
    /// List cached indexes and driver packs with sizes
    List,
    /// Download all driver packs from the mirror (pre-stage for offline use)
    Prefetch,
    /// Drop cached packs past the configured size budget (LRU eviction)
    Clean,
    /// Verify Authenticode signatures on .cat files in the extraction cache
    Verify,
}
