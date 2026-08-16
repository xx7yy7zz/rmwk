pub mod standard_theme;
mod theme_editor;
use gtk::gdk;
use gtk::prelude::*;
use gtk4 as gtk;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use tracing::{error, info};

#[derive(Clone, Debug)]
struct CopiedNode {
    label: String,
    icon: String,
    action_type: String,
    action_cmd: String,
    keep_open: bool,
    quick_select: String,
    children: Vec<CopiedNode>,
}

#[derive(Clone, Debug)]
struct MaterialIconItem {
    name: String,
    glyph: char,
    search_key: String,
}

#[derive(Clone, Debug)]
struct SystemIconItem {
    name: String,
    search_key: String,
}

pub struct SettingsApp {
    app: gtk::Application,
    menu_path: PathBuf,
    config_path: PathBuf,
}

impl SettingsApp {
    pub fn new(menu_path: PathBuf, config_path: PathBuf) -> Self {
        let app = gtk::Application::builder()
            .application_id("org.rmwk.settings")
            .build();

        Self {
            app,
            menu_path,
            config_path,
        }
    }

    pub fn run(&self) -> i32 {
        let menu_path = self.menu_path.clone();
        let config_path = self.config_path.clone();

        self.app.connect_activate(move |app| {
            if let Err(e) = Self::activate_ui(app, menu_path.clone(), config_path.clone()) {
                error!("Failed to activate settings UI: {}", e);
            }
        });

        self.app.run_with_args::<String>(&[]).into()
    }

    fn activate_ui(
        app: &gtk::Application,
        menu_path: PathBuf,
        config_path: PathBuf,
    ) -> anyhow::Result<()> {
        let window = gtk::ApplicationWindow::new(app);
        window.set_title(Some("rmwk Settings"));
        window.set_default_size(800, 500);


        let font_provider = gtk::CssProvider::new();
        let font_css = "
            .material-icon-glyph {
                font-family: 'Material Symbols Rounded';
                font-size: 24px;
            }
        ";
        font_provider.load_from_data(font_css);

        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &font_provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        // Load current config and menu
        let menu_config = match launcher_core::load_menu(&menu_path) {
            Ok(m) => m,
            Err(_) => launcher_core::MenuConfig { icon: None, menu: vec![] },
        };
        let ui_config = match launcher_core::load_config(&config_path) {
            Ok(cfg) => cfg,
            Err(_) => launcher_core::Config::default(),
        };

        // Main Layout: Horizontal Box (Left Column: TreeView & buttons, Right Column: Properties)
        let main_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        main_box.set_margin_start(15);
        main_box.set_margin_end(15);
        main_box.set_margin_top(15);
        main_box.set_margin_bottom(15);

        // --- LEFT COLUMN ---
        let left_vbox = gtk::Box::new(gtk::Orientation::Vertical, 10);
        left_vbox.set_vexpand(true);

        let active_menu_path = Rc::new(RefCell::new(menu_path.clone()));
        let is_saving = Rc::new(std::cell::Cell::new(false));

        // Menu Selector
        let menu_selector_hbox = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        let lbl_menu = gtk::Label::new(Some("Menu:"));
        let combo_menu_files = gtk::ComboBoxText::new();
        let is_rebuilding_combo = Rc::new(std::cell::Cell::new(false));
        let btn_new_menu = gtk::Button::from_icon_name("document-new-symbolic");
        let btn_edit_menu = gtk::Button::from_icon_name("document-edit-symbolic");
        let btn_delete_menu = gtk::Button::from_icon_name("user-trash-symbolic");

        let available_menus = Self::get_available_menus(&config_path);
        for m in &available_menus {
            combo_menu_files.append(Some(m), m);
        }
        if let Some(stem) = menu_path.file_stem().and_then(|s| s.to_str()) {
            if !available_menus.contains(&stem.to_string()) && menu_path.exists() {
                combo_menu_files.append(Some(stem), stem);
            }
            combo_menu_files.set_active_id(Some(stem));
        } else if !available_menus.is_empty() {
            combo_menu_files.set_active_id(Some(&available_menus[0]));
        }

        menu_selector_hbox.append(&lbl_menu);
        menu_selector_hbox.append(&combo_menu_files);
        menu_selector_hbox.append(&btn_new_menu);
        menu_selector_hbox.append(&btn_edit_menu);
        menu_selector_hbox.append(&btn_delete_menu);
        left_vbox.append(&menu_selector_hbox);

        // Scrollable window for TreeView
        let scroll_win = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(false)
            .height_request(200)
            .vexpand(false)
            .build();

        // Create TreeStore:
        // Col 0: Icon (String)
        // Col 1: Label (String)
        // Col 2: Action Type (String: "command", "submenu")
        // Col 3: Action Command (String)
        let store = gtk::TreeStore::new(&[
            glib::Type::STRING,
            glib::Type::STRING,
            glib::Type::STRING,
            glib::Type::STRING,
            glib::Type::BOOL,
            glib::Type::STRING, // Col 5: Quick Select Key
        ]);

        let stem_name = active_menu_path
            .borrow()
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("menu")
            .to_string();
        let root_icon = menu_config.icon.clone().unwrap_or_else(|| "menu".to_string());
        let root_iter = store.insert_with_values(
            None,
            None,
            &[
                (0, &root_icon.to_value()),
                (1, &format!("{} (Root)", stem_name).to_value()),
                (2, &"root".to_value()),
                (3, &"".to_value()),
                (4, &false.to_value()),
                (5, &"".to_value()),
            ],
        );
        Self::populate_store(&store, Some(&root_iter), &menu_config.menu);

        let tree_view = gtk::TreeView::with_model(&store);
        let path = store.path(&root_iter);
        tree_view.expand_row(&path, false);
        tree_view.set_headers_visible(true);

        // Col 1: Label
        let label_renderer = gtk::CellRendererText::new();
        label_renderer.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let label_column = gtk::TreeViewColumn::new();
        label_column.set_title("Menu Item");
        label_column.pack_start(&label_renderer, true);
        label_column.add_attribute(&label_renderer, "text", 1);
        label_column.set_sizing(gtk::TreeViewColumnSizing::Fixed);
        tree_view.append_column(&label_column);

        // Col 2: Action Type
        let type_renderer = gtk::CellRendererText::new();
        type_renderer.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let type_column = gtk::TreeViewColumn::new();
        type_column.set_title("Type");
        type_column.pack_start(&type_renderer, true);
        type_column.add_attribute(&type_renderer, "text", 2);
        type_column.set_sizing(gtk::TreeViewColumnSizing::Fixed);
        tree_view.append_column(&type_column);

        scroll_win.set_child(Some(&tree_view));
        left_vbox.append(&scroll_win);

        // Dynamically resize the tree panel to exactly 50% of the window's height and width
        let scroll_win_clone = scroll_win.clone();
        let left_vbox_clone = left_vbox.clone();
        let label_col_clone = label_column.clone();
        let type_col_clone = type_column.clone();

        window.connect_map(move |win| {
            if let Some(surface) = win.surface() {
                let scroll_win_c = scroll_win_clone.clone();
                let initial_h = (surface.height() / 2) - 15;
                if initial_h >= 100 {
                    scroll_win_c.set_height_request(initial_h);
                }

                let left_vbox_c = left_vbox_clone.clone();
                let label_col_c = label_col_clone.clone();
                let type_col_c = type_col_clone.clone();
                let initial_w = (surface.width() / 2) - 15;
                if initial_w >= 100 {
                    left_vbox_c.set_width_request(initial_w);
                    label_col_c.set_fixed_width((initial_w * 3) / 4);
                    type_col_c.set_fixed_width(initial_w / 4);
                }

                surface.connect_notify_local(Some("height"), move |surf, _| {
                    let dynamic_h = (surf.height() / 2) - 15;
                    if dynamic_h >= 100 {
                        scroll_win_c.set_height_request(dynamic_h);
                    }
                });

                let left_vbox_c2 = left_vbox_clone.clone();
                let label_col_c2 = label_col_clone.clone();
                let type_col_c2 = type_col_clone.clone();
                surface.connect_notify_local(Some("width"), move |surf, _| {
                    let dynamic_w = (surf.width() / 2) - 15;
                    if dynamic_w >= 100 {
                        left_vbox_c2.set_width_request(dynamic_w);
                        label_col_c2.set_fixed_width((dynamic_w * 3) / 4);
                        type_col_c2.set_fixed_width(dynamic_w / 4);
                    }
                });
            }
        });

        // Buttons under TreeView
        let btn_hbox = gtk::Box::new(gtk::Orientation::Horizontal, 5);

        let btn_add_item = gtk::Button::with_label("Add Command");
        let btn_add_sub = gtk::Button::with_label("Add Submenu");
        let btn_add_hotkey = gtk::Button::with_label("Add Hotkey");
        let btn_copy = gtk::Button::with_label("Copy");
        let btn_paste = gtk::Button::with_label("Paste");
        btn_paste.set_sensitive(false);
        let btn_delete = gtk::Button::with_label("Delete");
        let btn_up = gtk::Button::with_label("▲");
        let btn_down = gtk::Button::with_label("▼");

        btn_hbox.append(&btn_add_item);
        btn_hbox.append(&btn_add_sub);
        btn_hbox.append(&btn_add_hotkey);
        btn_hbox.append(&btn_copy);
        btn_hbox.append(&btn_paste);
        btn_hbox.append(&btn_delete);
        btn_hbox.append(&btn_up);
        btn_hbox.append(&btn_down);
        left_vbox.append(&btn_hbox);

        let themes = Self::get_available_themes(&config_path);
        let theme_editor = theme_editor::ThemeEditor::new(
            config_path.clone(),
            &ui_config.ui.theme,
            ui_config.ui.system_theme_overrides.clone(),
            &themes,
        );
        left_vbox.append(&theme_editor.container);
        let combo_theme = theme_editor.combo_theme.clone();
        let sys_overrides = theme_editor.current_system_overrides.clone();

        main_box.append(&left_vbox);

        // --- RIGHT COLUMN ---
        let right_vbox = gtk::Box::new(gtk::Orientation::Vertical, 15);
        right_vbox.set_hexpand(true);

        // Properties Frame
        let prop_frame = gtk::Frame::new(Some("Properties"));
        let prop_grid = gtk::Grid::new();
        prop_grid.set_row_spacing(10);
        prop_grid.set_column_spacing(10);
        prop_grid.set_margin_start(10);
        prop_grid.set_margin_end(10);
        prop_grid.set_margin_top(10);
        prop_grid.set_margin_bottom(10);

        // 1. Label Entry
        let lbl_label = gtk::Label::new(Some("Label:"));
        lbl_label.set_halign(gtk::Align::End);
        let entry_label = gtk::Entry::new();
        prop_grid.attach(&lbl_label, 0, 0, 1, 1);
        prop_grid.attach(&entry_label, 1, 0, 1, 1);

        // 2. Icon Type Dropdown
        let lbl_icon_type = gtk::Label::new(Some("Icon Type:"));
        lbl_icon_type.set_halign(gtk::Align::End);
        let combo_icon_type = gtk::ComboBoxText::new();
        combo_icon_type.append(Some("picker"), "Icon Picker");
        combo_icon_type.append(Some("char"), "Single Character");
        combo_icon_type.set_active_id(Some("picker"));
        prop_grid.attach(&lbl_icon_type, 0, 1, 1, 1);
        prop_grid.attach(&combo_icon_type, 1, 1, 1, 1);

        // 3. Icon Entry & Picker Button
        let lbl_icon = gtk::Label::new(Some("Icon:"));
        lbl_icon.set_halign(gtk::Align::End);

        let icon_box = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        let entry_icon = gtk::Entry::new();
        entry_icon.set_hexpand(true);
        let btn_pick_icon = gtk::Button::with_label("🔍 Select");
        icon_box.append(&entry_icon);
        icon_box.append(&btn_pick_icon);

        prop_grid.attach(&lbl_icon, 0, 2, 1, 1);
        prop_grid.attach(&icon_box, 1, 2, 1, 1);

        let entry_icon_clone = entry_icon.clone();
        let window_clone = window.clone();
        let config_path_clone = config_path.clone();
        btn_pick_icon.connect_clicked(move |_| {
            Self::show_icon_picker(&window_clone, &entry_icon_clone, config_path_clone.clone());
        });

        // Icon Type Changed signal connection
        let btn_pick_icon_toggle = btn_pick_icon.clone();
        let entry_icon_toggle = entry_icon.clone();
        combo_icon_type.connect_changed(move |combo| {
            let active_id = combo
                .active_id()
                .map(|s| s.to_string())
                .unwrap_or_else(|| "picker".to_string());
            if active_id == "char" {
                entry_icon_toggle.set_max_length(1);
                entry_icon_toggle.set_placeholder_text(Some("e.g. A, 🚀"));
                btn_pick_icon_toggle.set_visible(false);

                let txt = entry_icon_toggle.text().to_string();
                if txt.chars().count() > 1 {
                    if let Some(first_char) = txt.chars().next() {
                        entry_icon_toggle.set_text(&first_char.to_string());
                    }
                }
            } else {
                entry_icon_toggle.set_max_length(0); // unlimited
                entry_icon_toggle.set_placeholder_text(None);
                btn_pick_icon_toggle.set_visible(true);
            }
        });

        let lbl_cmd = gtk::Label::new(Some("Command:"));
        lbl_cmd.set_halign(gtk::Align::End);
        let entry_cmd = gtk::Entry::new();
        entry_cmd.set_hexpand(true);
        let btn_record_hotkey = gtk::ToggleButton::new();
        btn_record_hotkey.set_icon_name("media-record");
        let cmd_hbox = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        cmd_hbox.append(&entry_cmd);
        cmd_hbox.append(&btn_record_hotkey);

        prop_grid.attach(&lbl_cmd, 0, 3, 1, 1);
        prop_grid.attach(&cmd_hbox, 1, 3, 1, 1);

        let lbl_hotkey_status = gtk::Label::new(None);
        lbl_hotkey_status.set_halign(gtk::Align::Start);
        lbl_hotkey_status.set_use_markup(true);
        prop_grid.attach(&lbl_hotkey_status, 1, 4, 1, 1);

        let chk_item_keep_open = gtk::CheckButton::with_label("Keep Launcher Open");
        prop_grid.attach(&chk_item_keep_open, 1, 5, 1, 1);

        // 4. Quick Select Key
        let lbl_quick_select = gtk::Label::new(Some("Quick Select Key:"));
        lbl_quick_select.set_halign(gtk::Align::End);
        let entry_quick_select = gtk::Entry::new();
        entry_quick_select.set_max_length(1);
        entry_quick_select.set_placeholder_text(None);
        prop_grid.attach(&lbl_quick_select, 0, 6, 1, 1);
        prop_grid.attach(&entry_quick_select, 1, 6, 1, 1);

        prop_frame.set_child(Some(&prop_grid));
        right_vbox.append(&prop_frame);

        // Theme and Global settings Box
        let settings_hbox = gtk::Box::new(gtk::Orientation::Horizontal, 10);

        let lbl_extra_radius = gtk::Label::new(Some("Active Margin (px):"));
        let spin_extra_radius = gtk::SpinButton::with_range(0.0, 1000.0, 5.0);
        spin_extra_radius.set_value(ui_config.ui.extra_radius);

        let chk_symbolic_icons = gtk::CheckButton::with_label("Symbolic Icons");
        chk_symbolic_icons.set_active(ui_config.ui.use_symbolic_icons);

        let chk_bold_chars = gtk::CheckButton::with_label("Bold Text Icons");
        chk_bold_chars.set_active(ui_config.ui.bold_single_chars);

        let chk_center_layout = gtk::CheckButton::with_label("Center Slices on Axes");
        chk_center_layout.set_active(ui_config.ui.center_layout);

        let chk_disable_hover_anim = gtk::CheckButton::with_label("Disable Hover Animation");
        chk_disable_hover_anim.set_active(ui_config.ui.disable_hover_animation);

        let chk_enable_blur = gtk::CheckButton::with_label("Enable Blur");
        chk_enable_blur.set_active(ui_config.ui.enable_blur);
        if ui_config.ui.menu_style == "floating" {
            chk_enable_blur.set_sensitive(false);
        }

        let icon_blur_warning = gtk::Image::from_icon_name("dialog-warning");
        icon_blur_warning.set_tooltip_text(Some("Warning: This effect uses the ext-background-effect-v1 protocol. Your Wayland compositor must support this protocol for blur to work."));

        let blur_hbox = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        blur_hbox.append(&chk_enable_blur);
        blur_hbox.append(&icon_blur_warning);

        let lbl_visual_cue = gtk::Label::new(Some("Hover Visual Cue:"));
        lbl_visual_cue.set_halign(gtk::Align::Start);
        let combo_visual_cue = gtk::ComboBoxText::new();
        combo_visual_cue.append(Some("outwards"), "Expand Outwards");
        combo_visual_cue.append(Some("sides"), "Expand Sides");
        combo_visual_cue.append(Some("none"), "None");
        combo_visual_cue.set_active_id(Some(&ui_config.ui.hover_visual_cue));

        let visual_cue_hbox = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        visual_cue_hbox.append(&lbl_visual_cue);
        visual_cue_hbox.append(&combo_visual_cue);

        let lbl_menu_style = gtk::Label::new(Some("Menu Style:"));
        lbl_menu_style.set_halign(gtk::Align::Start);
        let combo_menu_style = gtk::ComboBoxText::new();
        combo_menu_style.append(Some("pie"), "Pie (Continuous Ring)");
        combo_menu_style.append(Some("floating"), "Floating Pills");
        combo_menu_style.set_active_id(Some(&ui_config.ui.menu_style));

        let menu_style_hbox = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        menu_style_hbox.append(&lbl_menu_style);
        menu_style_hbox.append(&combo_menu_style);

        let chk_enable_blur_style = chk_enable_blur.clone();
        combo_menu_style.connect_changed(move |combo| {
            if let Some(id) = combo.active_id() {
                if id == "floating" {
                    chk_enable_blur_style.set_sensitive(false);
                } else {
                    chk_enable_blur_style.set_sensitive(true);
                }
            }
        });

        settings_hbox.append(&lbl_extra_radius);
        settings_hbox.append(&spin_extra_radius);
        right_vbox.append(&settings_hbox);

        let checkboxes_vbox = gtk::Box::new(gtk::Orientation::Vertical, 10);
        checkboxes_vbox.append(&chk_symbolic_icons);
        checkboxes_vbox.append(&chk_bold_chars);
        checkboxes_vbox.append(&chk_center_layout);
        checkboxes_vbox.append(&chk_disable_hover_anim);
        checkboxes_vbox.append(&blur_hbox);
        checkboxes_vbox.append(&visual_cue_hbox);
        checkboxes_vbox.append(&menu_style_hbox);
        right_vbox.append(&checkboxes_vbox);

        // Save & Save/Reload buttons at the bottom
        let bottom_hbox = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let btn_save = gtk::Button::with_label("Save & Apply Settings");
        btn_save.set_hexpand(true);
        btn_save.add_css_class("suggested-action");
        bottom_hbox.append(&btn_save);

        right_vbox.append(&bottom_hbox);
        main_box.append(&right_vbox);

        window.set_child(Some(&main_box));

        // --- CONTROL LAYER / EVENT SIGNALS ---

        // Selection Change: Updates property inputs
        let selection = tree_view.selection();
        let sel_label = entry_label.clone();
        let sel_icon = entry_icon.clone();
        let sel_cmd = entry_cmd.clone();
        let sel_icon_type = combo_icon_type.clone();
        let sel_keep_open = chk_item_keep_open.clone();
        let sel_quick_select = entry_quick_select.clone();

        let lbl_cmd_clone = lbl_cmd.clone();
        let entry_cmd_clone = entry_cmd.clone();
        let btn_record_hotkey_clone = btn_record_hotkey.clone();
        let chk_keep_open_clone = chk_item_keep_open.clone();
        let lbl_hotkey_status_clone = lbl_hotkey_status.clone();
        let btn_pick_icon_clone = btn_pick_icon.clone();
        let btn_delete_clone = btn_delete.clone();
        let btn_up_clone = btn_up.clone();
        let btn_down_clone = btn_down.clone();
        let btn_copy_clone = btn_copy.clone();

        selection.connect_changed(move |sel| {
            if let Some((model, iter)) = sel.selected() {
                let icon: String = model.get(&iter, 0);
                let label: String = model.get(&iter, 1);
                let act_type: String = model.get(&iter, 2);
                let cmd: String = model.get(&iter, 3);
                let keep_open: bool = model.get(&iter, 4);
                let quick_select: String = model.get(&iter, 5);

                if act_type == "root" {
                    // Disable editing controls for Root node
                    sel_label.set_sensitive(false);
                    sel_icon.set_sensitive(true);
                    sel_icon_type.set_sensitive(true);
                    btn_pick_icon_clone.set_sensitive(true);
                    sel_cmd.set_sensitive(false);
                    sel_keep_open.set_sensitive(false);
                    sel_quick_select.set_sensitive(false);
                    btn_delete_clone.set_sensitive(false);
                    btn_up_clone.set_sensitive(false);
                    btn_down_clone.set_sensitive(false);
                    btn_copy_clone.set_sensitive(false);

                    lbl_cmd_clone.set_visible(false);
                    entry_cmd_clone.set_visible(false);
                    btn_record_hotkey_clone.set_visible(false);
                    chk_keep_open_clone.set_visible(false);
                    lbl_hotkey_status_clone.set_visible(false);
                } else {
                    // Enable editing controls for other nodes
                    sel_label.set_sensitive(true);
                    sel_icon.set_sensitive(true);
                    sel_icon_type.set_sensitive(true);
                    sel_cmd.set_sensitive(true);
                    sel_keep_open.set_sensitive(true);
                    sel_quick_select.set_sensitive(true);
                    btn_pick_icon_clone.set_sensitive(true);
                    btn_delete_clone.set_sensitive(true);
                    btn_up_clone.set_sensitive(true);
                    btn_down_clone.set_sensitive(true);
                    btn_copy_clone.set_sensitive(true);

                    // Show/hide command input dynamically
                    if act_type == "submenu" {
                        lbl_cmd_clone.set_visible(false);
                        entry_cmd_clone.set_visible(false);
                        btn_record_hotkey_clone.set_visible(false);
                        chk_keep_open_clone.set_visible(false);
                        lbl_hotkey_status_clone.set_visible(false);
                    } else if act_type == "hotkey" {
                        lbl_cmd_clone.set_label("Keystroke:");
                        lbl_cmd_clone.set_visible(true);
                        entry_cmd_clone.set_visible(true);
                        btn_record_hotkey_clone.set_visible(true);
                        chk_keep_open_clone.set_visible(true);
                        lbl_hotkey_status_clone.set_visible(true);
                    } else {
                        lbl_cmd_clone.set_label("Command:");
                        lbl_cmd_clone.set_visible(true);
                        entry_cmd_clone.set_visible(true);
                        btn_record_hotkey_clone.set_visible(false);
                        chk_keep_open_clone.set_visible(true);
                        lbl_hotkey_status_clone.set_visible(false);
                    }
                }

                sel_label.set_text(&label);

                if icon.chars().count() == 1 && !icon.starts_with('/') {
                    sel_icon_type.set_active_id(Some("char"));
                } else {
                    sel_icon_type.set_active_id(Some("picker"));
                }

                sel_icon.set_text(&icon);
                sel_cmd.set_text(&cmd);
                sel_keep_open.set_active(keep_open);
                sel_quick_select.set_text(&quick_select);

                let path = model.path(&iter);
                let indices = path.indices();
                if let Some(&last_index) = indices.last() {
                    let default_key = if last_index < 9 {
                        format!("{}", last_index + 1)
                    } else if last_index == 9 {
                        "0".to_string()
                    } else {
                        "".to_string()
                    };
                    sel_quick_select.set_placeholder_text(Some(&default_key));
                }
            }
        });

        // Live Store Updates: Modifying input fields updates store row
        let store_l = store.clone();
        let sel_l = tree_view.selection();
        entry_label.connect_changed(move |e| {
            if let Some((_, iter)) = sel_l.selected() {
                store_l.set_value(&iter, 1, &e.text().to_string().to_value());
            }
        });

        let store_i = store.clone();
        let sel_i = tree_view.selection();
        entry_icon.connect_changed(move |e| {
            if let Some((_, iter)) = sel_i.selected() {
                store_i.set_value(&iter, 0, &e.text().to_string().to_value());
            }
        });

        let store_c = store.clone();
        let sel_c = tree_view.selection();
        let lbl_hotkey_status_c = lbl_hotkey_status.clone();
        entry_cmd.connect_changed(move |e| {
            if let Some((model, iter)) = sel_c.selected() {
                let act_type: String = model.get(&iter, 2);
                let txt = e.text().to_string();
                if act_type == "hotkey" {
                    match launcher_core::parse_hotkey(&txt) {
                        Ok(_) => lbl_hotkey_status_c
                            .set_markup("<span foreground='green'>✔ Valid hotkey</span>"),
                        Err(err) => lbl_hotkey_status_c
                            .set_markup(&format!("<span foreground='red'>✘ {}</span>", err)),
                    }
                }
                store_c.set_value(&iter, 3, &txt.to_value());
            }
        });

        let sel_tree_keep_open = tree_view.selection();
        chk_item_keep_open.connect_toggled(move |chk| {
            if let Some((model, iter)) = sel_tree_keep_open.selected() {
                if let Ok(store) = model.downcast::<gtk::TreeStore>() {
                    store.set_value(&iter, 4, &chk.is_active().to_value());
                }
            }
        });

        let sel_tree_qs = tree_view.selection();
        entry_quick_select.connect_changed(move |e| {
            let txt = e.text().to_string();
            // Validate: only alphanumeric single chars
            let valid_txt = if txt.chars().count() > 1 {
                txt.chars().next().unwrap().to_string()
            } else {
                txt
            };
            if e.text().to_string() != valid_txt {
                e.set_text(&valid_txt);
            }
            if let Some((model, iter)) = sel_tree_qs.selected() {
                if let Ok(store) = model.downcast::<gtk::TreeStore>() {
                    store.set_value(&iter, 5, &valid_txt.to_value());
                }
            }
        });

        // Add Command Button
        let store_add = store.clone();
        let selection_add = tree_view.selection();
        btn_add_item.connect_clicked(move |_| {
            let (parent, sibling) = Self::resolve_insertion_coords(&store_add, &selection_add);
            let new_iter = store_add.insert_after(parent.as_ref(), sibling.as_ref());
            store_add.set_value(&new_iter, 0, &"application-x-executable".to_value());
            store_add.set_value(&new_iter, 1, &"New Command".to_value());
            store_add.set_value(&new_iter, 2, &"shell command".to_value());
            store_add.set_value(&new_iter, 3, &"".to_value());
            store_add.set_value(&new_iter, 4, &false.to_value());
            store_add.set_value(&new_iter, 5, &"".to_value());
            selection_add.select_iter(&new_iter);
        });

        // Add Submenu Button
        let store_sub = store.clone();
        let selection_sub = tree_view.selection();
        btn_add_sub.connect_clicked(move |_| {
            let (parent, sibling) = Self::resolve_insertion_coords(&store_sub, &selection_sub);
            let new_iter = store_sub.insert_after(parent.as_ref(), sibling.as_ref());
            store_sub.set_value(&new_iter, 0, &"folder".to_value());
            store_sub.set_value(&new_iter, 1, &"New Submenu".to_value());
            store_sub.set_value(&new_iter, 2, &"submenu".to_value());
            store_sub.set_value(&new_iter, 3, &"".to_value());
            store_sub.set_value(&new_iter, 4, &false.to_value());
            store_sub.set_value(&new_iter, 5, &"".to_value());
            // Insert a dummy item to make it a subdirectory
            store_sub.insert_with_values(
                Some(&new_iter),
                None,
                &[
                    (0, &"application-x-executable".to_value()),
                    (1, &"New Command".to_value()),
                    (2, &"shell command".to_value()),
                    (3, &"".to_value()),
                    (4, &false.to_value()),
                    (5, &"".to_value()),
                ],
            );
            selection_sub.select_iter(&new_iter);
        });

        // Add Hotkey Button
        let store_hot = store.clone();
        let selection_hot = tree_view.selection();
        btn_add_hotkey.connect_clicked(move |_| {
            let (parent, sibling) = Self::resolve_insertion_coords(&store_hot, &selection_hot);
            let new_iter = store_hot.insert_after(parent.as_ref(), sibling.as_ref());
            store_hot.set_value(&new_iter, 0, &"keyboard".to_value());
            store_hot.set_value(&new_iter, 1, &"New Hotkey".to_value());
            store_hot.set_value(&new_iter, 2, &"hotkey".to_value());
            store_hot.set_value(&new_iter, 3, &"".to_value());
            store_hot.set_value(&new_iter, 4, &false.to_value());
            store_hot.set_value(&new_iter, 5, &"".to_value());
            selection_hot.select_iter(&new_iter);
        });

        // Delete Button
        let store_del = store.clone();
        let selection_del = tree_view.selection();
        btn_delete.connect_clicked(move |_| {
            if let Some((_, iter)) = selection_del.selected() {
                let act_type: String = store_del.get(&iter, 2);
                if act_type == "root" {
                    return; // Cannot delete root
                }
                store_del.remove(&iter);
            }
        });

        // Move Up Button
        let store_up = store.clone();
        let selection_up = tree_view.selection();
        let entry_qs_up = entry_quick_select.clone();
        btn_up.connect_clicked(move |_| {
            if let Some((_, iter)) = selection_up.selected() {
                let mut prev = iter.clone();
                // To move up, we swap with the previous sibling in the TreeStore
                if store_up.iter_previous(&mut prev) {
                    store_up.swap(&iter, &prev);
                    selection_up.select_iter(&iter);
                    Self::update_quick_select_placeholder(&store_up, &iter, &entry_qs_up);
                }
            }
        });

        // Move Down Button
        let store_down = store.clone();
        let selection_down = tree_view.selection();
        let entry_qs_down = entry_quick_select.clone();
        btn_down.connect_clicked(move |_| {
            if let Some((_, iter)) = selection_down.selected() {
                let mut next = iter.clone();
                // In TreeStore, moving down swaps with the next sibling
                if store_down.iter_next(&mut next) {
                    store_down.swap(&iter, &next);
                    selection_down.select_iter(&iter);
                    Self::update_quick_select_placeholder(&store_down, &iter, &entry_qs_down);
                }
            }
        });

        // Copy & Paste Handlers
        let clipboard = Rc::new(RefCell::new(None::<CopiedNode>));

        let store_copy = store.clone();
        let selection_copy = tree_view.selection();
        let clipboard_copy = clipboard.clone();
        let btn_paste_enable = btn_paste.clone();
        btn_copy.connect_clicked(move |_| {
            if let Some((_, selected_iter)) = selection_copy.selected() {
                let act_type: String = store_copy.get(&selected_iter, 2);
                if act_type == "root" {
                    return; // Cannot copy root
                }
                let copied = Self::copy_node_recursive(&store_copy, &selected_iter);
                *clipboard_copy.borrow_mut() = Some(copied);
                btn_paste_enable.set_sensitive(true);
            }
        });

        let store_paste = store.clone();
        let selection_paste = tree_view.selection();
        let clipboard_paste = clipboard.clone();
        let tree_view_paste = tree_view.clone();
        btn_paste.connect_clicked(move |_| {
            let copied_opt = clipboard_paste.borrow().clone();
            if let Some(node) = copied_opt {
                let (parent, sibling) =
                    Self::resolve_insertion_coords(&store_paste, &selection_paste);
                let pasted_iter = Self::paste_node_recursive(
                    &store_paste,
                    parent.as_ref(),
                    sibling.as_ref(),
                    &node,
                );

                selection_paste.select_iter(&pasted_iter);
                let path = store_paste.path(&pasted_iter);
                tree_view_paste.expand_row(&path, true);
            }
        });

        // Save Button Handler
        let store_save = store.clone();
        let config_path_save = config_path.clone();
        let combo_theme_save = combo_theme.clone();
        let sys_overrides_save = sys_overrides.clone();
        let spin_extra_radius_save = spin_extra_radius.clone();
        let chk_symbolic_icons_save = chk_symbolic_icons.clone();
        let chk_bold_chars_save = chk_bold_chars.clone();
        let chk_center_layout_save = chk_center_layout.clone();
        let chk_disable_hover_anim_save = chk_disable_hover_anim.clone();
        let combo_visual_cue_save = combo_visual_cue.clone();
        let chk_enable_blur_save = chk_enable_blur.clone();
        let combo_menu_style_save = combo_menu_style.clone();
        let active_menu_path_save = active_menu_path.clone();
        let is_saving_save = is_saving.clone();
        btn_save.connect_clicked(move |_| {
            is_saving_save.set(true);

            // 1. Serialize and save the menu (serialize only the children of the permanent "Menu (Root)" node)
            let mut items = vec![];
            let mut root_icon = None;
            if let Some(root_iter) = store_save.iter_children(None) {
                let icon_val: String = store_save.get(&root_iter, 0);
                if !icon_val.is_empty() {
                    root_icon = Some(icon_val);
                }
                items = Self::serialize_store(&store_save, Some(&root_iter));
            }
            let menu_config = launcher_core::MenuConfig { icon: root_icon, menu: items };
            let current_menu_path = active_menu_path_save.borrow().clone();

            // Only save if there's a valid menu selected
            if let Some(_) = current_menu_path.file_stem() {
                if let Err(e) = launcher_core::save_menu(&current_menu_path, &menu_config) {
                    error!("Failed to save menu configuration: {}", e);
                } else {
                    info!("Menu config saved successfully to {:?}", current_menu_path);
                }
            }

            // 2. Save active theme, extra_radius, etc. back to config.toml
            if let Some(theme_id) = combo_theme_save.active_id() {
                let mut cfg = match launcher_core::load_config(&config_path_save) {
                    Ok(c) => c,
                    Err(_) => launcher_core::Config::default(),
                };
                cfg.ui.theme = theme_id.to_string();
                cfg.ui.system_theme_overrides = Some(sys_overrides_save.borrow().clone());
                cfg.ui.extra_radius = spin_extra_radius_save.value();
                cfg.ui.use_symbolic_icons = chk_symbolic_icons_save.is_active();
                cfg.ui.bold_single_chars = chk_bold_chars_save.is_active();
                cfg.ui.center_layout = chk_center_layout_save.is_active();
                cfg.ui.disable_hover_animation = chk_disable_hover_anim_save.is_active();
                cfg.ui.hover_visual_cue = combo_visual_cue_save
                    .active_id()
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "outwards".to_string());
                cfg.ui.menu_style = combo_menu_style_save
                    .active_id()
                    .map(|id| id.to_string())
                    .unwrap_or_else(|| "pie".to_string());
                cfg.ui.enable_blur = chk_enable_blur_save.is_active();
                cfg.last_edited_menu = current_menu_path.file_stem().and_then(|s| s.to_str()).map(|s| s.to_string());

                // Write back config.toml
                let content = toml::to_string_pretty(&cfg).unwrap();
                if let Err(e) = std::fs::write(&config_path_save, content) {
                    error!("Failed to save UI config: {}", e);
                } else {
                    info!("UI config saved successfully to {:?}", config_path_save);
                }
            }

            // 3. Trigger Hot-Reload over IPC socket synchronously
            let socket_path = launcher_ipc::get_socket_path();
            if socket_path.exists() {
                let _ = launcher_ipc::send_message_sync(
                    &socket_path,
                    &launcher_ipc::IpcMessage::ReloadConfig,
                );
            }

            let is_sav_timer = is_saving_save.clone();
            gtk::glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
                is_sav_timer.set(false);
                gtk::glib::ControlFlow::Break
            });
        });

        // 4. Hotkey Recorder Logic
        let key_ctrl = gtk::EventControllerKey::new();
        let record_btn_c = btn_record_hotkey.clone();
        let entry_cmd_c = entry_cmd.clone();
        let recorded_keys = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));

        record_btn_c.connect_toggled({
            let recorded_keys = recorded_keys.clone();
            let entry_cmd_c = entry_cmd_c.clone();
            move |btn| {
                if btn.is_active() {
                    recorded_keys.borrow_mut().clear();
                    entry_cmd_c.set_text("Press keys...");
                    entry_cmd_c.set_sensitive(false);
                    btn.grab_focus();
                } else {
                    entry_cmd_c.set_sensitive(true);
                }
            }
        });

        key_ctrl.connect_key_pressed(move |_, keyval, _keycode, _state| {
            if !record_btn_c.is_active() {
                return glib::Propagation::Proceed;
            }

            let name = keyval.name().map(|n| n.to_string()).unwrap_or_default();
            let lower = name.to_lowercase();

            let modifier = match lower.as_str() {
                "control_l" | "control_r" => Some("ctrl"),
                "shift_l" | "shift_r" => Some("shift"),
                "alt_l" | "alt_r" => Some("alt"),
                "super_l" | "super_r" | "meta_l" | "meta_r" => Some("super"),
                _ => None,
            };

            let mut keys = recorded_keys.borrow_mut();

            if let Some(m) = modifier {
                if !keys.contains(&m.to_string()) {
                    keys.push(m.to_string());
                    entry_cmd_c.set_text(&format!("{}+...", keys.join("+")));
                }
            } else {
                let mut final_keys = keys.clone();
                let readable = match lower.as_str() {
                    "return" => "Return",
                    "escape" => "Escape",
                    "space" => "space",
                    "tab" => "Tab",
                    _ => &lower,
                };

                final_keys.push(readable.to_string());
                entry_cmd_c.set_text(&final_keys.join("+"));
                record_btn_c.set_active(false);
            }

            glib::Propagation::Stop
        });

        window.add_controller(key_ctrl);

        let file_monitors = Rc::new(RefCell::new(Vec::new()));

        // Monitor config.toml
        let config_file = gtk::gio::File::for_path(&config_path);
        let config_path_watch = config_path.clone();

        let combo_theme_watch = combo_theme.clone();
        let sys_overrides_watch = sys_overrides.clone();
        let spin_extra_radius_watch = spin_extra_radius.clone();
        let chk_symbolic_icons_watch = chk_symbolic_icons.clone();
        let chk_bold_chars_watch = chk_bold_chars.clone();
        let chk_center_layout_watch = chk_center_layout.clone();
        let chk_disable_hover_anim_watch = chk_disable_hover_anim.clone();
        let combo_visual_cue_watch = combo_visual_cue.clone();
        let combo_menu_style_watch = combo_menu_style.clone();
        let chk_enable_blur_watch = chk_enable_blur.clone();
        let is_saving_cfg_watch = is_saving.clone();
        let pending_cfg_update = Rc::new(std::cell::Cell::new(false));

        if let Ok(monitor) = config_file.monitor_file(
            gtk::gio::FileMonitorFlags::NONE,
            gtk::gio::Cancellable::NONE,
        ) {
            let is_saving_m = is_saving_cfg_watch.clone();
            let pending_m = pending_cfg_update.clone();
            let cp_w = config_path_watch.clone();
            let combo_th = combo_theme_watch.clone();
            let spin_r = spin_extra_radius_watch.clone();
            let chk_sym = chk_symbolic_icons_watch.clone();
            let chk_bold = chk_bold_chars_watch.clone();
            let chk_cnt = chk_center_layout_watch.clone();
            let chk_dis = chk_disable_hover_anim_watch.clone();
            let chk_blr = chk_enable_blur_watch.clone();
            let combo_vis = combo_visual_cue_watch.clone();
            let combo_sty = combo_menu_style_watch.clone();
            let sys_ov = sys_overrides_watch.clone();

            monitor.connect_changed(move |_, _, _, event| {
                if is_saving_m.get() {
                    return;
                }
                if (event == gtk::gio::FileMonitorEvent::ChangesDoneHint
                    || event == gtk::gio::FileMonitorEvent::Created)
                    && !pending_m.get()
                {
                    pending_m.set(true);
                    let pending_cb = pending_m.clone();
                    let is_sav_cb = is_saving_m.clone();
                    let cp_cb = cp_w.clone();
                    let combo_th_cb = combo_th.clone();
                    let spin_r_cb = spin_r.clone();
                    let chk_sym_cb = chk_sym.clone();
                    let chk_bold_cb = chk_bold.clone();
                    let chk_cnt_cb = chk_cnt.clone();
                    let chk_dis_cb = chk_dis.clone();
                    let chk_blr_cb = chk_blr.clone();
                    let combo_vis_cb = combo_vis.clone();
                    let combo_sty_cb = combo_sty.clone();
                    let sys_ov_cb = sys_ov.clone();

                    gtk::glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
                        pending_cb.set(false);
                        if is_sav_cb.get() {
                            return gtk::glib::ControlFlow::Break;
                        }
                        if let Ok(cfg) = launcher_core::load_config(&cp_cb) {
                            combo_th_cb.set_active_id(Some(&cfg.ui.theme));
                            spin_r_cb.set_value(cfg.ui.extra_radius);
                            chk_sym_cb.set_active(cfg.ui.use_symbolic_icons);
                            chk_bold_cb.set_active(cfg.ui.bold_single_chars);
                            chk_cnt_cb.set_active(cfg.ui.center_layout);
                            chk_dis_cb.set_active(cfg.ui.disable_hover_animation);
                            chk_blr_cb.set_active(cfg.ui.enable_blur);
                            combo_vis_cb.set_active_id(Some(&cfg.ui.hover_visual_cue));
                            combo_sty_cb.set_active_id(Some(&cfg.ui.menu_style));
                            if let Some(sys) = cfg.ui.system_theme_overrides {
                                *sys_ov_cb.borrow_mut() = sys;
                            }
                            combo_th_cb.emit_by_name::<()>("changed", &[]);
                        }
                        gtk::glib::ControlFlow::Break
                    });
                }
            });
            file_monitors.borrow_mut().push(monitor);
        }

        let active_menu_monitor = Rc::new(RefCell::new(None::<gtk::gio::FileMonitor>));

        // Directory monitor for menus
        let menus_dir_path = config_path
            .parent()
            .map(|p| p.join("menus"))
            .unwrap_or_else(|| launcher_core::paths::get_config_dir().join("menus"));
        let menus_dir_file = gtk::gio::File::for_path(&menus_dir_path);
        let combo_menu_files_dir_watch = combo_menu_files.clone();
        let config_path_dir_watch = config_path.clone();
        let is_rebuilding_dir = is_rebuilding_combo.clone();
        let active_menu_path_dir_watch = active_menu_path.clone();
        let is_saving_dir_watch = is_saving.clone();
        let pending_dir_update = Rc::new(std::cell::Cell::new(false));
        
        if let Ok(monitor) = menus_dir_file.monitor_directory(
            gtk::gio::FileMonitorFlags::NONE,
            gtk::gio::Cancellable::NONE,
        ) {
            let is_sav_d = is_saving_dir_watch.clone();
            monitor.connect_changed(move |_, _, _, event| {
                if is_sav_d.get() {
                    return;
                }
                if event == gtk::gio::FileMonitorEvent::Created
                    || event == gtk::gio::FileMonitorEvent::Deleted
                    || event == gtk::gio::FileMonitorEvent::Renamed
                    || event == gtk::gio::FileMonitorEvent::Moved
                {
                    if !pending_dir_update.get() {
                        pending_dir_update.set(true);
                        let combo = combo_menu_files_dir_watch.clone();
                        let cp = config_path_dir_watch.clone();
                        let is_reb = is_rebuilding_dir.clone();
                        let pending = pending_dir_update.clone();
                        let active_path_ref = active_menu_path_dir_watch.clone();
                        let is_sav_cb = is_sav_d.clone();
                        
                        gtk::glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
                            pending.set(false);
                            if is_sav_cb.get() {
                                return gtk::glib::ControlFlow::Break;
                            }
                            is_reb.set(true);
                            
                            let intended_active = active_path_ref.borrow().file_stem().and_then(|s| s.to_str()).map(|s| s.to_string());
                            
                            combo.remove_all();
                            let available = Self::get_available_menus(&cp);
                            for m in &available {
                                combo.append(Some(m), m);
                            }
                            if let Some(act) = intended_active {
                                if available.contains(&act) {
                                    combo.set_active_id(Some(&act));
                                } else {
                                    combo.set_active_id(None::<&str>);
                                    is_reb.set(false);
                                    combo.emit_by_name::<()>("changed", &[]);
                                    return gtk::glib::ControlFlow::Break;
                                }
                            }
                            is_reb.set(false);
                            gtk::glib::ControlFlow::Break
                        });
                    }
                }
            });
            file_monitors.borrow_mut().push(monitor);
        }

        let monitors_keep = file_monitors.clone();
        window.connect_close_request(move |_| {
            let _ = monitors_keep.borrow();
            gtk::glib::Propagation::Proceed
        });

        // Menu Selector Callbacks
        let active_menu_path_clone = active_menu_path.clone();
        let config_path_clone = config_path.clone();
        let store_clone = store.clone();
        let tree_view_clone = tree_view.clone();
        let is_saving_menu_watch = is_saving.clone();

        let active_menu_monitor_clone = active_menu_monitor.clone();
        let is_rebuilding_changed = is_rebuilding_combo.clone();
        combo_menu_files.connect_changed(move |combo| {
            if is_rebuilding_changed.get() {
                return;
            }
            if let Some(id) = combo.active_id() {
                let name = id.to_string();
                let parent = config_path_clone
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(launcher_core::paths::get_config_dir);
                let new_path = parent.join("menus").join(format!("{}.toml", name));
                *active_menu_path_clone.borrow_mut() = new_path.clone();
                
                // Save last_edited_menu
                if let Ok(mut cfg) = launcher_core::load_config(&config_path_clone) {
                    cfg.last_edited_menu = Some(name.clone());
                    let content = toml::to_string_pretty(&cfg).unwrap();
                    let _ = std::fs::write(&config_path_clone, content);
                }

                // Load and repopulate
                if let Ok(m) = launcher_core::load_menu(&new_path) {
                    store_clone.clear();
                    let root_icon = m.icon.clone().unwrap_or_else(|| "menu".to_string());
                    let root_iter = store_clone.insert_with_values(
                        None,
                        None,
                        &[
                            (0, &root_icon.to_value()),
                            (1, &format!("{} (Root)", name).to_value()),
                            (2, &"root".to_value()),
                            (3, &"".to_value()),
                            (4, &false.to_value()),
                            (5, &"".to_value()),
                        ],
                    );
                    Self::populate_store(&store_clone, Some(&root_iter), &m.menu);
                    let path = store_clone.path(&root_iter);
                    tree_view_clone.expand_row(&path, false);
                }

                // Set up file monitor for this newly active menu file
                let menu_file = gtk::gio::File::for_path(&new_path);
                let store_watch = store_clone.clone();
                let tree_view_watch = tree_view_clone.clone();
                let menu_path_watch = new_path.clone();
                let is_sav_menu = is_saving_menu_watch.clone();

                let pending_file_update = Rc::new(std::cell::Cell::new(false));
                if let Ok(monitor) = menu_file.monitor_file(
                    gtk::gio::FileMonitorFlags::NONE,
                    gtk::gio::Cancellable::NONE,
                ) {
                    monitor.connect_changed(move |_, _, _, event| {
                        if is_sav_menu.get() {
                            return;
                        }
                        if event == gtk::gio::FileMonitorEvent::ChangesDoneHint
                            || event == gtk::gio::FileMonitorEvent::Created
                        {
                            if !pending_file_update.get() {
                                pending_file_update.set(true);
                                let store_w = store_watch.clone();
                                let tree_w = tree_view_watch.clone();
                                let path_w = menu_path_watch.clone();
                                let pending = pending_file_update.clone();
                                let name_w = name.clone();
                                let is_sav_cb = is_sav_menu.clone();
                                
                                gtk::glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
                                    pending.set(false);
                                    if is_sav_cb.get() {
                                        return gtk::glib::ControlFlow::Break;
                                    }
                                    if let Ok(m) = launcher_core::load_menu(&path_w) {
                                        let mut expanded = Vec::new();
                                        tree_w.map_expanded_rows(|_, path| {
                                            expanded.push(path.clone());
                                        });
                                        
                                        let mut selected_path = None;
                                        if let Some((_, iter)) = tree_w.selection().selected() {
                                            selected_path = Some(store_w.path(&iter));
                                        }

                                        store_w.clear();
                                        let root_icon = m.icon.clone().unwrap_or_else(|| "menu".to_string());
                                        let root_iter = store_w.insert_with_values(
                                            None,
                                            None,
                                            &[
                                                (0, &root_icon.to_value()),
                                                (1, &format!("{} (Root)", name_w).to_value()),
                                                (2, &"root".to_value()),
                                                (3, &"".to_value()),
                                                (4, &false.to_value()),
                                                (5, &"".to_value()),
                                            ],
                                        );
                                        Self::populate_store(&store_w, Some(&root_iter), &m.menu);

                                        for path in expanded {
                                            tree_w.expand_row(&path, false);
                                        }
                                        
                                        if let Some(path) = selected_path {
                                            tree_w.selection().select_path(&path);
                                        }
                                    }
                                    gtk::glib::ControlFlow::Break
                                });
                            }
                        }
                    });
                    *active_menu_monitor_clone.borrow_mut() = Some(monitor);
                } else {
                    *active_menu_monitor_clone.borrow_mut() = None;
                }
            } else {
                store_clone.clear();
                *active_menu_monitor_clone.borrow_mut() = None;
                *active_menu_path_clone.borrow_mut() = std::path::PathBuf::new();
            }
        });
        // Initial setup for the first file monitor
        combo_menu_files.emit_by_name::<()>("changed", &[]);

        let window_for_dialog = window.clone();
        let combo_menu_files_clone2 = combo_menu_files.clone();
        let config_path_clone2 = config_path.clone();
        btn_new_menu.connect_clicked(move |_| {
            let input_window = gtk::Window::builder()
                .title("New Menu")
                .modal(true)
                .transient_for(&window_for_dialog)
                .default_width(300)
                .default_height(100)
                .build();

            let vbox = gtk::Box::new(gtk::Orientation::Vertical, 10);
            vbox.set_margin_start(10);
            vbox.set_margin_end(10);
            vbox.set_margin_top(10);
            vbox.set_margin_bottom(10);
            let entry = gtk::Entry::new();
            entry.set_placeholder_text(Some("Menu Name"));
            let btn_ok = gtk::Button::with_label("Create");
            vbox.append(&entry);
            vbox.append(&btn_ok);
            input_window.set_child(Some(&vbox));

            let input_window_clone = input_window.clone();
            let combo_menu_files_clone3 = combo_menu_files_clone2.clone();
            let config_path_clone3 = config_path_clone2.clone();
            let window_for_alert = window_for_dialog.clone();

            btn_ok.connect_clicked(move |_| {
                let text = entry.text().to_string();
                if !text.is_empty() {
                    let parent = config_path_clone3
                        .parent()
                        .map(|p| p.to_path_buf())
                        .unwrap_or_else(launcher_core::paths::get_config_dir);
                    let new_path = parent.join("menus").join(format!("{}.toml", text));

                    let do_create = {
                        let np = new_path.clone();
                        let t = text.clone();
                        let combo = combo_menu_files_clone3.clone();
                        move || {
                            let default_content = format!(
                                r#"# {} menu configuration
[[menu]]
label = "Example"
icon = "application-x-executable"
"#,
                                t
                            );
                            if let Some(p) = np.parent() {
                                let _ = std::fs::create_dir_all(p);
                            }
                            let _ = std::fs::write(&np, default_content);
                            combo.append(Some(&t), &t);
                            combo.set_active_id(Some(&t));
                        }
                    };

                    if new_path.exists() {
                        let dialog = gtk::MessageDialog::builder()
                            .text("Menu already exists")
                            .secondary_text("Overwrite?")
                            .buttons(gtk::ButtonsType::YesNo)
                            .modal(true)
                            .transient_for(&window_for_alert)
                            .build();

                        let iw_clone = input_window_clone.clone();
                        dialog.connect_response(move |d, response| {
                            if response == gtk::ResponseType::Yes {
                                do_create();
                            }
                            d.destroy();
                            iw_clone.destroy();
                        });
                        dialog.present();
                    } else {
                        do_create();
                        input_window_clone.destroy();
                    }
                }
            });
            input_window.present();
        });

        let window_for_rename_dialog = window.clone();
        let active_menu_path_rename = active_menu_path.clone();
        let combo_menu_files_rename = combo_menu_files.clone();
        btn_edit_menu.connect_clicked(move |_| {
            let path = active_menu_path_rename.borrow().clone();
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                let current_name = stem.to_string();
                let input_window = gtk::Window::builder()
                    .title("Rename Menu")
                    .modal(true)
                    .transient_for(&window_for_rename_dialog)
                    .default_width(300)
                    .default_height(100)
                    .build();

                let vbox = gtk::Box::new(gtk::Orientation::Vertical, 10);
                vbox.set_margin_start(10);
                vbox.set_margin_end(10);
                vbox.set_margin_top(10);
                vbox.set_margin_bottom(10);
                let entry = gtk::Entry::new();
                entry.set_text(&current_name);
                let btn_ok = gtk::Button::with_label("Rename");
                vbox.append(&entry);
                vbox.append(&btn_ok);
                input_window.set_child(Some(&vbox));

                let input_window_clone = input_window.clone();
                let window_for_alert = window_for_rename_dialog.clone();
                let combo_rename = combo_menu_files_rename.clone();
                let active_path = active_menu_path_rename.clone();
                let old_path = path.clone();

                btn_ok.connect_clicked(move |_| {
                    let new_text = entry.text().to_string();
                    if !new_text.is_empty() && new_text != current_name {
                        let new_path = old_path.with_file_name(format!("{}.toml", new_text));
                        let do_rename = {
                            let old_p = old_path.clone();
                            let new_p = new_path.clone();
                            let n_txt = new_text.clone();
                            let o_txt = current_name.clone();
                            let combo = combo_rename.clone();
                            let active_p = active_path.clone();
                            move || {
                                if let Err(e) = std::fs::rename(&old_p, &new_p) {
                                    tracing::error!("Failed to rename menu: {}", e);
                                    return;
                                }
                                // Remove old item from combo, add new one, and set active
                                let mut to_remove_index = None;
                                if let Some(model) = combo.model() {
                                    let mut iter = model.iter_first();
                                    let mut i = 0;
                                    while let Some(mut it) = iter {
                                        let val: Option<String> = model.get(&it, 0);
                                        if val == Some(o_txt.clone()) {
                                            to_remove_index = Some(i);
                                            break;
                                        }
                                        i += 1;
                                        if !model.iter_next(&mut it) {
                                            break;
                                        }
                                        iter = Some(it);
                                    }
                                }
                                if let Some(idx) = to_remove_index {
                                    combo.remove(idx);
                                }
                                combo.append(Some(&n_txt), &n_txt);
                                
                                // Update active path pointer
                                *active_p.borrow_mut() = new_p.clone();
                                combo.set_active_id(Some(&n_txt));
                            }
                        };

                        if new_path.exists() {
                            let dialog = gtk::MessageDialog::builder()
                                .text("Menu already exists")
                                .secondary_text("Overwrite?")
                                .buttons(gtk::ButtonsType::YesNo)
                                .modal(true)
                                .transient_for(&window_for_alert)
                                .build();

                            let iw_clone = input_window_clone.clone();
                            dialog.connect_response(move |d, response| {
                                if response == gtk::ResponseType::Yes {
                                    do_rename();
                                }
                                d.destroy();
                                iw_clone.destroy();
                            });
                            dialog.present();
                        } else {
                            do_rename();
                            input_window_clone.destroy();
                        }
                    } else if new_text == current_name {
                        input_window_clone.destroy();
                    }
                });
                input_window.present();
            }
        });

        let active_menu_path_del = active_menu_path.clone();
        let combo_menu_files_del = combo_menu_files.clone();
        let config_path_clone_del = config_path.clone();
        let window_for_del = window.clone();
        let store_del = store.clone();

        btn_delete_menu.connect_clicked(move |_| {
            let path = active_menu_path_del.borrow().clone();
            if let Some(stem) = path.file_stem() {
                let stem_str = stem.to_string_lossy().into_owned();
                let dialog = gtk::MessageDialog::builder()
                    .text("Confirm Deletion")
                    .secondary_text(&format!(
                        "Are you sure you want to delete {}.toml?",
                        stem_str
                    ))
                    .buttons(gtk::ButtonsType::OkCancel)
                    .modal(true)
                    .transient_for(&window_for_del)
                    .build();

                let combo = combo_menu_files_del.clone();
                let cp = config_path_clone_del.clone();
                let store_response = store_del.clone();

                dialog.connect_response(move |d, response| {
                    if response == gtk::ResponseType::Ok {
                        let _ = std::fs::remove_file(&path);
                        combo.remove_all();
                        let menus = Self::get_available_menus(&cp);
                        for m in &menus {
                            combo.append(Some(m), m);
                        }
                        if menus.is_empty() {
                            combo.set_active_id(None);
                            store_response.clear();
                        } else {
                            combo.set_active_id(Some(&menus[0]));
                        }
                    }
                    d.destroy();
                });
                dialog.present();
            }
        });

        window.present();
        Ok(())
    }

    fn populate_store(
        store: &gtk::TreeStore,
        parent_iter: Option<&gtk::TreeIter>,
        items: &[launcher_core::MenuItem],
    ) {
        for item in items {
            let current_iter = store.insert_with_values(
                parent_iter,
                None,
                &[
                    (0, &item.icon.clone().unwrap_or_default().to_value()),
                    (1, &item.label.to_value()),
                    (
                        2,
                        &match &item.action {
                            Some(launcher_core::Action::Command { .. }) => {
                                "shell command".to_value()
                            }
                            Some(launcher_core::Action::Hotkey { .. }) => "hotkey".to_value(),
                            None => {
                                if item.children.is_empty() {
                                    "shell command".to_value()
                                } else {
                                    "submenu".to_value()
                                }
                            }
                        },
                    ),
                    (
                        3,
                        &match &item.action {
                            Some(launcher_core::Action::Command { cmd, .. }) => cmd.to_value(),
                            Some(launcher_core::Action::Hotkey { keys, .. }) => keys.to_value(),
                            None => "".to_value(),
                        },
                    ),
                    (
                        4,
                        &match &item.action {
                            Some(launcher_core::Action::Command { keep_open, .. }) => {
                                keep_open.to_value()
                            }
                            Some(launcher_core::Action::Hotkey { keep_open, .. }) => {
                                keep_open.to_value()
                            }
                            None => false.to_value(),
                        },
                    ),
                    (
                        5,
                        &item
                            .quick_select_key
                            .map(|c| c.to_string())
                            .unwrap_or_default()
                            .to_value(),
                    ),
                ],
            );

            if !item.children.is_empty() {
                Self::populate_store(store, Some(&current_iter), &item.children);
            }
        }
    }

    fn get_available_menus(config_path: &Path) -> Vec<String> {
        let menus_dir = config_path
            .parent()
            .map(|p| p.join("menus"))
            .unwrap_or_else(|| launcher_core::paths::get_config_dir().join("menus"));
        let mut menus = vec![];
        if let Ok(entries) = std::fs::read_dir(menus_dir) {
            for entry in entries.flatten() {
                if let Ok(file_type) = entry.file_type() {
                    if file_type.is_file() {
                        let path = entry.path();
                        if path.extension().and_then(|s| s.to_str()) == Some("toml") {
                            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                                menus.push(stem.to_string());
                            }
                        }
                    }
                }
            }
        }
        menus.sort();
        menus
    }

    fn serialize_store(
        store: &gtk::TreeStore,
        parent_iter: Option<&gtk::TreeIter>,
    ) -> Vec<launcher_core::MenuItem> {
        let mut items = vec![];
        let mut iter = if let Some(parent) = parent_iter {
            store.iter_children(Some(parent))
        } else {
            store.iter_children(None)
        };

        while let Some(current_iter) = iter {
            let label: String = store.get(&current_iter, 1);
            let icon: String = store.get(&current_iter, 0);
            let action_type: String = store.get(&current_iter, 2);
            let action_cmd: String = store.get(&current_iter, 3);
            let keep_open: bool = store.get(&current_iter, 4);
            let quick_select: String = store.get(&current_iter, 5);

            let icon_opt = if icon.is_empty() { None } else { Some(icon) };
            let quick_select_opt = quick_select.chars().next();

            let children = Self::serialize_store(store, Some(&current_iter));

            let action = if children.is_empty() {
                match action_type.as_str() {
                    "shell command" => Some(launcher_core::Action::Command {
                        cmd: action_cmd,
                        keep_open,
                    }),
                    "hotkey" => Some(launcher_core::Action::Hotkey {
                        keys: action_cmd,
                        keep_open,
                    }),
                    _ => None,
                }
            } else {
                None
            };

            items.push(launcher_core::MenuItem {
                label,
                icon: icon_opt,
                action,
                quick_select_key: quick_select_opt,
                children,
            });

            let mut next_iter = current_iter;
            if store.iter_next(&mut next_iter) {
                iter = Some(next_iter);
            } else {
                break;
            }
        }

        items
    }

    fn get_available_themes(config_path: &Path) -> Vec<String> {
        let mut themes = vec!["system".to_string(), "default".to_string()];
        if let Some(parent) = config_path.parent() {
            let themes_dir = parent.join("themes");
            if themes_dir.exists() {
                if let Ok(entries) = std::fs::read_dir(themes_dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file() && path.extension().map_or(false, |ext| ext == "css") {
                            if let Some(stem) = path.file_stem() {
                                let name = stem.to_string_lossy().into_owned();
                                if name != "default" && !themes.contains(&name) {
                                    themes.push(name);
                                }
                            }
                        }
                    }
                }
            }
        }
        themes
    }

    fn refresh_material_grid(
        flow_box: &gtk::FlowBox,
        search_text: &str,
        items: &[MaterialIconItem],
        entry_icon: &gtk::Entry,
        dialog: &gtk::Window,
    ) {
        // Clear existing children
        while let Some(child) = flow_box.first_child() {
            flow_box.remove(&child);
        }

        let search_lower = search_text.trim().to_lowercase();
        let mut count = 0;

        for item in items {
            if search_lower.is_empty() || item.search_key.contains(&search_lower) {
                let btn = gtk::Button::builder().has_frame(false).build();
                let btn_box = gtk::Box::new(gtk::Orientation::Vertical, 2);

                let mut glyph_buf = [0u8; 4];
                let glyph_str = item.glyph.encode_utf8(&mut glyph_buf);
                let lbl_glyph = gtk::Label::new(Some(glyph_str));
                lbl_glyph.add_css_class("material-icon-glyph");

                let lbl_name = gtk::Label::new(Some(&item.name));
                lbl_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                lbl_name.set_max_width_chars(10);

                btn_box.append(&lbl_glyph);
                btn_box.append(&lbl_name);
                btn.set_child(Some(&btn_box));

                flow_box.insert(&btn, -1);

                let name_clone = item.name.clone();
                let entry_clone = entry_icon.clone();
                let dialog_clone = dialog.clone();
                btn.connect_clicked(move |_| {
                    entry_clone.set_text(&name_clone);
                    dialog_clone.close();
                });

                count += 1;
                if count >= 100 {
                    break;
                }
            }
        }
    }

    fn refresh_system_grid(
        flow_box: &gtk::FlowBox,
        search_text: &str,
        icons: &[SystemIconItem],
        entry_icon: &gtk::Entry,
        dialog: &gtk::Window,
    ) {
        // Clear existing children
        while let Some(child) = flow_box.first_child() {
            flow_box.remove(&child);
        }

        let search_lower = search_text.trim().to_lowercase();
        let mut count = 0;

        for item in icons {
            if search_lower.is_empty() || item.search_key.contains(&search_lower) {
                let btn = gtk::Button::builder().has_frame(false).build();
                let btn_box = gtk::Box::new(gtk::Orientation::Vertical, 2);

                let img = gtk::Image::from_icon_name(&item.name);
                img.set_icon_size(gtk::IconSize::Large);

                let lbl_name = gtk::Label::new(Some(&item.name));
                lbl_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                lbl_name.set_max_width_chars(10);

                btn_box.append(&img);
                btn_box.append(&lbl_name);
                btn.set_child(Some(&btn_box));

                flow_box.insert(&btn, -1);

                let name_clone = item.name.clone();
                let entry_clone = entry_icon.clone();
                let dialog_clone = dialog.clone();
                btn.connect_clicked(move |_| {
                    entry_clone.set_text(&format!("sys:{}", name_clone));
                    dialog_clone.close();
                });

                count += 1;
                if count >= 100 {
                    break;
                }
            }
        }
    }

    fn show_icon_picker(
        parent_window: &gtk::ApplicationWindow,
        entry_icon: &gtk::Entry,
        config_path: PathBuf,
    ) {
        let dialog = gtk::Window::builder()
            .title("Select Icon")
            .transient_for(parent_window)
            .modal(true)
            .default_width(600)
            .default_height(500)
            .build();

        let main_box = gtk::Box::new(gtk::Orientation::Vertical, 10);
        main_box.set_margin_start(10);
        main_box.set_margin_end(10);
        main_box.set_margin_top(10);
        main_box.set_margin_bottom(10);
        main_box.set_vexpand(true);
        main_box.set_hexpand(true);

        // Search entry
        let search_entry = gtk::SearchEntry::new();
        main_box.append(&search_entry);

        // Tabs
        let notebook = gtk::Notebook::new();
        notebook.set_vexpand(true);
        notebook.set_hexpand(true);

        // Tab 1: Material Symbols
        let material_scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .build();
        material_scrolled.set_vexpand(true);
        material_scrolled.set_hexpand(true);

        let material_flow = gtk::FlowBox::new();
        material_flow.set_max_children_per_line(8);
        material_flow.set_selection_mode(gtk::SelectionMode::None);
        material_flow.set_vexpand(true);
        material_flow.set_hexpand(true);
        material_scrolled.set_child(Some(&material_flow));
        notebook.append_page(
            &material_scrolled,
            Some(&gtk::Label::new(Some("Material Symbols"))),
        );

        // Tab 2: System Icons
        let system_scrolled = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .build();
        system_scrolled.set_vexpand(true);
        system_scrolled.set_hexpand(true);

        let system_flow = gtk::FlowBox::new();
        system_flow.set_max_children_per_line(8);
        system_flow.set_selection_mode(gtk::SelectionMode::None);
        system_flow.set_vexpand(true);
        system_flow.set_hexpand(true);
        system_scrolled.set_child(Some(&system_flow));
        notebook.append_page(
            &system_scrolled,
            Some(&gtk::Label::new(Some("System Icons"))),
        );

        main_box.append(&notebook);
        dialog.set_child(Some(&main_box));

        // Load Material codepoints and tags metadata
        let codepoints = launcher_core::load_material_codepoints(&config_path);
        let tags_map = launcher_core::load_material_tags(&config_path);
        let mut material_items: Vec<MaterialIconItem> = codepoints
            .into_iter()
            .map(|(name, glyph)| {
                let search_key = if let Some(tags) = tags_map.get(&name) {
                    format!("{} {}", name.to_lowercase(), tags.join(" "))
                } else {
                    name.to_lowercase()
                };
                MaterialIconItem {
                    name,
                    glyph,
                    search_key,
                }
            })
            .collect();
        material_items.sort_by(|a, b| a.name.cmp(&b.name));

        // Load System icons using gtk::IconTheme
        let display = WidgetExt::display(parent_window);
        let icon_theme = gtk::IconTheme::for_display(&display);
        let mut system_icons_vec: Vec<SystemIconItem> = icon_theme
            .icon_names()
            .into_iter()
            .map(|s| {
                let name = s.to_string();
                let search_key = name.to_lowercase();
                SystemIconItem { name, search_key }
            })
            .collect();
        system_icons_vec.sort_by(|a, b| a.name.cmp(&b.name));

        // Initial populate (first 100 items)
        Self::refresh_material_grid(&material_flow, "", &material_items, entry_icon, &dialog);
        Self::refresh_system_grid(&system_flow, "", &system_icons_vec, entry_icon, &dialog);

        // Connect Search with 150ms debounce
        let material_flow_clone = material_flow.clone();
        let system_flow_clone = system_flow.clone();
        let material_items_clone = material_items.clone();
        let system_icons_clone = system_icons_vec.clone();
        let entry_icon_clone = entry_icon.clone();
        let dialog_clone = dialog.clone();
        let debounce_source = Rc::new(RefCell::new(None::<gtk::glib::SourceId>));
        let debounce_source_clone = debounce_source.clone();

        search_entry.connect_search_changed(move |entry| {
            if let Some(src) = debounce_source_clone.borrow_mut().take() {
                src.remove();
            }
            let text = entry.text().to_string();
            let mat_flow = material_flow_clone.clone();
            let sys_flow = system_flow_clone.clone();
            let mat_items = material_items_clone.clone();
            let sys_items = system_icons_clone.clone();
            let ent_icon = entry_icon_clone.clone();
            let dlg = dialog_clone.clone();
            let src_holder = debounce_source_clone.clone();

            let source_id = gtk::glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
                *src_holder.borrow_mut() = None;
                Self::refresh_material_grid(
                    &mat_flow,
                    &text,
                    &mat_items,
                    &ent_icon,
                    &dlg,
                );
                Self::refresh_system_grid(
                    &sys_flow,
                    &text,
                    &sys_items,
                    &ent_icon,
                    &dlg,
                );
                gtk::glib::ControlFlow::Break
            });
            *debounce_source_clone.borrow_mut() = Some(source_id);
        });

        dialog.present();
    }

    fn resolve_insertion_coords(
        store: &gtk::TreeStore,
        selection: &gtk::TreeSelection,
    ) -> (Option<gtk::TreeIter>, Option<gtk::TreeIter>) {
        if let Some((_, selected_iter)) = selection.selected() {
            let act_type: String = store.get(&selected_iter, 2);
            if act_type == "submenu" || act_type == "root" {
                (Some(selected_iter), None)
            } else {
                let parent = store.iter_parent(&selected_iter);
                (parent, Some(selected_iter))
            }
        } else {
            if let Some(root_iter) = store.iter_children(None) {
                (Some(root_iter), None)
            } else {
                (None, None)
            }
        }
    }

    fn copy_node_recursive(store: &gtk::TreeStore, iter: &gtk::TreeIter) -> CopiedNode {
        let icon: String = store.get(iter, 0);
        let label: String = store.get(iter, 1);
        let action_type: String = store.get(iter, 2);
        let action_cmd: String = store.get(iter, 3);
        let keep_open: bool = store.get(iter, 4);
        let quick_select: String = store.get(iter, 5);

        let mut children = vec![];
        if let Some(child_iter) = store.iter_children(Some(iter)) {
            let mut current = child_iter;
            loop {
                children.push(Self::copy_node_recursive(store, &current));
                if !store.iter_next(&mut current) {
                    break;
                }
            }
        }

        CopiedNode {
            label,
            icon,
            action_type,
            action_cmd,
            keep_open,
            quick_select,
            children,
        }
    }

    fn paste_node_recursive(
        store: &gtk::TreeStore,
        parent: Option<&gtk::TreeIter>,
        sibling: Option<&gtk::TreeIter>,
        node: &CopiedNode,
    ) -> gtk::TreeIter {
        let new_iter = store.insert_after(parent, sibling);
        store.set_value(&new_iter, 0, &node.icon.to_value());
        store.set_value(&new_iter, 1, &node.label.to_value());
        store.set_value(&new_iter, 2, &node.action_type.to_value());
        store.set_value(&new_iter, 3, &node.action_cmd.to_value());
        store.set_value(&new_iter, 4, &node.keep_open.to_value());
        store.set_value(&new_iter, 5, &node.quick_select.to_value());

        let mut prev_child = None;
        for child in &node.children {
            let inserted_child =
                Self::paste_node_recursive(store, Some(&new_iter), prev_child.as_ref(), child);
            prev_child = Some(inserted_child);
        }

        new_iter
    }

    fn update_quick_select_placeholder(
        store: &gtk::TreeStore,
        iter: &gtk::TreeIter,
        entry: &gtk::Entry,
    ) {
        let path = store.path(iter);
        let indices = path.indices();
        if let Some(&last_index) = indices.last() {
            let default_key = if last_index < 9 {
                format!("{}", last_index + 1)
            } else if last_index == 9 {
                "0".to_string()
            } else {
                "".to_string()
            };
            entry.set_placeholder_text(Some(&default_key));
        } else {
            entry.set_placeholder_text(None);
        }
    }
}
