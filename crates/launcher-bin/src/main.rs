use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};
use std::fs;
use tracing::{info, debug, error, Level};
use tracing_subscriber::EnvFilter;
use launcher_ipc::IpcMessage;

#[derive(Parser)]
#[command(name = "radial-launcher")]
#[command(version)]
#[command(about = "A fast, native radial launcher for Wayland", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Path to the menu configuration TOML file
    #[arg(long, global = true)]
    menu: Option<PathBuf>,

    /// Path to the UI configuration TOML file
    #[arg(long, global = true)]
    config: Option<PathBuf>,
}

#[derive(Subcommand, Clone)]
enum Commands {
    /// Open the radial launcher menu (default)
    Open,
    /// Open the settings GUI editor
    Settings,
    /// Reload config and themes of the running instance
    Reload,
    /// Start the radial launcher daemon explicitly (starts hidden)
    Daemon,
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

fn get_default_paths() -> (PathBuf, PathBuf) {
    let base_dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("/home/karim/.config"))
        .join("radial-launcher");
    
    (
        base_dir.join("menu.toml"),
        base_dir.join("config.toml")
    )
}

fn ensure_default_configs(menu_path: &Path, config_path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = menu_path.parent() {
        fs::create_dir_all(parent)?;
    }

    if !menu_path.exists() {
        info!("Writing default menu.toml to {:?}", menu_path);
        let default_menu = r#"# Default menu configuration
[[menu]]
label = "Apps"
icon = "application-x-executable"

  [[menu.children]]
  label = "Terminal"
  icon = "utilities-terminal"
  action = { type = "exec", cmd = "foot" }

  [[menu.children]]
  label = "Browser"
  icon = "firefox"
  action = { type = "exec", cmd = "firefox" }

[[menu]]
label = "System"
icon = "preferences-desktop"

  [[menu.children]]
  label = "Reload config"
  icon = "view-refresh"
  action = { type = "shell", cmd = "swaymsg reload" }
"#;
        fs::write(menu_path, default_menu)?;
    }

    if !config_path.exists() {
        info!("Writing default config.toml to {:?}", config_path);
        let default_config = r#"# Default UI configuration
[ui]
theme = "default"
font = "Sans 11"
"#;
        fs::write(config_path, default_config)?;
    }

    Ok(())
}

fn main() -> anyhow::Result<()> {
    init_logging();

    let cli = Cli::parse();
    let command = cli.command.clone().unwrap_or(Commands::Open);

    // Resolve config and menu paths
    let (def_menu, def_config) = get_default_paths();
    let menu_path = cli.menu.unwrap_or(def_menu);
    let config_path = cli.config.unwrap_or(def_config);

    if let Err(e) = ensure_default_configs(&menu_path, &config_path) {
        error!("Failed to initialize default configuration files: {}", e);
    }

    match command {
        Commands::Open => {
            // Check if there is an active running instance we can toggle
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            
            let socket_path = launcher_ipc::get_socket_path();
            let toggle_succeeded = rt.block_on(async {
                if socket_path.exists() {
                    debug!("Socket file exists at {:?}, attempting to send Toggle command", socket_path);
                    match launcher_ipc::send_message(&socket_path, &IpcMessage::Toggle).await {
                        Ok(_) => {
                            info!("Toggled running instance of radial-launcher");
                            true
                        }
                        Err(e) => {
                            debug!("Could not connect to existing socket: {}. Stale socket will be cleaned up by server.", e);
                            false
                        }
                    }
                } else {
                    false
                }
            });

            if toggle_succeeded {
                return Ok(());
            }

            info!("Starting new launcher window...");
            let app = launcher_ui::LauncherApp::new(menu_path, config_path, false);
            let exit_code = app.run();
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
        }
        Commands::Settings => {
            info!("Starting settings window...");
            let app = launcher_settings_ui::SettingsApp::new(menu_path, config_path);
            let exit_code = app.run();
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
        }
        Commands::Reload => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let socket_path = launcher_ipc::get_socket_path();
            rt.block_on(async {
                if socket_path.exists() {
                    match launcher_ipc::send_message(&socket_path, &IpcMessage::ReloadConfig).await {
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
            });
        }
        Commands::Daemon => {
            let socket_path = launcher_ipc::get_socket_path();
            if socket_path.exists() {
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                let is_alive = rt.block_on(async {
                    launcher_ipc::send_message(&socket_path, &IpcMessage::Open).await.is_ok()
                });
                if is_alive {
                    error!("Daemon is already running!");
                    std::process::exit(1);
                }
            }

            info!("Starting radial launcher daemon...");
            let app = launcher_ui::LauncherApp::new(menu_path, config_path, true);
            let exit_code = app.run();
            if exit_code != 0 {
                std::process::exit(exit_code);
            }
        }
    }

    Ok(())
}
