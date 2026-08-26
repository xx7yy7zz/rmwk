use gtk4::gdk;
use std::str::FromStr;

#[derive(Clone, Debug)]
pub struct StandardThemeOverrides {
    pub entry_surface: gdk::RGBA,
    pub entry_surface_hover: gdk::RGBA,
    pub entry_border: gdk::RGBA,
    pub entry_border_hover: gdk::RGBA,
    pub label: gdk::RGBA,
    pub label_hover: gdk::RGBA,
    pub entry_icon: gdk::RGBA,
    pub entry_icon_hover: gdk::RGBA,
    pub floating_icon_surface: gdk::RGBA,
    pub floating_icon_surface_hover: gdk::RGBA,
    pub hub_surface: gdk::RGBA,
    pub hub_border: gdk::RGBA,
    pub hub_label: gdk::RGBA,
    pub hub_icon: gdk::RGBA,
    pub pie_outer_border: gdk::RGBA,
}

impl Default for StandardThemeOverrides {
    fn default() -> Self {
        // Mirrors the current default.css palette
        let blue = gdk::RGBA::new(137.0 / 255.0, 180.0 / 255.0, 250.0 / 255.0, 1.0);
        let text = gdk::RGBA::new(205.0 / 255.0, 214.0 / 255.0, 244.0 / 255.0, 1.0);
        Self {
            entry_surface: gdk::RGBA::new(17.0 / 255.0, 17.0 / 255.0, 27.0 / 255.0, 0.850),
            entry_surface_hover: gdk::RGBA::new(30.0 / 255.0, 30.0 / 255.0, 46.0 / 255.0, 1.0),
            entry_border: blue.with_alpha(0.400),
            entry_border_hover: blue.with_alpha(0.950),
            label: text,
            label_hover: blue,
            entry_icon: text,
            entry_icon_hover: blue,
            floating_icon_surface: gdk::RGBA::new(30.0 / 255.0, 30.0 / 255.0, 46.0 / 255.0, 1.0),
            floating_icon_surface_hover: gdk::RGBA::new(17.0 / 255.0, 17.0 / 255.0, 27.0 / 255.0, 1.0),
            hub_surface: gdk::RGBA::new(30.0 / 255.0, 30.0 / 255.0, 46.0 / 255.0, 1.0),
            hub_border: blue,
            hub_label: gdk::RGBA::new(1.0, 1.0, 1.0, 1.0),
            hub_icon: text,
            pie_outer_border: blue.with_alpha(0.600),
        }
    }
}

pub fn extract_css_color(css: &str, exact_selector: &str) -> Option<gdk::RGBA> {
    // Split by blocks
    for block in css.split('}') {
        if let Some(brace_idx) = block.find('{') {
            let selectors = &block[..brace_idx];
            let rules = &block[brace_idx + 1..];

            // Check if exact_selector is one of the selectors (comma separated)
            if selectors.split(',').any(|s| s.trim() == exact_selector) {
                // Parse declarations split by ';' to strictly match property 'color'
                for decl in rules.split(';') {
                    let decl = decl.trim();
                    if let Some(colon_idx) = decl.find(':') {
                        let prop = decl[..colon_idx].trim();
                        let val = decl[colon_idx + 1..].trim();
                        if prop == "color" {
                            if let Ok(c) = gdk::RGBA::from_str(val) {
                                return Some(c);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

pub fn load_standard_theme(theme_name: &str) -> StandardThemeOverrides {
    let mut overrides = StandardThemeOverrides::default();
    if let Some(mut theme_path) = dirs::config_dir() {
        theme_path.push("rmwk");
        theme_path.push("themes");
        let file_name = if theme_name.ends_with(".css") {
            theme_name.to_string()
        } else {
            format!("{}.css", theme_name)
        };
        theme_path.push(file_name);

        if let Ok(css) = std::fs::read_to_string(&theme_path) {
            if let Some(c) = extract_css_color(&css, ".entry-surface") {
                overrides.entry_surface = c;
            }
            if let Some(c) = extract_css_color(&css, ".entry-surface:hover") {
                overrides.entry_surface_hover = c;
            }
            if let Some(c) = extract_css_color(&css, ".entry-border") {
                overrides.entry_border = c;
            }
            if let Some(c) = extract_css_color(&css, ".entry-border:hover") {
                overrides.entry_border_hover = c;
            }
            if let Some(c) = extract_css_color(&css, ".label") {
                overrides.label = c;
            }
            if let Some(c) = extract_css_color(&css, ".label:hover") {
                overrides.label_hover = c;
            }
            if let Some(c) = extract_css_color(&css, ".entry-icon") {
                overrides.entry_icon = c;
            }
            if let Some(c) = extract_css_color(&css, ".entry-icon:hover") {
                overrides.entry_icon_hover = c;
            }
            if let Some(c) = extract_css_color(&css, ".floating-icon-surface") {
                overrides.floating_icon_surface = c;
            }
            if let Some(c) = extract_css_color(&css, ".floating-icon-surface:hover") {
                overrides.floating_icon_surface_hover = c;
            }
            if let Some(c) = extract_css_color(&css, ".hub-surface") {
                overrides.hub_surface = c;
            }
            if let Some(c) = extract_css_color(&css, ".hub-border") {
                overrides.hub_border = c;
            }
            if let Some(c) = extract_css_color(&css, ".hub-label") {
                overrides.hub_label = c;
            }
            if let Some(c) = extract_css_color(&css, ".hub-icon") {
                overrides.hub_icon = c;
            }
            if let Some(c) = extract_css_color(&css, ".pie-outer-border") {
                overrides.pie_outer_border = c;
            }
        }
    }
    overrides
}

pub fn save_standard_theme(theme_name: &str, overrides: &StandardThemeOverrides) {
    if let Some(mut theme_path) = dirs::config_dir() {
        theme_path.push("rmwk");
        theme_path.push("themes");
        std::fs::create_dir_all(&theme_path).unwrap_or_default();
        let file_name = if theme_name.ends_with(".css") {
            theme_name.to_string()
        } else {
            format!("{}.css", theme_name)
        };
        theme_path.push(file_name);

        let css = format!(
            "
.entry-surface {{ color: rgba({}, {}, {}, {:.3}); }}
.entry-surface:hover {{ color: rgba({}, {}, {}, {:.3}); }}
.entry-border {{ color: rgba({}, {}, {}, {:.3}); }}
.entry-border:hover {{ color: rgba({}, {}, {}, {:.3}); }}
.label {{ color: rgba({}, {}, {}, {:.3}); }}
.label:hover {{ color: rgba({}, {}, {}, {:.3}); }}
.entry-icon {{ color: rgba({}, {}, {}, {:.3}); }}
.entry-icon:hover {{ color: rgba({}, {}, {}, {:.3}); }}
.hub-surface {{ color: rgba({}, {}, {}, {:.3}); }}
.hub-border {{ color: rgba({}, {}, {}, {:.3}); }}
.hub-label {{ color: rgba({}, {}, {}, {:.3}); }}
.hub-icon {{ color: rgba({}, {}, {}, {:.3}); }}
.pie-outer-border {{ color: rgba({}, {}, {}, {:.3}); }}
.floating-icon-surface {{ color: rgba({}, {}, {}, {:.3}); }}
.floating-icon-surface:hover {{ color: rgba({}, {}, {}, {:.3}); }}
",
            (overrides.entry_surface.red() * 255.0) as u8,
            (overrides.entry_surface.green() * 255.0) as u8,
            (overrides.entry_surface.blue() * 255.0) as u8,
            overrides.entry_surface.alpha(),
            (overrides.entry_surface_hover.red() * 255.0) as u8,
            (overrides.entry_surface_hover.green() * 255.0) as u8,
            (overrides.entry_surface_hover.blue() * 255.0) as u8,
            overrides.entry_surface_hover.alpha(),
            (overrides.entry_border.red() * 255.0) as u8,
            (overrides.entry_border.green() * 255.0) as u8,
            (overrides.entry_border.blue() * 255.0) as u8,
            overrides.entry_border.alpha(),
            (overrides.entry_border_hover.red() * 255.0) as u8,
            (overrides.entry_border_hover.green() * 255.0) as u8,
            (overrides.entry_border_hover.blue() * 255.0) as u8,
            overrides.entry_border_hover.alpha(),
            (overrides.label.red() * 255.0) as u8,
            (overrides.label.green() * 255.0) as u8,
            (overrides.label.blue() * 255.0) as u8,
            overrides.label.alpha(),
            (overrides.label_hover.red() * 255.0) as u8,
            (overrides.label_hover.green() * 255.0) as u8,
            (overrides.label_hover.blue() * 255.0) as u8,
            overrides.label_hover.alpha(),
            (overrides.entry_icon.red() * 255.0) as u8,
            (overrides.entry_icon.green() * 255.0) as u8,
            (overrides.entry_icon.blue() * 255.0) as u8,
            overrides.entry_icon.alpha(),
            (overrides.entry_icon_hover.red() * 255.0) as u8,
            (overrides.entry_icon_hover.green() * 255.0) as u8,
            (overrides.entry_icon_hover.blue() * 255.0) as u8,
            overrides.entry_icon_hover.alpha(),
            (overrides.hub_surface.red() * 255.0) as u8,
            (overrides.hub_surface.green() * 255.0) as u8,
            (overrides.hub_surface.blue() * 255.0) as u8,
            overrides.hub_surface.alpha(),
            (overrides.hub_border.red() * 255.0) as u8,
            (overrides.hub_border.green() * 255.0) as u8,
            (overrides.hub_border.blue() * 255.0) as u8,
            overrides.hub_border.alpha(),
            (overrides.hub_label.red() * 255.0) as u8,
            (overrides.hub_label.green() * 255.0) as u8,
            (overrides.hub_label.blue() * 255.0) as u8,
            overrides.hub_label.alpha(),
            (overrides.hub_icon.red() * 255.0) as u8,
            (overrides.hub_icon.green() * 255.0) as u8,
            (overrides.hub_icon.blue() * 255.0) as u8,
            overrides.hub_icon.alpha(),
            (overrides.pie_outer_border.red() * 255.0) as u8,
            (overrides.pie_outer_border.green() * 255.0) as u8,
            (overrides.pie_outer_border.blue() * 255.0) as u8,
            overrides.pie_outer_border.alpha(),
            (overrides.floating_icon_surface.red() * 255.0) as u8,
            (overrides.floating_icon_surface.green() * 255.0) as u8,
            (overrides.floating_icon_surface.blue() * 255.0) as u8,
            overrides.floating_icon_surface.alpha(),
            (overrides.floating_icon_surface_hover.red() * 255.0) as u8,
            (overrides.floating_icon_surface_hover.green() * 255.0) as u8,
            (overrides.floating_icon_surface_hover.blue() * 255.0) as u8,
            overrides.floating_icon_surface_hover.alpha(),
        );
        let _ = std::fs::write(&theme_path, css);
    }
}
