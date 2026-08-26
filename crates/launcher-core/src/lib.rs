use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

pub mod paths {
    use std::path::PathBuf;

    /// Resolves the base rmwk config directory following XDG specification:
    /// 1. $XDG_CONFIG_HOME/rmwk
    /// 2. $HOME/.config/rmwk
    /// 3. /tmp/rmwk (fallback)
    pub fn get_config_dir() -> PathBuf {
        dirs::config_dir()
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("rmwk")
    }

    pub fn get_default_menu_path() -> PathBuf {
        get_config_dir().join("menus").join("menu.toml")
    }

    pub fn get_default_config_path() -> PathBuf {
        get_config_dir().join("config.toml")
    }

    pub fn get_themes_dir() -> PathBuf {
        get_config_dir().join("themes")
    }

    pub fn get_fonts_dir() -> PathBuf {
        get_config_dir().join("fonts")
    }

    pub fn get_font_file() -> PathBuf {
        get_fonts_dir().join("MaterialSymbolsRounded.ttf")
    }

    pub fn get_codepoints_file() -> PathBuf {
        get_config_dir().join("MaterialSymbolsRounded.codepoints")
    }

    pub fn get_tags_file() -> PathBuf {
        get_config_dir().join("MaterialSymbolsRounded.tags")
    }

    pub fn get_gtk_css_path() -> Option<PathBuf> {
        dirs::config_dir().map(|p| p.join("gtk-4.0").join("gtk.css"))
    }
}

/// Automatically configure kernel process reaping for child processes
/// without creating zombie/defunct processes or spawning background OS threads.
pub fn init_process_reaper() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGCHLD, libc::SIG_IGN);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub last_edited_menu: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ui: UiConfig::default(),
            last_edited_menu: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemThemeColor {
    pub variable: String,
    pub opacity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemThemeOverrides {
    #[serde(alias = "slice_normal")]
    pub entry_surface: SystemThemeColor,
    #[serde(alias = "slice_hover")]
    pub entry_surface_hover: SystemThemeColor,
    #[serde(alias = "slice_active")]
    pub entry_border: SystemThemeColor,
    #[serde(alias = "slice_selected")]
    pub entry_border_hover: SystemThemeColor,
    #[serde(alias = "label_normal")]
    pub label: SystemThemeColor,
    pub label_hover: SystemThemeColor,
    pub entry_icon: SystemThemeColor,
    pub entry_icon_hover: SystemThemeColor,
    #[serde(default = "default_floating_icon_surface")]
    pub floating_icon_surface: SystemThemeColor,
    #[serde(default = "default_floating_icon_surface_hover")]
    pub floating_icon_surface_hover: SystemThemeColor,
    #[serde(alias = "hub_normal")]
    pub hub_surface: SystemThemeColor,
    #[serde(alias = "hub_active")]
    pub hub_border: SystemThemeColor,
    #[serde(alias = "hub_hover")]
    pub hub_label: SystemThemeColor,
    pub hub_icon: SystemThemeColor,
    #[serde(alias = "outer_border")]
    pub pie_outer_border: SystemThemeColor,
}

impl Default for SystemThemeOverrides {
    fn default() -> Self {
        Self {
            entry_surface: SystemThemeColor { variable: "@theme_bg_color".to_string(), opacity: 0.85 },
            entry_surface_hover: SystemThemeColor { variable: "@theme_selected_bg_color".to_string(), opacity: 0.70 },
            entry_border: SystemThemeColor { variable: "@theme_selected_bg_color".to_string(), opacity: 0.15 },
            entry_border_hover: SystemThemeColor { variable: "@theme_selected_bg_color".to_string(), opacity: 1.0 },
            label: SystemThemeColor { variable: "@theme_text_color".to_string(), opacity: 1.0 },
            label_hover: SystemThemeColor { variable: "@theme_base_color".to_string(), opacity: 0.80 },
            entry_icon: SystemThemeColor { variable: "@theme_text_color".to_string(), opacity: 1.0 },
            entry_icon_hover: SystemThemeColor { variable: "@theme_bg_color".to_string(), opacity: 1.0 },
            floating_icon_surface: SystemThemeColor { variable: "@theme_bg_color".to_string(), opacity: 1.0 },
            floating_icon_surface_hover: SystemThemeColor { variable: "@theme_selected_bg_color".to_string(), opacity: 1.0 },
            hub_surface: SystemThemeColor { variable: "@theme_bg_color".to_string(), opacity: 1.0 },
            hub_border: SystemThemeColor { variable: "@theme_selected_bg_color".to_string(), opacity: 1.0 },
            hub_label: SystemThemeColor { variable: "@theme_text_color".to_string(), opacity: 1.0 },
            hub_icon: SystemThemeColor { variable: "@theme_text_color".to_string(), opacity: 1.0 },
            pie_outer_border: SystemThemeColor { variable: "@theme_selected_bg_color".to_string(), opacity: 1.0 },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_font")]
    pub font: String,
    #[serde(default = "default_menu_style")]
    pub menu_style: String,
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default = "default_extra_radius")]
    pub extra_radius: f64,
    #[serde(default = "default_pill_roundness")]
    pub pill_roundness: f64,
    /// Dynamically grow the gap between hub and entry ring in pie mode when
    /// there are many entries (mirrors floating mode's behavior).
    #[serde(default = "default_enable_pie_spacing")]
    pub enable_pie_spacing: bool,
    #[serde(default = "default_use_symbolic_icons")]
    pub use_symbolic_icons: bool,
    #[serde(default = "default_bold_single_chars")]
    pub bold_single_chars: bool,
    #[serde(default = "default_center_layout")]
    pub center_layout: bool,

    #[serde(default = "default_disable_hover_animation")]
    pub disable_hover_animation: bool,
    #[serde(default = "default_hover_visual_cue")]
    pub hover_visual_cue: String,
    #[serde(default = "default_enable_blur")]
    pub enable_blur: bool,
    #[serde(default = "default_system_theme_overrides")]
    pub system_theme_overrides: Option<SystemThemeOverrides>,
    #[serde(default = "default_hide_back_entry")]
    pub hide_back_entry: bool,
}

fn default_scale() -> f64 { 1.0 }

fn default_hide_back_entry() -> bool { false }

fn default_system_theme_overrides() -> Option<SystemThemeOverrides> {
    None
}

fn default_floating_icon_surface() -> SystemThemeColor {
    SystemThemeColor { variable: "@theme_bg_color".to_string(), opacity: 1.0 }
}

fn default_floating_icon_surface_hover() -> SystemThemeColor {
    SystemThemeColor { variable: "@theme_selected_bg_color".to_string(), opacity: 1.0 }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            font: default_font(),
            menu_style: default_menu_style(),
            scale: default_scale(),
            extra_radius: default_extra_radius(),
            pill_roundness: default_pill_roundness(),
            enable_pie_spacing: default_enable_pie_spacing(),
            use_symbolic_icons: default_use_symbolic_icons(),
            bold_single_chars: default_bold_single_chars(),
            center_layout: default_center_layout(),

            disable_hover_animation: default_disable_hover_animation(),
            hover_visual_cue: default_hover_visual_cue(),
            enable_blur: default_enable_blur(),
            system_theme_overrides: default_system_theme_overrides(),
            hide_back_entry: default_hide_back_entry(),
        }
    }
}

fn default_theme() -> String {
    "system".to_string()
}

fn default_font() -> String {
    "Sans 11".to_string()
}

fn default_menu_style() -> String {
    "pie".to_string()
}

fn default_extra_radius() -> f64 {
    50.0
}

/// Roundness factor for the floating mode pills: 1.0 = full capsule,
/// 0.0 = sharp rectangle. Applies to the pill corners and the icon tile.
fn default_pill_roundness() -> f64 {
    1.0
}

/// Dynamic hub-to-ring spacing in pie mode, like floating mode.
fn default_enable_pie_spacing() -> bool {
    false
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



fn default_disable_hover_animation() -> bool {
    false
}

fn default_hover_visual_cue() -> String {
    "outwards".to_string()
}

fn default_enable_blur() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MenuItem {
    pub label: String,
    pub icon: Option<String>,
    pub action: Option<Action>,
    pub quick_select_key: Option<char>,
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
    OpenUri {
        uri: String,
        #[serde(default)]
        keep_open: bool,
    },
    OpenPath {
        path: String,
        #[serde(default)]
        keep_open: bool,
    },
}

impl Action {
    pub fn should_keep_open(&self) -> bool {
        match self {
            Action::Command { keep_open, .. } => *keep_open,
            Action::Hotkey { keep_open, .. } => *keep_open,
            Action::OpenUri { keep_open, .. } => *keep_open,
            Action::OpenPath { keep_open, .. } => *keep_open,
        }
    }
}

pub fn parse_hotkey(hotkey: &str) -> Result<Vec<String>, String> {
    let hotkey = hotkey.trim();
    if hotkey.is_empty() {
        return Err("Hotkey cannot be empty".to_string());
    }

    // Handle trailing "+" or "++" (e.g., "ctrl++" or "+")
    let (prefix, key_part) = if hotkey.ends_with("++") {
        (&hotkey[..hotkey.len() - 2], "+")
    } else if hotkey == "+" {
        ("", "+")
    } else if let Some(last_plus) = hotkey.rfind('+') {
        (&hotkey[..last_plus], hotkey[last_plus + 1..].trim())
    } else {
        ("", hotkey)
    };

    let mut modifiers = Vec::new();
    if !prefix.is_empty() {
        for part in prefix.split('+').map(|s| s.trim()) {
            if part.is_empty() {
                continue;
            }
            let mod_name = match part.to_lowercase().as_str() {
                "ctrl" | "control" => "ctrl",
                "shift" => "shift",
                "alt" | "mod1" => "alt",
                "super" | "meta" | "win" | "windows" | "logo" | "mod4" => "logo",
                "altgr" | "mod5" => "mod5",
                _ => return Err(format!("'{}' is not a valid modifier.", part)),
            };
            modifiers.push(mod_name);
        }
    }

    if key_part.is_empty() {
        return Err("Missing a key to press".to_string());
    }

    let normalized_key = match key_part.to_lowercase().as_str() {
        "enter" | "return" => "Return",
        "esc" | "escape" => "Escape",
        "space" => "space",
        "tab" => "Tab",
        "backspace" => "BackSpace",
        "delete" => "Delete",
        "up" => "Up",
        "down" => "Down",
        "left" => "Left",
        "right" => "Right",
        "+" | "plus" => "plus",
        "-" | "minus" => "minus",
        "=" | "equal" => "equal",
        _ => key_part,
    };

    let mut wtype_args = Vec::new();
    for &m in &modifiers {
        wtype_args.push("-M".to_string());
        wtype_args.push(m.to_string());
    }
    wtype_args.push("-k".to_string());
    wtype_args.push(normalized_key.to_string());
    for &m in modifiers.iter().rev() {
        wtype_args.push("-m".to_string());
        wtype_args.push(m.to_string());
    }

    Ok(wtype_args)
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MenuConfig {
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
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
            std::process::Command::new("sh")
                .arg("-c")
                .arg(cmd)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .context("Failed to spawn command")?;
            // With SIG_IGN on SIGCHLD (configured via init_process_reaper),
            // the kernel auto-reaps child processes without thread overhead.
        }
        Action::Hotkey { keys, .. } => {
            match parse_hotkey(keys) {
                Ok(args) => {
                    std::thread::spawn(move || {
                        // Wait for launcher window to close and relinquish Wayland focus
                        std::thread::sleep(std::time::Duration::from_millis(100));

                        let _ = std::process::Command::new("wtype")
                            .args(&args)
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null())
                            .spawn();
                    });
                }
                Err(e) => {
                    tracing::error!("Failed to parse hotkey '{}': {}", keys, e);
                }
            }
        }
        Action::OpenUri { uri, .. } => {
            spawn_xdg_open(uri.clone())?;
        }
        Action::OpenPath { path, .. } => {
            // Expand a leading ~ so xdg-open resolves it correctly
            let target = if let Some(rest) = path.strip_prefix("~/") {
                match std::env::var_os("HOME") {
                    Some(home) => format!("{}/{}", home.to_string_lossy(), rest),
                    None => path.clone(),
                }
            } else {
                path.clone()
            };
            spawn_xdg_open(target)?;
        }
    }
    Ok(())
}

fn spawn_xdg_open(target: String) -> Result<()> {
    std::process::Command::new("xdg-open")
        .arg(target)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("Failed to spawn xdg-open")?;
    Ok(())
}

pub fn load_material_codepoints<P: AsRef<Path>>(
    config_path: P,
) -> std::collections::HashMap<String, char> {
    let mut map = std::collections::HashMap::new();
    let path = config_path
        .as_ref()
        .parent()
        .map(|p| p.join("MaterialSymbolsRounded.codepoints"))
        .unwrap_or_else(paths::get_codepoints_file);

    let resolved_path = if path.exists() {
        path
    } else {
        paths::get_codepoints_file()
    };

    if let Ok(content) = fs::read_to_string(&resolved_path) {
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

pub fn load_material_tags<P: AsRef<Path>>(
    config_path: P,
) -> std::collections::HashMap<String, Vec<String>> {
    let mut map = std::collections::HashMap::new();
    let path = config_path
        .as_ref()
        .parent()
        .map(|p| p.join("MaterialSymbolsRounded.tags"))
        .unwrap_or_else(paths::get_tags_file);

    let resolved_path = if path.exists() {
        path
    } else {
        paths::get_tags_file()
    };

    if let Ok(content) = fs::read_to_string(&resolved_path) {
        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(colon_idx) = line.find(':') {
                let name = line[..colon_idx].trim().to_string();
                let tags_str = &line[colon_idx + 1..];
                let tags: Vec<String> = tags_str
                    .split(',')
                    .map(|s| s.trim().to_lowercase())
                    .filter(|s| !s.is_empty())
                    .collect();
                map.entry(name).or_insert_with(Vec::new).extend(tags);
            }
        }
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hotkey_combinations() {
        assert_eq!(
            parse_hotkey("ctrl+c").unwrap(),
            vec!["-M", "ctrl", "-k", "c", "-m", "ctrl"]
        );
        assert_eq!(
            parse_hotkey("ctrl+shift+t").unwrap(),
            vec!["-M", "ctrl", "-M", "shift", "-k", "t", "-m", "shift", "-m", "ctrl"]
        );
        assert_eq!(
            parse_hotkey("ctrl++").unwrap(),
            vec!["-M", "ctrl", "-k", "plus", "-m", "ctrl"]
        );
        assert_eq!(
            parse_hotkey("+").unwrap(),
            vec!["-k", "plus"]
        );
        assert_eq!(
            parse_hotkey("Return").unwrap(),
            vec!["-k", "Return"]
        );
        assert_eq!(
            parse_hotkey("super+space").unwrap(),
            vec!["-M", "logo", "-k", "space", "-m", "logo"]
        );
    }

    #[test]
    fn test_uri_and_path_action_roundtrip() {
        let menu: MenuConfig = toml::from_str(
            r#"
[[menu]]
label = "Site"
action = { type = "open_uri", uri = "https://example.com" }

[[menu]]
label = "Docs"
action = { type = "open_path", path = "~/Documents", keep_open = true }
"#,
        )
        .unwrap();

        assert_eq!(
            menu.menu[0].action,
            Some(Action::OpenUri {
                uri: "https://example.com".to_string(),
                keep_open: false
            })
        );
        assert_eq!(
            menu.menu[1].action,
            Some(Action::OpenPath {
                path: "~/Documents".to_string(),
                keep_open: true
            })
        );
        assert!(!menu.menu[0].action.as_ref().unwrap().should_keep_open());
        assert!(menu.menu[1].action.as_ref().unwrap().should_keep_open());

        let out = toml::to_string_pretty(&menu).unwrap();
        assert!(out.contains("open_uri"));
        assert!(out.contains("open_path"));
    }

    #[test]
    fn test_ui_config_scale_roundtrip() {
        let default_cfg: Config = toml::from_str("").unwrap();
        assert_eq!(default_cfg.ui.scale, 1.0);

        let custom_cfg: Config = toml::from_str(
            r#"
[ui]
scale = 1.35
"#,
        )
        .unwrap();
        assert_eq!(custom_cfg.ui.scale, 1.35);

        let out = toml::to_string_pretty(&custom_cfg).unwrap();
        assert!(out.contains("scale = 1.35"));
    }
}
