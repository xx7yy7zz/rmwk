use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub ui: UiConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ui: UiConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_font")]
    pub font: String,
    #[serde(default = "default_extra_radius")]
    pub extra_radius: f64,
    #[serde(default = "default_use_symbolic_icons")]
    pub use_symbolic_icons: bool,
    #[serde(default = "default_bold_single_chars")]
    pub bold_single_chars: bool,
    #[serde(default = "default_center_layout")]
    pub center_layout: bool,
    #[serde(default = "default_disable_animations")]
    pub disable_animations: bool,
    #[serde(default = "default_enable_blur")]
    pub enable_blur: bool,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            font: default_font(),
            extra_radius: default_extra_radius(),
            use_symbolic_icons: default_use_symbolic_icons(),
            bold_single_chars: default_bold_single_chars(),
            center_layout: default_center_layout(),
            disable_animations: default_disable_animations(),
            enable_blur: default_enable_blur(),
        }
    }
}

fn default_theme() -> String {
    "system".to_string()
}

fn default_font() -> String {
    "Sans 11".to_string()
}

fn default_extra_radius() -> f64 {
    50.0
}

fn default_use_symbolic_icons() -> bool {
    false
}

fn default_bold_single_chars() -> bool {
    true
}

fn default_center_layout() -> bool {
    false
}

fn default_disable_animations() -> bool {
    false
}

fn default_enable_blur() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuItem {
    pub label: String,
    pub icon: Option<String>,
    pub action: Option<Action>,
    #[serde(default)]
    pub children: Vec<MenuItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Action {
    Command {
        cmd: String,
        #[serde(default)]
        keep_open: bool,
    },
    Hotkey {
        keys: String,
        #[serde(default)]
        keep_open: bool,
    },
}

impl Action {
    pub fn should_keep_open(&self) -> bool {
        match self {
            Action::Command { keep_open, .. } => *keep_open,
            Action::Hotkey { keep_open, .. } => *keep_open,
        }
    }
}

pub fn parse_hotkey(hotkey: &str) -> Result<Vec<String>, String> {
    let mut wtype_args = Vec::new();
    let parts: Vec<&str> = hotkey.split('+').map(|s| s.trim()).collect();
    if parts.is_empty() || parts.iter().all(|s| s.is_empty()) {
        return Err("Hotkey cannot be empty".to_string());
    }

    let mut modifiers = Vec::new();
    let mut key = None;

    for (i, &part) in parts.iter().enumerate() {
        if part.is_empty() {
            return Err("Missing key between '+'".to_string());
        }

        let is_last = i == parts.len() - 1;
        let lower = part.to_lowercase();

        let modifier = match lower.as_str() {
            "ctrl" | "control" => Some("ctrl"),
            "shift" => Some("shift"),
            "alt" | "mod1" => Some("alt"),
            "super" | "meta" | "win" | "windows" | "logo" | "mod4" => Some("logo"),
            _ => None,
        };

        if let Some(m) = modifier {
            if is_last && parts.len() > 1 {
                return Err(format!("Trailing modifier '{}' without a final key", part));
            }
            modifiers.push(m);
        } else {
            if !is_last {
                return Err(format!(
                    "'{}' is not a valid modifier. Only the last item can be a regular key.",
                    part
                ));
            }
            key = Some(part);
        }
    }

    if let Some(k) = key {
        for &m in &modifiers {
            wtype_args.push("-M".to_string());
            wtype_args.push(m.to_string());
        }
        wtype_args.push("-k".to_string());
        wtype_args.push(k.to_string());
        for &m in modifiers.iter().rev() {
            wtype_args.push("-m".to_string());
            wtype_args.push(m.to_string());
        }
    } else if !modifiers.is_empty() {
        return Err("Missing a key to press".to_string());
    }

    Ok(wtype_args)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuConfig {
    pub menu: Vec<MenuItem>,
}

pub fn load_config<P: AsRef<Path>>(path: P) -> Result<Config> {
    let content = fs::read_to_string(path).context("Failed to read UI config file")?;
    let config: Config = toml::from_str(&content).context("Failed to parse UI config TOML")?;
    Ok(config)
}

pub fn load_menu<P: AsRef<Path>>(path: P) -> Result<MenuConfig> {
    let content = fs::read_to_string(path).context("Failed to read menu config file")?;
    let menu: MenuConfig = toml::from_str(&content).context("Failed to parse menu TOML")?;
    Ok(menu)
}

pub fn save_menu<P: AsRef<Path>>(path: P, menu: &MenuConfig) -> Result<()> {
    let content = toml::to_string_pretty(menu).context("Failed to serialize menu to TOML")?;
    if let Some(parent) = path.as_ref().parent() {
        fs::create_dir_all(parent).context("Failed to create parent directories for menu file")?;
    }
    fs::write(path, content).context("Failed to write menu TOML to file")?;
    Ok(())
}

pub fn run_action(action: &Action) -> Result<()> {
    match action {
        Action::Command { cmd, .. } => {
            let mut child = std::process::Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .spawn()
                .context("Failed to spawn command")?;

            // Reap the child in a background thread to prevent zombie (defunct) processes
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Action::Hotkey { keys, .. } => {
            match parse_hotkey(keys) {
                Ok(args) => {
                    std::thread::spawn(move || {
                        // Wait for the launcher window to close and relinquish Wayland focus
                        std::thread::sleep(std::time::Duration::from_millis(350));

                        let mut child =
                            match std::process::Command::new("wtype").args(&args).spawn() {
                                Ok(c) => c,
                                Err(e) => {
                                    eprintln!("Failed to spawn wtype. Is it installed? {}", e);
                                    return;
                                }
                            };
                        let _ = child.wait();
                    });
                }
                Err(e) => {
                    eprintln!("Failed to parse hotkey '{}': {}", keys, e);
                }
            }
        }
    }
    Ok(())
}

pub fn load_material_codepoints<P: AsRef<Path>>(
    config_path: P,
) -> std::collections::HashMap<String, char> {
    let mut map = std::collections::HashMap::new();
    let path = config_path
        .as_ref()
        .parent()
        .unwrap_or_else(|| Path::new("/home/karim/.config/rmwk"))
        .join("MaterialSymbolsRounded.codepoints");

    if let Ok(content) = fs::read_to_string(&path) {
        for line in content.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() == 2 {
                let name = parts[0].to_string();
                if let Ok(code_val) = u32::from_str_radix(parts[1], 16) {
                    if let Some(c) = char::from_u32(code_val) {
                        map.insert(name, c);
                    }
                }
            }
        }
    }
    map
}
