use gtk4::gdk;
use std::str::FromStr;

#[derive(Clone, Debug)]
pub struct StandardThemeOverrides {
    pub slice_normal: gdk::RGBA,
    pub slice_hover: gdk::RGBA,
    pub slice_active: gdk::RGBA,
    pub slice_selected: gdk::RGBA,
    pub label_normal: gdk::RGBA,
    pub label_hover: gdk::RGBA,
    pub hub_normal: gdk::RGBA,
    pub hub_active: gdk::RGBA,
    pub hub_hover: gdk::RGBA,
    pub outer_border: gdk::RGBA,
}

impl Default for StandardThemeOverrides {
    fn default() -> Self {
        Self {
            slice_normal: gdk::RGBA::new(0.0, 0.0, 0.0, 0.85),
            slice_hover: gdk::RGBA::new(0.3, 0.3, 0.3, 0.70),
            slice_active: gdk::RGBA::new(0.5, 0.5, 0.5, 0.15),
            slice_selected: gdk::RGBA::new(0.2, 0.5, 0.8, 1.0),
            label_normal: gdk::RGBA::new(1.0, 1.0, 1.0, 1.0),
            label_hover: gdk::RGBA::new(0.8, 0.8, 0.8, 0.80),
            hub_normal: gdk::RGBA::new(0.1, 0.1, 0.1, 0.85),
            hub_active: gdk::RGBA::new(0.2, 0.5, 0.8, 1.0),
            hub_hover: gdk::RGBA::new(1.0, 1.0, 1.0, 1.0),
            outer_border: gdk::RGBA::new(0.64, 0.64, 1.0, 1.0),
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
        let file_name = if theme_name.ends_with(".css") { theme_name.to_string() } else { format!("{}.css", theme_name) };
        theme_path.push(file_name);
        
        if let Ok(css) = std::fs::read_to_string(&theme_path) {
            if let Some(c) = extract_css_color(&css, ".radial-slice") { overrides.slice_normal = c; }
            if let Some(c) = extract_css_color(&css, ".radial-slice:hover") { overrides.slice_hover = c; }
            if let Some(c) = extract_css_color(&css, ".radial-slice:active") { overrides.slice_active = c; }
            if let Some(c) = extract_css_color(&css, ".radial-slice:selected") { overrides.slice_selected = c; }
            if let Some(c) = extract_css_color(&css, ".radial-label") { overrides.label_normal = c; }
            if let Some(c) = extract_css_color(&css, ".radial-label:hover") { overrides.label_hover = c; }
            if let Some(c) = extract_css_color(&css, ".radial-hub") { overrides.hub_normal = c; }
            if let Some(c) = extract_css_color(&css, ".radial-hub:active") { overrides.hub_active = c; }
            if let Some(c) = extract_css_color(&css, ".radial-hub:hover") { overrides.hub_hover = c; }
            if let Some(c) = extract_css_color(&css, ".radial-outer") { overrides.outer_border = c; }
        }
    }
    overrides
}

pub fn save_standard_theme(theme_name: &str, overrides: &StandardThemeOverrides) {
    if let Some(mut theme_path) = dirs::config_dir() {
        theme_path.push("rmwk");
        theme_path.push("themes");
        std::fs::create_dir_all(&theme_path).unwrap_or_default();
        let file_name = if theme_name.ends_with(".css") { theme_name.to_string() } else { format!("{}.css", theme_name) };
        theme_path.push(file_name);
        
        let css = format!("
.radial-slice {{ color: rgba({}, {}, {}, {:.3}); }}
.radial-slice:hover {{ color: rgba({}, {}, {}, {:.3}); }}
.radial-slice:active {{ color: rgba({}, {}, {}, {:.3}); }}
.radial-slice:selected {{ color: rgba({}, {}, {}, {:.3}); }}
.radial-label {{ color: rgba({}, {}, {}, {:.3}); }}
.radial-label:hover {{ color: rgba({}, {}, {}, {:.3}); }}
.radial-hub {{ color: rgba({}, {}, {}, {:.3}); }}
.radial-hub:active {{ color: rgba({}, {}, {}, {:.3}); }}
.radial-hub:hover {{ color: rgba({}, {}, {}, {:.3}); }}
.radial-outer {{ color: rgba({}, {}, {}, {:.3}); }}
",
            (overrides.slice_normal.red() * 255.0) as u8, (overrides.slice_normal.green() * 255.0) as u8, (overrides.slice_normal.blue() * 255.0) as u8, overrides.slice_normal.alpha(),
            (overrides.slice_hover.red() * 255.0) as u8, (overrides.slice_hover.green() * 255.0) as u8, (overrides.slice_hover.blue() * 255.0) as u8, overrides.slice_hover.alpha(),
            (overrides.slice_active.red() * 255.0) as u8, (overrides.slice_active.green() * 255.0) as u8, (overrides.slice_active.blue() * 255.0) as u8, overrides.slice_active.alpha(),
            (overrides.slice_selected.red() * 255.0) as u8, (overrides.slice_selected.green() * 255.0) as u8, (overrides.slice_selected.blue() * 255.0) as u8, overrides.slice_selected.alpha(),
            (overrides.label_normal.red() * 255.0) as u8, (overrides.label_normal.green() * 255.0) as u8, (overrides.label_normal.blue() * 255.0) as u8, overrides.label_normal.alpha(),
            (overrides.label_hover.red() * 255.0) as u8, (overrides.label_hover.green() * 255.0) as u8, (overrides.label_hover.blue() * 255.0) as u8, overrides.label_hover.alpha(),
            (overrides.hub_normal.red() * 255.0) as u8, (overrides.hub_normal.green() * 255.0) as u8, (overrides.hub_normal.blue() * 255.0) as u8, overrides.hub_normal.alpha(),
            (overrides.hub_active.red() * 255.0) as u8, (overrides.hub_active.green() * 255.0) as u8, (overrides.hub_active.blue() * 255.0) as u8, overrides.hub_active.alpha(),
            (overrides.hub_hover.red() * 255.0) as u8, (overrides.hub_hover.green() * 255.0) as u8, (overrides.hub_hover.blue() * 255.0) as u8, overrides.hub_hover.alpha(),
            (overrides.outer_border.red() * 255.0) as u8, (overrides.outer_border.green() * 255.0) as u8, (overrides.outer_border.blue() * 255.0) as u8, overrides.outer_border.alpha(),
        );
        let _ = std::fs::write(&theme_path, css);
    }
}
