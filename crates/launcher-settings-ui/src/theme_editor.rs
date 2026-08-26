use crate::standard_theme::{load_standard_theme, save_standard_theme, StandardThemeOverrides};
use gtk4::gdk;
use gtk4::gio;
use gtk4::prelude::*;
use launcher_core::SystemThemeOverrides;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;

#[allow(deprecated)]
fn lookup_style_color(style: &gtk4::StyleContext, color_name: &str) -> Option<gdk::RGBA> {
    style.lookup_color(color_name)
}

fn system_to_standard(
    sys: &SystemThemeOverrides,
    style: &gtk4::StyleContext,
) -> StandardThemeOverrides {
    let c = |var: &str, opacity: f64| -> gdk::RGBA {
        let name = var.trim_start_matches('@');
        let mut col = lookup_style_color(style, name).unwrap_or(gdk::RGBA::BLACK);
        col.set_alpha(col.alpha() * opacity as f32);
        col
    };
    StandardThemeOverrides {
        entry_surface: c(&sys.entry_surface.variable, sys.entry_surface.opacity),
        entry_surface_hover: c(&sys.entry_surface_hover.variable, sys.entry_surface_hover.opacity),
        entry_border: c(&sys.entry_border.variable, sys.entry_border.opacity),
        entry_border_hover: c(&sys.entry_border_hover.variable, sys.entry_border_hover.opacity),
        label: c(&sys.label.variable, sys.label.opacity),
        label_hover: c(&sys.label_hover.variable, sys.label_hover.opacity),
        entry_icon: c(&sys.entry_icon.variable, sys.entry_icon.opacity),
        entry_icon_hover: c(&sys.entry_icon_hover.variable, sys.entry_icon_hover.opacity),
        floating_icon_surface: c(
            &sys.floating_icon_surface.variable,
            sys.floating_icon_surface.opacity,
        ),
        floating_icon_surface_hover: c(
            &sys.floating_icon_surface_hover.variable,
            sys.floating_icon_surface_hover.opacity,
        ),
        hub_surface: c(&sys.hub_surface.variable, sys.hub_surface.opacity),
        hub_border: c(&sys.hub_border.variable, sys.hub_border.opacity),
        hub_label: c(&sys.hub_label.variable, sys.hub_label.opacity),
        hub_icon: c(&sys.hub_icon.variable, sys.hub_icon.opacity),
        pie_outer_border: c(&sys.pie_outer_border.variable, sys.pie_outer_border.opacity),
    }
}

fn rounded_rect_path(cr: &gtk4::cairo::Context, x: f64, y: f64, w: f64, h: f64, radius: f64) {
    cr.new_sub_path();
    cr.arc(x + radius, y + radius, radius, std::f64::consts::PI, 1.5 * std::f64::consts::PI);
    cr.arc(x + w - radius, y + radius, radius, 1.5 * std::f64::consts::PI, 2.0 * std::f64::consts::PI);
    cr.arc(x + w - radius, y + h - radius, radius, 0.0, 0.5 * std::f64::consts::PI);
    cr.arc(x + radius, y + h - radius, radius, 0.5 * std::f64::consts::PI, std::f64::consts::PI);
    cr.close_path();
}

fn theme_file_path(theme_name: &str) -> PathBuf {
    let file_name = if theme_name.ends_with(".css") {
        theme_name.to_string()
    } else {
        format!("{}.css", theme_name)
    };
    launcher_core::paths::get_themes_dir().join(file_name)
}

fn scan_available_themes() -> Vec<String> {
    let mut themes = vec!["system".to_string(), "default".to_string()];
    let themes_dir = launcher_core::paths::get_themes_dir();
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
    themes
}

fn combo_has_id(combo: &gtk4::DropDown, id: &str) -> bool {
    crate::dropdown_utils::dropdown_has_id(combo, id)
}

fn combo_remove_id(combo: &gtk4::DropDown, id: &str) {
    crate::dropdown_utils::dropdown_remove_id(combo, id);
}

#[allow(dead_code)]
pub struct ThemeEditor {
    pub container: gtk4::Box,
    pub combo_theme: gtk4::DropDown,
    pub btn_save: gtk4::Button,
    pub btn_save_as: gtk4::Button,
    pub btn_reset: gtk4::Button,
    pub current_system_overrides: Rc<RefCell<SystemThemeOverrides>>,
    pub current_standard_overrides: Rc<RefCell<StandardThemeOverrides>>,
    pub active_monitor: Rc<RefCell<Option<gio::FileMonitor>>>,
    pub dir_monitor: Rc<RefCell<Option<gio::FileMonitor>>>,
}

impl ThemeEditor {
    pub fn new(
        config_path: PathBuf,
        initial_theme: &str,
        initial_sys: Option<SystemThemeOverrides>,
        available_themes: &[String],
        is_saving: Rc<std::cell::Cell<bool>>,
    ) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 10);

        let header_hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        let lbl_theme = gtk4::Label::new(Some("Theme:"));
        let combo_theme = crate::dropdown_utils::create_dropdown();
        for t in available_themes {
            crate::dropdown_utils::dropdown_append(&combo_theme, t);
        }
        crate::dropdown_utils::dropdown_set_active_id(&combo_theme, initial_theme);

        let scroll_ctrl_theme = gtk4::EventControllerScroll::new(
            gtk4::EventControllerScrollFlags::VERTICAL
                | gtk4::EventControllerScrollFlags::HORIZONTAL,
        );
        scroll_ctrl_theme.connect_scroll(|_, _, _| glib::Propagation::Stop);
        combo_theme.add_controller(scroll_ctrl_theme);

        let btn_new_theme = gtk4::Button::from_icon_name("document-new-symbolic");
        btn_new_theme.set_tooltip_text(Some("Create a new theme with default values."));
        let btn_rename_theme = gtk4::Button::from_icon_name("document-edit-symbolic");
        btn_rename_theme.set_tooltip_text(Some("Rename the current theme."));
        let btn_delete_theme = gtk4::Button::from_icon_name("user-trash-symbolic");
        btn_delete_theme.set_tooltip_text(Some("Delete the current theme."));

        header_hbox.append(&lbl_theme);
        header_hbox.append(&combo_theme);
        header_hbox.append(&btn_new_theme);
        header_hbox.append(&btn_rename_theme);
        header_hbox.append(&btn_delete_theme);
        container.append(&header_hbox);

        let tweaks_scrolled = gtk4::ScrolledWindow::new();
        tweaks_scrolled.set_min_content_height(200);
        tweaks_scrolled.set_vexpand(true);
        let tweaks_box = gtk4::Box::new(gtk4::Orientation::Vertical, 5);
        tweaks_scrolled.set_child(Some(&tweaks_box));
        container.append(&tweaks_scrolled);

        let btn_save = gtk4::Button::with_label("Save Theme");
        let btn_save_as = gtk4::Button::with_label("Save As...");
        btn_save_as.set_tooltip_text(Some("Save the current color values as a completely new theme."));
        let btn_reset = gtk4::Button::with_label("Reset to Defaults");
        btn_reset.set_tooltip_text(Some("Reset the system (GTK) theme to its default values."));
        let btn_hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 5);
        btn_hbox.append(&btn_save);
        btn_hbox.append(&btn_save_as);
        btn_hbox.append(&btn_reset);
        container.append(&btn_hbox);

        let sys_overrides = Rc::new(RefCell::new(initial_sys.unwrap_or_default()));
        let std_overrides = Rc::new(RefCell::new(load_standard_theme(initial_theme)));

        let tweaks_box_c = tweaks_box.clone();
        let sys_overrides_c = sys_overrides.clone();
        let std_overrides_c = std_overrides.clone();
        let btn_save_as_c = btn_save_as.clone();
        let btn_reset_c = btn_reset.clone();
        let btn_rename_theme_c = btn_rename_theme.clone();
        let btn_delete_theme_c = btn_delete_theme.clone();
        let active_monitor = Rc::new(RefCell::new(None::<gio::FileMonitor>));
        let active_monitor_c = active_monitor.clone();
        let is_saving_mon = is_saving.clone();

        combo_theme.connect_notify_local(Some("selected"), move |combo, _| {
            if let Some(m) = active_monitor_c.borrow_mut().take() {
                m.cancel();
            }
            if let Some(id) = crate::dropdown_utils::dropdown_active_id(combo) {
                if id == "system" {
                    btn_save_as_c.set_sensitive(true);
                    btn_reset_c.set_sensitive(true);
                    btn_rename_theme_c.set_sensitive(false);
                    btn_delete_theme_c.set_sensitive(false);
                    Self::populate_system_tweaks(&tweaks_box_c, sys_overrides_c.clone());
                } else {
                    btn_save_as_c.set_sensitive(true);
                    btn_reset_c.set_sensitive(false);
                    btn_rename_theme_c.set_sensitive(true);
                    btn_delete_theme_c.set_sensitive(true);
                    *std_overrides_c.borrow_mut() = load_standard_theme(&id);
                    Self::populate_standard_tweaks(&tweaks_box_c, std_overrides_c.clone());

                    let mut theme_path = dirs::config_dir().unwrap_or_default();
                    theme_path.push("rmwk");
                    theme_path.push("themes");
                    let file_name = if id.ends_with(".css") {
                        id.to_string()
                    } else {
                        format!("{}.css", id)
                    };
                    theme_path.push(file_name);

                    // If the theme file is empty/missing, write the default
                    // colors into it so the launcher renders them too, not just
                    // the editor preview.
                    if std::fs::read_to_string(&theme_path)
                        .map_or(true, |c| c.trim().is_empty())
                    {
                        save_standard_theme(&id, &std_overrides_c.borrow());
                    }

                    let file = gio::File::for_path(&theme_path);
                    if let Ok(monitor) =
                        file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
                    {
                        let std_overrides_mc = std_overrides_c.clone();
                        let tweaks_box_mc = tweaks_box_c.clone();
                        let id_mc = id.to_string();
                        let is_saving_m = is_saving_mon.clone();
                        let pending_theme_update = Rc::new(std::cell::Cell::new(false));
                        monitor.connect_changed(move |_, _, _, event| {
                            if is_saving_m.get() {
                                return;
                            }
                            if (event == gio::FileMonitorEvent::ChangesDoneHint
                                || event == gio::FileMonitorEvent::Created)
                                && !pending_theme_update.get()
                            {
                                pending_theme_update.set(true);
                                let std_ov = std_overrides_mc.clone();
                                let twk_box = tweaks_box_mc.clone();
                                let theme_id = id_mc.clone();
                                let pending = pending_theme_update.clone();
                                let is_sav = is_saving_m.clone();
                                gtk4::glib::timeout_add_local(
                                    std::time::Duration::from_millis(150),
                                    move || {
                                        pending.set(false);
                                        if is_sav.get() {
                                            return gtk4::glib::ControlFlow::Break;
                                        }
                                        *std_ov.borrow_mut() = load_standard_theme(&theme_id);
                                        Self::populate_standard_tweaks(&twk_box, std_ov.clone());
                                        gtk4::glib::ControlFlow::Break
                                    },
                                );
                            }
                        });
                        *active_monitor_c.borrow_mut() = Some(monitor);
                    }
                }
            }
        });

        // Directory monitor for themes: pick up theme files created/removed while open
        let dir_monitor = Rc::new(RefCell::new(None::<gio::FileMonitor>));
        let combo_dir = combo_theme.clone();
        let themes_dir_file = gio::File::for_path(launcher_core::paths::get_themes_dir());
        if let Ok(monitor) = themes_dir_file.monitor_directory(
            gio::FileMonitorFlags::NONE,
            gio::Cancellable::NONE,
        ) {
            // Self-cycle: ThemeEditor is dropped after its widgets are extracted,
            // so the monitor must keep itself alive to keep delivering events.
            let mon_keep = monitor.clone();
            let is_sav_d = is_saving.clone();
            let pending_dir = Rc::new(std::cell::Cell::new(false));
            monitor.connect_changed(move |_, _, _, event| {
                let _ = &mon_keep;
                if is_sav_d.get() {
                    return;
                }
                if matches!(
                    event,
                    gio::FileMonitorEvent::Created
                        | gio::FileMonitorEvent::Deleted
                        | gio::FileMonitorEvent::Renamed
                        | gio::FileMonitorEvent::Moved
                ) {
                    if !pending_dir.get() {
                        pending_dir.set(true);
                        let combo = combo_dir.clone();
                        let pending = pending_dir.clone();
                        let is_sav_cb = is_sav_d.clone();
                        gtk4::glib::timeout_add_local(
                            std::time::Duration::from_millis(150),
                            move || {
                                pending.set(false);
                                if is_sav_cb.get() {
                                    return gtk4::glib::ControlFlow::Break;
                                }
                                let intended: Option<String> = crate::dropdown_utils::dropdown_active_id(&combo);
                                crate::dropdown_utils::dropdown_remove_all(&combo);
                                let available = scan_available_themes();
                                for t in &available {
                                    crate::dropdown_utils::dropdown_append(&combo, t);
                                }
                                if let Some(act) = intended {
                                    if available.contains(&act) {
                                        crate::dropdown_utils::dropdown_set_active_id(&combo, &act);
                                    } else if !available.is_empty() {
                                        crate::dropdown_utils::dropdown_set_active_id(&combo, &available[0]);
                                    }
                                }
                                gtk4::glib::ControlFlow::Break
                            },
                        );
                    }
                }
            });
            *dir_monitor.borrow_mut() = Some(monitor);
        }

        let tweaks_box_reset = tweaks_box.clone();
        let sys_overrides_reset = sys_overrides.clone();
        btn_reset.connect_clicked(move |_| {
            *sys_overrides_reset.borrow_mut() = SystemThemeOverrides::default();
            Self::populate_system_tweaks(&tweaks_box_reset, sys_overrides_reset.clone());
        });

        // New Theme Button
        let combo_new = combo_theme.clone();
        btn_new_theme.connect_clicked(move |btn| {
            let win = btn.root().and_downcast::<gtk4::Window>().unwrap();
            let input_window = gtk4::Window::builder()
                .title("New Theme")
                .modal(true)
                .transient_for(&win)
                .default_width(300)
                .default_height(100)
                .build();

            let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
            vbox.set_margin_start(10);
            vbox.set_margin_end(10);
            vbox.set_margin_top(10);
            vbox.set_margin_bottom(10);
            let entry = gtk4::Entry::new();
            entry.set_placeholder_text(Some("Theme Name"));
            let btn_ok = gtk4::Button::with_label("Create");
            vbox.append(&entry);
            vbox.append(&btn_ok);
            input_window.set_child(Some(&vbox));

            let input_window_clone = input_window.clone();
            let win_alert = win.clone();
            let combo_new2 = combo_new.clone();
            btn_ok.connect_clicked(move |_| {
                let text = entry.text().to_string();
                if !text.is_empty() {
                    let do_create = {
                        let combo = combo_new2.clone();
                        let name = text.clone();
                        move || {
                            save_standard_theme(&name, &load_standard_theme("default"));
                            if !combo_has_id(&combo, &name) {
                                crate::dropdown_utils::dropdown_append(&combo, &name);
                            }
                            crate::dropdown_utils::dropdown_set_active_id(&combo, &name);
                        }
                    };

                    if theme_file_path(&text).exists() {
                        let dialog = gtk4::MessageDialog::builder()
                            .text("Theme already exists")
                            .secondary_text("Overwrite?")
                            .buttons(gtk4::ButtonsType::YesNo)
                            .modal(true)
                            .transient_for(&win_alert)
                            .build();

                        let iw_clone = input_window_clone.clone();
                        dialog.connect_response(move |d, response| {
                            if response == gtk4::ResponseType::Yes {
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

        // Rename Theme Button
        let combo_rename = combo_theme.clone();
        btn_rename_theme.connect_clicked(move |btn| {
            if let Some(id) = crate::dropdown_utils::dropdown_active_id(&combo_rename) {
                if id == "system" {
                    return;
                }
                let current_name = id.to_string();
                let win = btn.root().and_downcast::<gtk4::Window>().unwrap();
                let input_window = gtk4::Window::builder()
                    .title("Rename Theme")
                    .modal(true)
                    .transient_for(&win)
                    .default_width(300)
                    .default_height(100)
                    .build();

                let vbox = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
                vbox.set_margin_start(10);
                vbox.set_margin_end(10);
                vbox.set_margin_top(10);
                vbox.set_margin_bottom(10);
                let entry = gtk4::Entry::new();
                entry.set_text(&current_name);
                let btn_ok = gtk4::Button::with_label("Rename");
                vbox.append(&entry);
                vbox.append(&btn_ok);
                input_window.set_child(Some(&vbox));

                let input_window_clone = input_window.clone();
                let win_alert = win.clone();
                let combo_rename2 = combo_rename.clone();
                let old_name = current_name.clone();
                btn_ok.connect_clicked(move |_| {
                    let new_text = entry.text().to_string();
                    if !new_text.is_empty() && new_text != old_name {
                        let do_rename = {
                            let combo = combo_rename2.clone();
                            let old_n = old_name.clone();
                            let new_n = new_text.clone();
                            move || {
                                if let Err(e) =
                                    std::fs::rename(theme_file_path(&old_n), theme_file_path(&new_n))
                                {
                                    tracing::error!("Failed to rename theme: {}", e);
                                    return;
                                }
                                combo_remove_id(&combo, &old_n);
                                if !combo_has_id(&combo, &new_n) {
                                    crate::dropdown_utils::dropdown_append(&combo, &new_n);
                                }
                                crate::dropdown_utils::dropdown_set_active_id(&combo, &new_n);
                            }
                        };

                        if theme_file_path(&new_text).exists() {
                            let dialog = gtk4::MessageDialog::builder()
                                .text("Theme already exists")
                                .secondary_text("Overwrite?")
                                .buttons(gtk4::ButtonsType::YesNo)
                                .modal(true)
                                .transient_for(&win_alert)
                                .build();

                            let iw_clone = input_window_clone.clone();
                            dialog.connect_response(move |d, response| {
                                if response == gtk4::ResponseType::Yes {
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
                    }
                });
                input_window.present();
            }
        });

        // Delete Theme Button
        let combo_del = combo_theme.clone();
        btn_delete_theme.connect_clicked(move |btn| {
            if let Some(id) = crate::dropdown_utils::dropdown_active_id(&combo_del) {
                if id == "system" {
                    return;
                }
                let win = btn.root().and_downcast::<gtk4::Window>().unwrap();
                let dialog = gtk4::MessageDialog::builder()
                    .text("Confirm Deletion")
                    .secondary_text(&format!("Are you sure you want to delete {}.css?", id))
                    .buttons(gtk4::ButtonsType::OkCancel)
                    .modal(true)
                    .transient_for(&win)
                    .build();

                let combo = combo_del.clone();
                dialog.connect_response(move |d, response| {
                    if response == gtk4::ResponseType::Ok {
                        let _ = std::fs::remove_file(theme_file_path(&id));
                        crate::dropdown_utils::dropdown_remove_all(&combo);
                        let themes = scan_available_themes();
                        for t in &themes {
                            crate::dropdown_utils::dropdown_append(&combo, t);
                        }
                        if themes.is_empty() {
                            combo.set_selected(gtk4::INVALID_LIST_POSITION);
                        } else {
                            crate::dropdown_utils::dropdown_set_active_id(&combo, &themes[0]);
                        }
                    }
                    d.destroy();
                });
                dialog.present();
            }
        });

        // Trigger initial population
        combo_theme.notify("selected");

        let btn_save_config_path = config_path.clone();
        let btn_save_combo = combo_theme.clone();
        let btn_save_sys = sys_overrides.clone();
        let btn_save_std = std_overrides.clone();
        let is_saving_btn = is_saving.clone();
        btn_save.connect_clicked(move |_| {
            if let Some(theme_id) = crate::dropdown_utils::dropdown_active_id(&btn_save_combo) {
                is_saving_btn.set(true);
                if let Ok(mut cfg) = launcher_core::load_config(&btn_save_config_path) {
                    if theme_id == "system" {
                        cfg.ui.system_theme_overrides = Some(btn_save_sys.borrow().clone());
                        if let Ok(content) = toml::to_string_pretty(&cfg) {
                            let _ = std::fs::write(&btn_save_config_path, content);
                        }
                    } else {
                        save_standard_theme(&theme_id, &btn_save_std.borrow());
                    }
                    // Trigger hot-reload over sync IPC
                    let socket_path = launcher_ipc::get_socket_path();
                    if socket_path.exists() {
                        let _ = launcher_ipc::send_message_sync(
                            &socket_path,
                            &launcher_ipc::IpcMessage::ReloadConfig,
                        );
                    }
                }
                let is_sav_timer = is_saving_btn.clone();
                gtk4::glib::timeout_add_local(std::time::Duration::from_millis(300), move || {
                    is_sav_timer.set(false);
                    gtk4::glib::ControlFlow::Break
                });
            }
        });

        let save_as_std = std_overrides.clone();
        let save_as_sys = sys_overrides.clone();
        let combo_save_as = combo_theme.clone();
        btn_save_as.connect_clicked(move |btn| {
            let win = btn.root().and_downcast::<gtk4::Window>().unwrap();

            // If saving from the system theme, resolve its GTK variables (with
            // their opacities) into a concrete standard palette up front.
            let is_system = crate::dropdown_utils::dropdown_active_id(&combo_save_as).as_deref() == Some("system");
            let resolved_std = if is_system {
                Some(system_to_standard(&save_as_sys.borrow(), &win.style_context()))
            } else {
                None
            };

            let dialog = gtk4::Dialog::with_buttons(
                Some("Save Theme As"),
                Some(&win),
                gtk4::DialogFlags::MODAL,
                &[
                    ("Cancel", gtk4::ResponseType::Cancel),
                    ("Save", gtk4::ResponseType::Accept),
                ],
            );

            let content_area = dialog.content_area();
            let entry = gtk4::Entry::new();
            entry.set_placeholder_text(Some("New Theme Name"));
            entry.set_margin_top(10);
            entry.set_margin_bottom(10);
            entry.set_margin_start(10);
            entry.set_margin_end(10);
            content_area.append(&entry);

            let save_as_std_c = save_as_std.clone();
            let combo_sa = combo_save_as.clone();
            let win_c = win.clone();
            dialog.connect_response(move |d, res| {
                if res == gtk4::ResponseType::Accept {
                    let mut name = entry.text().to_string();
                    if name.is_empty() {
                        name = "custom_theme".to_string();
                    }

                    let do_save = {
                        let combo = combo_sa.clone();
                        let n = name.clone();
                        let std_c = save_as_std_c.clone();
                        let resolved = resolved_std.clone();
                        move || {
                            if let Some(std_ov) = &resolved {
                                save_standard_theme(&n, std_ov);
                            } else {
                                save_standard_theme(&n, &std_c.borrow());
                            }
                            if !combo_has_id(&combo, &n) {
                                crate::dropdown_utils::dropdown_append(&combo, &n);
                            }
                            crate::dropdown_utils::dropdown_set_active_id(&combo, &n);
                        }
                    };

                    if theme_file_path(&name).exists() {
                        let dialog = gtk4::MessageDialog::builder()
                            .text("Theme already exists")
                            .secondary_text("Overwrite?")
                            .buttons(gtk4::ButtonsType::YesNo)
                            .modal(true)
                            .transient_for(&win_c)
                            .build();

                        dialog.connect_response(move |d2, response| {
                            if response == gtk4::ResponseType::Yes {
                                do_save();
                            }
                            d2.destroy();
                        });
                        dialog.present();
                    } else {
                        do_save();
                    }
                }
                d.close();
            });
            dialog.show();
        });

        Self {
            container,
            combo_theme,
            btn_save,
            btn_save_as,
            btn_reset,
            current_system_overrides: sys_overrides,
            current_standard_overrides: std_overrides,
            active_monitor,
            dir_monitor,
        }
    }

    fn populate_standard_tweaks(
        tweaks_box: &gtk4::Box,
        overrides: Rc<RefCell<StandardThemeOverrides>>,
    ) {
        // Clear box
        while let Some(child) = tweaks_box.first_child() {
            tweaks_box.remove(&child);
        }

        let grid = gtk4::Grid::new();
        grid.set_column_spacing(10);
        grid.set_row_spacing(10);
        grid.set_margin_start(5);
        grid.set_margin_end(5);
        grid.set_margin_top(5);
        grid.set_margin_bottom(5);

        // Headers
        grid.attach(&gtk4::Label::new(Some("Element")), 0, 0, 1, 1);
        grid.attach(&gtk4::Label::new(Some("Color (Hex)")), 1, 0, 1, 1);
        grid.attach(&gtk4::Label::new(Some("Opacity")), 2, 0, 1, 1);

        let items = vec![
            ("Entry Surface", overrides.borrow().entry_surface.clone()),
            (
                "Entry Surface Hover",
                overrides.borrow().entry_surface_hover.clone(),
            ),
            ("Entry Border", overrides.borrow().entry_border.clone()),
            (
                "Entry Border Hover",
                overrides.borrow().entry_border_hover.clone(),
            ),
            ("Label", overrides.borrow().label.clone()),
            ("Label Hover", overrides.borrow().label_hover.clone()),
            ("Entry Icon", overrides.borrow().entry_icon.clone()),
            (
                "Entry Icon Hover",
                overrides.borrow().entry_icon_hover.clone(),
            ),
            ("Hub Surface", overrides.borrow().hub_surface.clone()),
            ("Hub Border", overrides.borrow().hub_border.clone()),
            ("Hub Label", overrides.borrow().hub_label.clone()),
            ("Hub Icon", overrides.borrow().hub_icon.clone()),
            (
                "Pie Outer Border",
                overrides.borrow().pie_outer_border.clone(),
            ),
            (
                "Floating Icon Surface",
                overrides.borrow().floating_icon_surface.clone(),
            ),
            (
                "Floating Icon Surface Hover",
                overrides.borrow().floating_icon_surface_hover.clone(),
            ),
        ];

        for (i, (label, initial_color)) in items.into_iter().enumerate() {
            let row = (i + 1) as i32;
            let lbl = gtk4::Label::new(Some(label));
            grid.attach(&lbl, 0, row, 1, 1);

            let c = initial_color;

            let color_hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
            let hex_entry = gtk4::Entry::new();
            hex_entry.set_width_chars(9);
            // Format to hex (ignore alpha, which is handled by spin)
            let hex_str = format!(
                "#{:02x}{:02x}{:02x}",
                (c.red() * 255.0) as u8,
                (c.green() * 255.0) as u8,
                (c.blue() * 255.0) as u8
            );
            hex_entry.set_text(&hex_str);

            let preview = gtk4::DrawingArea::new();
            preview.set_size_request(24, 24);

            let current_rgba = Rc::new(RefCell::new(c));
            let preview_c = current_rgba.clone();
            preview.set_draw_func(move |widget, cr, width, height| {
                let col = preview_c.borrow();
                let w = width as f64;
                let h = height as f64;
                let radius = 4.0_f64.min(w / 2.0).min(h / 2.0);
                rounded_rect_path(cr, 0.0, 0.0, w, h, radius);
                cr.set_source_rgba(
                    col.red() as f64,
                    col.green() as f64,
                    col.blue() as f64,
                    col.alpha() as f64,
                );
                cr.fill_preserve();
                if let Some(accent) = lookup_style_color(&widget.style_context(), "accent_color") {
                    cr.set_source_rgba(
                        accent.red() as f64,
                        accent.green() as f64,
                        accent.blue() as f64,
                        accent.alpha() as f64,
                    );
                } else {
                    cr.set_source_rgb(0.0, 0.0, 0.0);
                }
                cr.set_line_width(1.5);
                let _ = cr.stroke();
            });

            color_hbox.append(&hex_entry);
            color_hbox.append(&preview);

            let op_spin = gtk4::SpinButton::with_range(0.0, 1.0, 0.05);
            op_spin.set_value(c.alpha() as f64);

            let scroll_ctrl2 = gtk4::EventControllerScroll::new(
                gtk4::EventControllerScrollFlags::VERTICAL
                    | gtk4::EventControllerScrollFlags::HORIZONTAL,
            );
            scroll_ctrl2.connect_scroll(|_, _, _| glib::Propagation::Stop);
            op_spin.add_controller(scroll_ctrl2);

            grid.attach(&color_hbox, 1, row, 1, 1);
            grid.attach(&op_spin, 2, row, 1, 1);

            let overrides_c = overrides.clone();
            let label_name = label.to_string();
            let update_fn = move |new_col: gdk::RGBA, new_op: f64| {
                let mut o = overrides_c.borrow_mut();
                let target = match label_name.as_str() {
                    "Entry Surface" => &mut o.entry_surface,
                    "Entry Surface Hover" => &mut o.entry_surface_hover,
                    "Entry Border" => &mut o.entry_border,
                    "Entry Border Hover" => &mut o.entry_border_hover,
                    "Label" => &mut o.label,
                    "Label Hover" => &mut o.label_hover,
                    "Entry Icon" => &mut o.entry_icon,
                    "Entry Icon Hover" => &mut o.entry_icon_hover,
                    "Floating Icon Surface" => &mut o.floating_icon_surface,
                    "Floating Icon Surface Hover" => &mut o.floating_icon_surface_hover,
                    "Hub Surface" => &mut o.hub_surface,
                    "Hub Border" => &mut o.hub_border,
                    "Hub Label" => &mut o.hub_label,
                    "Hub Icon" => &mut o.hub_icon,
                    "Pie Outer Border" => &mut o.pie_outer_border,
                    _ => unreachable!(),
                };
                let mut final_col = new_col;
                final_col.set_alpha(new_op as f32);
                *target = final_col;
            };

            let hex_entry_c = hex_entry.clone();
            let op_spin_c = op_spin.clone();
            let preview_redraw = preview.clone();
            let rgba_state = current_rgba.clone();
            let update_fn_rc = Rc::new(update_fn);

            let update_fn_c1 = update_fn_rc.clone();
            hex_entry.connect_changed(move |e| {
                if let Ok(mut parsed) = gdk::RGBA::from_str(&e.text().to_string()) {
                    parsed.set_alpha(op_spin_c.value() as f32);
                    *rgba_state.borrow_mut() = parsed;
                    preview_redraw.queue_draw();
                    update_fn_c1(parsed, op_spin_c.value());
                }
            });

            let update_fn_c2 = update_fn_rc.clone();
            let hex_entry_c2 = hex_entry_c.clone();
            let rgba_state_c2 = current_rgba.clone();
            let preview_c2 = preview.clone();
            op_spin.connect_value_changed(move |s| {
                let mut col =
                    if let Ok(parsed) = gdk::RGBA::from_str(&hex_entry_c2.text().to_string()) {
                        parsed
                    } else {
                        *rgba_state_c2.borrow()
                    };
                col.set_alpha(s.value() as f32);
                *rgba_state_c2.borrow_mut() = col;
                preview_c2.queue_draw();
                update_fn_c2(col, s.value());
            });

            // Clicking the swatch opens the native HSV dialog and syncs the row
            let color_dialog = gtk4::ColorDialog::new();
            color_dialog.set_title("Pick a color");
            color_dialog.set_with_alpha(true);
            let update_fn_c3 = update_fn_rc.clone();
            let hex_entry_c3 = hex_entry_c.clone();
            let preview_c3 = preview.clone();
            let rgba_state_c3 = current_rgba.clone();
            let op_spin_c3 = op_spin.clone();
            let swatch_gesture = gtk4::GestureClick::new();
            swatch_gesture.connect_released(move |_, _, _, _| {
                let win = preview_c3.root().and_downcast::<gtk4::Window>();
                let dlg = color_dialog.clone();
                let update_fn_c4 = update_fn_c3.clone();
                let hex_c4 = hex_entry_c3.clone();
                let preview_c4 = preview_c3.clone();
                let rgba_c4 = rgba_state_c3.clone();
                let op_c4 = op_spin_c3.clone();
                let initial = *rgba_state_c3.borrow();
                dlg.choose_rgba(
                    win.as_ref(),
                    Some(&initial),
                    None::<&gio::Cancellable>,
                    move |res| {
                        if let Ok(col) = res {
                            let mut final_col = col;
                            let alpha = final_col.alpha();
                            op_c4.set_value(alpha as f64);
                            *rgba_c4.borrow_mut() = final_col;
                            let hex = format!(
                                "#{:02x}{:02x}{:02x}",
                                (final_col.red() * 255.0) as u8,
                                (final_col.green() * 255.0) as u8,
                                (final_col.blue() * 255.0) as u8
                            );
                            hex_c4.set_text(&hex);
                            preview_c4.queue_draw();
                            update_fn_c4(final_col, alpha as f64);
                        }
                    },
                );
            });
            preview.add_controller(swatch_gesture);
        }

        tweaks_box.append(&grid);
    }

    fn populate_system_tweaks(
        tweaks_box: &gtk4::Box,
        overrides: Rc<RefCell<SystemThemeOverrides>>,
    ) {
        // Clear box
        while let Some(child) = tweaks_box.first_child() {
            tweaks_box.remove(&child);
        }

        let grid = gtk4::Grid::new();
        grid.set_column_spacing(10);
        grid.set_row_spacing(10);
        grid.set_margin_start(5);
        grid.set_margin_end(5);
        grid.set_margin_top(5);
        grid.set_margin_bottom(5);

        // Headers
        grid.attach(&gtk4::Label::new(Some("Element")), 0, 0, 1, 1);
        grid.attach(&gtk4::Label::new(Some("GTK Variable")), 1, 0, 1, 1);
        grid.attach(&gtk4::Label::new(Some("Opacity")), 2, 0, 1, 1);

        let items = vec![
            ("Entry Surface", overrides.borrow().entry_surface.clone()),
            (
                "Entry Surface Hover",
                overrides.borrow().entry_surface_hover.clone(),
            ),
            ("Entry Border", overrides.borrow().entry_border.clone()),
            (
                "Entry Border Hover",
                overrides.borrow().entry_border_hover.clone(),
            ),
            ("Label", overrides.borrow().label.clone()),
            ("Label Hover", overrides.borrow().label_hover.clone()),
            ("Entry Icon", overrides.borrow().entry_icon.clone()),
            (
                "Entry Icon Hover",
                overrides.borrow().entry_icon_hover.clone(),
            ),
            ("Hub Surface", overrides.borrow().hub_surface.clone()),
            ("Hub Border", overrides.borrow().hub_border.clone()),
            ("Hub Label", overrides.borrow().hub_label.clone()),
            ("Hub Icon", overrides.borrow().hub_icon.clone()),
            (
                "Pie Outer Border",
                overrides.borrow().pie_outer_border.clone(),
            ),
            (
                "Floating Icon Surface",
                overrides.borrow().floating_icon_surface.clone(),
            ),
            (
                "Floating Icon Surface Hover",
                overrides.borrow().floating_icon_surface_hover.clone(),
            ),
        ];

        let gtk_vars = [
            "@theme_bg_color",
            "@theme_fg_color",
            "@theme_base_color",
            "@theme_text_color",
            "@theme_selected_bg_color",
            "@theme_selected_fg_color",
            "@theme_unfocused_bg_color",
            "@theme_unfocused_fg_color",
            "@warning_color",
            "@error_color",
            "@success_color",
        ];

        for (i, (label, initial_color)) in items.into_iter().enumerate() {
            let row = (i + 1) as i32;
            let lbl = gtk4::Label::new(Some(label));
            grid.attach(&lbl, 0, row, 1, 1);

            let var_combo = crate::dropdown_utils::create_dropdown();
            for var in &gtk_vars {
                crate::dropdown_utils::dropdown_append(&var_combo, var);
            }
            if gtk_vars.contains(&initial_color.variable.as_str()) {
                crate::dropdown_utils::dropdown_set_active_id(&var_combo, &initial_color.variable);
            } else {
                crate::dropdown_utils::dropdown_append(&var_combo, &initial_color.variable);
                crate::dropdown_utils::dropdown_set_active_id(&var_combo, &initial_color.variable);
            }

            let op_spin = gtk4::SpinButton::with_range(0.0, 1.0, 0.05);
            op_spin.set_value(initial_color.opacity);

            let scroll_ctrl1 = gtk4::EventControllerScroll::new(
                gtk4::EventControllerScrollFlags::VERTICAL
                    | gtk4::EventControllerScrollFlags::HORIZONTAL,
            );
            scroll_ctrl1.connect_scroll(|_, _, _| glib::Propagation::Stop);
            var_combo.add_controller(scroll_ctrl1);

            let scroll_ctrl2 = gtk4::EventControllerScroll::new(
                gtk4::EventControllerScrollFlags::VERTICAL
                    | gtk4::EventControllerScrollFlags::HORIZONTAL,
            );
            scroll_ctrl2.connect_scroll(|_, _, _| glib::Propagation::Stop);
            op_spin.add_controller(scroll_ctrl2);

            grid.attach(&var_combo, 1, row, 1, 1);
            grid.attach(&op_spin, 2, row, 1, 1);

            let overrides_c = overrides.clone();

            let label_name = label.to_string();
            let update_fn = move |new_var: String, new_op: f64| {
                let mut o = overrides_c.borrow_mut();
                let target = match label_name.as_str() {
                    "Entry Surface" => &mut o.entry_surface,
                    "Entry Surface Hover" => &mut o.entry_surface_hover,
                    "Entry Border" => &mut o.entry_border,
                    "Entry Border Hover" => &mut o.entry_border_hover,
                    "Label" => &mut o.label,
                    "Label Hover" => &mut o.label_hover,
                    "Entry Icon" => &mut o.entry_icon,
                    "Entry Icon Hover" => &mut o.entry_icon_hover,
                    "Floating Icon Surface" => &mut o.floating_icon_surface,
                    "Floating Icon Surface Hover" => &mut o.floating_icon_surface_hover,
                    "Hub Surface" => &mut o.hub_surface,
                    "Hub Border" => &mut o.hub_border,
                    "Hub Label" => &mut o.hub_label,
                    "Hub Icon" => &mut o.hub_icon,
                    "Pie Outer Border" => &mut o.pie_outer_border,
                    _ => unreachable!(),
                };
                target.variable = new_var;
                target.opacity = new_op;
            };

            let var_combo_c = var_combo.clone();
            let op_spin_c = op_spin.clone();
            let update_fn_rc = Rc::new(update_fn);

            let update_fn_c1 = update_fn_rc.clone();
            var_combo.connect_notify_local(Some("selected"), move |c, _| {
                if let Some(id) = crate::dropdown_utils::dropdown_active_id(c) {
                    update_fn_c1(id.to_string(), op_spin_c.value());
                }
            });

            let update_fn_c2 = update_fn_rc.clone();
            let var_combo_c2 = var_combo_c.clone();
            op_spin.connect_value_changed(move |s| {
                if let Some(id) = crate::dropdown_utils::dropdown_active_id(&var_combo_c2) {
                    update_fn_c2(id.to_string(), s.value());
                }
            });
        }

        tweaks_box.append(&grid);
    }
}
