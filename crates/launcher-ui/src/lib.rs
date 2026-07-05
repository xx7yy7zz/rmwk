use gdk::prelude::*;
use gdk4 as gdk;
use gtk::prelude::*;
use gtk4 as gtk;
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

struct MenuState {
    root_items: Vec<launcher_core::MenuItem>,
    current_items: Vec<launcher_core::MenuItem>,
    history: Vec<Vec<launcher_core::MenuItem>>,
    hovered_index: Option<usize>,

    // Animation state
    is_opening: bool,
    is_closing: bool,
    open_progress: f64,         // 0.0 -> 1.0
    hover_progresses: Vec<f64>, // 0.0 -> 1.0 for each slice

    // Cached icons to avoid loading on every frame tick
    icon_cache: HashMap<String, Option<gtk::gdk_pixbuf::Pixbuf>>,

    // Extra interactivity margin beyond slices
    extra_radius: f64,

    // Material Symbols codepoints index
    codepoints: HashMap<String, char>,

    // FPS counter state
    fps_last_time: std::time::Instant,
    fps_frame_count: u32,
    fps_current: f64,
}

impl MenuState {
    fn get_display_items(&self) -> Vec<launcher_core::MenuItem> {
        let mut items = self.current_items.clone();
        if !self.history.is_empty() {
            items.push(launcher_core::MenuItem {
                label: "Back".to_string(),
                icon: Some("go-previous".to_string()),
                action: None,
                children: vec![],
            });
        }
        items
    }

    fn get_display_items_count(&self) -> usize {
        if self.history.is_empty() {
            self.current_items.len()
        } else {
            self.current_items.len() + 1
        }
    }

    fn preload_icons(&mut self, display: &gdk::Display) {
        let display_items = self.get_display_items();
        for item in &display_items {
            if let Some(icon_name) = &item.icon {
                if self.codepoints.contains_key(icon_name) {
                    continue;
                }
                if !self.icon_cache.contains_key(icon_name) {
                    let pixbuf = load_icon_pixbuf(display, icon_name, 24);
                    self.icon_cache.insert(icon_name.clone(), pixbuf);
                }
            }
        }
    }
}

fn load_icon_pixbuf(
    display: &gdk::Display,
    icon_name: &str,
    size: i32,
) -> Option<gtk::gdk_pixbuf::Pixbuf> {
    let icon_theme = gtk::IconTheme::for_display(display);
    let paintable = icon_theme.lookup_icon(
        icon_name,
        &[],
        size,
        1,
        gtk::TextDirection::None,
        gtk::IconLookupFlags::empty(),
    );

    if let Some(file) = paintable.file() {
        if let Some(path) = file.path() {
            return gtk::gdk_pixbuf::Pixbuf::from_file_at_size(path, size, size).ok();
        }
    }
    None
}

fn load_and_apply_theme(config_path: &Path, theme_provider: &gtk::CssProvider) {
    let theme_name = match launcher_core::load_config(config_path) {
        Ok(cfg) => cfg.ui.theme,
        Err(e) => {
            warn!(
                "Failed to load config: {}. Defaulting to theme 'default'",
                e
            );
            "default".to_string()
        }
    };

    let theme_file = config_path
        .parent()
        .map(|p| p.join("themes").join(format!("{}.css", theme_name)))
        .unwrap_or_else(|| {
            PathBuf::from("/home/karim/.config/rmwk/themes").join(format!("{}.css", theme_name))
        });

    debug!("Loading theme from {:?}", theme_file);
    if theme_file.exists() {
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
            .radial-slice { color: rgba(30, 30, 46, 0.90); }
            .radial-slice:hover { color: rgba(49, 50, 68, 0.95); }
            .radial-slice:active { color: rgba(137, 180, 250, 0.40); }
            .radial-slice:selected { color: rgba(137, 180, 250, 0.95); }
            .radial-label { color: rgba(205, 214, 244, 1.0); }
            .radial-label:hover { color: rgba(255, 255, 255, 1.0); }
            .radial-hub { color: rgba(17, 17, 27, 0.95); }
            .radial-hub:active { color: rgba(137, 180, 250, 0.70); }
            .radial-hub:hover { color: rgba(205, 214, 244, 1.0); }
        ";
        theme_provider.load_from_data(std::str::from_utf8(fallback).unwrap());
    }
}

fn activate_index(state: &mut MenuState, index: usize, area: &gtk::DrawingArea) {
    let display_items = state.get_display_items();
    let display_items_count = display_items.len();
    if index >= display_items_count {
        return;
    }

    if !state.history.is_empty() && index == display_items_count - 1 {
        debug!("Back wedge activated, popping history");
        if let Some(prev) = state.history.pop() {
            state.current_items = prev;
            state.hovered_index = None;
            if let Some(display) = gdk::Display::default() {
                state.preload_icons(&display);
            }
            area.queue_draw();
        }
    } else {
        let selected = display_items[index].clone();
        if !selected.children.is_empty() {
            let current_items = state.current_items.clone();
            state.history.push(current_items);
            state.current_items = selected.children;
            state.hovered_index = None;
            if let Some(display) = gdk::Display::default() {
                state.preload_icons(&display);
            }
            area.queue_draw();
        } else if let Some(action) = selected.action {
            info!("Running action: {:?}", action);
            if let Err(e) = launcher_core::run_action(&action) {
                error!("Failed to execute action: {}", e);
            }
            state.is_closing = true;
            state.is_opening = false;
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

    pub fn run(&self) -> i32 {
        let menu_path = self.menu_path.clone();
        let config_path = self.config_path.clone();
        let start_hidden = self.start_hidden;

        self.app.connect_activate(move |app| {
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
        // Load the menu config from disk
        let menu_config = match launcher_core::load_menu(&menu_path) {
            Ok(m) => m,
            Err(e) => {
                warn!(
                    "Failed to load menu config from {:?}, using empty menu: {}",
                    menu_path, e
                );
                launcher_core::MenuConfig { menu: vec![] }
            }
        };

        let window = gtk::ApplicationWindow::new(app);
        window.set_title(Some("rmwk"));

        // 1. Initialize Layer Shell before realizing the window
        window.init_layer_shell();
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
        load_and_apply_theme(&config_path, &theme_provider);

        let font_path = config_path
            .parent()
            .map(|p| p.join("fonts").join("MaterialSymbolsRounded.ttf"))
            .unwrap_or_else(|| {
                PathBuf::from("/home/karim/.config/rmwk/fonts/MaterialSymbolsRounded.ttf")
            });

        let font_provider = gtk::CssProvider::new();
        let font_css = format!(
            "
            @font-face {{
                font-family: 'Material Symbols Rounded';
                src: url('{}');
            }}
        ",
            font_path.to_string_lossy()
        );
        font_provider.load_from_data(&font_css);

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
            gtk::style_context_add_provider_for_display(
                &display,
                &font_provider,
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
            root_items: menu_config.menu.clone(),
            current_items: menu_config.menu.clone(),
            history: vec![],
            hovered_index: None,
            is_opening: true,
            is_closing: false,
            open_progress: 0.0,
            hover_progresses: vec![],
            icon_cache: HashMap::new(),
            extra_radius: ui_config.extra_radius,
            codepoints,
            fps_last_time: std::time::Instant::now(),
            fps_frame_count: 0,
            fps_current: 0.0,
        }));

        if let Some(display) = gdk::Display::default() {
            state.borrow_mut().preload_icons(&display);
        }

        // 4. Create drawing area for radial wedges
        let drawing_area = gtk::DrawingArea::new();
        let draw_state = state.clone();
        drawing_area.set_draw_func(move |area, cr, width, height| {
            let cx = width as f64 / 2.0;
            let cy = height as f64 / 2.0;

            let mut state_ref = draw_state.borrow_mut();

            // Update FPS counter
            let now = std::time::Instant::now();
            state_ref.fps_frame_count += 1;
            let elapsed = now.duration_since(state_ref.fps_last_time).as_secs_f64();
            if elapsed >= 0.5 {
                state_ref.fps_current = state_ref.fps_frame_count as f64 / elapsed;
                state_ref.fps_frame_count = 0;
                state_ref.fps_last_time = now;
            }

            let display_items = state_ref.get_display_items();
            let n = display_items.len();

            let ease_progress = {
                let t = 1.0 - state_ref.open_progress;
                1.0 - t * t * t // Ease-out cubic
            };

            // Clear surface (ensure transparent background is clean)
            cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
            cr.set_operator(cairo::Operator::Source);
            cr.paint().unwrap();
            cr.set_operator(cairo::Operator::Over);

            // Fetch theme colors dynamically from CSS StyleContext
            let context = area.style_context();

            // 1. Get wedge colors
            context.save();
            context.add_class("radial-slice");

            context.set_state(gtk::StateFlags::NORMAL);
            let fill_color = context.color();

            context.set_state(gtk::StateFlags::PRELIGHT);
            let hover_fill_color = context.color();

            context.set_state(gtk::StateFlags::ACTIVE);
            let border_color = context.color();

            context.set_state(gtk::StateFlags::SELECTED);
            let hover_border_color = context.color();

            context.restore();

            // 2. Get label colors
            context.save();
            context.add_class("radial-label");

            context.set_state(gtk::StateFlags::NORMAL);
            let label_color = context.color();

            context.set_state(gtk::StateFlags::PRELIGHT);
            let hover_label_color = context.color();

            context.restore();

            // 3. Get hub colors
            context.save();
            context.add_class("radial-hub");

            context.set_state(gtk::StateFlags::NORMAL);
            let hub_fill = context.color();

            context.set_state(gtk::StateFlags::ACTIVE);
            let hub_border = context.color();

            context.set_state(gtk::StateFlags::PRELIGHT);
            let hub_text_color = context.color();

            context.restore();

            if n > 0 {
                let angle_per_slice = 2.0 * PI / n as f64;

                for i in 0..n {
                    cr.new_path();
                    let item = &display_items[i];
                    let hp = if i < state_ref.hover_progresses.len() {
                        state_ref.hover_progresses[i]
                    } else {
                        0.0
                    };

                    let start_angle = i as f64 * angle_per_slice - PI / 2.0;
                    let end_angle = start_angle + angle_per_slice;

                    let inner_radius = BASE_R * ease_progress;
                    let outer_radius = (BASE_R + SLICE_WIDTH + HOVER_GROW * hp) * ease_progress;

                    // Draw wedge
                    cr.arc(cx, cy, outer_radius, start_angle, end_angle);
                    cr.arc_negative(cx, cy, inner_radius, end_angle, start_angle);
                    cr.close_path();

                    // Fill wedge
                    if state_ref.hovered_index == Some(i) && !state_ref.is_closing {
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
                    cr.fill_preserve().unwrap();

                    // Stroke border
                    if state_ref.hovered_index == Some(i) && !state_ref.is_closing {
                        cr.set_source_rgba(
                            hover_border_color.red() as f64,
                            hover_border_color.green() as f64,
                            hover_border_color.blue() as f64,
                            hover_border_color.alpha() as f64 * ease_progress,
                        );
                        cr.set_line_width(2.0);
                    } else {
                        cr.set_source_rgba(
                            border_color.red() as f64,
                            border_color.green() as f64,
                            border_color.blue() as f64,
                            border_color.alpha() as f64 * ease_progress,
                        );
                        cr.set_line_width(1.0);
                    }
                    cr.stroke().unwrap();

                    // Draw icon if present (labels are only in the center hub now)
                    let mid_angle = start_angle + angle_per_slice / 2.0;
                    let r_center = (inner_radius + outer_radius) / 2.0;

                    let arc_width = r_center * angle_per_slice;
                    let radial_depth = outer_radius - inner_radius;
                    let max_space = arc_width.min(radial_depth);
                    let icon_size = (max_space * 0.5).clamp(16.0, 64.0);

                    if let Some(icon_name) = &item.icon {
                        if icon_name.chars().count() == 1 {
                            let ix = cx + r_center * mid_angle.cos();
                            let iy = cy + r_center * mid_angle.sin();
                            let _ = cr.save();

                            if state_ref.hovered_index == Some(i) && !state_ref.is_closing {
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

                            let glyph_str = icon_name.clone();
                            let layout = area.create_pango_layout(Some(&glyph_str));
                            let mut font_desc = gtk::pango::FontDescription::new();
                            font_desc.set_family("Sans");
                            font_desc.set_weight(gtk::pango::Weight::Bold);
                            // Multiply by 0.75 to convert pixels to points, matching cairo's pixel size
                            font_desc.set_size(gtk::pango::units_from_double(icon_size * 0.75 * ease_progress));
                            layout.set_font_description(Some(&font_desc));

                            let (pango_w, pango_h) = layout.pixel_size();
                            let rx = ix - (pango_w as f64 / 2.0);
                            let ry = iy - (pango_h as f64 / 2.0);

                            cr.move_to(rx, ry);
                            pangocairo::functions::show_layout(&cr, &layout);

                            let _ = cr.restore();
                        } else if let Some(&codepoint) = state_ref.codepoints.get(icon_name) {
                            let ix = cx + r_center * mid_angle.cos();
                            let iy = cy + r_center * mid_angle.sin();
                            let _ = cr.save();
                            cr.select_font_face(
                                "Material Symbols Rounded",
                                cairo::FontSlant::Normal,
                                cairo::FontWeight::Normal,
                            );
                            cr.set_font_size(icon_size * ease_progress);
                            let glyph_str = codepoint.to_string();
                            if let Ok(extents) = cr.text_extents(&glyph_str) {
                                let rx = ix - extents.width() / 2.0 - extents.x_bearing();
                                let ry = iy - extents.height() / 2.0 - extents.y_bearing();
                                cr.move_to(rx, ry);
                                if state_ref.hovered_index == Some(i) && !state_ref.is_closing {
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
                                let _ = cr.show_text(&glyph_str);
                            }
                            let _ = cr.restore();
                        } else if let Some(Some(pixbuf)) = state_ref.icon_cache.get(icon_name) {
                            let current_w = pixbuf.width() as f64;
                            let current_h = pixbuf.height() as f64;
                            let scale =
                                (icon_size * ease_progress) / current_w.max(current_h).max(1.0);
                            if scale > 0.001 {
                                let _ = cr.save();
                                cr.translate(
                                    cx + r_center * mid_angle.cos(),
                                    cy + r_center * mid_angle.sin(),
                                );
                                cr.scale(scale, scale);
                                cr.set_source_pixbuf(pixbuf, -current_w / 2.0, -current_h / 2.0);
                                let _ = cr.paint();
                                let _ = cr.restore();
                            }
                        }
                    }
                }
            }

            // Draw center circular hub
            cr.new_path();
            cr.set_source_rgba(
                hub_fill.red() as f64,
                hub_fill.green() as f64,
                hub_fill.blue() as f64,
                hub_fill.alpha() as f64 * ease_progress,
            );
            cr.arc(cx, cy, BASE_R * ease_progress, 0.0, 2.0 * PI);
            cr.fill_preserve().unwrap();

            cr.set_source_rgba(
                hub_border.red() as f64,
                hub_border.green() as f64,
                hub_border.blue() as f64,
                hub_border.alpha() as f64 * ease_progress,
            );
            cr.set_line_width(2.0);
            cr.stroke().unwrap();

            // Render hub label
            cr.set_source_rgba(
                hub_text_color.red() as f64,
                hub_text_color.green() as f64,
                hub_text_color.blue() as f64,
                hub_text_color.alpha() as f64 * ease_progress,
            );
            cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
            cr.set_font_size(16.0 * ease_progress);

            let center_text = if let Some(idx) = state_ref.hovered_index {
                if idx < display_items.len() {
                    &display_items[idx].label
                } else {
                    "Radial Menu"
                }
            } else if !state_ref.history.is_empty() {
                "Back"
            } else {
                "Radial Menu"
            };

            if let Ok(extents) = cr.text_extents(center_text) {
                cr.move_to(
                    cx - extents.width() / 2.0 - extents.x_bearing(),
                    cy - extents.height() / 2.0 - extents.y_bearing(),
                );
                let _ = cr.show_text(center_text);
            }

            // Draw FPS counter (bright green, centered below the menu)
            let outer_radius = BASE_R + SLICE_WIDTH;
            let fps_str = format!("FPS: {:.1}", state_ref.fps_current);
            let layout = area.create_pango_layout(Some(&fps_str));
            let mut font_desc = gtk::pango::FontDescription::new();
            font_desc.set_family("Sans");
            font_desc.set_weight(gtk::pango::Weight::Bold);
            font_desc.set_size(gtk::pango::units_from_double(14.0 * ease_progress));
            layout.set_font_description(Some(&font_desc));

            let (pango_w, _pango_h) = layout.pixel_size();
            let rx = cx - (pango_w as f64 / 2.0);
            let ry = cy + (outer_radius + 40.0) * ease_progress;

            let _ = cr.save();
            cr.set_source_rgba(0.0, 0.8, 0.0, 1.0 * ease_progress);
            cr.move_to(rx, ry);
            pangocairo::functions::show_layout(&cr, &layout);
            let _ = cr.restore();
        });
        window.set_child(Some(&drawing_area));

        // 5. Connect motion events to handle slice hover calculations
        let motion_controller = gtk::EventControllerMotion::new();
        let motion_state = state.clone();
        let area_clone = drawing_area.clone();
        motion_controller.connect_motion(move |_ctrl, x, y| {
            let mut state = motion_state.borrow_mut();
            if state.is_closing {
                return;
            }

            let width = area_clone.width() as f64;
            let height = area_clone.height() as f64;
            let cx = width / 2.0;
            let cy = height / 2.0;

            let mx = x - cx;
            let my = y - cy;
            let dist = (mx * mx + my * my).sqrt();

            let display_items_count = state.get_display_items_count();
            let max_interactive_dist = BASE_R + SLICE_WIDTH + HOVER_GROW + state.extra_radius;

            if display_items_count > 0 && dist >= BASE_R && dist <= max_interactive_dist {
                let mut angle = my.atan2(mx) + PI / 2.0;
                if angle < 0.0 {
                    angle += 2.0 * PI;
                }

                let angle_per_slice = 2.0 * PI / display_items_count as f64;
                let index = (angle / angle_per_slice) as usize;

                if index < display_items_count {
                    if state.hovered_index != Some(index) {
                        state.hovered_index = Some(index);
                        area_clone.queue_draw();
                    }
                } else {
                    if state.hovered_index.is_some() {
                        state.hovered_index = None;
                        area_clone.queue_draw();
                    }
                }
            } else {
                if state.hovered_index.is_some() {
                    state.hovered_index = None;
                    area_clone.queue_draw();
                }
            }
        });

        let leave_state = state.clone();
        let area_clone_leave = drawing_area.clone();
        motion_controller.connect_leave(move |_ctrl| {
            let mut state = leave_state.borrow_mut();
            if state.hovered_index.is_some() {
                state.hovered_index = None;
                area_clone_leave.queue_draw();
            }
        });
        window.add_controller(motion_controller);

        // 6. Connect mouse press controller for navigation and execution triggers
        let click_controller = gtk::GestureClick::new();
        click_controller.set_button(0); // Any mouse button
        let click_state = state.clone();
        let area_clone_click = drawing_area.clone();
        click_controller.connect_pressed(move |gesture, _n_press, x, y| {
            let button = gesture.current_button();
            debug!("Mouse pressed at ({}, {}), button: {}", x, y, button);

            if button == 3 {
                // Right click dismisses launcher
                let mut state = click_state.borrow_mut();
                state.is_closing = true;
                state.is_opening = false;
                return;
            }

            if button == 1 {
                // Left click
                let mut state = click_state.borrow_mut();
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

                let display_items = state.get_display_items();
                let display_items_count = display_items.len();
                let max_interactive_dist = BASE_R + SLICE_WIDTH + HOVER_GROW + state.extra_radius;
                let mut activated = false;

                // Center hub click - goes back in history if not at root
                if dist < BASE_R {
                    if !state.history.is_empty() {
                        if let Some(prev) = state.history.pop() {
                            state.current_items = prev;
                            state.hovered_index = None;
                            if let Some(display) = gdk::Display::default() {
                                state.preload_icons(&display);
                            }
                            area_clone_click.queue_draw();
                            activated = true;
                        }
                    }
                } else if display_items_count > 0 && dist >= BASE_R && dist <= max_interactive_dist
                {
                    let mut angle = my.atan2(mx) + PI / 2.0;
                    if angle < 0.0 {
                        angle += 2.0 * PI;
                    }

                    let angle_per_slice = 2.0 * PI / display_items_count as f64;
                    let index = (angle / angle_per_slice) as usize;

                    if index < display_items_count {
                        activate_index(&mut state, index, &area_clone_click);
                        activated = true;
                    }
                }

                if !activated {
                    // Clicked outside active zones (outside max_interactive_dist or center hub when at root)
                    debug!("Clicked outside active area, closing");
                    state.is_closing = true;
                    state.is_opening = false;
                }
            }
        });
        window.add_controller(click_controller);

        // 7. Keyboard listener to navigate using Tab / arrow keys and activate via Enter / Space
        let key_controller = gtk::EventControllerKey::new();
        let key_state = state.clone();
        let area_clone_key = drawing_area.clone();
        key_controller.connect_key_pressed(move |_ctrl, key, _keycode, _state| {
            let mut state = key_state.borrow_mut();
            if state.is_closing {
                return glib::Propagation::Proceed;
            }

            let n = state.get_display_items_count();
            if n == 0 {
                return glib::Propagation::Proceed;
            }

            match key {
                gdk::Key::Escape => {
                    debug!("Escape pressed, initiating close animation");
                    state.is_closing = true;
                    state.is_opening = false;
                    glib::Propagation::Stop
                }
                gdk::Key::Tab | gdk::Key::Down | gdk::Key::Right => {
                    // Cycle forward
                    let next = match state.hovered_index {
                        Some(idx) => (idx + 1) % n,
                        None => 0,
                    };
                    state.hovered_index = Some(next);
                    area_clone_key.queue_draw();
                    glib::Propagation::Stop
                }
                gdk::Key::ISO_Left_Tab | gdk::Key::Up | gdk::Key::Left => {
                    // Cycle backward
                    let next = match state.hovered_index {
                        Some(idx) => (idx + n - 1) % n,
                        None => n - 1,
                    };
                    state.hovered_index = Some(next);
                    area_clone_key.queue_draw();
                    glib::Propagation::Stop
                }
                gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::space => {
                    // Activate hovered item
                    if let Some(idx) = state.hovered_index {
                        activate_index(&mut state, idx, &area_clone_key);
                    }
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        window.add_controller(key_controller);

        // 8. Setup scroll events to cycle through wedges
        let scroll_controller =
            gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        let scroll_state = state.clone();
        let area_clone_scroll = drawing_area.clone();
        scroll_controller.connect_scroll(move |_ctrl, _dx, dy| {
            let mut state = scroll_state.borrow_mut();
            if state.is_closing {
                return glib::Propagation::Proceed;
            }

            let n = state.get_display_items_count();
            if n == 0 {
                return glib::Propagation::Proceed;
            }

            if dy > 0.0 {
                let next = match state.hovered_index {
                    Some(idx) => (idx + 1) % n,
                    None => 0,
                };
                state.hovered_index = Some(next);
                area_clone_scroll.queue_draw();
                glib::Propagation::Stop
            } else if dy < 0.0 {
                let next = match state.hovered_index {
                    Some(idx) => (idx + n - 1) % n,
                    None => n - 1,
                };
                state.hovered_index = Some(next);
                area_clone_scroll.queue_draw();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        window.add_controller(scroll_controller);

        // 9. Watch for focus loss to trigger close animation
        let focus_state = state.clone();
        window.connect_notify_local(Some("is-active"), move |w, _| {
            if !w.is_active() {
                debug!("Window lost focus, initiating close animation");
                if let Ok(mut state) = focus_state.try_borrow_mut() {
                    state.is_closing = true;
                    state.is_opening = false;
                }
            }
        });

        // 10. Setup main context tick callback to drive open/close and hover lerps
        let tick_state = state.clone();
        let area_clone_tick = drawing_area.clone();
        let window_clone_tick = window.clone();
        let last_frame_time = Rc::new(RefCell::new(None));
        let menu_config_tick = menu_config.clone();

        drawing_area.add_tick_callback(move |_widget, frame_clock| {
            let mut state = tick_state.borrow_mut();
            let now = frame_clock.frame_time(); // in microseconds

            let dt = if let Some(last) = *last_frame_time.borrow() {
                (now - last) as f64 / 1_000_000.0 // in seconds
            } else {
                0.0
            };
            *last_frame_time.borrow_mut() = Some(now);

            let mut needs_redraw = false;

            // Open transition (~120ms)
            if state.is_opening {
                state.open_progress += dt / 0.120;
                if state.open_progress >= 1.0 {
                    state.open_progress = 1.0;
                    state.is_opening = false;
                }
                needs_redraw = true;
            }

            // Close transition (~80ms)
            if state.is_closing {
                state.open_progress -= dt / 0.080;
                if state.open_progress <= 0.0 {
                    state.open_progress = 0.0;
                    state.is_closing = false;
                    window_clone_tick.hide();

                    // Reset to root menu
                    state.current_items = menu_config_tick.menu.clone();
                    state.history.clear();
                    state.hovered_index = None;
                    if let Some(display) = gdk::Display::default() {
                        state.preload_icons(&display);
                    }
                    return glib::ControlFlow::Continue;
                }
                needs_redraw = true;
            }

            // Hover animations (~60ms)
            let n = state.get_display_items_count();
            if state.hover_progresses.len() != n {
                state.hover_progresses.resize(n, 0.0);
            }

            for i in 0..n {
                let target = if state.hovered_index == Some(i) && !state.is_closing {
                    1.0
                } else {
                    0.0
                };
                let diff = target - state.hover_progresses[i];
                if diff.abs() > 0.01 {
                    let step = dt / 0.060;
                    state.hover_progresses[i] += diff.signum() * step.min(diff.abs());
                    needs_redraw = true;
                } else {
                    state.hover_progresses[i] = target;
                }
            }

            if needs_redraw {
                area_clone_tick.queue_draw();
            }

            glib::ControlFlow::Continue
        });

        // 11. Setup tokio channel to listen for IPC socket events
        let (ipc_tx, mut ipc_rx) = tokio::sync::mpsc::unbounded_channel::<IpcMessage>();

        // Start the IPC server and forward its commands to this channel
        let socket_path = launcher_ipc::get_socket_path();
        let server_handle = launcher_ipc::start_server(socket_path, move |msg| {
            let _ = ipc_tx.send(msg);
        })?;

        // Keep the server handle alive as long as the window exists
        let server_handle_wrapper = Arc::new(Mutex::new(Some(server_handle)));
        let server_handle_clone = server_handle_wrapper.clone();

        let ipc_state = state.clone();
        let area_clone_ipc = drawing_area.clone();
        let theme_provider_clone = theme_provider.clone();
        let config_path_clone = config_path.clone();
        let menu_path_clone = menu_path.clone();

        let window_clone_ipc = window.clone();
        glib::MainContext::default().spawn_local(async move {
            while let Some(msg) = ipc_rx.recv().await {
                debug!("Received IPC message in UI thread: {:?}", msg);
                match msg {
                    IpcMessage::Toggle => {
                        let is_visible = window_clone_ipc.is_visible();
                        let mut state = ipc_state.borrow_mut();
                        if is_visible && !state.is_closing {
                            info!("Hiding window via IPC Toggle");
                            state.is_closing = true;
                            state.is_opening = false;
                        } else {
                            info!("Showing window via IPC Toggle");
                            state.current_items = state.root_items.clone();
                            state.history.clear();
                            state.hovered_index = None;
                            if let Some(display) = gdk::Display::default() {
                                state.preload_icons(&display);
                            }
                            state.is_opening = true;
                            state.is_closing = false;
                            state.open_progress = 0.0;
                            drop(state);
                            window_clone_ipc.present();
                            area_clone_ipc.queue_draw();
                        }
                    }
                    IpcMessage::Close => {
                        let mut state = ipc_state.borrow_mut();
                        if window_clone_ipc.is_visible() && !state.is_closing {
                            info!("Hiding window via IPC Close");
                            state.is_closing = true;
                            state.is_opening = false;
                        }
                    }
                    IpcMessage::Open => {
                        let is_visible = window_clone_ipc.is_visible();
                        if !is_visible {
                            info!("Showing window via IPC Open");
                            let mut state = ipc_state.borrow_mut();
                            state.current_items = state.root_items.clone();
                            state.history.clear();
                            state.hovered_index = None;
                            if let Some(display) = gdk::Display::default() {
                                state.preload_icons(&display);
                            }
                            state.is_opening = true;
                            state.is_closing = false;
                            state.open_progress = 0.0;
                            drop(state);
                            window_clone_ipc.present();
                            area_clone_ipc.queue_draw();
                        }
                    }
                    IpcMessage::ReloadConfig => {
                        info!("Reload config request received via IPC");
                        // 1. Reload the theme CSS from file
                        load_and_apply_theme(&config_path_clone, &theme_provider_clone);

                        // 2. Reload the menu TOML config from file
                        let mut state = ipc_state.borrow_mut();
                        state.codepoints =
                            launcher_core::load_material_codepoints(&config_path_clone);
                        if let Ok(cfg) = launcher_core::load_config(&config_path_clone) {
                            state.extra_radius = cfg.ui.extra_radius;
                            info!("Reloaded extra_radius: {}", state.extra_radius);
                        }
                        match launcher_core::load_menu(&menu_path_clone) {
                            Ok(m) => {
                                state.root_items = m.menu.clone();
                                state.current_items = m.menu;
                                state.history.clear();
                                state.hovered_index = None;
                                if let Some(display) = gdk::Display::default() {
                                    state.preload_icons(&display);
                                }
                                info!("Menu config reloaded successfully");
                            }
                            Err(e) => {
                                error!("Failed to reload menu config: {}", e);
                            }
                        }
                        area_clone_ipc.queue_draw();
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
                state_mut.is_opening = true;
                state_mut.is_closing = false;
                state_mut.open_progress = 0.0;
            }
            window.present();
        } else {
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                state_mut.is_opening = false;
                state_mut.is_closing = false;
                state_mut.open_progress = 0.0;
            }
            window.hide();
        }
        Ok(())
    }
}
