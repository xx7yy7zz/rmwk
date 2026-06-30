use gtk::prelude::*;
use gtk4 as gtk;
use std::path::{Path, PathBuf};
use tracing::{error, info};

pub struct SettingsApp {
    app: gtk::Application,
    menu_path: PathBuf,
    config_path: PathBuf,
}

impl SettingsApp {
    pub fn new(menu_path: PathBuf, config_path: PathBuf) -> Self {
        let app = gtk::Application::builder()
            .application_id("org.radial_launcher.settings")
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
        window.set_title(Some("Radial Launcher Settings"));
        window.set_default_size(800, 500);

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
        left_vbox.set_width_request(350);

        // Scrollable window for TreeView
        let scroll_win = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .build();

        // Create TreeStore:
        // Col 0: Icon (String)
        // Col 1: Label (String)
        // Col 2: Action Type (String: "exec", "shell", "submenu")
        // Col 3: Action Command (String)
        let store = gtk::TreeStore::new(&[
            glib::Type::STRING,
            glib::Type::STRING,
            glib::Type::STRING,
            glib::Type::STRING,
        ]);

        Self::populate_store(&store, None, &menu_config.menu);

        let tree_view = gtk::TreeView::with_model(&store);
        tree_view.set_headers_visible(true);

        // Col 1: Label
        let label_renderer = gtk::CellRendererText::new();
        let label_column = gtk::TreeViewColumn::new();
        label_column.set_title("Menu Item");
        label_column.pack_start(&label_renderer, true);
        label_column.add_attribute(&label_renderer, "text", 1);
        tree_view.append_column(&label_column);

        // Col 2: Action Type
        let type_renderer = gtk::CellRendererText::new();
        let type_column = gtk::TreeViewColumn::new();
        type_column.set_title("Type");
        type_column.pack_start(&type_renderer, true);
        type_column.add_attribute(&type_renderer, "text", 2);
        tree_view.append_column(&type_column);

        scroll_win.set_child(Some(&tree_view));
        left_vbox.append(&scroll_win);

        // Buttons under TreeView
        let btn_hbox = gtk::Box::new(gtk::Orientation::Horizontal, 5);

        let btn_add_item = gtk::Button::with_label("Add Item");
        let btn_add_sub = gtk::Button::with_label("Add Submenu");
        let btn_delete = gtk::Button::with_label("Delete");
        let btn_up = gtk::Button::with_label("▲");
        let btn_down = gtk::Button::with_label("▼");

        btn_hbox.append(&btn_add_item);
        btn_hbox.append(&btn_add_sub);
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

        // 2. Icon Entry
        let lbl_icon = gtk::Label::new(Some("Icon:"));
        lbl_icon.set_halign(gtk::Align::End);
        let entry_icon = gtk::Entry::new();
        prop_grid.attach(&lbl_icon, 0, 1, 1, 1);
        prop_grid.attach(&entry_icon, 1, 1, 1, 1);

        // 3. Type Dropdown
        let lbl_type = gtk::Label::new(Some("Type:"));
        lbl_type.set_halign(gtk::Align::End);
        let combo_type = gtk::ComboBoxText::new();
        combo_type.append(Some("exec"), "Execute Process");
        combo_type.append(Some("shell"), "Shell Command");
        combo_type.append(Some("submenu"), "Submenu Directory");
        prop_grid.attach(&lbl_type, 0, 2, 1, 1);
        prop_grid.attach(&combo_type, 1, 2, 1, 1);

        // 4. Command Entry
        let lbl_cmd = gtk::Label::new(Some("Command:"));
        lbl_cmd.set_halign(gtk::Align::End);
        let entry_cmd = gtk::Entry::new();
        prop_grid.attach(&lbl_cmd, 0, 3, 1, 1);
        prop_grid.attach(&entry_cmd, 1, 3, 1, 1);

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

        settings_hbox.append(&lbl_theme);
        settings_hbox.append(&combo_theme);
        settings_hbox.append(&lbl_extra_radius);
        settings_hbox.append(&spin_extra_radius);
        right_vbox.append(&settings_hbox);

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

        // Prevent commands from being edited if it is a submenu
        let entry_cmd_clone = entry_cmd.clone();
        combo_type.connect_changed(move |combo| {
            if let Some(id) = combo.active_id() {
                if id == "submenu" {
                    entry_cmd_clone.set_sensitive(false);
                    entry_cmd_clone.set_text("");
                } else {
                    entry_cmd_clone.set_sensitive(true);
                }
            }
        });

        // Selection Change: Updates property inputs
        let selection = tree_view.selection();
        let sel_label = entry_label.clone();
        let sel_icon = entry_icon.clone();
        let sel_type = combo_type.clone();
        let sel_cmd = entry_cmd.clone();

        selection.connect_changed(move |sel| {
            if let Some((model, iter)) = sel.selected() {
                let icon: String = model.get(&iter, 0);
                let label: String = model.get(&iter, 1);
                let act_type: String = model.get(&iter, 2);
                let cmd: String = model.get(&iter, 3);

                sel_label.set_text(&label);
                sel_icon.set_text(&icon);
                sel_type.set_active_id(Some(&act_type));
                sel_cmd.set_text(&cmd);
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

        let store_t = store.clone();
        let sel_t = tree_view.selection();
        combo_type.connect_changed(move |c| {
            if let Some(id) = c.active_id() {
                if let Some((_, iter)) = sel_t.selected() {
                    store_t.set_value(&iter, 2, &id.to_string().to_value());
                }
            }
        });

        let store_c = store.clone();
        let sel_c = tree_view.selection();
        entry_cmd.connect_changed(move |e| {
            if let Some((_, iter)) = sel_c.selected() {
                store_c.set_value(&iter, 3, &e.text().to_string().to_value());
            }
        });

        // Add Item Button
        let store_add = store.clone();
        let selection_add = tree_view.selection();
        btn_add_item.connect_clicked(move |_| {
            let parent_iter = selection_add.selected().map(|(_, iter)| iter);
            let new_iter = store_add.insert_with_values(
                parent_iter.as_ref(),
                None,
                &[
                    (0, &"application-x-executable".to_value()),
                    (1, &"New Item".to_value()),
                    (2, &"exec".to_value()),
                    (3, &"".to_value()),
                ],
            );
            selection_add.select_iter(&new_iter);
        });

        // Add Submenu Button
        let store_sub = store.clone();
        let selection_sub = tree_view.selection();
        btn_add_sub.connect_clicked(move |_| {
            let parent_iter = selection_sub.selected().map(|(_, iter)| iter);
            let new_iter = store_sub.insert_with_values(
                parent_iter.as_ref(),
                None,
                &[
                    (0, &"folder".to_value()),
                    (1, &"New Submenu".to_value()),
                    (2, &"submenu".to_value()),
                    (3, &"".to_value()),
                ],
            );
            // Insert a dummy item to make it a subdirectory
            store_sub.insert_with_values(
                Some(&new_iter),
                None,
                &[
                    (0, &"application-x-executable".to_value()),
                    (1, &"Placeholder Item".to_value()),
                    (2, &"exec".to_value()),
                    (3, &"".to_value()),
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

        // Save Button Handler
        let store_save = store.clone();
        let config_path_save = config_path.clone();
        let combo_theme_save = combo_theme.clone();
        let spin_extra_radius_save = spin_extra_radius.clone();
        btn_save.connect_clicked(move |_| {
            // 1. Serialize and save the menu
            let items = Self::serialize_store(&store_save, None);
            let menu_config = launcher_core::MenuConfig { menu: items };
            if let Err(e) = launcher_core::save_menu(&menu_path, &menu_config) {
                error!("Failed to save menu configuration: {}", e);
            } else {
                info!("Menu config saved successfully to {:?}", menu_path);
            }

            // 2. Save active theme & extra_radius back to config.toml
            if let Some(theme_id) = combo_theme_save.active_id() {
                let mut cfg = match launcher_core::load_config(&config_path_save) {
                    Ok(c) => c,
                    Err(_) => launcher_core::Config::default(),
                };
                cfg.ui.theme = theme_id.to_string();
                cfg.ui.extra_radius = spin_extra_radius_save.value();

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
                            Some(launcher_core::Action::Exec { .. }) => "exec".to_value(),
                            Some(launcher_core::Action::Shell { .. }) => "shell".to_value(),
                            None => {
                                if item.children.is_empty() {
                                    "exec".to_value()
                                } else {
                                    "submenu".to_value()
                                }
                            }
                        },
                    ),
                    (
                        3,
                        &match &item.action {
                            Some(launcher_core::Action::Exec { cmd }) => cmd.to_value(),
                            Some(launcher_core::Action::Shell { cmd }) => cmd.to_value(),
                            None => "".to_value(),
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

            let icon_opt = if icon.is_empty() { None } else { Some(icon) };

            let children = Self::serialize_store(store, Some(&current_iter));

            let action = if children.is_empty() {
                match action_type.as_str() {
                    "exec" => Some(launcher_core::Action::Exec { cmd: action_cmd }),
                    "shell" => Some(launcher_core::Action::Shell { cmd: action_cmd }),
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
}
