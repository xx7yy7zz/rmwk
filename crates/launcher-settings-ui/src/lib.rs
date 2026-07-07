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
    children: Vec<CopiedNode>,
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
            .material-icon-glyph {{
                font-family: 'Material Symbols Rounded';
                font-size: 24px;
            }}
        ",
            font_path.to_string_lossy()
        );
        font_provider.load_from_data(&font_css);

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
            Err(_) => launcher_core::MenuConfig { menu: vec![] },
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
        ]);

        let root_iter = store.insert_with_values(
            None,
            None,
            &[
                (0, &"menu".to_value()),
                (1, &"Menu (Root)".to_value()),
                (2, &"root".to_value()),
                (3, &"".to_value()),
                (4, &false.to_value()),
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
        let btn_copy = gtk::Button::with_label("Copy");
        let btn_paste = gtk::Button::with_label("Paste");
        btn_paste.set_sensitive(false);
        let btn_delete = gtk::Button::with_label("Delete");
        let btn_up = gtk::Button::with_label("▲");
        let btn_down = gtk::Button::with_label("▼");

        btn_hbox.append(&btn_add_item);
        btn_hbox.append(&btn_add_sub);
        btn_hbox.append(&btn_copy);
        btn_hbox.append(&btn_paste);
        btn_hbox.append(&btn_delete);
        btn_hbox.append(&btn_up);
        btn_hbox.append(&btn_down);
        left_vbox.append(&btn_hbox);

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

        // 4. Command Entry
        let lbl_cmd = gtk::Label::new(Some("Command:"));
        lbl_cmd.set_halign(gtk::Align::End);
        let entry_cmd = gtk::Entry::new();
        prop_grid.attach(&lbl_cmd, 0, 3, 1, 1);
        prop_grid.attach(&entry_cmd, 1, 3, 1, 1);

        let chk_item_keep_open = gtk::CheckButton::with_label("Keep Launcher Open");
        prop_grid.attach(&chk_item_keep_open, 1, 4, 1, 1);

        prop_frame.set_child(Some(&prop_grid));
        right_vbox.append(&prop_frame);

        // Theme and Global settings Box
        let settings_hbox = gtk::Box::new(gtk::Orientation::Horizontal, 10);

        let lbl_theme = gtk::Label::new(Some("Theme:"));
        let combo_theme = gtk::ComboBoxText::new();

        // Populate available themes
        let themes = Self::get_available_themes(&config_path);
        for theme in &themes {
            combo_theme.append(Some(theme), theme);
        }
        combo_theme.set_active_id(Some(&ui_config.ui.theme));

        let lbl_extra_radius = gtk::Label::new(Some("Active Margin (px):"));
        let spin_extra_radius = gtk::SpinButton::with_range(0.0, 300.0, 5.0);
        spin_extra_radius.set_value(ui_config.ui.extra_radius);

        let chk_symbolic_icons = gtk::CheckButton::with_label("Symbolic Icons");
        chk_symbolic_icons.set_active(ui_config.ui.use_symbolic_icons);

        let chk_bold_chars = gtk::CheckButton::with_label("Bold Text Icons");
        chk_bold_chars.set_active(ui_config.ui.bold_single_chars);

        let chk_center_layout = gtk::CheckButton::with_label("Center Slices on Axes");
        chk_center_layout.set_active(ui_config.ui.center_layout);

        let chk_disable_anim = gtk::CheckButton::with_label("Disable Animations");
        chk_disable_anim.set_active(ui_config.ui.disable_animations);

        settings_hbox.append(&lbl_theme);
        settings_hbox.append(&combo_theme);
        settings_hbox.append(&lbl_extra_radius);
        settings_hbox.append(&spin_extra_radius);
        right_vbox.append(&settings_hbox);

        let checkboxes_hbox = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        checkboxes_hbox.append(&chk_symbolic_icons);
        checkboxes_hbox.append(&chk_bold_chars);
        checkboxes_hbox.append(&chk_center_layout);
        checkboxes_hbox.append(&chk_disable_anim);
        right_vbox.append(&checkboxes_hbox);

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

        let lbl_cmd_clone = lbl_cmd.clone();
        let entry_cmd_clone = entry_cmd.clone();
        let chk_keep_open_clone = chk_item_keep_open.clone();
        let btn_pick_icon_clone = btn_pick_icon.clone();
        let btn_delete_clone = btn_delete.clone();
        let btn_up_clone = btn_up.clone();
        let btn_down_clone = btn_down.clone();

        selection.connect_changed(move |sel| {
            if let Some((model, iter)) = sel.selected() {
                let icon: String = model.get(&iter, 0);
                let label: String = model.get(&iter, 1);
                let act_type: String = model.get(&iter, 2);
                let cmd: String = model.get(&iter, 3);

                if act_type == "root" {
                    // Disable editing controls for Root node
                    sel_label.set_sensitive(false);
                    sel_icon.set_sensitive(false);
                    sel_icon_type.set_sensitive(false);
                    sel_cmd.set_sensitive(false);
                    sel_keep_open.set_sensitive(false);
                    btn_pick_icon_clone.set_sensitive(false);
                    btn_delete_clone.set_sensitive(false);
                    btn_up_clone.set_sensitive(false);
                    btn_down_clone.set_sensitive(false);

                    lbl_cmd_clone.set_visible(false);
                    entry_cmd_clone.set_visible(false);
                    chk_keep_open_clone.set_visible(false);
                } else {
                    // Enable editing controls for other nodes
                    sel_label.set_sensitive(true);
                    sel_icon.set_sensitive(true);
                    sel_icon_type.set_sensitive(true);
                    sel_cmd.set_sensitive(true);
                    sel_keep_open.set_sensitive(true);
                    btn_pick_icon_clone.set_sensitive(true);
                    btn_delete_clone.set_sensitive(true);
                    btn_up_clone.set_sensitive(true);
                    btn_down_clone.set_sensitive(true);

                    // Show/hide command input dynamically
                    if act_type == "submenu" {
                        lbl_cmd_clone.set_visible(false);
                        entry_cmd_clone.set_visible(false);
                        chk_keep_open_clone.set_visible(false);
                    } else {
                        lbl_cmd_clone.set_visible(true);
                        entry_cmd_clone.set_visible(true);
                        chk_keep_open_clone.set_visible(true);
                    }
                }

                let keep_open: bool = model.get(&iter, 4);

                sel_label.set_text(&label);

                if icon.chars().count() == 1 {
                    sel_icon_type.set_active_id(Some("char"));
                } else {
                    sel_icon_type.set_active_id(Some("picker"));
                }

                sel_icon.set_text(&icon);
                sel_cmd.set_text(&cmd);
                sel_keep_open.set_active(keep_open);
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
        entry_cmd.connect_changed(move |e| {
            if let Some((_, iter)) = sel_c.selected() {
                store_c.set_value(&iter, 3, &e.text().to_string().to_value());
            }
        });
        
        let store_k = store.clone();
        let sel_k = tree_view.selection();
        chk_item_keep_open.connect_toggled(move |c| {
            if let Some((_, iter)) = sel_k.selected() {
                store_k.set_value(&iter, 4, &c.is_active().to_value());
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
                ],
            );
            selection_sub.select_iter(&new_iter);
        });

        // Delete Button
        let store_del = store.clone();
        let selection_del = tree_view.selection();
        btn_delete.connect_clicked(move |_| {
            if let Some((_, iter)) = selection_del.selected() {
                store_del.remove(&iter);
            }
        });

        // Move Up Button
        let store_up = store.clone();
        let selection_up = tree_view.selection();
        btn_up.connect_clicked(move |_| {
            if let Some((_, iter)) = selection_up.selected() {
                let mut prev = iter.clone();
                // To move up, we swap with the previous sibling in the TreeStore
                if store_up.iter_previous(&mut prev) {
                    store_up.swap(&iter, &prev);
                }
            }
        });

        // Move Down Button
        let store_down = store.clone();
        let selection_down = tree_view.selection();
        btn_down.connect_clicked(move |_| {
            if let Some((_, iter)) = selection_down.selected() {
                let mut next = iter.clone();
                // In TreeStore, moving down swaps with the next sibling
                if store_down.iter_next(&mut next) {
                    store_down.swap(&iter, &next);
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
        let spin_extra_radius_save = spin_extra_radius.clone();
        let chk_symbolic_icons_save = chk_symbolic_icons.clone();
        let chk_bold_chars_save = chk_bold_chars.clone();
        let chk_center_layout_save = chk_center_layout.clone();
        let chk_disable_anim_save = chk_disable_anim.clone();
        btn_save.connect_clicked(move |_| {
            // 1. Serialize and save the menu (serialize only the children of the permanent "Menu (Root)" node)
            let mut items = vec![];
            if let Some(root_iter) = store_save.iter_children(None) {
                items = Self::serialize_store(&store_save, Some(&root_iter));
            }
            let menu_config = launcher_core::MenuConfig { menu: items };
            if let Err(e) = launcher_core::save_menu(&menu_path, &menu_config) {
                error!("Failed to save menu configuration: {}", e);
            } else {
                info!("Menu config saved successfully to {:?}", menu_path);
            }

            // 2. Save active theme, extra_radius, etc. back to config.toml
            if let Some(theme_id) = combo_theme_save.active_id() {
                let mut cfg = match launcher_core::load_config(&config_path_save) {
                    Ok(c) => c,
                    Err(_) => launcher_core::Config::default(),
                };
                cfg.ui.theme = theme_id.to_string();
                cfg.ui.extra_radius = spin_extra_radius_save.value();
                cfg.ui.use_symbolic_icons = chk_symbolic_icons_save.is_active();
                cfg.ui.bold_single_chars = chk_bold_chars_save.is_active();
                cfg.ui.center_layout = chk_center_layout_save.is_active();
                cfg.ui.disable_animations = chk_disable_anim_save.is_active();

                // Write back config.toml
                let content = toml::to_string_pretty(&cfg).unwrap();
                if let Err(e) = std::fs::write(&config_path_save, content) {
                    error!("Failed to save UI config: {}", e);
                } else {
                    info!("UI config saved successfully to {:?}", config_path_save);
                }
            }

            // 3. Trigger Hot-Reload over IPC socket
            let socket_path = launcher_ipc::get_socket_path();
            if socket_path.exists() {
                // Spawn a quick tokio block to send the reload message
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap();
                let _ = rt.block_on(async {
                    let _ = launcher_ipc::send_message(
                        &socket_path,
                        &launcher_ipc::IpcMessage::ReloadConfig,
                    )
                    .await;
                });
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
                            Some(launcher_core::Action::Command { .. }) => "shell command".to_value(),
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
                            None => "".to_value(),
                        },
                    ),
                    (
                        4,
                        &match &item.action {
                            Some(launcher_core::Action::Command { keep_open, .. }) => keep_open.to_value(),
                            None => false.to_value(),
                        },
                    ),
                ],
            );

            if !item.children.is_empty() {
                Self::populate_store(store, Some(&current_iter), &item.children);
            }
        }
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

            let icon_opt = if icon.is_empty() { None } else { Some(icon) };

            let children = Self::serialize_store(store, Some(&current_iter));

            let action = if children.is_empty() {
                match action_type.as_str() {
                    "shell command" => Some(launcher_core::Action::Command { cmd: action_cmd, keep_open }),
                    _ => None,
                }
            } else {
                None
            };

            items.push(launcher_core::MenuItem {
                label,
                icon: icon_opt,
                action,
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
        let mut themes = vec!["default".to_string()];
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
        codepoints: &[(String, char)],
        entry_icon: &gtk::Entry,
        dialog: &gtk::Window,
    ) {
        // Clear existing children
        while let Some(child) = flow_box.first_child() {
            flow_box.remove(&child);
        }

        let search_lower = search_text.to_lowercase();
        let mut count = 0;

        for (name, glyph) in codepoints {
            if search_lower.is_empty() || name.to_lowercase().contains(&search_lower) {
                let btn = gtk::Button::builder().has_frame(false).build();
                let btn_box = gtk::Box::new(gtk::Orientation::Vertical, 2);

                let lbl_glyph = gtk::Label::new(Some(&glyph.to_string()));
                lbl_glyph.add_css_class("material-icon-glyph");

                let lbl_name = gtk::Label::new(Some(name));
                lbl_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                lbl_name.set_max_width_chars(10);

                btn_box.append(&lbl_glyph);
                btn_box.append(&lbl_name);
                btn.set_child(Some(&btn_box));

                flow_box.insert(&btn, -1);

                let name_clone = name.clone();
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
        icons: &[String],
        entry_icon: &gtk::Entry,
        dialog: &gtk::Window,
    ) {
        // Clear existing children
        while let Some(child) = flow_box.first_child() {
            flow_box.remove(&child);
        }

        let search_lower = search_text.to_lowercase();
        let mut count = 0;

        for name in icons {
            if search_lower.is_empty() || name.to_lowercase().contains(&search_lower) {
                let btn = gtk::Button::builder().has_frame(false).build();
                let btn_box = gtk::Box::new(gtk::Orientation::Vertical, 2);

                let img = gtk::Image::from_icon_name(name);
                img.set_icon_size(gtk::IconSize::Large);

                let lbl_name = gtk::Label::new(Some(name));
                lbl_name.set_ellipsize(gtk::pango::EllipsizeMode::End);
                lbl_name.set_max_width_chars(10);

                btn_box.append(&img);
                btn_box.append(&lbl_name);
                btn.set_child(Some(&btn_box));

                flow_box.insert(&btn, -1);

                let name_clone = name.clone();
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

        // Load Material codepoints
        let codepoints = launcher_core::load_material_codepoints(&config_path);
        let mut codepoints_vec: Vec<(String, char)> = codepoints.into_iter().collect();
        codepoints_vec.sort_by(|a, b| a.0.cmp(&b.0));

        // Load System icons using gtk::IconTheme
        let display = WidgetExt::display(parent_window);
        let icon_theme = gtk::IconTheme::for_display(&display);
        let mut system_icons_vec: Vec<String> = icon_theme
            .icon_names()
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        system_icons_vec.sort();

        // Initial populate (first 100 items)
        Self::refresh_material_grid(&material_flow, "", &codepoints_vec, entry_icon, &dialog);
        Self::refresh_system_grid(&system_flow, "", &system_icons_vec, entry_icon, &dialog);

        // Connect Search
        let material_flow_clone = material_flow.clone();
        let system_flow_clone = system_flow.clone();
        let codepoints_clone = codepoints_vec.clone();
        let system_icons_clone = system_icons_vec.clone();
        let entry_icon_clone = entry_icon.clone();
        let dialog_clone = dialog.clone();

        search_entry.connect_search_changed(move |entry| {
            let text = entry.text().to_lowercase();
            Self::refresh_material_grid(
                &material_flow_clone,
                &text,
                &codepoints_clone,
                &entry_icon_clone,
                &dialog_clone,
            );
            Self::refresh_system_grid(
                &system_flow_clone,
                &text,
                &system_icons_clone,
                &entry_icon_clone,
                &dialog_clone,
            );
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

        let mut prev_child = None;
        for child in &node.children {
            let inserted_child =
                Self::paste_node_recursive(store, Some(&new_iter), prev_child.as_ref(), child);
            prev_child = Some(inserted_child);
        }

        new_iter
    }
}
