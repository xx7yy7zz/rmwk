use clap::{Parser, Subcommand};
use launcher_ipc::IpcMessage;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, error, info, Level};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "rmwk")]
#[command(version)]
#[command(about = "A fast, native radial launcher for Wayland (rmwk)", long_about = None)]
#[command(after_help = "With no subcommand, rmwk starts the daemon hidden in the background.\nPass --menu <file> with no subcommand to open that menu instead.")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Path to the menu configuration TOML file
    #[arg(long, global = true)]
    menu: Option<PathBuf>,
}

#[derive(Subcommand, Clone)]
enum Commands {
    /// Open or toggle a menu (needs a menu name or --menu <file>)
    Open {
        menu_name: Option<String>,
    },
    /// Open the settings GUI editor
    Settings,
    /// Reload config and themes of the running instance
    Reload,
}

fn init_logging() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(Level::INFO.into())
                .from_env_lossy(),
        )
        .init();
}

fn ensure_default_configs(menu_path: &Path, config_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = menu_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
        
        // Migration: move old menu.toml to menus/menu.toml if it exists
        if let Some(base_dir) = parent.parent() {
            let old_menu = base_dir.join("menu.toml");
            if old_menu.exists() && !menu_path.exists() {
                info!("Migrating old menu.toml to menus/menu.toml");
                fs::rename(&old_menu, menu_path)?;
            }
        }
    }

    if !config_path.exists() {
        info!("Writing default config.toml to {:?}", config_path);
        let default_config = r#"# Default UI configuration
[ui]
theme = "default"
font = "Sans"
"#;
        fs::write(config_path, default_config)?;
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    launcher_core::init_process_reaper();
    init_logging();
    // Register the embedded Material Symbols font before any text is laid
    // out, so glyphs render without a system/config-dir font install.
    launcher_core::register_embedded_font();

    let cli = Cli::parse();
    let cli_menu_was_none = cli.menu.is_none();
    let command = cli.command.clone();

    // Resolve config and menu paths using centralized XDG paths
    let menu_path = cli.menu.unwrap_or_else(launcher_core::paths::get_default_menu_path);
    let config_path = launcher_core::paths::get_default_config_path();

    if let Err(e) = ensure_default_configs(&menu_path, &config_path) {
        error!("Failed to initialize default configuration files: {}", e);
    }

    match command {
        Some(Commands::Open { menu_name }) => {
            if menu_name.is_none() && cli_menu_was_none {
                info!(
                    "No menu specified. Use 'rmwk open <menu>' or 'rmwk --menu <file>', \
                     or run 'rmwk' alone to start the daemon."
                );
                return Ok(());
            }

            let mut resolved = menu_path.clone();
            if let Some(name) = &menu_name {
                resolved = launcher_core::paths::get_config_dir()
                    .join("menus")
                    .join(format!("{}.toml", name));
            }
            open_or_become(&resolved, &config_path)?;
        }
        None => {
            // Bare `rmwk` starts the daemon; `rmwk --menu <file>` opens that menu.
            if cli_menu_was_none {
                start_daemon(&menu_path, &config_path)?;
            } else {
                open_or_become(&menu_path, &config_path)?;
            }
        }
        Some(Commands::Settings) => {
            info!("Starting settings window...");
            let mut resolved_menu_path = menu_path;
            if cli_menu_was_none {
                if let Ok(cfg) = launcher_core::load_config(&config_path) {
                    if let Some(last) = cfg.last_edited_menu {
                        let potential_path = launcher_core::paths::get_config_dir()
                            .join("menus")
                            .join(format!("{}.toml", last));
                        if potential_path.exists() {
                            resolved_menu_path = potential_path;
                        }
                    }
                }
            }
            let app = launcher_settings_ui::SettingsApp::new(resolved_menu_path, config_path);
            let exit_code = app.run();
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
        }
        Some(Commands::Reload) => {
            let socket_path = launcher_ipc::get_socket_path();
            if socket_path.exists() {
                match launcher_ipc::send_message_sync(&socket_path, &IpcMessage::ReloadConfig) {
                    Ok(_) => {
                        info!("Sent ReloadConfig command to running instance.");
                    }
                    Err(e) => {
                        error!("Failed to communicate with running instance: {}", e);
                    }
                }
            } else {
                error!("No running instance socket found at {:?}", socket_path);
            }
        }
    }

    Ok(())
}

/// Toggle a menu on the running instance over IPC, or become the instance
/// ourselves (visible) when no daemon is alive.
fn open_or_become(menu_path: &Path, config_path: &Path) -> anyhow::Result<()> {
    if !menu_path.exists() {
        error!("Specified menu file does not exist: {:?}", menu_path);
        return Ok(());
    }

    let socket_path = launcher_ipc::get_socket_path();
    let toggle_succeeded = if socket_path.exists() {
        debug!("Socket file exists at {:?}, attempting to send OpenMenu command", socket_path);
        match launcher_ipc::send_message_sync(
            &socket_path,
            &IpcMessage::OpenMenu { menu_path: menu_path.to_path_buf() },
        ) {
            Ok(_) => {
                info!("Toggled running instance of rmwk with new menu");
                true
            }
            Err(e) => {
                debug!("Could not connect to existing socket: {}. Stale socket will be cleaned up by server.", e);
                false
            }
        }
    } else {
        false
    };

    if toggle_succeeded {
        return Ok(());
    }

    info!("Starting new launcher window...");
    let app = launcher_ui::LauncherApp::new(menu_path.to_path_buf(), config_path.to_path_buf(), false);
    let exit_code = app.run();
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

/// Start the daemon hidden, owning the IPC socket and tray icon.
fn start_daemon(menu_path: &Path, config_path: &Path) -> anyhow::Result<()> {
    let socket_path = launcher_ipc::get_socket_path();
    // Pure connect probe: sending any message would disturb the running
    // instance (e.g. popping the menu open).
    let is_alive = socket_path.exists()
        && std::os::unix::net::UnixStream::connect(&socket_path).is_ok();
    if is_alive {
        error!("Daemon is already running!");
        std::process::exit(1);
    }

    info!("Starting rmwk daemon...");
    let app = launcher_ui::LauncherApp::new(menu_path.to_path_buf(), config_path.to_path_buf(), true);
    let exit_code = app.run();
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}
