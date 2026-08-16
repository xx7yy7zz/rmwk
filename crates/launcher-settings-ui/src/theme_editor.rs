use crate::standard_theme::{load_standard_theme, save_standard_theme, StandardThemeOverrides};
use gtk4::gdk;
use gtk4::gio;
use gtk4::prelude::*;
use launcher_core::SystemThemeOverrides;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr;

#[allow(dead_code)]
pub struct ThemeEditor {
    pub container: gtk4::Box,
    pub combo_theme: gtk4::ComboBoxText,
    pub btn_save: gtk4::Button,
    pub btn_save_as: gtk4::Button,
    pub btn_reset: gtk4::Button,
    pub current_system_overrides: Rc<RefCell<SystemThemeOverrides>>,
    pub current_standard_overrides: Rc<RefCell<StandardThemeOverrides>>,
    pub active_monitor: Rc<RefCell<Option<gio::FileMonitor>>>,
}

impl ThemeEditor {
    pub fn new(
        config_path: PathBuf,
        initial_theme: &str,
        initial_sys: Option<SystemThemeOverrides>,
        available_themes: &[String],
    ) -> Self {
        let container = gtk4::Box::new(gtk4::Orientation::Vertical, 10);

        let header_hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
        let lbl_theme = gtk4::Label::new(Some("Theme:"));
        let combo_theme = gtk4::ComboBoxText::new();
        for t in available_themes {
            combo_theme.append(Some(t), t);
        }
        combo_theme.set_active_id(Some(initial_theme));

        let scroll_ctrl_theme = gtk4::EventControllerScroll::new(
            gtk4::EventControllerScrollFlags::VERTICAL
                | gtk4::EventControllerScrollFlags::HORIZONTAL,
        );
        scroll_ctrl_theme.connect_scroll(|_, _, _| glib::Propagation::Stop);
        combo_theme.add_controller(scroll_ctrl_theme);

        header_hbox.append(&lbl_theme);
        header_hbox.append(&combo_theme);
        container.append(&header_hbox);

        let tweaks_scrolled = gtk4::ScrolledWindow::new();
        tweaks_scrolled.set_min_content_height(200);
        tweaks_scrolled.set_vexpand(true);
        let tweaks_box = gtk4::Box::new(gtk4::Orientation::Vertical, 5);
        tweaks_scrolled.set_child(Some(&tweaks_box));
        container.append(&tweaks_scrolled);

        let btn_save = gtk4::Button::with_label("Save Theme");
        let btn_save_as = gtk4::Button::with_label("Save As...");
        let btn_reset = gtk4::Button::with_label("Reset to Defaults");
        let btn_hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 5);
        btn_hbox.append(&btn_save);
        btn_hbox.append(&btn_save_as);
        btn_hbox.append(&btn_reset);
        container.append(&btn_hbox);

        let sys_overrides = Rc::new(RefCell::new(initial_sys.unwrap_or_default()));
        let std_overrides = Rc::new(RefCell::new(load_standard_theme(initial_theme)));
        let is_saving = Rc::new(std::cell::Cell::new(false));

        let tweaks_box_c = tweaks_box.clone();
        let sys_overrides_c = sys_overrides.clone();
        let std_overrides_c = std_overrides.clone();
        let btn_save_as_c = btn_save_as.clone();
        let btn_reset_c = btn_reset.clone();
        let active_monitor = Rc::new(RefCell::new(None::<gio::FileMonitor>));
        let active_monitor_c = active_monitor.clone();
        let is_saving_mon = is_saving.clone();

        combo_theme.connect_changed(move |combo| {
            if let Some(m) = active_monitor_c.borrow_mut().take() {
                m.cancel();
            }
            if let Some(id) = combo.active_id() {
                if id == "system" {
                    btn_save_as_c.set_sensitive(false);
                    btn_reset_c.set_sensitive(true);
                    Self::populate_system_tweaks(&tweaks_box_c, sys_overrides_c.clone());
                } else {
                    btn_save_as_c.set_sensitive(true);
                    btn_reset_c.set_sensitive(false);
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
                                gtk4::glib::timeout_add_local(std::time::Duration::from_millis(150), move || {
                                    pending.set(false);
                                    if is_sav.get() {
                                        return gtk4::glib::ControlFlow::Break;
                                    }
                                    *std_ov.borrow_mut() = load_standard_theme(&theme_id);
                                    Self::populate_standard_tweaks(
                                        &twk_box,
                                        std_ov.clone(),
                                    );
                                    gtk4::glib::ControlFlow::Break
                                });
                            }
                        });
                        *active_monitor_c.borrow_mut() = Some(monitor);
                    }
                }
            }
        });

        let tweaks_box_reset = tweaks_box.clone();
        let sys_overrides_reset = sys_overrides.clone();
        btn_reset.connect_clicked(move |_| {
            *sys_overrides_reset.borrow_mut() = SystemThemeOverrides::default();
            Self::populate_system_tweaks(&tweaks_box_reset, sys_overrides_reset.clone());
        });

        // Trigger initial population
        combo_theme.emit_by_name::<()>("changed", &[]);

        let btn_save_config_path = config_path.clone();
        let btn_save_combo = combo_theme.clone();
        let btn_save_sys = sys_overrides.clone();
        let btn_save_std = std_overrides.clone();
        let is_saving_btn = is_saving.clone();
        btn_save.connect_clicked(move |_| {
            if let Some(theme_id) = btn_save_combo.active_id() {
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
        let save_as_config_path = config_path.clone();
        btn_save_as.connect_clicked(move |btn| {
            let win = btn.root().and_downcast::<gtk4::Window>().unwrap();

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
            let cp_c = save_as_config_path.clone();
            dialog.connect_response(move |d, res| {
                if res == gtk4::ResponseType::Accept {
                    let mut name = entry.text().to_string();
                    if name.is_empty() {
                        name = "custom_theme".to_string();
                    }

                    save_standard_theme(&name, &save_as_std_c.borrow());

                    if let Ok(mut cfg) = launcher_core::load_config(&cp_c) {
                        cfg.ui.theme = name.clone();
                        if let Ok(content) = toml::to_string_pretty(&cfg) {
                            let _ = std::fs::write(&cp_c, content);
                        }
                    }

                    let socket_path = launcher_ipc::get_socket_path();
                    if socket_path.exists() {
                        if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                        {
                            let _ = rt.block_on(async {
                                let _ = launcher_ipc::send_message(
                                    &socket_path,
                                    &launcher_ipc::IpcMessage::ReloadConfig,
                                )
                                .await;
                            });
                        }
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
            ("Slice Normal", overrides.borrow().slice_normal.clone()),
            ("Slice Hover", overrides.borrow().slice_hover.clone()),
            ("Slice Active", overrides.borrow().slice_active.clone()),
            ("Slice Selected", overrides.borrow().slice_selected.clone()),
            ("Label Normal", overrides.borrow().label_normal.clone()),
            ("Label Hover", overrides.borrow().label_hover.clone()),
            ("Hub Normal", overrides.borrow().hub_normal.clone()),
            ("Hub Active", overrides.borrow().hub_active.clone()),
            ("Hub Hover", overrides.borrow().hub_hover.clone()),
            ("Outer Border", overrides.borrow().outer_border.clone()),
        ];

        for (i, (label, initial_color)) in items.into_iter().enumerate() {
            let row = (i + 1) as i32;
            let lbl = gtk4::Label::new(Some(label));
            grid.attach(&lbl, 0, row, 1, 1);

            let c = initial_color;

            let color_hbox = gtk4::Box::new(gtk4::Orientation::Horizontal, 5);
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
            preview.set_draw_func(move |_, cr, width, height| {
                let col = preview_c.borrow();
                cr.set_source_rgb(col.red() as f64, col.green() as f64, col.blue() as f64);
                cr.rectangle(0.0, 0.0, width as f64, height as f64);
                let _ = cr.fill();
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
                    "Slice Normal" => &mut o.slice_normal,
                    "Slice Hover" => &mut o.slice_hover,
                    "Slice Active" => &mut o.slice_active,
                    "Slice Selected" => &mut o.slice_selected,
                    "Label Normal" => &mut o.label_normal,
                    "Label Hover" => &mut o.label_hover,
                    "Hub Normal" => &mut o.hub_normal,
                    "Hub Active" => &mut o.hub_active,
                    "Hub Hover" => &mut o.hub_hover,
                    "Outer Border" => &mut o.outer_border,
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
            op_spin.connect_value_changed(move |s| {
                if let Ok(parsed) = gdk::RGBA::from_str(&hex_entry_c2.text().to_string()) {
                    update_fn_c2(parsed, s.value());
                } else {
                    let col = *rgba_state_c2.borrow();
                    update_fn_c2(col, s.value());
                }
            });
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
            ("Slice Normal", overrides.borrow().slice_normal.clone()),
            ("Slice Hover", overrides.borrow().slice_hover.clone()),
            ("Slice Active", overrides.borrow().slice_active.clone()),
            ("Slice Selected", overrides.borrow().slice_selected.clone()),
            ("Label Normal", overrides.borrow().label_normal.clone()),
            ("Label Hover", overrides.borrow().label_hover.clone()),
            ("Hub Normal", overrides.borrow().hub_normal.clone()),
            ("Hub Active", overrides.borrow().hub_active.clone()),
            ("Hub Hover", overrides.borrow().hub_hover.clone()),
            ("Outer Border", overrides.borrow().outer_border.clone()),
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

            let var_combo = gtk4::ComboBoxText::new();
            for var in &gtk_vars {
                var_combo.append(Some(var), var);
            }
            if gtk_vars.contains(&initial_color.variable.as_str()) {
                var_combo.set_active_id(Some(&initial_color.variable));
            } else {
                var_combo.append(Some(&initial_color.variable), &initial_color.variable);
                var_combo.set_active_id(Some(&initial_color.variable));
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
                    "Slice Normal" => &mut o.slice_normal,
                    "Slice Hover" => &mut o.slice_hover,
                    "Slice Active" => &mut o.slice_active,
                    "Slice Selected" => &mut o.slice_selected,
                    "Label Normal" => &mut o.label_normal,
                    "Label Hover" => &mut o.label_hover,
                    "Hub Normal" => &mut o.hub_normal,
                    "Hub Active" => &mut o.hub_active,
                    "Hub Hover" => &mut o.hub_hover,
                    "Outer Border" => &mut o.outer_border,
                    _ => unreachable!(),
                };
                target.variable = new_var;
                target.opacity = new_op;
            };

            let var_combo_c = var_combo.clone();
            let op_spin_c = op_spin.clone();
            let update_fn_rc = Rc::new(update_fn);

            let update_fn_c1 = update_fn_rc.clone();
            var_combo.connect_changed(move |c| {
                if let Some(id) = c.active_id() {
                    update_fn_c1(id.to_string(), op_spin_c.value());
                }
            });

            let update_fn_c2 = update_fn_rc.clone();
            let var_combo_c2 = var_combo_c.clone();
            op_spin.connect_value_changed(move |s| {
                if let Some(id) = var_combo_c2.active_id() {
                    update_fn_c2(id.to_string(), s.value());
                }
            });
        }

        tweaks_box.append(&grid);
    }
}
