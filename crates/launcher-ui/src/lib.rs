use gdk::prelude::*;
use gdk4 as gdk;
use gtk::prelude::*;
use gtk4 as gtk;
pub mod wayland;

use gtk4_layer_shell::{KeyboardMode, Layer, LayerShell};
use launcher_ipc::IpcMessage;
use pangocairo;
use std::cell::RefCell;
use std::collections::HashMap;
use std::f64::consts::PI;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use tracing::{debug, error, info, warn};

const BASE_R: f64 = 80.0;
const SLICE_WIDTH: f64 = 110.0;
const HOVER_GROW: f64 = 15.0;

const MATERIAL_ICON_WEIGHT: f64 = 400.0;

// Trace a rounded rectangle from (rx0, ry0) with the given size and corner radius.
fn rounded_rect_path(cr: &cairo::Context, rx0: f64, ry0: f64, rw: f64, rh: f64, rad: f64) {
    cr.new_path();
    if rad < 0.5 {
        cr.rectangle(rx0, ry0, rw, rh);
        cr.close_path();
        return;
    }
    let cx1 = rx0 + rw;
    let cy1 = ry0 + rh;
    cr.move_to(rx0 + rad, ry0);
    cr.arc(cx1 - rad, ry0 + rad, rad, -std::f64::consts::PI / 2.0, 0.0);
    cr.arc(cx1 - rad, cy1 - rad, rad, 0.0, std::f64::consts::PI / 2.0);
    cr.arc(
        rx0 + rad,
        cy1 - rad,
        rad,
        std::f64::consts::PI / 2.0,
        std::f64::consts::PI,
    );
    cr.arc(
        rx0 + rad,
        ry0 + rad,
        rad,
        std::f64::consts::PI,
        3.0 * std::f64::consts::PI / 2.0,
    );
    cr.close_path();
}

#[derive(Clone)]
struct ThemeColors {
    fill_color: gtk::gdk::RGBA,
    hover_fill_color: gtk::gdk::RGBA,
    border_color: gtk::gdk::RGBA,
    hover_border_color: gtk::gdk::RGBA,
    label_color: gtk::gdk::RGBA,
    hover_label_color: gtk::gdk::RGBA,
    icon_color: gtk::gdk::RGBA,
    hover_icon_color: gtk::gdk::RGBA,
    icon_tile_color: gtk::gdk::RGBA,
    hover_icon_tile_color: gtk::gdk::RGBA,
    hub_fill: gtk::gdk::RGBA,
    hub_border: gtk::gdk::RGBA,
    hub_text_color: gtk::gdk::RGBA,
    hub_icon_color: gtk::gdk::RGBA,
    outer_border_color: gtk::gdk::RGBA,
}

struct MenuState {
    current_menu_path: PathBuf,
    root_items: Vec<launcher_core::MenuItem>,
    current_items: Vec<launcher_core::MenuItem>,
    history: Vec<Vec<launcher_core::MenuItem>>,
    forward_history: Vec<Vec<launcher_core::MenuItem>>,
    root_icon: Option<String>,
    current_icon: Option<String>,
    history_icons: Vec<Option<String>>,
    forward_history_icons: Vec<Option<String>>,
    hovered_index: Option<usize>,
    hide_back_entry: bool,

    // Animation state
    is_closing: bool,
    hover_progresses: Vec<f64>, // 0.0 -> 1.0 for each slice

    // Cached icons to avoid loading on every frame tick
    icon_cache: HashMap<String, Option<cairo::ImageSurface>>,

    // Cached Pango layouts for single char icons (avoids shaping every frame)
    text_layout_cache: HashMap<(String, u32), gtk::pango::Layout>,

    // Cached Pango layouts for Material Symbols glyphs (avoids shaping every frame)
    material_layout_cache: HashMap<(String, u32), gtk::pango::Layout>,

    // Cached Pango layouts for slice labels
    label_layout_cache: HashMap<String, gtk::pango::Layout>,

    // Extra interactivity margin beyond slices
    extra_radius: f64,
    pill_roundness: f64,
    use_symbolic_icons: bool,
    bold_single_chars: bool,
    center_layout: bool,
    disable_hover_animation: bool,
    hover_visual_cue: String,
    menu_style: String,
    enable_blur: bool,
    last_cx: f64,
    last_cy: f64,
    last_blur_radius: f64,

    // Material Symbols codepoints index
    codepoints: HashMap<String, char>,

    // Cached theme colors
    theme_colors: std::cell::RefCell<Option<ThemeColors>>,

    // Temporarily suppress close-on-focus-loss for hotkey macros
    suppress_focus_loss: std::rc::Rc<std::cell::Cell<bool>>,

    // Keep monitors alive
    _config_monitor: Option<gtk::gio::FileMonitor>,
    _menu_monitor: Option<gtk::gio::FileMonitor>,
}

impl MenuState {
    fn reset_to_root(&mut self) {
        self.current_items = self.root_items.clone();
        self.current_icon = self.root_icon.clone();
        self.history.clear();
        self.forward_history.clear();
        self.history_icons.clear();
        self.forward_history_icons.clear();
        self.hovered_index = None;
    }

    fn get_display_items(&self) -> Vec<launcher_core::MenuItem> {
        let mut items = self.current_items.clone();
        if !self.history.is_empty() && !self.hide_back_entry {
            items.push(launcher_core::MenuItem {
                label: "Back".to_string(),
                icon: Some("go-previous".to_string()),
                action: None,
                quick_select_key: Some('B'), // Added quick select for "Back"
                children: vec![],
            });
        }
        items
    }

    fn get_display_items_count(&self) -> usize {
        if self.history.is_empty() || self.hide_back_entry {
            self.current_items.len()
        } else {
            self.current_items.len() + 1
        }
    }

    fn preload_icons(&mut self, display: &gdk::Display) {
        let display_items = self.get_display_items();
        let mut icons_to_load = Vec::new();
        if let Some(icon) = &self.current_icon {
            icons_to_load.push(icon.clone());
        }
        for item in &display_items {
            if let Some(icon) = &item.icon {
                icons_to_load.push(icon.clone());
            }
        }
        for raw_icon_name in &icons_to_load {
            if self.codepoints.contains_key(raw_icon_name) {
                continue;
            }
            if !self.icon_cache.contains_key(raw_icon_name) {
                let is_sys_forced = raw_icon_name.starts_with("sys:");
                let icon_name = if is_sys_forced {
                    &raw_icon_name[4..]
                } else {
                    raw_icon_name.as_str()
                };

                let pixbuf = load_icon_pixbuf(display, icon_name, 128, self.use_symbolic_icons);
                let surface = pixbuf.and_then(|p| {
                    let format = if p.has_alpha() {
                        cairo::Format::ARgb32
                    } else {
                        cairo::Format::Rgb24
                    };
                    if let Ok(surf) = cairo::ImageSurface::create(format, p.width(), p.height()) {
                        if let Ok(cr) = cairo::Context::new(&surf) {
                            cr.set_source_pixbuf(&p, 0.0, 0.0);
                            let _ = cr.paint();
                            return Some(surf);
                        }
                    }
                    None
                });
                self.icon_cache.insert(raw_icon_name.clone(), surface);
            }
        }
    }

    fn material_glyph_layout(
        &mut self,
        area: &gtk::DrawingArea,
        codepoint: char,
        size: f64,
    ) -> gtk::pango::Layout {
        let font_size = size.round().max(1.0) as u32;
        let key = (codepoint.to_string(), font_size);
        if let Some(l) = self.material_layout_cache.get(&key) {
            return l.clone();
        }
        let l = area.create_pango_layout(Some(&codepoint.to_string()));
        let mut font_desc = gtk::pango::FontDescription::new();
        font_desc.set_family("Material Symbols Rounded");
        font_desc.set_absolute_size(size * gtk::pango::SCALE as f64);
        font_desc.set_variations(Some(&format!("wght {}", MATERIAL_ICON_WEIGHT.round())));
        l.set_font_description(Some(&font_desc));
        self.material_layout_cache.insert(key, l.clone());
        l
    }

    fn hit_test(&self, x: f64, y: f64, cx: f64, cy: f64) -> Option<usize> {
        let display_items = self.get_display_items();
        let n = display_items.len();
        if n == 0 {
            return None;
        }

        let mx = x - cx;
        let my = y - cy;
        let dist = (mx * mx + my * my).sqrt();

        if dist < BASE_R {
            return None;
        }

        let max_interactive_dist = if self.menu_style == "floating" {
            let required_r = n as f64 * 82.0 / (2.0 * PI);
            let base_dist = BASE_R + 60.0;
            let pill_dist = base_dist.max(required_r);
            pill_dist + SLICE_WIDTH + HOVER_GROW + self.extra_radius + 40.0
        } else {
            BASE_R + SLICE_WIDTH + HOVER_GROW + self.extra_radius
        };

        if dist <= max_interactive_dist {
            let angle_per_slice = 2.0 * PI / n as f64;
            let mut angle = my.atan2(mx) + PI / 2.0;
            if self.center_layout {
                angle += angle_per_slice / 2.0;
            }
            if angle < 0.0 {
                angle += 2.0 * PI;
            } else if angle >= 2.0 * PI {
                angle -= 2.0 * PI;
            }
            let index = (angle / angle_per_slice) as usize;
            if index < n {
                Some(index)
            } else {
                None
            }
        } else {
            None
        }
    }
}

fn load_icon_pixbuf(
    display: &gdk::Display,
    icon_name: &str,
    raster_size: i32,
    use_symbolic: bool,
) -> Option<gtk::gdk_pixbuf::Pixbuf> {
    let icon_theme = gtk::IconTheme::for_display(display);

    // First, lookup at 64 (or 16 for symbolic) to catch the detailed SVGs
    // SVGs usually map their "detailed" scalable versions to 48px or 64px.
    let initial_size = if use_symbolic { 16 } else { 64 };
    let flags = if use_symbolic {
        gtk::IconLookupFlags::FORCE_SYMBOLIC
    } else {
        gtk::IconLookupFlags::FORCE_REGULAR
    };

    let paintable = icon_theme.lookup_icon(
        icon_name,
        &[],
        initial_size,
        1,
        gtk::TextDirection::None,
        flags,
    );

    if let Some(file) = paintable.file() {
        if let Some(path) = file.path() {
            // SVGs scale infinitely, so the 64px detailed version will scale perfectly to raster_size.
            let is_svg = path.extension().and_then(|s| s.to_str()) == Some("svg");

            if is_svg {
                return gtk::gdk_pixbuf::Pixbuf::from_file_at_size(path, raster_size, raster_size)
                    .ok();
            } else {
                // It's a raster image (PNG) and we want high quality!
                // Do a second lookup asking for the target raster_size so we get the high-res PNG.
                let high_res_paintable = icon_theme.lookup_icon(
                    icon_name,
                    &[],
                    raster_size,
                    1,
                    gtk::TextDirection::None,
                    gtk::IconLookupFlags::FORCE_REGULAR,
                );

                if let Some(hr_file) = high_res_paintable.file() {
                    if let Some(hr_path) = hr_file.path() {
                        return gtk::gdk_pixbuf::Pixbuf::from_file_at_size(
                            hr_path,
                            raster_size,
                            raster_size,
                        )
                        .ok();
                    }
                }

                // Fallback to the original if the second lookup somehow failed
                return gtk::gdk_pixbuf::Pixbuf::from_file_at_size(path, raster_size, raster_size)
                    .ok();
            }
        }
    }
    None
}

fn load_and_apply_theme(
    config_path: &Path,
    theme_provider: &gtk::CssProvider,
    user_provider: &gtk::CssProvider,
) {
    if let Some(gtk_css) = launcher_core::paths::get_gtk_css_path() {
        if gtk_css.exists() {
            user_provider.load_from_path(&gtk_css);
        }
    }
    let (theme_name, sys_overrides) = match launcher_core::load_config(config_path) {
        Ok(cfg) => (cfg.ui.theme.clone(), cfg.ui.system_theme_overrides.clone()),
        Err(e) => {
            warn!(
                "Failed to load config: {}. Defaulting to theme 'default'",
                e
            );
            ("default".to_string(), None)
        }
    };

    let theme_file = config_path
        .parent()
        .map(|p| p.join("themes").join(format!("{}.css", theme_name)))
        .unwrap_or_else(|| {
            launcher_core::paths::get_themes_dir().join(format!("{}.css", theme_name))
        });

    debug!("Loading theme from {:?}", theme_file);
    if theme_name == "system" {
        // Dynamic GTK system theme using named colors
        let overrides = sys_overrides.unwrap_or_default();
        let system_css = format!(
            "
            .entry-surface {{ color: alpha({}, {:.3}); }}
            .entry-surface:hover {{ color: alpha({}, {:.3}); }}
            .entry-border {{ color: alpha({}, {:.3}); }}
            .entry-border:hover {{ color: alpha({}, {:.3}); }}
            .label {{ color: alpha({}, {:.3}); }}
            .label:hover {{ color: alpha({}, {:.3}); }}
            .entry-icon {{ color: alpha({}, {:.3}); }}
            .entry-icon:hover {{ color: alpha({}, {:.3}); }}
            .floating-icon-surface {{ color: alpha({}, {:.3}); }}
            .floating-icon-surface:hover {{ color: alpha({}, {:.3}); }}
            .hub-surface {{ color: alpha({}, {:.3}); }}
            .hub-border {{ color: alpha({}, {:.3}); }}
            .hub-label {{ color: alpha({}, {:.3}); }}
            .hub-icon {{ color: alpha({}, {:.3}); }}
            .pie-outer-border {{ color: alpha({}, {:.3}); }}
        ",
            overrides.entry_surface.variable,
            overrides.entry_surface.opacity,
            overrides.entry_surface_hover.variable,
            overrides.entry_surface_hover.opacity,
            overrides.entry_border.variable,
            overrides.entry_border.opacity,
            overrides.entry_border_hover.variable,
            overrides.entry_border_hover.opacity,
            overrides.label.variable,
            overrides.label.opacity,
            overrides.label_hover.variable,
            overrides.label_hover.opacity,
            overrides.entry_icon.variable,
            overrides.entry_icon.opacity,
            overrides.entry_icon_hover.variable,
            overrides.entry_icon_hover.opacity,
            overrides.floating_icon_surface.variable,
            overrides.floating_icon_surface.opacity,
            overrides.floating_icon_surface_hover.variable,
            overrides.floating_icon_surface_hover.opacity,
            overrides.hub_surface.variable,
            overrides.hub_surface.opacity,
            overrides.hub_border.variable,
            overrides.hub_border.opacity,
            overrides.hub_label.variable,
            overrides.hub_label.opacity,
            overrides.hub_icon.variable,
            overrides.hub_icon.opacity,
            overrides.pie_outer_border.variable,
            overrides.pie_outer_border.opacity
        );
        theme_provider.load_from_data(&system_css);
        info!("Theme 'system' applied successfully dynamically.");
    } else if theme_file.exists() {
        match std::fs::read_to_string(&theme_file) {
            Ok(css_content) => {
                theme_provider.load_from_data(&css_content);
                info!(
                    "Theme '{}' applied successfully from {:?}",
                    theme_name, theme_file
                );
            }
            Err(e) => {
                error!("Failed to read theme file {:?}: {}", theme_file, e);
            }
        }
    } else {
        warn!(
            "Theme file {:?} not found, using default styling.",
            theme_file
        );
        // Load default fallbacks
        let fallback = b"
            .entry-surface { color: rgba(30, 30, 46, 0.90); }
            .entry-surface:hover { color: rgba(49, 50, 68, 0.95); }
            .entry-border { color: rgba(137, 180, 250, 0.40); }
            .entry-border:hover { color: rgba(137, 180, 250, 0.95); }
            .label { color: rgba(205, 214, 244, 1.0); }
            .label:hover { color: rgba(255, 255, 255, 1.0); }
            .entry-icon { color: rgba(205, 214, 244, 1.0); }
            .entry-icon:hover { color: rgba(255, 255, 255, 1.0); }
            .floating-icon-surface { color: rgba(30, 30, 46, 1.0); }
            .floating-icon-surface:hover { color: rgba(137, 180, 250, 1.0); }
            .hub-surface { color: rgba(17, 17, 27, 0.95); }
            .hub-border { color: rgba(137, 180, 250, 0.70); }
            .hub-label { color: rgba(205, 214, 244, 1.0); }
            .hub-icon { color: rgba(205, 214, 244, 1.0); }
            .pie-outer-border { color: rgba(137, 180, 250, 1.0); }
        ";
        theme_provider.load_from_data(std::str::from_utf8(fallback).unwrap());
    }
}


fn go_back(state: &mut MenuState, area: &gtk::DrawingArea) -> bool {
    if let Some(prev) = state.history.pop() {
        let prev_icon = state.history_icons.pop().unwrap_or(None);
        
        // Push current state to forward history
        state.forward_history.push(state.current_items.clone());
        state.forward_history_icons.push(state.current_icon.clone());

        if let Some(icon) = prev_icon.clone() {
            state.current_icon = Some(icon);
        } else if prev_icon.is_none() && state.history.is_empty() {
            state.current_icon = state.root_icon.clone();
        } else {
            state.current_icon = None; // fallback
        }
        
        state.current_items = prev;
        state.hovered_index = None;
        if let Some(display) = gdk::Display::default() {
            state.preload_icons(&display);
        }
        area.queue_draw();
        return true;
    }
    false
}

fn go_forward(state: &mut MenuState, area: &gtk::DrawingArea) -> bool {
    if let Some(next) = state.forward_history.pop() {
        let next_icon = state.forward_history_icons.pop().flatten();

        // Push current state to history
        state.history.push(state.current_items.clone());
        state.history_icons.push(state.current_icon.clone());

        state.current_icon = next_icon;
        state.current_items = next;
        state.hovered_index = None;
        
        if let Some(display) = gdk::Display::default() {
            state.preload_icons(&display);
        }
        area.queue_draw();
        return true;
    }
    false
}

fn activate_index(state: &mut MenuState, index: usize, area: &gtk::DrawingArea) {
    let display_items = state.get_display_items();
    let display_items_count = display_items.len();
    if index >= display_items_count {
        return;
    }

    let is_back_button = !state.history.is_empty() && !state.hide_back_entry && index == display_items_count - 1;
    if is_back_button {
        debug!("Back wedge activated, popping history");
        go_back(state, area);
    } else {
        let selected = display_items[index].clone();
        if !selected.children.is_empty() {
            let current_items = state.current_items.clone();
            
            // Clear forward history because we took a new path
            state.forward_history.clear();
            state.forward_history_icons.clear();
            
            state.history.push(current_items);
            state.history_icons.push(state.current_icon.clone());
            state.current_icon = selected.icon.clone();
            state.current_items = selected.children;
            state.hovered_index = None;
            if let Some(display) = gdk::Display::default() {
                state.preload_icons(&display);
            }
            area.queue_draw();
        } else if let Some(action) = selected.action {
            info!("Running action: {:?}", action);

            if let launcher_core::Action::Hotkey { keys, keep_open } = &action {
                if *keep_open {
                    if let Some(window) = area
                        .root()
                        .and_then(|r| r.downcast::<gtk::ApplicationWindow>().ok())
                    {
                        // Drop keyboard focus to let the background app receive the macro
                        window.set_keyboard_mode(KeyboardMode::None);

                        let keys_clone = keys.clone();
                        let window_clone = window.clone();
                        let suppress = state.suppress_focus_loss.clone();

                        suppress.set(true);

                        // Wait briefly for the compositor to transfer focus
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(50),
                            move || {
                                if let Ok(args) = launcher_core::parse_hotkey(&keys_clone) {
                                    // wtype executes very quickly, so blocking for a few ms is fine
                                    let _ =
                                        std::process::Command::new("wtype").args(&args).status();
                                }

                                // Regain focus after wtype has finished
                                window_clone.set_keyboard_mode(KeyboardMode::Exclusive);

                                // Re-enable focus loss closing shortly after
                                glib::timeout_add_local_once(
                                    std::time::Duration::from_millis(50),
                                    move || {
                                        suppress.set(false);
                                    },
                                );
                            },
                        );
                        return;
                    }
                }
            }

            if let Err(e) = launcher_core::run_action(&action) {
                error!("Failed to execute action: {}", e);
            }
            if !action.should_keep_open() {
                state.is_closing = true;
            }
        }
    }
}

pub struct LauncherApp {
    app: gtk::Application,
    menu_path: PathBuf,
    config_path: PathBuf,
    start_hidden: bool,
}

impl LauncherApp {
    pub fn new(menu_path: PathBuf, config_path: PathBuf, start_hidden: bool) -> Self {
        let app = gtk::Application::builder()
            .application_id("org.rmwk.launcher")
            .build();

        Self {
            app,
            menu_path,
            config_path,
            start_hidden,
        }
    }

    fn init_color_scheme_sync() -> Option<gtk::gio::Settings> {
        let apply_preference = |is_dark: bool| {
            if let Some(gtk_settings) = gtk::Settings::default() {
                gtk_settings.set_gtk_application_prefer_dark_theme(is_dark);
            }
        };

        let source = gtk::gio::SettingsSchemaSource::default();
        if source.map_or(false, |s| {
            s.lookup("org.gnome.desktop.interface", true).is_some()
        }) {
            let gsettings = gtk::gio::Settings::new("org.gnome.desktop.interface");
            let update_from_gsettings = {
                let gsettings = gsettings.clone();
                move || {
                    let color_scheme = gsettings.string("color-scheme");
                    let is_dark = if color_scheme == "prefer-dark" {
                        true
                    } else if color_scheme == "prefer-light" {
                        false
                    } else {
                        let theme = gsettings.string("gtk-theme");
                        theme.to_lowercase().contains("dark")
                    };
                    apply_preference(is_dark);
                }
            };

            update_from_gsettings();

            gsettings.connect_changed(Some("color-scheme"), {
                let gsettings = gsettings.clone();
                move |_, _| {
                    let color_scheme = gsettings.string("color-scheme");
                    let is_dark = if color_scheme == "prefer-dark" {
                        true
                    } else if color_scheme == "prefer-light" {
                        false
                    } else {
                        let theme = gsettings.string("gtk-theme");
                        theme.to_lowercase().contains("dark")
                    };
                    apply_preference(is_dark);
                }
            });

            gsettings.connect_changed(Some("gtk-theme"), {
                let gsettings = gsettings.clone();
                move |_, _| {
                    let color_scheme = gsettings.string("color-scheme");
                    if color_scheme != "prefer-dark" && color_scheme != "prefer-light" {
                        let theme = gsettings.string("gtk-theme");
                        apply_preference(theme.to_lowercase().contains("dark"));
                    }
                }
            });

            Some(gsettings)
        } else {
            let is_dark = if let Ok(gtk_theme_env) = std::env::var("GTK_THEME") {
                gtk_theme_env.to_lowercase().contains("dark")
            } else if let Some(gtk_settings) = gtk::Settings::default() {
                gtk_settings
                    .gtk_theme_name()
                    .map_or(false, |t| t.to_lowercase().contains("dark"))
            } else {
                false
            };
            apply_preference(is_dark);
            None
        }
    }

    pub fn run(&self) -> i32 {
        let menu_path = self.menu_path.clone();
        let config_path = self.config_path.clone();
        let start_hidden = self.start_hidden;

        let activated = std::rc::Rc::new(std::cell::RefCell::new(false));

        self.app.connect_activate(move |app| {
            if *activated.borrow() {
                tracing::debug!("App already activated. Ignoring secondary D-Bus activation.");
                return;
            }
            *activated.borrow_mut() = true;

            let guard = app.hold(); // Hold application alive even when windows are hidden
            std::mem::forget(guard); // Keep the hold active for the lifecycle of the daemon
            if let Err(e) =
                Self::activate_ui(app, menu_path.clone(), config_path.clone(), start_hidden)
            {
                tracing::error!("Failed to activate UI: {}", e);
            }
        });

        // Run the GTK main loop (this blocks until all windows are closed/app exits)
        self.app.run_with_args::<String>(&[]).into()
    }

    fn activate_ui(
        app: &gtk::Application,
        menu_path: PathBuf,
        config_path: PathBuf,
        start_hidden: bool,
    ) -> anyhow::Result<()> {
        let _gsettings = Self::init_color_scheme_sync();

        // Load the menu config from disk
        let menu_config = match launcher_core::load_menu(&menu_path) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    "Failed to load menu config from {:?}, using empty menu: {}",
                    menu_path, e
                );
                launcher_core::MenuConfig {
                    icon: None,
                    menu: vec![],
                }
            }
        };

        let window = gtk::ApplicationWindow::new(app);
        window.set_title(Some("rmwk"));

        // 1. Initialize Layer Shell before realizing the window
        window.init_layer_shell();
        window.set_namespace(Some("rmwk"));
        window.set_layer(Layer::Overlay);
        window.set_keyboard_mode(KeyboardMode::Exclusive);
        window.set_exclusive_zone(-1);

        // 2. Anchor to all four edges so the transparent window spans the whole monitor
        window.set_anchor(gtk4_layer_shell::Edge::Top, true);
        window.set_anchor(gtk4_layer_shell::Edge::Bottom, true);
        window.set_anchor(gtk4_layer_shell::Edge::Left, true);
        window.set_anchor(gtk4_layer_shell::Edge::Right, true);

        // 3. Make window background transparent using CSS style provider
        let base_provider = gtk::CssProvider::new();
        base_provider.load_from_data(
            "
            window.background, .radial-surface {
                background-color: rgba(0, 0, 0, 0);
                background: transparent;
            }
        ",
        );

        let theme_provider = gtk::CssProvider::new();
        let user_provider = gtk::CssProvider::new();
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &user_provider,
                gtk::STYLE_PROVIDER_PRIORITY_USER,
            );
        }
        load_and_apply_theme(&config_path, &theme_provider, &user_provider);

        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &base_provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            gtk::style_context_add_provider_for_display(
                &display,
                &theme_provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        window.add_css_class("radial-surface");

        let ui_config = match launcher_core::load_config(&config_path) {
            Ok(cfg) => cfg.ui,
            Err(e) => {
                warn!("Failed to load config: {}. Using default UI settings", e);
                launcher_core::UiConfig::default()
            }
        };

        let codepoints = launcher_core::load_material_codepoints(&config_path);

        // Initialize state
        let state = Rc::new(RefCell::new(MenuState {
            current_menu_path: menu_path.clone(),
            root_items: menu_config.menu.clone(),
            current_items: menu_config.menu.clone(),
            root_icon: menu_config.icon.clone(),
            current_icon: menu_config.icon.clone(),
            history: vec![],
            forward_history: vec![],
            history_icons: vec![],
            forward_history_icons: vec![],
            hovered_index: None,
            hide_back_entry: ui_config.hide_back_entry,
            is_closing: false,
            hover_progresses: vec![],
            icon_cache: HashMap::new(),
            text_layout_cache: HashMap::new(),
            material_layout_cache: HashMap::new(),
            label_layout_cache: HashMap::new(),
            extra_radius: ui_config.extra_radius,
            pill_roundness: ui_config.pill_roundness,
            use_symbolic_icons: ui_config.use_symbolic_icons,
            bold_single_chars: ui_config.bold_single_chars,
            menu_style: ui_config.menu_style.clone(),
            center_layout: ui_config.center_layout,
            disable_hover_animation: ui_config.disable_hover_animation,
            hover_visual_cue: ui_config.hover_visual_cue.clone(),
            enable_blur: ui_config.enable_blur && ui_config.menu_style != "floating",
            last_cx: 0.0,
            last_cy: 0.0,
            last_blur_radius: -1.0,
            codepoints,
            theme_colors: std::cell::RefCell::new(None),
            suppress_focus_loss: std::rc::Rc::new(std::cell::Cell::new(false)),
            _config_monitor: None,
            _menu_monitor: None,
        }));

        if let Some(display) = gdk::Display::default() {
            state.borrow_mut().preload_icons(&display);
        }

        let wayland_blur = Rc::new(RefCell::new(None));
        let blur_realize = wayland_blur.clone();
        let state_realize = state.clone();

        window.connect_realize(move |w| {
            // Only take over blur via ext-background-effect-v1 when the app is
            // actually managing it. Binding the effect surface and clearing the
            // region otherwise would disable compositor-side blur (e.g. Hyprland
            // layer rules) for this surface.
            if !state_realize.borrow().enable_blur {
                return;
            }
            if let Some(blur) = wayland::WaylandBlur::new(w) {
                let width = w.width() as f64;
                let height = w.height() as f64;
                let cx = width / 2.0;
                let cy = height / 2.0;
                let radius = BASE_R + SLICE_WIDTH + HOVER_GROW;
                blur.update_circular_region(radius, cx, cy);
                *blur_realize.borrow_mut() = Some(blur);
            }
        });

        // 4. Create drawing area for radial wedges
        let drawing_area = gtk::DrawingArea::new();

        let blur_resize = wayland_blur.clone();
        let state_resize = state.clone();
        let window_resize = window.clone();
        drawing_area.connect_resize(move |_area, _width, _height| {
            if let Some(blur) = blur_resize.borrow().as_ref() {
                let win_w = window_resize.width() as f64;
                let win_h = window_resize.height() as f64;
                let cx = win_w / 2.0;
                let cy = win_h / 2.0;
                let mut state = state_resize.borrow_mut();
                state.last_cx = cx;
                state.last_cy = cy;
                let radius = if state.enable_blur {
                    BASE_R + SLICE_WIDTH
                } else {
                    0.0
                };
                blur.update_circular_region(radius, cx, cy);
            }
        });

        let draw_state = state.clone();
        let blur_draw = wayland_blur.clone();
        drawing_area.set_draw_func(move |area, cr, width, height| {
            let cx = width as f64 / 2.0;
            let cy = height as f64 / 2.0;

            let mut state_ref = match draw_state.try_borrow_mut() {
                Ok(s) => s,
                Err(_) => return,
            };

            // Update blur region based on animation progress
            if let Some(blur) = blur_draw.borrow().as_ref() {
                let target_radius = if state_ref.enable_blur {
                    if state_ref.is_closing {
                        0.0
                    } else {
                        BASE_R + SLICE_WIDTH
                    }
                } else {
                    0.0
                };

                // Only update Wayland region if it has actually changed to avoid IPC overhead
                if (state_ref.last_blur_radius - target_radius).abs() > 0.01 {
                    blur.update_circular_region(target_radius, cx, cy);
                    state_ref.last_blur_radius = target_radius;
                }
            }

            let display_items = state_ref.get_display_items();
            let n = display_items.len();

            let ease_progress = 1.0;

            // Clear surface (ensure transparent background is clean)
            let max_interactive_dist =
                BASE_R + SLICE_WIDTH + HOVER_GROW + state_ref.extra_radius + 80.0;
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            cr.set_operator(cairo::Operator::Source);
            cr.rectangle(
                cx - max_interactive_dist,
                cy - max_interactive_dist,
                max_interactive_dist * 2.0,
                max_interactive_dist * 2.0,
            );
            cr.fill().unwrap();
            cr.set_operator(cairo::Operator::Over);

            // 1. Get wedge colors
            let cached = state_ref.theme_colors.borrow().clone();
            let colors = if let Some(c) = cached {
                c
            } else {
                let context = area.style_context();

                context.save();
                context.add_class("entry-surface");
                context.set_state(gtk::StateFlags::NORMAL);
                let fill_color = context.color();
                context.set_state(gtk::StateFlags::PRELIGHT);
                let hover_fill_color = context.color();
                context.restore();

                context.save();
                context.add_class("entry-border");
                context.set_state(gtk::StateFlags::NORMAL);
                let border_color = context.color();
                context.set_state(gtk::StateFlags::PRELIGHT);
                let hover_border_color = context.color();
                context.restore();

                context.save();
                context.add_class("label");
                context.set_state(gtk::StateFlags::NORMAL);
                let label_color = context.color();
                context.set_state(gtk::StateFlags::PRELIGHT);
                let hover_label_color = context.color();
                context.restore();

                context.save();
                context.add_class("entry-icon");
                context.set_state(gtk::StateFlags::NORMAL);
                let icon_color = context.color();
                context.set_state(gtk::StateFlags::PRELIGHT);
                let hover_icon_color = context.color();
                context.restore();

                context.save();
                context.add_class("floating-icon-surface");
                context.set_state(gtk::StateFlags::NORMAL);
                let icon_tile_color = context.color();
                context.set_state(gtk::StateFlags::PRELIGHT);
                let hover_icon_tile_color = context.color();
                context.restore();

                context.save();
                context.add_class("hub-surface");
                context.set_state(gtk::StateFlags::NORMAL);
                let hub_fill = context.color();
                context.restore();

                context.save();
                context.add_class("hub-border");
                context.set_state(gtk::StateFlags::NORMAL);
                let hub_border = context.color();
                context.restore();

                context.save();
                context.add_class("hub-label");
                context.set_state(gtk::StateFlags::NORMAL);
                let hub_text_color = context.color();
                context.restore();

                context.save();
                context.add_class("hub-icon");
                context.set_state(gtk::StateFlags::NORMAL);
                let hub_icon_color = context.color();
                context.restore();

                context.save();
                context.add_class("pie-outer-border");
                context.set_state(gtk::StateFlags::NORMAL);
                let outer_border_color = context.color();
                context.restore();

                let c = ThemeColors {
                    fill_color,
                    hover_fill_color,
                    border_color,
                    hover_border_color,
                    label_color,
                    hover_label_color,
                    icon_color,
                    hover_icon_color,
                    icon_tile_color,
                    hover_icon_tile_color,
                    hub_fill,
                    hub_border,
                    hub_text_color,
                    hub_icon_color,
                    outer_border_color,
                };

                *state_ref.theme_colors.borrow_mut() = Some(c.clone());
                c
            };

            let fill_color = colors.fill_color;
            let hover_fill_color = colors.hover_fill_color;
            let border_color = colors.border_color;
            let hover_border_color = colors.hover_border_color;
            let label_color = colors.label_color;
            let hover_label_color = colors.hover_label_color;
            let icon_color = colors.icon_color;
            let hover_icon_color = colors.hover_icon_color;
            let icon_tile_color = colors.icon_tile_color;
            let hover_icon_tile_color = colors.hover_icon_tile_color;
            let hub_fill = colors.hub_fill;
            let hub_border = colors.hub_border;
            let hub_text_color = colors.hub_text_color;
            let hub_icon_color = colors.hub_icon_color;
            let outer_border_color = colors.outer_border_color;

            let mut center_text = None;
            let mut center_icon = None;

            if state_ref.menu_style == "floating" || state_ref.menu_style == "pill" {
                center_icon = state_ref.current_icon.clone();
            } else {
                if let Some(idx) = state_ref.hovered_index {
                    if idx < display_items.len() {
                        center_text = Some(display_items[idx].label.clone());
                    }
                }
                if center_text.is_none() {
                    center_icon = state_ref.current_icon.clone();
                }
            }

            let draw_hub = |state_ref: &mut std::cell::RefMut<MenuState>| {
                // Draw center hub if visible (circle in pie mode, roundness-aware in floating mode)
                let hub_rad = if state_ref.menu_style == "floating" {
                    60.0 * ease_progress
                } else {
                    BASE_R * ease_progress
                };
                let hub_round = if state_ref.menu_style == "floating" {
                    hub_rad * state_ref.pill_roundness.clamp(0.0, 1.0)
                } else {
                    hub_rad
                };
                let hub_path = |cr: &cairo::Context| {
                    if hub_round >= hub_rad - 0.001 {
                        cr.new_path();
                        cr.arc(cx, cy, hub_rad, 0.0, 2.0 * std::f64::consts::PI);
                    } else {
                        rounded_rect_path(
                            cr,
                            cx - hub_rad,
                            cy - hub_rad,
                            2.0 * hub_rad,
                            2.0 * hub_rad,
                            hub_round,
                        );
                    }
                };

                if hub_fill.alpha() > 0.001 {
                    cr.set_source_rgba(
                        hub_fill.red() as f64,
                        hub_fill.green() as f64,
                        hub_fill.blue() as f64,
                        hub_fill.alpha() as f64 * ease_progress,
                    );
                    hub_path(cr);
                    cr.fill().unwrap();
                }

                if hub_border.alpha() > 0.001 {
                    hub_path(cr);
                    cr.set_source_rgba(
                        hub_border.red() as f64,
                        hub_border.green() as f64,
                        hub_border.blue() as f64,
                        hub_border.alpha() as f64 * ease_progress,
                    );
                    cr.set_line_width(2.0);
                    cr.stroke().unwrap();
                }

                if let Some(ref icon_name) = center_icon {
                    cr.set_source_rgba(
                        hub_icon_color.red() as f64,
                        hub_icon_color.green() as f64,
                        hub_icon_color.blue() as f64,
                        hub_icon_color.alpha() as f64 * ease_progress,
                    );

                    let mut icon_w = 0.0;
                    let mut icon_h = 0.0;
                    let mut icon_layout = None;
                    // Hub icon size in px (material glyph / system image / single-char).
                    // PIE MODE: adjust the `80.0` in the else branch; floating stays at 80.0.
                    let icon_size = if state_ref.menu_style == "floating" {
                        80.0 * ease_progress
                    } else {
                        92.0 * ease_progress
                    };
                    let mut surf_to_draw = None;

                    if icon_name.chars().count() == 1 && !icon_name.starts_with('/') {
                        let font_size = icon_size.round() as u32;
                        let key = (icon_name.clone(), font_size);
                        let l = if let Some(l) = state_ref.text_layout_cache.get(&key) {
                            l.clone()
                        } else {
                            let l = area.create_pango_layout(Some(icon_name));
                            let mut font_desc = gtk::pango::FontDescription::new();
                            if state_ref.bold_single_chars {
                                font_desc.set_weight(gtk::pango::Weight::Bold);
                            }
                            font_desc.set_family("Sans");
                            font_desc.set_absolute_size(icon_size * gtk::pango::SCALE as f64);
                            l.set_font_description(Some(&font_desc));
                            state_ref.text_layout_cache.insert(key, l.clone());
                            l
                        };
                        let (iw, ih) = l.pixel_size();
                        icon_w = iw as f64 * ease_progress;
                        icon_h = ih as f64 * ease_progress;
                        icon_layout = Some(l);
                    } else if let Some(&codepoint) = state_ref.codepoints.get(icon_name) {
                        let layout = state_ref.material_glyph_layout(&area, codepoint, icon_size);
                        let (ink, _logical) = layout.pixel_extents();
                        icon_w = ink.width() as f64;
                        icon_h = ink.height() as f64;
                    } else if let Some(Some(surf)) = state_ref.icon_cache.get(icon_name) {
                        let cw = surf.width() as f64;
                        let ch = surf.height() as f64;
                        let scale = icon_size / cw.max(ch).max(1.0);
                        icon_w = cw * scale;
                        icon_h = ch * scale;
                        surf_to_draw = Some((surf.clone(), scale));
                    }

                    if let Some(l) = icon_layout {
                        let _ = cr.save();
                        cr.translate(cx, cy);
                        if ease_progress > 0.001 {
                            cr.scale(ease_progress, ease_progress);
                        }
                        cr.move_to(-(icon_w / 2.0), -(icon_h / 2.0));
                        pangocairo::functions::show_layout(&cr, &l);
                        let _ = cr.restore();
                    } else if let Some(&codepoint) = state_ref.codepoints.get(icon_name) {
                        let layout = state_ref.material_glyph_layout(&area, codepoint, icon_size);
                        let (ink, _logical) = layout.pixel_extents();
                        let _ = cr.save();
                        cr.translate(cx, cy);
                        if ease_progress > 0.001 {
                            cr.scale(ease_progress, ease_progress);
                        }
                        cr.move_to(
                            -(ink.x() as f64 + ink.width() as f64 / 2.0),
                            -(ink.y() as f64 + ink.height() as f64 / 2.0),
                        );
                        let _ = pangocairo::functions::show_layout(&cr, &layout);
                        let _ = cr.restore();
                    } else if let Some((surf, scale)) = surf_to_draw {
                        let _ = cr.save();
                        cr.translate(cx - icon_w / 2.0, cy - icon_h / 2.0);
                        if scale * ease_progress > 0.001 {
                            cr.scale(scale * ease_progress, scale * ease_progress);
                        }
                        let _ = cr.set_source_surface(&surf, 0.0, 0.0);
                        let _ = cr.paint();
                        let _ = cr.restore();
                    }
                } else if let Some(text) = &center_text {
                    cr.set_source_rgba(
                        hub_text_color.red() as f64,
                        hub_text_color.green() as f64,
                        hub_text_color.blue() as f64,
                        hub_text_color.alpha() as f64 * ease_progress,
                    );

                    let center_layout = if let Some(l) = state_ref.label_layout_cache.get(text) {
                        l.clone()
                    } else {
                        let l = area.create_pango_layout(Some(text));
                        let mut font_desc = gtk::pango::FontDescription::new();
                        font_desc.set_family("Sans");
                        font_desc.set_weight(gtk::pango::Weight::Bold);
                        font_desc.set_absolute_size(16.0 * gtk::pango::SCALE as f64);
                        l.set_font_description(Some(&font_desc));
                        state_ref.label_layout_cache.insert(text.clone(), l.clone());
                        l
                    };

                    let (pango_w, pango_h) = center_layout.pixel_size();
                    cr.save().unwrap();
                    cr.translate(cx, cy);
                    if ease_progress > 0.001 {
                        cr.scale(ease_progress, ease_progress);
                    }
                    cr.move_to(-(pango_w as f64) / 2.0, -(pango_h as f64) / 2.0);
                    pangocairo::functions::show_layout(&cr, &center_layout);
                    cr.restore().unwrap();
                }
            };

            if state_ref.menu_style == "floating" {
                if n > 0 {
                    let angle_per_slice = 2.0 * std::f64::consts::PI / n as f64;
                    let mut draw_order: Vec<usize> = (0..n).rev().collect();
                    if let Some(hovered_i) = state_ref.hovered_index {
                        if !state_ref.is_closing && hovered_i < n {
                            draw_order.retain(|&idx| idx != hovered_i);
                            draw_order.push(hovered_i);
                        }
                    }

                    for &i in &draw_order {
                        let item = &display_items[i];
                        let hp = if i < state_ref.hover_progresses.len() {
                            state_ref.hover_progresses[i]
                        } else {
                            0.0
                        };
                        let is_hovered =
                            state_ref.hovered_index == Some(i) && !state_ref.is_closing;

                        let mut base_start_angle =
                            i as f64 * angle_per_slice - std::f64::consts::PI / 2.0;
                        if state_ref.center_layout {
                            base_start_angle -= angle_per_slice / 2.0;
                        }
                        let mid_angle = base_start_angle + angle_per_slice / 2.0;

                        // Dynamic radius scaling
                        let required_r = n as f64 * 82.0 / (2.0 * std::f64::consts::PI);
                        let base_dist = BASE_R + 52.5;
                        let pill_dist =
                            base_dist.max(required_r) + (hp * HOVER_GROW) * ease_progress;

                        let icon_center_x = cx + pill_dist * mid_angle.cos();
                        let icon_center_y = cy + pill_dist * mid_angle.sin();

                        // Measure text
                        let text = &item.label;
                        let text_layout = if let Some(l) = state_ref.label_layout_cache.get(text) {
                            l.clone()
                        } else {
                            let l = area.create_pango_layout(Some(text));
                            let mut font_desc = gtk::pango::FontDescription::new();
                            font_desc.set_family("Sans");
                            font_desc.set_size(gtk::pango::units_from_double(14.0));
                            l.set_font_description(Some(&font_desc));
                            state_ref.label_layout_cache.insert(text.clone(), l.clone());
                            l
                        };
                        let (tw, th) = text_layout.pixel_size();
                        let (tw_f, th_f) = (tw as f64 * ease_progress, th as f64 * ease_progress);

                        // Measure icon
                        let mut icon_w = 0.0;
                        let mut icon_h = 0.0;
                        let icon_size = 32.0 * ease_progress; // fixed icon size for pills
                        let mut icon_layout: Option<gtk::pango::Layout> = None;

                        if let Some(icon_name) = &item.icon {
                            if icon_name.chars().count() == 1 {
                                let font_size = 48u32;
                                let key = (icon_name.clone(), font_size);
                                let l = if let Some(l) = state_ref.text_layout_cache.get(&key) {
                                    l.clone()
                                } else {
                                    let l = area.create_pango_layout(Some(icon_name));
                                    let mut font_desc = gtk::pango::FontDescription::new();
                                    font_desc.set_family("Sans");
                                    let weight = if state_ref.bold_single_chars {
                                        gtk::pango::Weight::Bold
                                    } else {
                                        gtk::pango::Weight::Normal
                                    };
                                    font_desc.set_weight(weight);
                                    font_desc.set_size(gtk::pango::units_from_double(64.0 * 0.75));
                                    l.set_font_description(Some(&font_desc));
                                    state_ref.text_layout_cache.insert(key, l.clone());
                                    l
                                };
                                let (_iw, _ih) = l.pixel_size();
                                icon_layout = Some(l);
                                icon_w = icon_size * 0.75;
                                icon_h = icon_size * 0.75;
                            } else if let Some(&codepoint) = state_ref.codepoints.get(icon_name) {
                                let layout =
                                    state_ref.material_glyph_layout(&area, codepoint, 32.0);
                                let (ink, _logical) = layout.pixel_extents();
                                icon_w = ink.width() as f64 * ease_progress;
                                icon_h = ink.height() as f64 * ease_progress;
                            } else if let Some(Some(surf)) = state_ref.icon_cache.get(icon_name) {
                                let cw = surf.width() as f64;
                                let ch = surf.height() as f64;
                                let scale = icon_size / cw.max(ch).max(1.0);
                                icon_w = cw * scale;
                                icon_h = ch * scale;
                            }
                        }

                        let padding_x = 16.0 * ease_progress;

                        #[derive(PartialEq)]
                        enum PillMode {
                            Right,
                            Left,
                            Top,
                            Bottom,
                        }
                        let threshold = if n >= 11 {
                            (angle_per_slice / 2.0).sin().abs() + 0.02
                        } else {
                            0.15
                        };
                        let mode = if mid_angle.cos().abs() <= threshold {
                            if mid_angle.sin() < 0.0 {
                                PillMode::Top
                            } else {
                                PillMode::Bottom
                            }
                        } else if mid_angle.cos() >= 0.0 {
                            PillMode::Right
                        } else {
                            PillMode::Left
                        };

                        let has_text = tw_f > 0.0;
                        let r = (icon_size / 2.0 + 8.0) * ease_progress;

                        // Parameters controlling spacing between label and icon
                        let gap_between = 12.0 * ease_progress; // Horizontal gap for Left/Right entries
                        let vertical_gap = -8.0 * ease_progress; // Vertical gap for Top/Bottom entries
                        let text_pill_h = r * 2.0; // Enforce strict height for all labels

                        // Minimum pill width for Top/Bottom entries based on a 4-character label
                        let min_4ch_w = if let Some(l) = state_ref.label_layout_cache.get("MMM") {
                            let (w, _) = l.pixel_size();
                            w as f64 * ease_progress
                        } else {
                            let l = area.create_pango_layout(Some("MMM"));
                            let mut font_desc = gtk::pango::FontDescription::new();
                            font_desc.set_family("Sans");
                            font_desc.set_size(gtk::pango::units_from_double(14.0));
                            l.set_font_description(Some(&font_desc));
                            state_ref
                                .label_layout_cache
                                .insert("MMM".to_string(), l.clone());
                            let (w, _) = l.pixel_size();
                            w as f64 * ease_progress
                        };
                        let min_top_bottom_pill_w = min_4ch_w + padding_x * 2.0;

                        let icon_x = icon_center_x - icon_w / 2.0;
                        let icon_y = icon_center_y - icon_h / 2.0;

                        let (text_x, text_y, text_pill_x, text_pill_y, text_pill_w) = match mode {
                            PillMode::Right => {
                                let tx = icon_center_x + r + gap_between;
                                let ty = icon_center_y - th_f / 2.0;
                                let tpx = icon_center_x - r;
                                let tpy = icon_center_y - r;
                                let tpw = (tx + tw_f + padding_x) - tpx;
                                (tx, ty, tpx, tpy, tpw)
                            }
                            PillMode::Left => {
                                let tx = icon_center_x - r - gap_between - tw_f;
                                let ty = icon_center_y - th_f / 2.0;
                                let tpx = tx - padding_x;
                                let tpy = icon_center_y - r;
                                let tpw = (icon_center_x + r) - tpx;
                                (tx, ty, tpx, tpy, tpw)
                            }
                            PillMode::Top => {
                                let tx = icon_center_x - tw_f / 2.0;
                                let tpw = (tw_f + padding_x * 2.0).max(min_top_bottom_pill_w);
                                let tpx = icon_center_x - tpw / 2.0;
                                let tpy = icon_center_y - r - vertical_gap - text_pill_h;
                                let ty = tpy + r - th_f / 2.0;
                                (tx, ty, tpx, tpy, tpw)
                            }
                            PillMode::Bottom => {
                                let tx = icon_center_x - tw_f / 2.0;
                                let tpw = (tw_f + padding_x * 2.0).max(min_top_bottom_pill_w);
                                let tpx = icon_center_x - tpw / 2.0;
                                let tpy = icon_center_y + r + vertical_gap;
                                let ty = tpy + r - th_f / 2.0;
                                (tx, ty, tpx, tpy, tpw)
                            }
                        };

                        // Bar bounding box = union of the icon tile (square of side 2r) and the label pill
                        let (bx0, by0, bw, bh) = if !has_text || r < 0.1 {
                            (icon_center_x - r, icon_center_y - r, 2.0 * r, 2.0 * r)
                        } else {
                            let x0 = (icon_center_x - r).min(text_pill_x);
                            let y0 = (icon_center_y - r).min(text_pill_y);
                            (
                                x0,
                                y0,
                                (icon_center_x + r).max(text_pill_x + text_pill_w) - x0,
                                (icon_center_y + r).max(text_pill_y + text_pill_h) - y0,
                            )
                        };
                        // Corner radius scales with the roundness setting (1.0 = full capsule / circular tile)
                        let round = (r * state_ref.pill_roundness.clamp(0.0, 1.0))
                            .min(bw / 2.0)
                            .min(bh / 2.0);

                        // Top/Bottom entries keep separate shapes (a rounded-rect label pill unioned with a
                        // roundness-scaled icon tile), traced as a single silhouette so the outline strokes once.
                        // The union contour is built by sampling the max-envelope of the two right-side boundary
                        // functions (x-symmetric about the icon center) and mirroring the result on the left.
                        let pill_capsule_union = |cr: &cairo::Context| {
                            cr.new_path();
                            let tile_x1 = icon_center_x + r;
                            let tile_y0 = icon_center_y - r;
                            let tile_y1 = icon_center_y + r;
                            let pill_x1 = text_pill_x + text_pill_w;
                            let pill_y0 = text_pill_y;
                            let pill_y1 = text_pill_y + text_pill_h;
                            let rad = round;
                            let tile_right = |y: f64, rad: f64| -> f64 {
                                if y < tile_y0 + rad {
                                    tile_x1 - rad
                                        + (rad * rad - (y - (tile_y0 + rad)).powi(2))
                                            .max(0.0)
                                            .sqrt()
                                } else if y > tile_y1 - rad {
                                    tile_x1 - rad
                                        + (rad * rad - (y - (tile_y1 - rad)).powi(2))
                                            .max(0.0)
                                            .sqrt()
                                } else {
                                    tile_x1
                                }
                            };
                            let pill_right = |y: f64, rad: f64| -> f64 {
                                if y < pill_y0 + rad {
                                    pill_x1 - rad
                                        + (rad * rad - (y - (pill_y0 + rad)).powi(2))
                                            .max(0.0)
                                            .sqrt()
                                } else if y > pill_y1 - rad {
                                    pill_x1 - rad
                                        + (rad * rad - (y - (pill_y1 - rad)).powi(2))
                                            .max(0.0)
                                            .sqrt()
                                } else {
                                    pill_x1
                                }
                            };
                            let right = |y: f64| -> f64 {
                                let t = y >= tile_y0 && y <= tile_y1;
                                let p = y >= pill_y0 && y <= pill_y1;
                                if p && t {
                                    tile_right(y, rad).max(pill_right(y, rad))
                                } else if p {
                                    pill_right(y, rad)
                                } else {
                                    tile_right(y, rad)
                                }
                            };
                            let y_top = pill_y0.min(tile_y0);
                            let y_bot = pill_y1.max(tile_y1);
                            let n = 64.0;
                            let height = (y_bot - y_top).max(1.0);
                            let mut pts: Vec<(f64, f64)> = Vec::with_capacity(2 * (n as usize + 1));
                            for i in 0..=n as i32 {
                                let f = i as f64 / n;
                                let y = y_top + height * f;
                                pts.push((2.0 * icon_center_x - right(y), y));
                            }
                            for i in (0..=n as i32).rev() {
                                let f = i as f64 / n;
                                let y = y_top + height * f;
                                pts.push((right(y), y));
                            }
                            if let Some(&(x0, y0)) = pts.first() {
                                cr.move_to(x0, y0);
                                for &(x, y) in pts.iter().skip(1) {
                                    cr.line_to(x, y);
                                }
                            }
                            cr.close_path();
                        };

                        // Outline of the whole entry: unified rounded-rect bar for horizontal entries,
                        // separate keyhole-shaped union for top/bottom entries
                        let entry_outline = |cr: &cairo::Context| {
                            if !has_text || r < 0.1 {
                                cr.new_path();
                                cr.arc(
                                    icon_center_x,
                                    icon_center_y,
                                    r,
                                    0.0,
                                    2.0 * std::f64::consts::PI,
                                );
                                return;
                            }
                            match mode {
                                PillMode::Right | PillMode::Left => {
                                    rounded_rect_path(cr, bx0, by0, bw, bh, round)
                                }
                                PillMode::Top | PillMode::Bottom => pill_capsule_union(cr),
                            }
                        };

                        // 1. Fill translucent background for the whole entry
                        entry_outline(cr);
                        if is_hovered {
                            cr.set_source_rgba(
                                hover_fill_color.red() as f64,
                                hover_fill_color.green() as f64,
                                hover_fill_color.blue() as f64,
                                hover_fill_color.alpha() as f64 * ease_progress,
                            );
                        } else {
                            cr.set_source_rgba(
                                fill_color.red() as f64,
                                fill_color.green() as f64,
                                fill_color.blue() as f64,
                                fill_color.alpha() as f64 * ease_progress,
                            );
                        }
                        let _ = cr.fill();

                        // 2. Draw Opaque Icon Tile on top (rounded square matching the corner radius)
                        rounded_rect_path(
                            cr,
                            icon_center_x - r,
                            icon_center_y - r,
                            2.0 * r,
                            2.0 * r,
                            round,
                        );
                        if is_hovered {
                            cr.set_source_rgba(
                                hover_icon_tile_color.red() as f64,
                                hover_icon_tile_color.green() as f64,
                                hover_icon_tile_color.blue() as f64,
                                hover_icon_tile_color.alpha() as f64 * ease_progress,
                            );
                        } else {
                            cr.set_source_rgba(
                                icon_tile_color.red() as f64,
                                icon_tile_color.green() as f64,
                                icon_tile_color.blue() as f64,
                                icon_tile_color.alpha() as f64 * ease_progress,
                            );
                        }
                        let _ = cr.fill();

                        // 3. Stroke the Outline (single unified path)
                        entry_outline(cr);
                        if is_hovered {
                            cr.set_source_rgba(
                                hover_border_color.red() as f64,
                                hover_border_color.green() as f64,
                                hover_border_color.blue() as f64,
                                hover_border_color.alpha() as f64 * ease_progress,
                            );
                            cr.set_line_width(3.0 * ease_progress);
                        } else {
                            cr.set_source_rgba(
                                border_color.red() as f64,
                                border_color.green() as f64,
                                border_color.blue() as f64,
                                border_color.alpha() as f64 * ease_progress,
                            );
                            cr.set_line_width(2.0 * ease_progress);
                        }
                        let _ = cr.stroke();

                        // 4. Render Text
                        if has_text {
                            if is_hovered {
                                cr.set_source_rgba(
                                    hover_label_color.red() as f64,
                                    hover_label_color.green() as f64,
                                    hover_label_color.blue() as f64,
                                    hover_label_color.alpha() as f64 * ease_progress,
                                );
                            } else {
                                cr.set_source_rgba(
                                    label_color.red() as f64,
                                    label_color.green() as f64,
                                    label_color.blue() as f64,
                                    label_color.alpha() as f64 * ease_progress,
                                );
                            }
                            let _ = cr.save();
                            cr.translate(text_x, text_y);
                            if ease_progress > 0.001 {
                                cr.scale(ease_progress, ease_progress);
                            }
                            cr.move_to(0.0, 0.0);
                            pangocairo::functions::show_layout(&cr, &text_layout);
                            let _ = cr.restore();
                        }

                        // 5. Render Icon
                        if icon_w > 0.0 {
                            if is_hovered {
                                cr.set_source_rgba(
                                    hover_icon_color.red() as f64,
                                    hover_icon_color.green() as f64,
                                    hover_icon_color.blue() as f64,
                                    hover_icon_color.alpha() as f64 * ease_progress,
                                );
                            } else {
                                cr.set_source_rgba(
                                    icon_color.red() as f64,
                                    icon_color.green() as f64,
                                    icon_color.blue() as f64,
                                    icon_color.alpha() as f64 * ease_progress,
                                );
                            }

                            if let Some(icon_name) = &item.icon {
                                if icon_name.chars().count() == 1 {
                                    if let Some(l) = icon_layout {
                                        let (pango_w, pango_h) = l.pixel_size();
                                        let scale = (icon_size * 0.75) / (pango_w as f64).max(1.0);
                                        let _ = cr.save();
                                        cr.translate(icon_x + icon_w / 2.0, icon_y + icon_h / 2.0);
                                        cr.scale(scale, scale);
                                        cr.move_to(
                                            -(pango_w as f64 / 2.0),
                                            -(pango_h as f64 / 2.0),
                                        );
                                        pangocairo::functions::show_layout(&cr, &l);
                                        let _ = cr.restore();
                                    }
                                } else if let Some(&codepoint) = state_ref.codepoints.get(icon_name)
                                {
                                    let layout =
                                        state_ref.material_glyph_layout(&area, codepoint, 32.0);
                                    let (ink, _logical) = layout.pixel_extents();
                                    let _ = cr.save();
                                    cr.translate(icon_x + icon_w / 2.0, icon_y + icon_h / 2.0);
                                    if ease_progress > 0.001 {
                                        cr.scale(ease_progress, ease_progress);
                                    }
                                    cr.move_to(
                                        -(ink.x() as f64 + ink.width() as f64 / 2.0),
                                        -(ink.y() as f64 + ink.height() as f64 / 2.0),
                                    );
                                    let _ = pangocairo::functions::show_layout(&cr, &layout);
                                    let _ = cr.restore();
                                } else if let Some(Some(surf)) = state_ref.icon_cache.get(icon_name)
                                {
                                    let _ = cr.save();
                                    cr.translate(icon_x + icon_w / 2.0, icon_y + icon_h / 2.0);
                                    let scale = icon_size / surf.width().max(surf.height()) as f64;
                                    cr.scale(scale, scale);
                                    let _ = cr.set_source_surface(
                                        surf,
                                        -(surf.width() as f64) / 2.0,
                                        -(surf.height() as f64) / 2.0,
                                    );
                                    let _ = cr.paint_with_alpha(ease_progress);
                                    let _ = cr.restore();
                                }
                            }
                        }
                    }
                }
                draw_hub(&mut state_ref);
            } else {
                // 1. Draw continuous base background ring if fill_color is visible
                let base_outer_radius = (BASE_R + SLICE_WIDTH - 0.5) * ease_progress;
                let base_inner_radius = BASE_R * ease_progress;
                if fill_color.alpha() > 0.001 {
                    cr.new_path();
                    cr.arc(cx, cy, base_outer_radius, 0.0, 2.0 * PI);
                    cr.arc_negative(cx, cy, base_inner_radius, 2.0 * PI, 0.0);
                    cr.close_path();
                    cr.set_source_rgba(
                        fill_color.red() as f64,
                        fill_color.green() as f64,
                        fill_color.blue() as f64,
                        fill_color.alpha() as f64 * ease_progress,
                    );
                    cr.fill().unwrap();
                }

                if n > 0 {
                    let angle_per_slice = 2.0 * PI / n as f64;

                    // Determine draw order so the hovered slice paints over its neighbors' strokes
                    let mut draw_order: Vec<usize> = (0..n).collect();
                    if let Some(hovered_i) = state_ref.hovered_index {
                        if !state_ref.is_closing && hovered_i < n {
                            draw_order.retain(|&idx| idx != hovered_i);
                            draw_order.push(hovered_i);
                        }
                    }

                    for i in draw_order {
                        cr.new_path();
                        let item = &display_items[i];
                        let hp = if i < state_ref.hover_progresses.len() {
                            state_ref.hover_progresses[i]
                        } else {
                            0.0
                        };

                        let mut base_start_angle = i as f64 * angle_per_slice - PI / 2.0;
                        if state_ref.center_layout {
                            base_start_angle -= angle_per_slice / 2.0;
                        }
                        let base_end_angle = base_start_angle + angle_per_slice;

                        let hp_curr = hp;
                        let hp_prev = if state_ref.hover_progresses.len() > 0 {
                            state_ref.hover_progresses[(i + n - 1) % n]
                        } else {
                            0.0
                        };
                        let hp_next = if state_ref.hover_progresses.len() > 0 {
                            state_ref.hover_progresses[(i + 1) % n]
                        } else {
                            0.0
                        };

                        let is_hovered =
                            state_ref.hovered_index == Some(i) && !state_ref.is_closing;

                        let mut start_angle = base_start_angle;
                        let mut end_angle = base_end_angle;

                        let mut fill_outer_radius = (BASE_R + SLICE_WIDTH - 0.5) * ease_progress;
                        let mut stroke_outer_radius = (BASE_R + SLICE_WIDTH - 0.5) * ease_progress;

                        match state_ref.hover_visual_cue.as_str() {
                            "sides" => {
                                let hover_angle_grow = HOVER_GROW / (BASE_R + SLICE_WIDTH);
                                start_angle += (hp_prev - hp_curr) * hover_angle_grow;
                                end_angle += (hp_curr - hp_next) * hover_angle_grow;
                            }
                            "outwards" => {
                                stroke_outer_radius =
                                    (BASE_R + SLICE_WIDTH + (hp_curr * HOVER_GROW) - 0.5)
                                        * ease_progress;

                                if is_hovered {
                                    fill_outer_radius = stroke_outer_radius;
                                } else {
                                    // Instantly retreat the fill when unhovering so it doesn't leave an invisible trail
                                    fill_outer_radius =
                                        (BASE_R + SLICE_WIDTH - 0.5) * ease_progress;
                                }
                            }
                            _ => { // "none"
                                 // keep default values
                            }
                        }

                        let stroke_inner_radius = BASE_R * ease_progress;

                        // Fill wedge only if hovered, or if fill_color was transparent
                        if is_hovered || fill_color.alpha() <= 0.001 {
                            cr.new_path();
                            cr.arc(cx, cy, fill_outer_radius, start_angle, end_angle);
                            cr.arc_negative(cx, cy, stroke_inner_radius, end_angle, start_angle);
                            cr.close_path();

                            if is_hovered {
                                cr.set_source_rgba(
                                    hover_fill_color.red() as f64,
                                    hover_fill_color.green() as f64,
                                    hover_fill_color.blue() as f64,
                                    hover_fill_color.alpha() as f64 * ease_progress,
                                );
                                cr.set_operator(cairo::Operator::Source);
                                cr.fill().unwrap();
                                cr.set_operator(cairo::Operator::Over);
                            } else {
                                cr.set_source_rgba(
                                    fill_color.red() as f64,
                                    fill_color.green() as f64,
                                    fill_color.blue() as f64,
                                    fill_color.alpha() as f64 * ease_progress,
                                );
                                cr.fill().unwrap();
                            }
                        }

                        // Now establish the stroke path (radial sides and outer arc)
                        cr.new_path();
                        cr.move_to(
                            cx + stroke_inner_radius * start_angle.cos(),
                            cy + stroke_inner_radius * start_angle.sin(),
                        );
                        cr.line_to(
                            cx + stroke_outer_radius * start_angle.cos(),
                            cy + stroke_outer_radius * start_angle.sin(),
                        );
                        cr.arc(cx, cy, stroke_outer_radius, start_angle, end_angle);
                        cr.line_to(
                            cx + stroke_inner_radius * end_angle.cos(),
                            cy + stroke_inner_radius * end_angle.sin(),
                        );

                        let draw_wedge_stroke = |cr: &cairo::Context| {
                            let alpha = if is_hovered {
                                hover_border_color.alpha()
                            } else {
                                border_color.alpha()
                            };
                            if alpha > 0.001 {
                                if is_hovered {
                                    cr.set_source_rgba(
                                        hover_border_color.red() as f64,
                                        hover_border_color.green() as f64,
                                        hover_border_color.blue() as f64,
                                        hover_border_color.alpha() as f64 * ease_progress,
                                    );
                                    cr.set_line_width(3.0);
                                } else {
                                    cr.set_source_rgba(
                                        border_color.red() as f64,
                                        border_color.green() as f64,
                                        border_color.blue() as f64,
                                        border_color.alpha() as f64 * ease_progress,
                                    );
                                    cr.set_line_width(2.0);
                                }
                                cr.stroke().unwrap();
                            } else {
                                cr.new_path();
                            }
                        };

                        let draw_outer_stroke = |cr: &cairo::Context| {
                            if outer_border_color.alpha() > 0.001 {
                                cr.new_path();
                                cr.arc(cx, cy, stroke_outer_radius, start_angle, end_angle);
                                cr.set_source_rgba(
                                    outer_border_color.red() as f64,
                                    outer_border_color.green() as f64,
                                    outer_border_color.blue() as f64,
                                    outer_border_color.alpha() as f64 * ease_progress,
                                );
                                cr.set_line_width(2.0);
                                cr.stroke().unwrap();
                            }
                        };

                        if is_hovered {
                            // Preserve path for wedge stroke to be drawn after
                            let path = cr.copy_path().unwrap();
                            cr.new_path(); // Clear it so outer stroke doesn't stroke it

                            if state_ref.hover_visual_cue != "outwards" {
                                draw_outer_stroke(cr);
                            }

                            cr.append_path(&path);
                            draw_wedge_stroke(cr);
                        } else {
                            // Wedge stroke first, then outer stroke on top
                            draw_wedge_stroke(cr);
                            draw_outer_stroke(cr);
                        }

                        // Draw icon if present (labels are only in the center hub now)
                        let mid_angle = (start_angle + end_angle) / 2.0;
                        let r_center = (stroke_inner_radius + stroke_outer_radius) / 2.0;

                        let arc_width = r_center * angle_per_slice;
                        let radial_depth = stroke_outer_radius - stroke_inner_radius;
                        let max_space = arc_width.min(radial_depth);
                        let icon_size = (max_space * 0.5).clamp(16.0, 64.0);

                        if let Some(icon_name) = &item.icon {
                            if icon_name.chars().count() == 1 {
                                let ix = cx + r_center * mid_angle.cos();
                                let iy = cy + r_center * mid_angle.sin();
                                let _ = cr.save();

                                if state_ref.hovered_index == Some(i) && !state_ref.is_closing {
                                    cr.set_source_rgba(
                                        hover_icon_color.red() as f64,
                                        hover_icon_color.green() as f64,
                                        hover_icon_color.blue() as f64,
                                        hover_icon_color.alpha() as f64 * ease_progress,
                                    );
                                } else {
                                    cr.set_source_rgba(
                                        icon_color.red() as f64,
                                        icon_color.green() as f64,
                                        icon_color.blue() as f64,
                                        icon_color.alpha() as f64 * ease_progress,
                                    );
                                }

                                let font_size = 48u32;
                                let key = (icon_name.clone(), font_size);
                                let layout = if let Some(l) = state_ref.text_layout_cache.get(&key)
                                {
                                    l.clone()
                                } else {
                                    let l = area.create_pango_layout(Some(icon_name));
                                    let mut font_desc = gtk::pango::FontDescription::new();
                                    font_desc.set_family("Sans");
                                    let weight = if state_ref.bold_single_chars {
                                        gtk::pango::Weight::Bold
                                    } else {
                                        gtk::pango::Weight::Normal
                                    };
                                    font_desc.set_weight(weight);
                                    font_desc.set_size(gtk::pango::units_from_double(64.0 * 0.75));
                                    l.set_font_description(Some(&font_desc));
                                    state_ref.text_layout_cache.insert(key, l.clone());
                                    l
                                };

                                let (pango_w, pango_h) = layout.pixel_size();
                                let scale = (icon_size * ease_progress) / 64.0;

                                if scale > 0.001 {
                                    cr.translate(ix, iy);
                                    cr.scale(scale, scale);
                                    cr.move_to(-(pango_w as f64 / 2.0), -(pango_h as f64 / 2.0));
                                    pangocairo::functions::show_layout(&cr, &layout);
                                }

                                let _ = cr.restore();
                            } else if let Some(&codepoint) = state_ref.codepoints.get(icon_name) {
                                let ix = cx + r_center * mid_angle.cos();
                                let iy = cy + r_center * mid_angle.sin();
                                let layout =
                                    state_ref.material_glyph_layout(&area, codepoint, icon_size);
                                let (ink, _logical) = layout.pixel_extents();
                                let _ = cr.save();
                                cr.translate(ix, iy);
                                if ease_progress > 0.001 {
                                    cr.scale(ease_progress, ease_progress);
                                }
                                cr.move_to(
                                    -(ink.x() as f64 + ink.width() as f64 / 2.0),
                                    -(ink.y() as f64 + ink.height() as f64 / 2.0),
                                );
                                if state_ref.hovered_index == Some(i) && !state_ref.is_closing {
                                    cr.set_source_rgba(
                                        hover_icon_color.red() as f64,
                                        hover_icon_color.green() as f64,
                                        hover_icon_color.blue() as f64,
                                        hover_icon_color.alpha() as f64 * ease_progress,
                                    );
                                } else {
                                    cr.set_source_rgba(
                                        icon_color.red() as f64,
                                        icon_color.green() as f64,
                                        icon_color.blue() as f64,
                                        icon_color.alpha() as f64 * ease_progress,
                                    );
                                }
                                let _ = pangocairo::functions::show_layout(&cr, &layout);
                                let _ = cr.restore();
                            } else if let Some(Some(surf)) = state_ref.icon_cache.get(icon_name) {
                                let current_w = surf.width() as f64;
                                let current_h = surf.height() as f64;
                                let scale =
                                    (icon_size * ease_progress) / current_w.max(current_h).max(1.0);
                                if scale > 0.001 {
                                    let _ = cr.save();
                                    cr.translate(
                                        cx + r_center * mid_angle.cos(),
                                        cy + r_center * mid_angle.sin(),
                                    );
                                    cr.scale(scale, scale);
                                    let _ = cr.set_source_surface(
                                        surf,
                                        -current_w / 2.0,
                                        -current_h / 2.0,
                                    );
                                    let _ = cr.paint_with_alpha(ease_progress);
                                    let _ = cr.restore();
                                }
                            }
                        }
                    }
                }

                // If using 'expand outwards', draw a continuous base outer ring to mask the Wayland blur edge
                // We draw this AFTER the wedge loop so that it completely covers the wedge strokes and prevents blur leakage
                if state_ref.enable_blur && state_ref.hover_visual_cue == "outwards" {
                    let base_outer = (BASE_R + SLICE_WIDTH - 0.5) * ease_progress;

                    let draw_full = || {
                        cr.new_path();
                        cr.arc(cx, cy, base_outer, 0.0, 2.0 * PI);
                        cr.set_source_rgba(
                            outer_border_color.red() as f64,
                            outer_border_color.green() as f64,
                            outer_border_color.blue() as f64,
                            outer_border_color.alpha() as f64 * ease_progress,
                        );
                        cr.set_line_width(2.0);
                        cr.stroke().unwrap();
                    };

                    if let Some(hovered_i) = state_ref.hovered_index {
                        if !state_ref.is_closing && hovered_i < n {
                            let angle_per_slice = 2.0 * PI / n as f64;
                            let mut start_angle = hovered_i as f64 * angle_per_slice - PI / 2.0;
                            if state_ref.center_layout {
                                start_angle -= angle_per_slice / 2.0;
                            }
                            let end_angle = start_angle + angle_per_slice;

                            cr.set_source_rgba(
                                outer_border_color.red() as f64,
                                outer_border_color.green() as f64,
                                outer_border_color.blue() as f64,
                                outer_border_color.alpha() as f64 * ease_progress,
                            );

                            // Draw unhovered segment (2px width)
                            cr.new_path();
                            cr.arc(cx, cy, base_outer, end_angle, start_angle + 2.0 * PI);
                            cr.set_line_width(2.0);
                            cr.stroke().unwrap();
                        } else {
                            draw_full();
                        }
                    } else {
                        draw_full();
                    }
                }

                draw_hub(&mut state_ref);

                // Re-stroke the inner arc for the hovered slice to cover the hub's active border
                if let Some(hovered_i) = state_ref.hovered_index {
                    let n_items = display_items.len();
                    if !state_ref.is_closing && hovered_i < n_items {
                        let angle_per_slice = 2.0 * PI / n_items as f64;
                        let mut start_angle = hovered_i as f64 * angle_per_slice - PI / 2.0;
                        if state_ref.center_layout {
                            start_angle -= angle_per_slice / 2.0;
                        }
                        let end_angle = start_angle + angle_per_slice;

                        let hp = if hovered_i < state_ref.hover_progresses.len() {
                            state_ref.hover_progresses[hovered_i]
                        } else {
                            0.0
                        };

                        let hp_curr = hp;
                        let hp_prev = if state_ref.hover_progresses.len() > 0 {
                            state_ref.hover_progresses[(hovered_i + n_items - 1) % n_items]
                        } else {
                            0.0
                        };
                        let hp_next = if state_ref.hover_progresses.len() > 0 {
                            state_ref.hover_progresses[(hovered_i + 1) % n_items]
                        } else {
                            0.0
                        };

                        let mut start_a = start_angle;
                        let mut end_a = end_angle;

                        if state_ref.hover_visual_cue == "sides" {
                            let hover_angle_grow = HOVER_GROW / (BASE_R + SLICE_WIDTH);
                            start_a += (hp_prev - hp_curr) * hover_angle_grow;
                            end_a += (hp_curr - hp_next) * hover_angle_grow;
                        }

                        let mut stroke_outer_radius = (BASE_R + SLICE_WIDTH - 0.5) * ease_progress;
                        if state_ref.hover_visual_cue == "outwards" {
                            stroke_outer_radius = (BASE_R + SLICE_WIDTH + (hp_curr * HOVER_GROW)
                                - 0.5)
                                * ease_progress;
                        }
                        let stroke_inner_radius = BASE_R * ease_progress;

                        if hover_border_color.alpha() > 0.001 {
                            cr.new_path();
                            cr.arc(cx, cy, stroke_inner_radius, start_a, end_a);
                            cr.line_to(
                                cx + stroke_outer_radius * end_a.cos(),
                                cy + stroke_outer_radius * end_a.sin(),
                            );
                            cr.arc_negative(cx, cy, stroke_outer_radius, end_a, start_a);
                            cr.close_path();

                            cr.set_line_join(cairo::LineJoin::Round);
                            cr.set_source_rgba(
                                hover_border_color.red() as f64,
                                hover_border_color.green() as f64,
                                hover_border_color.blue() as f64,
                                hover_border_color.alpha() as f64 * ease_progress,
                            );
                            cr.set_line_width(3.0);
                            cr.stroke().unwrap();
                        }
                    }
                }
            }
        });
        window.set_child(Some(&drawing_area));

        // Pausable frame clock controller: ensures 0.0% CPU when stationary/closed
        let is_animating = Rc::new(std::cell::Cell::new(false));
        let last_frame_time = Rc::new(RefCell::new(None));

        let trigger_anim = {
            let is_animating = is_animating.clone();
            let last_frame_time = last_frame_time.clone();
            let tick_state = state.clone();
            let area_clone_tick = drawing_area.clone();
            let window_clone_tick = window.clone();
            let menu_config_tick = menu_config.clone();

            Rc::new(move || {
                if is_animating.get() {
                    return;
                }

                is_animating.set(true);
                *last_frame_time.borrow_mut() = None;

                let state_tick = tick_state.clone();
                let area_tick = area_clone_tick.clone();
                let win_tick = window_clone_tick.clone();
                let config_tick = menu_config_tick.clone();
                let anim_flag = is_animating.clone();
                let lft = last_frame_time.clone();

                area_clone_tick.add_tick_callback(move |_widget, frame_clock| {
                    let mut state = match state_tick.try_borrow_mut() {
                        Ok(s) => s,
                        Err(_) => return glib::ControlFlow::Continue,
                    };
                    let now = frame_clock.frame_time();

                    let dt = if let Some(last) = *lft.borrow() {
                        let actual_dt = (now - last) as f64 / 1_000_000.0;
                        actual_dt.min(0.050)
                    } else {
                        0.0
                    };
                    *lft.borrow_mut() = Some(now);

                    // Close instantly
                    if state.is_closing {
                        state.is_closing = false;
                        win_tick.hide();

                        // Reset to root menu
                        state.root_items = config_tick.menu.clone();
                        state.root_icon = config_tick.icon.clone();
                        state.reset_to_root();
                        if let Some(display) = gdk::Display::default() {
                            state.preload_icons(&display);
                        }
                        anim_flag.set(false);
                        return glib::ControlFlow::Break;
                    }

                    let n = state.get_display_items_count();
                    if state.hover_progresses.len() != n {
                        state.hover_progresses.resize(n, 0.0);
                    }

                    let mut still_animating = false;
                    let mut needs_redraw = false;

                    for i in 0..n {
                        let target = if state.hovered_index == Some(i) {
                            1.0
                        } else {
                            0.0
                        };
                        let diff = target - state.hover_progresses[i];
                        if state.disable_hover_animation {
                            if diff != 0.0 {
                                state.hover_progresses[i] = target;
                                needs_redraw = true;
                            }
                        } else if diff.abs() > 0.005 {
                            let step = dt / 0.080;
                            state.hover_progresses[i] += diff.signum() * step.min(diff.abs());
                            still_animating = true;
                            needs_redraw = true;
                        } else if diff != 0.0 {
                            state.hover_progresses[i] = target;
                            needs_redraw = true;
                        }
                    }

                    if needs_redraw {
                        area_tick.queue_draw();
                    }

                    if !still_animating {
                        anim_flag.set(false);
                        glib::ControlFlow::Break
                    } else {
                        glib::ControlFlow::Continue
                    }
                });
            })
        };

        // 5. Connect motion events to handle slice hover calculations
        let motion_controller = gtk::EventControllerMotion::new();
        let motion_state = state.clone();
        let area_clone = drawing_area.clone();
        let trigger_anim_motion = trigger_anim.clone();
        motion_controller.connect_motion(move |_ctrl, x, y| {
            let mut state = match motion_state.try_borrow_mut() {
                Ok(s) => s,
                Err(_) => return,
            };
            if state.is_closing {
                return;
            }

            let width = area_clone.width() as f64;
            let height = area_clone.height() as f64;
            let cx = width / 2.0;
            let cy = height / 2.0;

            let hovered = state.hit_test(x, y, cx, cy);
            let mut hovered_changed = false;

            if state.hovered_index != hovered {
                state.hovered_index = hovered;
                hovered_changed = true;
            }

            drop(state);
            if hovered_changed {
                trigger_anim_motion();
            }
        });

        let leave_state = state.clone();
        let trigger_anim_leave = trigger_anim.clone();
        motion_controller.connect_leave(move |_ctrl| {
            let mut hovered_changed = false;
            if let Ok(mut state) = leave_state.try_borrow_mut() {
                if state.hovered_index.is_some() {
                    state.hovered_index = None;
                    hovered_changed = true;
                }
            }
            if hovered_changed {
                trigger_anim_leave();
            }
        });
        window.add_controller(motion_controller);

        // 6. Connect mouse press controller for navigation and execution triggers
        let click_controller = gtk::GestureClick::new();
        click_controller.set_button(0); // Any mouse button


        let click_state = state.clone();
        let area_clone_click = drawing_area.clone();
        let trigger_anim_click = trigger_anim.clone();
        click_controller.connect_released(move |gesture, _n_press, x, y| {
            let button = gesture.current_button();

            let mut state = match click_state.try_borrow_mut() {
                Ok(s) => s,
                Err(_) => return,
            };



            if button == 3 {
                // Right click goes back, or dismisses launcher if at root
                if !go_back(&mut state, &area_clone_click) {
                    state.is_closing = true;
                }
                drop(state);
                trigger_anim_click();
                return;
            }

            if button == 1 {
                // Left click
                if state.is_closing {
                    return;
                }

                let width = area_clone_click.width() as f64;
                let height = area_clone_click.height() as f64;
                let cx = width / 2.0;
                let cy = height / 2.0;

                let mx = x - cx;
                let my = y - cy;
                let dist = (mx * mx + my * my).sqrt();

                let mut activated = false;

                // Center hub click - goes back in history if not at root
                if dist < BASE_R {
                    if !state.history.is_empty() {
                        go_back(&mut state, &area_clone_click);
                        activated = true;
                    }
                } else if let Some(index) = state.hit_test(x, y, cx, cy) {
                    activate_index(&mut state, index, &area_clone_click);
                    activated = true;
                }

                if !activated {
                    // Clicked outside active zones (outside max_interactive_dist or center hub when at root)
                    debug!("Clicked outside active area, closing");
                    state.is_closing = true;
                }

                drop(state);
                trigger_anim_click();
            }
        });
        window.add_controller(click_controller);

        // GestureDrag for button 8 (Back) to ignore drag threshold cancellations
        let back_drag = gtk::GestureDrag::new();
        back_drag.set_button(8);
        let back_state = state.clone();
        let back_area = drawing_area.clone();
        let trigger_back = trigger_anim.clone();
        back_drag.connect_drag_end(move |_gesture, _offset_x, _offset_y| {
            let mut state = match back_state.try_borrow_mut() {
                Ok(s) => s,
                Err(_) => return,
            };
            if !go_back(&mut state, &back_area) {
                state.is_closing = true;
            }
            drop(state);
            trigger_back();
        });
        window.add_controller(back_drag);

        // GestureDrag for button 9 (Forward) to ignore drag threshold cancellations
        let forward_drag = gtk::GestureDrag::new();
        forward_drag.set_button(9);
        let forward_state = state.clone();
        let forward_area = drawing_area.clone();
        let trigger_forward = trigger_anim.clone();
        forward_drag.connect_drag_end(move |_gesture, _offset_x, _offset_y| {
            let mut state = match forward_state.try_borrow_mut() {
                Ok(s) => s,
                Err(_) => return,
            };
            if go_forward(&mut state, &forward_area) {
                drop(state);
                trigger_forward();
            }
        });
        window.add_controller(forward_drag);

        // 7. Keyboard listener to navigate using Tab / arrow keys and activate via Enter / Space
        let key_controller = gtk::EventControllerKey::new();
        let key_state = state.clone();
        let area_clone_key = drawing_area.clone();
        let trigger_anim_key = trigger_anim.clone();
        key_controller.connect_key_pressed(move |_ctrl, key, _keycode, _state| {
            let mut state = match key_state.try_borrow_mut() {
                Ok(s) => s,
                Err(_) => return glib::Propagation::Proceed,
            };
            if state.is_closing {
                return glib::Propagation::Proceed;
            }

            let n = state.get_display_items_count();
            if n == 0 {
                return glib::Propagation::Proceed;
            }

            if _state.contains(gdk::ModifierType::ALT_MASK) {
                if key == gdk::Key::Left {
                    if go_back(&mut state, &area_clone_key) {
                        state.hovered_index = Some(0);
                        drop(state);
                        trigger_anim_key();
                    }
                    return glib::Propagation::Stop;
                } else if key == gdk::Key::Right {
                    if go_forward(&mut state, &area_clone_key) {
                        state.hovered_index = Some(0);
                        drop(state);
                        trigger_anim_key();
                    }
                    return glib::Propagation::Stop;
                }
            }

            match key {
                gdk::Key::Escape => {
                    debug!("Escape pressed, initiating close animation");
                    state.is_closing = true;
                    drop(state);
                    trigger_anim_key();
                    glib::Propagation::Stop
                }
                gdk::Key::BackSpace => {
                    if !state.history.is_empty() {
                        if go_back(&mut state, &area_clone_key) {
                            state.hovered_index = Some(0);
                            drop(state);
                            trigger_anim_key();
                        }
                    } else {
                        debug!("Backspace pressed at root menu, initiating close animation");
                        state.is_closing = true;
                        drop(state);
                        trigger_anim_key();
                    }
                    glib::Propagation::Stop
                }
                gdk::Key::Tab | gdk::Key::Down | gdk::Key::Right => {
                    // Cycle forward
                    let next = match state.hovered_index {
                        Some(idx) => (idx + 1) % n,
                        None => 0,
                    };
                    state.hovered_index = Some(next);
                    drop(state);
                    trigger_anim_key();
                    glib::Propagation::Stop
                }
                gdk::Key::ISO_Left_Tab | gdk::Key::Up | gdk::Key::Left => {
                    // Cycle backward
                    let next = match state.hovered_index {
                        Some(idx) => (idx + n - 1) % n,
                        None => n - 1,
                    };
                    state.hovered_index = Some(next);
                    drop(state);
                    trigger_anim_key();
                    glib::Propagation::Stop
                }
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => {
                    // Activate hovered item
                    if let Some(idx) = state.hovered_index {
                        let old_history_len = state.history.len();
                        activate_index(&mut state, idx, &area_clone_key);
                        if state.history.len() != old_history_len {
                            state.hovered_index = Some(0);
                        }
                        drop(state);
                        trigger_anim_key();
                    } else {
                        drop(state);
                    }
                    glib::Propagation::Stop
                }
                _ => {
                    if let Some(ch) = key.to_unicode() {
                        let upper_ch = ch.to_ascii_uppercase();
                        let display_items = state.get_display_items();
                        let mut activated = false;

                        // 1. Check for manual quick_select_key match
                        let old_history_len = state.history.len();
                        for (i, item) in display_items.iter().enumerate() {
                            if let Some(q) = item.quick_select_key {
                                if q.to_ascii_uppercase() == upper_ch {
                                    activate_index(&mut state, i, &area_clone_key);
                                    activated = true;
                                    break;
                                }
                            }
                        }

                        // 2. Check for default 1-0 fallback
                        if !activated && ch.is_ascii_digit() {
                            let digit = ch.to_digit(10).unwrap();
                            let target_index = if digit == 0 { 9 } else { (digit - 1) as usize };

                            if target_index < display_items.len() {
                                activate_index(&mut state, target_index, &area_clone_key);
                                activated = true;
                            }
                        }

                        if activated && state.history.len() != old_history_len {
                            state.hovered_index = Some(0);
                        }

                        drop(state);
                        if activated {
                            trigger_anim_key();
                            glib::Propagation::Stop
                        } else {
                            glib::Propagation::Proceed
                        }
                    } else {
                        drop(state);
                        glib::Propagation::Proceed
                    }
                }
            }
        });
        window.add_controller(key_controller);

        // 9. Watch for focus loss to trigger close animation
        let focus_state = state.clone();
        let trigger_anim_focus = trigger_anim.clone();
        window.connect_notify_local(Some("is-active"), move |w, _| {
            if !w.is_active() {
                if let Ok(mut state) = focus_state.try_borrow_mut() {
                    if state.suppress_focus_loss.get() {
                        debug!("Window lost focus, but suppression is active. Ignoring.");
                        return;
                    }
                    debug!("Window lost focus, initiating close animation");
                    state.is_closing = true;
                    drop(state);
                    trigger_anim_focus();
                }
            }
        });

        // 10. Setup tokio channel to listen for IPC socket events
        let (ipc_tx, mut ipc_rx) = tokio::sync::mpsc::unbounded_channel::<IpcMessage>();

        // Start the IPC server and forward its commands to this channel
        let socket_path = launcher_ipc::get_socket_path();
        let ipc_tx_server = ipc_tx.clone();
        let server_handle = launcher_ipc::start_server(socket_path, move |msg| {
            let _ = ipc_tx_server.send(msg);
        })?;

        // Keep the server handle alive as long as the window exists
        let server_handle_wrapper = Arc::new(Mutex::new(Some(server_handle)));
        let server_handle_clone = server_handle_wrapper.clone();

        // Monitor config and menu files using exact same logic as theme_editor
        let config_file = gtk::gio::File::for_path(&config_path);
        let ipc_tx_config = ipc_tx.clone();
        if let Ok(monitor) = config_file.monitor_file(
            gtk::gio::FileMonitorFlags::NONE,
            gtk::gio::Cancellable::NONE,
        ) {
            monitor.connect_changed(move |_, _, _, event| {
                if event == gtk::gio::FileMonitorEvent::ChangesDoneHint
                    || event == gtk::gio::FileMonitorEvent::Created
                {
                    let tx = ipc_tx_config.clone();
                    gtk::glib::MainContext::default().invoke(move || {
                        let _ = tx.send(IpcMessage::ReloadConfig);
                    });
                }
            });
            if let Ok(mut s) = state.try_borrow_mut() {
                s._config_monitor = Some(monitor);
            }
        }

        let menu_file = gtk::gio::File::for_path(&menu_path);
        let ipc_tx_menu = ipc_tx.clone();
        if let Ok(monitor) = menu_file.monitor_file(
            gtk::gio::FileMonitorFlags::NONE,
            gtk::gio::Cancellable::NONE,
        ) {
            monitor.connect_changed(move |_, _, _, event| {
                if event == gtk::gio::FileMonitorEvent::ChangesDoneHint
                    || event == gtk::gio::FileMonitorEvent::Created
                {
                    let tx = ipc_tx_menu.clone();
                    gtk::glib::MainContext::default().invoke(move || {
                        let _ = tx.send(IpcMessage::ReloadConfig);
                    });
                }
            });
            if let Ok(mut s) = state.try_borrow_mut() {
                s._menu_monitor = Some(monitor);
            }
        }

        let ipc_state = state.clone();
        let theme_provider_clone = theme_provider.clone();
        let user_provider_clone = user_provider.clone();
        let config_path_clone = config_path.clone();
        let trigger_anim_ipc = trigger_anim.clone();

        let window_clone_ipc = window.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Some(msg) = ipc_rx.recv().await {
                debug!("Received IPC message in UI thread: {:?}", msg);
                match msg {
                    IpcMessage::Toggle => {
                        let is_visible = window_clone_ipc.is_visible();
                        let mut state = match ipc_state.try_borrow_mut() {
                            Ok(s) => s,
                            Err(_) => continue,
                        };
                        if is_visible && !state.is_closing {
                            info!("Hiding window via IPC Toggle");
                            state.is_closing = true;
                            drop(state);
                            trigger_anim_ipc();
                        } else {
                            info!("Showing window via IPC Toggle");
                            state.reset_to_root();
                            if let Some(display) = gdk::Display::default() {
                                state.preload_icons(&display);
                            }
                            state.is_closing = false;
                            *state.theme_colors.borrow_mut() = None;
                            drop(state);

                            load_and_apply_theme(
                                &config_path_clone,
                                &theme_provider_clone,
                                &user_provider_clone,
                            );
                            window_clone_ipc.present();
                            trigger_anim_ipc();
                        }
                    }
                    IpcMessage::Close => {
                        let mut state = match ipc_state.try_borrow_mut() {
                            Ok(s) => s,
                            Err(_) => continue,
                        };
                        if window_clone_ipc.is_visible() && !state.is_closing {
                            info!("Hiding window via IPC Close");
                            state.is_closing = true;
                            drop(state);
                            trigger_anim_ipc();
                        }
                    }
                    IpcMessage::OpenMenu {
                        menu_path: new_menu_path,
                    } => {
                        let is_visible = window_clone_ipc.is_visible();
                        let mut state = match ipc_state.try_borrow_mut() {
                            Ok(s) => s,
                            Err(_) => continue,
                        };

                        let same_menu = state.current_menu_path == new_menu_path;
                        state.current_menu_path = new_menu_path.clone();

                        match launcher_core::load_menu(&new_menu_path) {
                            Ok(m) => {
                                state.root_items = m.menu.clone();
                                state.root_icon = m.icon.clone();
                                state.reset_to_root();
                                if let Some(display) = gdk::Display::default() {
                                    state.preload_icons(&display);
                                }
                            }
                            Err(e) => {
                                tracing::error!(
                                    "Failed to load new menu from {:?}: {}",
                                    new_menu_path,
                                    e
                                );
                            }
                        }

                        if !is_visible || state.is_closing {
                            state.is_closing = false;
                            *state.theme_colors.borrow_mut() = None;
                            drop(state);

                            load_and_apply_theme(
                                &config_path_clone,
                                &theme_provider_clone,
                                &user_provider_clone,
                            );
                            window_clone_ipc.present();
                            trigger_anim_ipc();
                        } else if !same_menu {
                            drop(state);
                            trigger_anim_ipc();
                        } else {
                            // same_menu is true, and it's currently visible and not closing
                            tracing::info!("Hiding window via IPC OpenMenu (toggle)");
                            state.is_closing = true;
                            drop(state);
                            trigger_anim_ipc();
                        }
                    }
                    IpcMessage::Open => {
                        let is_visible = window_clone_ipc.is_visible();
                        let is_closing = ipc_state
                            .try_borrow()
                            .map(|s| s.is_closing)
                            .unwrap_or(false);
                        if !is_visible || is_closing {
                            tracing::info!("Showing window via IPC Open");
                            if let Ok(mut state) = ipc_state.try_borrow_mut() {
                                state.reset_to_root();
                                if let Some(display) = gdk::Display::default() {
                                    state.preload_icons(&display);
                                }
                                state.is_closing = false;
                                *state.theme_colors.borrow_mut() = None;
                            }

                            load_and_apply_theme(
                                &config_path_clone,
                                &theme_provider_clone,
                                &user_provider_clone,
                            );
                            window_clone_ipc.present();
                            trigger_anim_ipc();
                        }
                    }
                    IpcMessage::ReloadConfig => {
                        info!("Reload config request received via IPC");
                        // 1. Reload the theme CSS from file
                        load_and_apply_theme(
                            &config_path_clone,
                            &theme_provider_clone,
                            &user_provider_clone,
                        );

                        // 2. Reload the menu TOML config from file
                        let mut state = match ipc_state.try_borrow_mut() {
                            Ok(s) => s,
                            Err(_) => continue,
                        };
                        state.codepoints =
                            launcher_core::load_material_codepoints(&config_path_clone);
                        let mut blur_needs_update = false;
                        if let Ok(cfg) = launcher_core::load_config(&config_path_clone) {
                            state.extra_radius = cfg.ui.extra_radius;
                            state.pill_roundness = cfg.ui.pill_roundness;
                            state.use_symbolic_icons = cfg.ui.use_symbolic_icons;
                            state.bold_single_chars = cfg.ui.bold_single_chars;
                            state.center_layout = cfg.ui.center_layout;
                            if state.disable_hover_animation != cfg.ui.disable_hover_animation {
                                state.disable_hover_animation = cfg.ui.disable_hover_animation;
                            }
                            if state.menu_style != cfg.ui.menu_style {
                                state.menu_style = cfg.ui.menu_style.clone();
                            }
                            if state.hover_visual_cue != cfg.ui.hover_visual_cue {
                                state.hover_visual_cue = cfg.ui.hover_visual_cue.clone();
                            }
                            let new_blur = cfg.ui.enable_blur && cfg.ui.menu_style != "floating";
                            if state.enable_blur != new_blur {
                                state.enable_blur = new_blur;
                                blur_needs_update = true;
                            }
                            state.icon_cache.clear();
                            state.text_layout_cache.clear();
                            state.label_layout_cache.clear();
                            *state.theme_colors.borrow_mut() = None;
                            if let Some(display) = gdk::Display::default() {
                                state.preload_icons(&display);
                            }
                            info!("Reloaded extra_radius: {}", state.extra_radius);
                        }

                        if blur_needs_update {
                            let radius = if state.enable_blur {
                                BASE_R + SLICE_WIDTH + HOVER_GROW
                            } else {
                                0.0
                            };
                            // The WaylandBlur is normally created at realize time,
                            // only when blur is already enabled. If it was enabled
                            // at runtime (e.g. switching to pie mode), create it now.
                            if state.enable_blur && wayland_blur.borrow().is_none() {
                                if let Some(blur) = wayland::WaylandBlur::new(&window_clone_ipc) {
                                    *wayland_blur.borrow_mut() = Some(blur);
                                }
                            }
                            let cx = window_clone_ipc.width() as f64 / 2.0;
                            let cy = window_clone_ipc.height() as f64 / 2.0;
                            state.last_cx = cx;
                            state.last_cy = cy;
                            if let Some(blur) = wayland_blur.borrow().as_ref() {
                                blur.update_circular_region(radius, cx, cy);
                            }
                        }
                        let current_path = { state.current_menu_path.clone() };
                        match launcher_core::load_menu(&current_path) {
                            Ok(m) => {
                                state.root_items = m.menu.clone();
                                state.root_icon = m.icon.clone();
                                state.reset_to_root();
                                if let Some(display) = gdk::Display::default() {
                                    state.preload_icons(&display);
                                }
                                info!("Menu config reloaded successfully");
                            }
                            Err(e) => {
                                error!("Failed to reload menu config: {}", e);
                            }
                        }
                        drop(state);
                        trigger_anim_ipc();
                    }
                }
            }
        });

        // When window is destroyed, shut down the IPC server to release the socket
        window.connect_destroy(move |_| {
            debug!("Window destroyed, shutting down IPC server");
            if let Some(handle) = server_handle_clone.lock().unwrap().take() {
                handle.shutdown();
            }
        });

        // Intercept close-request from compositor to hide window instead of destroying
        window.connect_close_request(move |w| {
            debug!("Close request received, hiding window instead");
            w.hide();
            glib::Propagation::Stop
        });

        if !start_hidden {
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                state_mut.is_closing = false;
            }
            window.present();
            trigger_anim();
        } else {
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                state_mut.is_closing = false;
            }
            window.hide();
        }
        Ok(())
    }
}
