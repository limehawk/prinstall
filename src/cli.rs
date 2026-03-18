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
        prinstall scan                         Scan local subnet for printers\n  \
        prinstall scan 192.168.1.0/24          Scan a specific subnet\n  \
        prinstall id 192.168.1.100             Identify a printer by IP\n  \
        prinstall drivers 192.168.1.100        Show matched drivers for a printer\n  \
        prinstall install 192.168.1.100        Install printer with best-match driver\n  \
        prinstall install 192.168.1.100 --driver \"HP Universal Print Driver PCL6\"\n\n\
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
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Scan the local network for printers via SNMP
    ///
    /// Sends SNMP queries to every IP on the subnet to discover printers.
    /// Each discovered printer shows its IP, model, and status.
    ///
    /// Without a subnet argument, scans the local machine's subnet.
    /// Subnets larger than /24 require --force.
    #[command(
        after_help = "EXAMPLES:\n  \
            prinstall scan                     Scan local subnet\n  \
            prinstall scan 192.168.1.0/24      Scan specific subnet\n  \
            prinstall scan 10.0.0.0/24 --community private\n\n\
            HOW IT WORKS:\n  \
            Sends SNMP v2c GET requests to each IP on UDP port 161.\n  \
            Queries OID 1.3.6.1.2.1.25.3.2.1.3 (device description) to\n  \
            identify printer make and model. Max 64 concurrent probes,\n  \
            2-second timeout per host.\n\n\
            TROUBLESHOOTING:\n  \
            No results? Common causes:\n  \
            • SNMP disabled on printer — enable via printer web UI\n  \
            • Non-default community string — try --community <string>\n  \
            • Firewall blocking UDP 161 — check network rules"
    )]
    Scan {
        /// Subnet in CIDR notation (e.g., 192.168.1.0/24)
        subnet: Option<String>,
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

    /// Install a printer (port + driver + queue)
    ///
    /// Full printer installation: creates a TCP/IP port, installs the
    /// driver, and adds the printer queue. If no --driver is specified,
    /// auto-selects the best matched driver.
    #[command(
        after_help = "EXAMPLES:\n  \
            prinstall install 192.168.1.100\n  \
            prinstall install 192.168.1.100 --driver \"HP Universal Print Driver PCL6\"\n  \
            prinstall install 192.168.1.100 --name \"Front Desk Printer\"\n  \
            prinstall install 192.168.1.100 --model \"HP LaserJet\" --driver \"HP UPD\"\n\n\
            HOW IT WORKS:\n  \
            1. Identifies printer (SNMP or --model)\n  \
            2. Finds best driver (or uses --driver)\n  \
            3. Downloads driver if not locally staged\n  \
            4. Runs: Add-PrinterPort → Add-PrinterDriver → Add-Printer\n\n\
            REQUIRES:\n  \
            Administrator privileges (UAC prompt if not elevated).\n  \
            Existing ports/drivers are reused, not duplicated."
    )]
    Install {
        /// Printer IP address
        ip: String,

        /// Specific driver name to install (skip auto-matching)
        #[arg(long)]
        driver: Option<String>,

        /// Display name for the printer (default: model string)
        #[arg(long)]
        name: Option<String>,

        /// Manually specify printer model (bypass SNMP discovery)
        #[arg(long)]
        model: Option<String>,
    },
}
