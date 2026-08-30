use gdk::prelude::*;
use gdk4 as gdk;
use gtk::prelude::*;
use gtk4 as gtk;
pub mod tray;
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

/// Circumference each pie entry is guaranteed when auto-growing the gap
/// between hub and entry ring (mirrors floating mode's 82px per pill,
/// gentler since pie slices carry icons only, no labels).
const PIE_ARC_PER_ENTRY: f64 = 70.0;

/// Circumference guaranteed per floating-icons entry when auto-growing the
/// ring radius, interpolated by pill_roundness: square tiles (roundness 0)
/// occupy more space than circles (roundness 1). Tweak to taste.
const FLOATING_ICONS_ARC_SQUARE: f64 = 110.0;
const FLOATING_ICONS_ARC_ROUND: f64 = 80.0;

/// Base gap (hub edge -> entry center) for floating-icons entries, used
/// when the auto-grown radius is smaller than this floor, interpolated by
/// pill_roundness: square tiles need more clearance than circles.
/// Tweak to taste.
const FLOATING_ICONS_BASE_GAP_SQUARE: f64 = 110.0;
const FLOATING_ICONS_BASE_GAP_ROUND: f64 = 80.0;

/// Padding around the icon inside a floating-icons tile (on top of
/// icon_size / 2). Higher = chunkier tiles. Tweak to taste.
const FLOATING_ICONS_TILE_PADDING: f64 = 14.0;

/// Fixed icon size inside floating mode label pills (px).
const FLOATING_PILL_ICON_SIZE: f64 = 40.0;

/// Circumference guaranteed per floating pill entry when auto-growing the
/// ring radius, interpolated by pill_roundness: square-ish pills (0) need
/// more room than capsules (1). Tweak to taste.
const FLOATING_ARC_SQUARE: f64 = 90.0;
const FLOATING_ARC_ROUND: f64 = 50.0;

/// Base gap (hub edge -> entry center) for floating pills, used when the
/// auto-grown radius is smaller than this floor, interpolated by
/// pill_roundness. Tweak to taste.
const FLOATING_BASE_GAP_SQUARE: f64 = 90.0;
const FLOATING_BASE_GAP_ROUND: f64 = 50.0;

const MATERIAL_ICON_WEIGHT: f64 = 400.0;

/// Minimum gap kept between the clamped menu edge and the monitor border
/// when spawning the menu at the cursor position.
const SCREEN_MARGIN: f64 = 8.0;

/// Sentinel blur-cache key while the menu is held transparent waiting for
/// the pointer position. The -1.0 spacing can never occur in a real key.
const BLUR_PENDING_KEY: (bool, bool, f64, u64) = (false, false, -1.0, u64::MAX);

/// Marking mode (hold & drag): distance (px, at menu scale) the pointer
/// must travel from the press point before a click turns into a marking
/// session, and the hold time that triggers one even without movement.
const MARKING_TRIGGER_DIST: f64 = 16.0;
const MARKING_TRIGGER_MS: u64 = 250;

/// Fallback dwell (ms) when the setting can't be read; the effective
/// value comes from `ui.marking_dwell_ms` (Marking Speed slider).
const MARKING_DWELL_MS: u32 = 180;

/// Non-anchored mode: fraction of the parent menu's visual radius that a
/// submenu shifts away from it, along the direction of the entry that
/// opened it.
const SUBMENU_SHIFT_FACTOR: f64 = 0.75;

/// Radius (base px) of the little "came from" badge and its gap from the
/// entry ring in non-anchored mode. Deeper ancestors shrink and fade by
/// these factors and are linked with BREADCRUMB_LINK_GAP px of space.
const BREADCRUMB_R: f64 = 26.0;
const BREADCRUMB_GAP: f64 = 14.0;
const BREADCRUMB_SHRINK: f64 = 0.82;
const BREADCRUMB_FADE: f64 = 0.7;
const BREADCRUMB_LINK_GAP: f64 = 8.0;

/// Size (px) of the invisible settings-hotspot square in the chosen
/// screen corner while the overlay is visible.
const SETTINGS_HOTSPOT: f64 = 32.0;

// Emojis render larger than text glyphs at the same point size because emoji
// fonts fill the full em box. Shrink single-char emoji icons by this factor.
// Adjust EMOJI_SIZE_SCALE to tune emoji icon size (1.0 = no shrink).
const EMOJI_SIZE_SCALE: f64 = 0.775;

/// Insert zero-width spaces into long alphanumeric runs so Pango can wrap
/// them at those points with WORD wrapping, which never inserts the visible
/// hyphens that mid-word (WordChar) line breaks do.
fn soft_break_long_runs(s: &str, max_run: usize) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    let mut run = 0usize;
    for ch in s.chars() {
        if ch.is_alphanumeric() {
            run += 1;
        } else {
            run = 0;
        }
        out.push(ch);
        if run >= max_run {
            out.push('\u{200B}');
            run = 0;
        }
    }
    out
}

fn is_emoji_char(c: char) -> bool {
    matches!(c as u32,
        0x231A..=0x231B | 0x2328 | 0x23CF | 0x23E9..=0x23FA | 0x24C2
        | 0x25AA..=0x25AB | 0x25B6 | 0x25C0 | 0x25FB..=0x25FE
        | 0x2934..=0x2935
        | 0x2B05..=0x2B07 | 0x2B1B..=0x2B1C | 0x2B50 | 0x2B55
        | 0x3030 | 0x303D | 0x3297 | 0x3299
        | 0xFE00..=0xFE0F | 0x200D
        | 0x1F000..=0x1FAFF | 0x1FC00..=0x1FFFD
    )
}

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

    // Spawn-at-cursor: origin captured from the pointer event that arrives
    // when the menu opens, plus the latest pointer position it can be
    // captured from while the surface is already mapped.
    spawn_at_cursor: bool,
    origin: Option<(f64, f64)>,
    pointer_pos: Option<(f64, f64)>,
    reveal_pending: bool,
    reveal_seq: u64,

    // Marking mode (hold & drag) session state
    marking_mode: bool,
    marking_pressed: bool,
    marking_active: bool,
    marking_press_pos: Option<(f64, f64)>,
    marking_press_time: Option<std::time::Instant>,
    marking_dwell: Option<glib::SourceId>,
    marking_dwell_ms: u32,

    // Non-anchored mode: cumulative shift (base px) of the current menu
    // from its spawn anchor, plus the parallel stacks of parent/forward
    // offsets kept alongside the history vectors.
    submenu_shift: bool,
    show_breadcrumbs: bool,
    settings_hotspot_corner: String,
    hotspot_hovered: bool,
    nav_offset: (f64, f64),
    history_offsets: Vec<(f64, f64)>,
    forward_history_offsets: Vec<(f64, f64)>,

    // Animation state
    is_closing: bool,
    hover_progresses: Vec<f64>, // 0.0 -> 1.0 for each slice

    // Cached icons to avoid loading on every frame tick
    icon_cache: HashMap<String, Option<cairo::ImageSurface>>,

    // Animated image icons (GIFs), decoded once at preload; the pair carries
    // the wall-clock start time so every redraw derives the correct frame
    anim_cache: HashMap<String, (gtk::gdk_pixbuf::PixbufAnimation, std::time::SystemTime)>,

    // Cached Pango layouts for single char icons (avoids shaping every frame)
    text_layout_cache: HashMap<(String, u32), gtk::pango::Layout>,

    // Cached Pango layouts for Material Symbols glyphs (avoids shaping every frame)
    material_layout_cache: HashMap<(char, u32), gtk::pango::Layout>,

    // Cached Pango layouts for slice labels
    label_layout_cache: HashMap<String, gtk::pango::Layout>,

    // Extra interactivity margin beyond slices
    scale: f64,
    extra_radius: f64,
    enable_pie_spacing: bool,
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
    last_blur_key: Option<(bool, bool, f64, u64)>,

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
        self.history_offsets.clear();
        self.forward_history_offsets.clear();
        self.nav_offset = (0.0, 0.0);
        self.hovered_index = None;
    }

    fn get_display_items(&self) -> Vec<launcher_core::MenuItem> {
        let mut items = self.current_items.clone();
        if !self.history.is_empty() && !self.hide_back_entry {
            items.push(launcher_core::MenuItem {
                label: "Back".to_string(),
                // Material glyph (not the "go-previous" system icon) so it
                // renders through the colored-glyph path and picks up the
                // entry icon / entry icon hover theme colors.
                icon: Some("arrow_back".to_string()),
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
            if !self.icon_cache.contains_key(raw_icon_name)
                && !self.anim_cache.contains_key(raw_icon_name)
            {
                if let Some(path) = resolve_image_path(raw_icon_name) {
                    if raw_icon_name.to_ascii_lowercase().ends_with(".gif") {
                        // Animated GIFs are decoded once here; frames are
                        // produced per redraw from the in-memory animation.
                        // Still GIFs go through the regular static pipeline.
                        match gtk::gdk_pixbuf::PixbufAnimation::from_file(&path) {
                            Ok(anim) => {
                                if anim.is_static_image() {
                                    let surface =
                                        gtk::gdk_pixbuf::Pixbuf::from_file_at_size(&path, 128, 128)
                                            .ok()
                                            .and_then(pixbuf_to_surface);
                                    self.icon_cache.insert(raw_icon_name.clone(), surface);
                                } else {
                                    self.anim_cache.insert(
                                        raw_icon_name.clone(),
                                        (anim, std::time::SystemTime::now()),
                                    );
                                }
                            }
                            Err(_) => {
                                // Corrupt file: cache as missing so we don't retry every frame
                                self.icon_cache.insert(raw_icon_name.clone(), None);
                            }
                        }
                    } else {
                        let pixbuf =
                            gtk::gdk_pixbuf::Pixbuf::from_file_at_size(&path, 128, 128).ok();
                        let surface = pixbuf.and_then(pixbuf_to_surface);
                        self.icon_cache.insert(raw_icon_name.clone(), surface);
                    }
                } else {
                    let is_sys_forced = raw_icon_name.starts_with("sys:");
                    let icon_name = if is_sys_forced {
                        &raw_icon_name[4..]
                    } else {
                        raw_icon_name.as_str()
                    };

                    let pixbuf = load_icon_pixbuf(display, icon_name, 128, self.use_symbolic_icons);
                    let surface = pixbuf.and_then(pixbuf_to_surface);
                    self.icon_cache.insert(raw_icon_name.clone(), surface);
                }
            }
        }
    }

    /// Current surface for an icon: the cached static surface, or for animated
    /// icons (GIFs) a fresh small surface holding the frame due right now.
    fn icon_frame_surface(&self, icon_name: &str) -> Option<cairo::ImageSurface> {
        if let Some(Some(surf)) = self.icon_cache.get(icon_name) {
            return Some(surf.clone());
        }
        let (anim, start) = self.anim_cache.get(icon_name)?;
        let iter = anim.iter(Some(*start));
        iter.advance(std::time::SystemTime::now());
        pixbuf_to_surface(iter.pixbuf())
    }

    /// True when any icon currently displayed (hub or visible entries) is
    /// animated, i.e. the scene needs continuous repaints to play frames.
    fn has_visible_animation(&self) -> bool {
        if self.anim_cache.is_empty() {
            return false;
        }
        if self
            .current_icon
            .as_deref()
            .map_or(false, |i| self.anim_cache.contains_key(i))
        {
            return true;
        }
        self.get_display_items().iter().any(|item| {
            item.icon
                .as_deref()
                .map_or(false, |i| self.anim_cache.contains_key(i))
        })
    }

    fn material_glyph_layout(
        &mut self,
        area: &gtk::DrawingArea,
        codepoint: char,
        size: f64,
    ) -> gtk::pango::Layout {
        let font_size = size.round().max(1.0) as u32;
        // Keyed by char (not String) to avoid a per-frame heap allocation:
        // this runs for every material glyph on every animated frame.
        let key = (codepoint, font_size);
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

    /// Both floating variants share layout, hub, and hit-testing;
    /// "floating-icons" renders bare icons without tiles or label pills.
    fn is_floating(&self) -> bool {
        self.menu_style == "floating" || self.menu_style == "floating-icons"
    }

    fn floating_icon_only(&self) -> bool {
        self.menu_style == "floating-icons"
    }

    /// Guaranteed circumference per entry, interpolated by pill_roundness:
    /// square shapes need more room than circles.
    fn floating_arc_per_entry(&self) -> f64 {
        let q = self.pill_roundness.clamp(0.0, 1.0);
        if self.floating_icon_only() {
            FLOATING_ICONS_ARC_SQUARE + (FLOATING_ICONS_ARC_ROUND - FLOATING_ICONS_ARC_SQUARE) * q
        } else {
            FLOATING_ARC_SQUARE + (FLOATING_ARC_ROUND - FLOATING_ARC_SQUARE) * q
        }
    }

    /// Base hub-edge -> entry-center gap, interpolated by pill_roundness.
    /// This dominates spacing for small menus (the arc formula only kicks
    /// in past ~10 entries), so it must be roundness-aware too.
    fn floating_base_gap(&self) -> f64 {
        let q = self.pill_roundness.clamp(0.0, 1.0);
        if self.floating_icon_only() {
            FLOATING_ICONS_BASE_GAP_SQUARE
                + (FLOATING_ICONS_BASE_GAP_ROUND - FLOATING_ICONS_BASE_GAP_SQUARE) * q
        } else {
            FLOATING_BASE_GAP_SQUARE + (FLOATING_BASE_GAP_ROUND - FLOATING_BASE_GAP_SQUARE) * q
        }
    }

    /// Dynamic hub-to-ring gap in pie mode: when enabled, the gap auto-grows
    /// with entry count like floating mode (gentler, icon-only slices).
    fn effective_pie_spacing(&self, n: usize) -> f64 {
        if !self.enable_pie_spacing {
            return 0.0;
        }
        let required_r = n as f64 * PIE_ARC_PER_ENTRY / (2.0 * PI);
        let base_mid_radius = BASE_R + SLICE_WIDTH / 2.0;
        (required_r - base_mid_radius).max(0.0)
    }

    /// Approximate radius of the *painted* menu (hub, ring, entries and
    /// hover expansion) in base (unscaled) px. Excludes extra_radius,
    /// which is only an interactivity margin. Used to clamp the cursor
    /// origin back inside the monitor so the menu stays fully visible,
    /// and to position the non-anchored mode badge.
    fn visual_radius_base(&self, n: usize) -> f64 {
        if self.is_floating() {
            let arc_per_entry = self.floating_arc_per_entry();
            let required_r = n as f64 * arc_per_entry / (2.0 * PI);
            let base_dist = BASE_R + self.floating_base_gap();
            base_dist.max(required_r) + SLICE_WIDTH + HOVER_GROW
        } else {
            BASE_R + self.effective_pie_spacing(n) + SLICE_WIDTH + HOVER_GROW
        }
    }

    fn visual_radius(&self, n: usize) -> f64 {
        self.visual_radius_base(n) * self.scale.max(0.01)
    }

    /// Hub center for one navigation level: the spawn anchor (cursor or
    /// monitor center) shifted by `offset` (base px) and clamped so the
    /// menu fits on screen. Shared by the draw callback, hit-testing and
    /// blur regions.
    fn center_for(&self, width: f64, height: f64, n: usize, offset: (f64, f64)) -> (f64, f64) {
        let cx = width / 2.0;
        let cy = height / 2.0;
        let s = self.scale.max(0.01);
        let (ox, oy) = match self.origin.filter(|_| self.spawn_at_cursor) {
            Some((ox, oy)) => (ox, oy),
            None => (cx, cy),
        };
        let mut m = self.visual_radius(n) + SCREEN_MARGIN;
        // Reserve room for the "came from" badge so it can't hang off
        // the monitor edge (conservative: applied in all directions).
        if self.submenu_shift && self.show_breadcrumbs && !self.history_offsets.is_empty() {
            m += (BREADCRUMB_GAP + 2.0 * BREADCRUMB_R) * s;
        }
        let x = ox + offset.0 * s;
        let y = oy + offset.1 * s;
        (
            if m * 2.0 >= width {
                cx
            } else {
                x.clamp(m, width - m)
            },
            if m * 2.0 >= height {
                cy
            } else {
                y.clamp(m, height - m)
            },
        )
    }

    fn menu_center(&self, width: f64, height: f64, n: usize) -> (f64, f64) {
        self.center_for(width, height, n, self.nav_offset)
    }

    /// Breadcrumb trail for non-anchored mode: one disc per ancestor,
    /// starting at the entry ring in the direction of the immediate
    /// parent, then walking each level's real displacement direction
    /// towards its own parent (so the trail respects the angles actually
    /// travelled). Returns (offset from hub center in base px, radius),
    /// ordered parent, grandparent, ..., root.
    fn breadcrumb_layout(&self, n: usize) -> Vec<((f64, f64), f64)> {
        let mut out = Vec::new();
        if self.history_offsets.is_empty() {
            return out;
        }
        let mut cur = (0.0, 0.0);
        let mut prev_offset = self.nav_offset;
        let mut prev_r = 0.0;
        for k in (0..self.history_offsets.len()).rev() {
            let po = self.history_offsets[k];
            let depth = (self.history_offsets.len() - 1 - k) as f64;
            let r = BREADCRUMB_R * BREADCRUMB_SHRINK.powf(depth);
            let mut vx = po.0 - prev_offset.0;
            let mut vy = po.1 - prev_offset.1;
            let vl = (vx * vx + vy * vy).sqrt();
            if vl > 1e-6 {
                vx /= vl;
                vy /= vl;
            }
            let lead = if out.is_empty() {
                self.visual_radius_base(n) + BREADCRUMB_GAP + r
            } else {
                prev_r + BREADCRUMB_LINK_GAP + r
            };
            cur = (cur.0 + vx * lead, cur.1 + vy * lead);
            out.push((cur, r));
            prev_offset = po;
            prev_r = r;
        }
        out
    }

    /// Which breadcrumb disc (if any) covers the given screen point;
    /// 0 = immediate parent. Clicking one jumps back that many levels.
    fn breadcrumb_hit(&self, x: f64, y: f64, cx: f64, cy: f64, n: usize) -> Option<usize> {
        if !self.submenu_shift || !self.show_breadcrumbs || self.history_offsets.is_empty() {
            return None;
        }
        let s = self.scale.max(0.01);
        let mx = (x - cx) / s;
        let my = (y - cy) / s;
        for (j, ((dx, dy), r)) in self.breadcrumb_layout(n).iter().enumerate() {
            let ddx = mx - dx;
            let ddy = my - dy;
            if (ddx * ddx + ddy * ddy).sqrt() <= r + 6.0 {
                return Some(j);
            }
        }
        None
    }

    /// True when the given screen point falls inside the invisible
    /// settings-hotspot square anchored to the configured screen corner.
    fn settings_hotspot_hit(&self, x: f64, y: f64, width: f64, height: f64) -> bool {
        let (hx, hy) = match self.settings_hotspot_corner.as_str() {
            "top-left" => (0.0, 0.0),
            "top-right" => (width - SETTINGS_HOTSPOT, 0.0),
            "bottom-left" => (0.0, height - SETTINGS_HOTSPOT),
            "bottom-right" => (width - SETTINGS_HOTSPOT, height - SETTINGS_HOTSPOT),
            _ => return false,
        };
        x >= hx && x <= hx + SETTINGS_HOTSPOT && y >= hy && y <= hy + SETTINGS_HOTSPOT
    }

    /// Unit vector pointing at the middle of slice `index`, matching the
    /// angle convention used by the renderer.
    fn slice_direction(&self, index: usize, n: usize) -> (f64, f64) {
        if n == 0 {
            return (0.0, 0.0);
        }
        let angle_per_slice = 2.0 * PI / n as f64;
        let mut base_start = index as f64 * angle_per_slice - PI / 2.0;
        if self.center_layout {
            base_start -= angle_per_slice / 2.0;
        }
        let mid = base_start + angle_per_slice / 2.0;
        (mid.cos(), mid.sin())
    }

    /// Forget the captured origin and hold the menu transparent until the
    /// next pointer event (the compositor's enter on remap) re-anchors it
    /// to the cursor. Returns the reveal sequence for the fallback timer.
    fn arm_cursor_capture(&mut self) -> u64 {
        self.origin = None;
        self.pointer_pos = None;
        self.reveal_pending = self.spawn_at_cursor;
        self.hotspot_hovered = false;
        self.reveal_seq += 1;
        self.reveal_seq
    }

    /// Record the latest pointer position and, on the first event after
    /// the menu opened, pin the menu to it and reveal it. Returns true
    /// when the painted result may have changed.
    fn note_pointer(&mut self, x: f64, y: f64) -> bool {
        self.pointer_pos = Some((x, y));
        let mut changed = false;
        if self.spawn_at_cursor && self.reveal_pending && self.origin.is_none() {
            self.origin = Some((x, y));
            changed = true;
        }
        if self.reveal_pending && self.origin.is_some() {
            self.reveal_pending = false;
            changed = true;
        }
        changed
    }

    /// Tear down any marking session: cancels the pending dwell timer
    /// and clears the press bookkeeping.
    fn end_marking(&mut self) {
        if let Some(id) = self.marking_dwell.take() {
            id.remove();
        }
        self.marking_pressed = false;
        self.marking_active = false;
        self.marking_press_pos = None;
        self.marking_press_time = None;
    }

    /// Whether the marking dwell timer should auto-trigger over `index`:
    /// submenu entries (non-empty children) plus the auto-generated Back
    /// entry, which carries no children but rewinds the history.
    fn marking_dwell_target(&self, index: usize) -> bool {
        let items = self.get_display_items();
        let is_back = !self.history.is_empty()
            && !self.hide_back_entry
            && index == items.len().saturating_sub(1);
        items
            .get(index)
            .map(|it| !it.children.is_empty() || is_back)
            .unwrap_or(false)
    }

    fn hit_test(&self, x: f64, y: f64, cx: f64, cy: f64) -> Option<usize> {
        let display_items = self.get_display_items();
        let n = display_items.len();
        if n == 0 {
            return None;
        }

        let s = self.scale.max(0.01);
        let mx = (x - cx) / s;
        let my = (y - cy) / s;
        let dist = (mx * mx + my * my).sqrt();

        // The whole area from the hub edge outwards is active: in pie mode
        // the separation gap still maps by angle to the nearest slice
        // (same as floating mode's hub-to-pill gap)
        if dist < BASE_R {
            return None;
        }

        // Breadcrumb discs live outside the ring and are resolved by
        // breadcrumb_hit() in the event handlers; ignore them here so a
        // disc never steals a wedge hover, and vice versa.
        if self.breadcrumb_hit(x, y, cx, cy, n).is_some() {
            return None;
        }

        let max_interactive_dist = if self.is_floating() {
            let arc_per_entry = self.floating_arc_per_entry();
            let required_r = n as f64 * arc_per_entry / (2.0 * PI);
            let base_dist = BASE_R + self.floating_base_gap();
            let pill_dist = base_dist.max(required_r);
            pill_dist + SLICE_WIDTH + HOVER_GROW + self.extra_radius + 40.0
        } else {
            BASE_R + self.effective_pie_spacing(n) + SLICE_WIDTH + HOVER_GROW + self.extra_radius
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

/// Converts a pixbuf into a small cairo image surface for drawing.
fn pixbuf_to_surface(p: gtk::gdk_pixbuf::Pixbuf) -> Option<cairo::ImageSurface> {
    let format = if p.has_alpha() {
        cairo::Format::ARgb32
    } else {
        cairo::Format::Rgb24
    };
    let surf = cairo::ImageSurface::create(format, p.width(), p.height()).ok()?;
    let cr = cairo::Context::new(&surf).ok()?;
    cr.set_source_pixbuf(&p, 0.0, 0.0);
    let _ = cr.paint();
    Some(surf)
}

/// Resolves icon strings that reference an image file on disk
/// (absolute paths or `~/...`) into an existing filesystem path.
/// Returns None for anything that isn't a path-shaped icon string.
fn resolve_image_path(icon: &str) -> Option<std::path::PathBuf> {
    let path = if let Some(rest) = icon.strip_prefix("~/") {
        std::path::PathBuf::from(std::env::var_os("HOME")?).join(rest)
    } else if icon.starts_with('/') {
        std::path::PathBuf::from(icon)
    } else {
        return None;
    };
    path.is_file().then_some(path)
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
            .hub-surface {{ color: alpha({}, {:.3}); }}
            .hub-border {{ color: alpha({}, {:.3}); }}
            .hub-label {{ color: alpha({}, {:.3}); }}
            .hub-icon {{ color: alpha({}, {:.3}); }}
            .pie-outer-border {{ color: alpha({}, {:.3}); }}
            .floating-icon-surface {{ color: alpha({}, {:.3}); }}
            .floating-icon-surface:hover {{ color: alpha({}, {:.3}); }}
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
            overrides.hub_surface.variable,
            overrides.hub_surface.opacity,
            overrides.hub_border.variable,
            overrides.hub_border.opacity,
            overrides.hub_label.variable,
            overrides.hub_label.opacity,
            overrides.hub_icon.variable,
            overrides.hub_icon.opacity,
            overrides.pie_outer_border.variable,
            overrides.pie_outer_border.opacity,
            overrides.floating_icon_surface.variable,
            overrides.floating_icon_surface.opacity,
            overrides.floating_icon_surface_hover.variable,
            overrides.floating_icon_surface_hover.opacity
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
            .hub-surface { color: rgba(17, 17, 27, 0.95); }
            .hub-border { color: rgba(137, 180, 250, 0.70); }
            .hub-label { color: rgba(205, 214, 244, 1.0); }
            .hub-icon { color: rgba(205, 214, 244, 1.0); }
            .pie-outer-border { color: rgba(137, 180, 250, 1.0); }
            .floating-icon-surface { color: rgba(30, 30, 46, 1.0); }
            .floating-icon-surface:hover { color: rgba(137, 180, 250, 1.0); }
        ";
        theme_provider.load_from_data(std::str::from_utf8(fallback).unwrap());
    }
}

fn expand_tilde_path(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(path)
}

/// If the command is an rmwk menu invocation (e.g. `rmwk open other`,
/// `rmwk --menu ~/cfg/menu.toml`), return the resolved menu path so the
/// running instance can switch menus in-process instead of spawning a
/// whole new binary (which forces a close/reopen round-trip).
fn resolve_self_menu_command(cmd: &str) -> Option<PathBuf> {
    let mut tokens = cmd.split_whitespace();
    let bin = Path::new(tokens.next()?)
        .file_stem()?
        .to_string_lossy()
        .to_lowercase();
    if bin != "rmwk" {
        return None;
    }

    let mut menu_path: Option<PathBuf> = None;
    let mut menu_name: Option<String> = None;
    let mut pending_menu_arg = false;
    let mut saw_open_subcommand = false;

    for tok in tokens {
        if pending_menu_arg {
            menu_path = Some(expand_tilde_path(tok));
            pending_menu_arg = false;
        } else if let Some(val) = tok.strip_prefix("--menu=") {
            menu_path = Some(expand_tilde_path(val));
        } else if tok == "--menu" || tok == "-m" {
            pending_menu_arg = true;
        } else if tok == "open" {
            saw_open_subcommand = true;
        } else if !tok.starts_with('-') && saw_open_subcommand && menu_name.is_none() {
            menu_name = Some(tok.to_string());
        }
    }

    if let Some(p) = menu_path {
        return Some(p);
    }
    menu_name.map(|name| {
        launcher_core::paths::get_config_dir()
            .join("menus")
            .join(format!("{}.toml", name))
    })
}

/// Single source of truth for the Wayland blur region so every code
/// path (realize, resize, draw tick, reload) agrees on the same shape.
///
/// The blur is split into two sections: a disc under the hub and an
/// annulus under the entry ring, so the pie_spacing gap stays unblurred.
/// With no spacing it degenerates to a single disc as before.
/// Deliberately covers only the base pie circle: with the "outwards" hover
/// cue the hovered slice expands past the blurred area, which is intended.
fn target_blur_regions(
    enable_blur: bool,
    is_closing: bool,
    pie_spacing: f64,
    scale: f64,
) -> Vec<(f64, Option<f64>)> {
    let s = scale.max(0.01);
    if !enable_blur || is_closing {
        Vec::new()
    } else if pie_spacing < 0.5 {
        vec![((BASE_R + SLICE_WIDTH) * s, None)]
    } else {
        vec![
            (BASE_R * s, None),
            (
                (BASE_R + pie_spacing + SLICE_WIDTH) * s,
                Some((BASE_R + pie_spacing) * s),
            ),
        ]
    }
}

/// Safety net for arm_cursor_capture(): if the compositor never delivers
/// the enter/motion event that reveals the menu, give up waiting after a
/// short grace period and show it at the monitor center instead.
fn schedule_reveal_fallback(state: &Rc<RefCell<MenuState>>, area: &gtk::DrawingArea, seq: u64) {
    let st = state.clone();
    let ar = area.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(300), move || {
        if let Ok(mut s) = st.try_borrow_mut() {
            if s.reveal_pending && s.reveal_seq == seq {
                s.reveal_pending = false;
                drop(s);
                ar.queue_draw();
            }
        }
    });
}

/// Start a marking dwell timer: if the button is still held, the pointer
/// is still over `index` and that entry is a dwell target (a submenu or
/// the auto-generated Back entry), it triggers automatically.
/// Re-hovering within the fresh menu does not start a
/// new dwell, so nested menus never chain-open while the user just holds
/// the button: they must move away and back in deliberately.
fn schedule_marking_dwell(
    state: &Rc<RefCell<MenuState>>,
    area: &gtk::DrawingArea,
    index: usize,
    trigger_anim: Rc<dyn Fn()>,
) {
    let st = state.clone();
    let ar = area.clone();
    let dwell_ms = state
        .try_borrow()
        .map(|s| s.marking_dwell_ms.max(30))
        .unwrap_or(MARKING_DWELL_MS);
    let id = glib::timeout_add_local(
        std::time::Duration::from_millis(dwell_ms as u64),
        move || {
            if let Ok(mut s) = st.try_borrow_mut() {
                s.marking_dwell = None;
                let still_marking = s.marking_mode
                    && s.marking_pressed
                    && s.marking_active
                    && !s.is_closing
                    && s.hovered_index == Some(index);
                let dwell_target = still_marking && s.marking_dwell_target(index);
                if !dwell_target {
                    return glib::ControlFlow::Break;
                }
                activate_index(&mut s, index, &ar);
                // Keep the highlight under the pointer in the fresh submenu
                if let Some((px, py)) = s.pointer_pos {
                    let (cx, cy) = s.menu_center(
                        ar.width() as f64,
                        ar.height() as f64,
                        s.get_display_items().len(),
                    );
                    s.hovered_index = s.hit_test(px, py, cx, cy);
                }
                drop(s);
                // Same activation path as a click: redraw and run the
                // hover animation tick rather than snapping.
                trigger_anim();
            }
            glib::ControlFlow::Break
        },
    );
    if let Ok(mut s) = state.try_borrow_mut() {
        if let Some(old) = s.marking_dwell.replace(id) {
            old.remove();
        }
    } else {
        id.remove();
    }
}

/// Compact replica of the hub icon renderer (single char/emoji, Material
/// glyph, or cached image surface) for arbitrary centers and sizes. Used
/// by the non-anchored mode "came from" badge.
fn draw_small_icon(
    state_ref: &mut MenuState,
    area: &gtk::DrawingArea,
    cr: &cairo::Context,
    icon_name: &str,
    icon_size: f64,
    cx: f64,
    cy: f64,
) {
    if icon_name.chars().count() == 1 && !icon_name.starts_with('/') {
        let font_size = icon_size.round().max(1.0) as u32;
        let key = (icon_name.to_string(), font_size);
        let l = if let Some(l) = state_ref.text_layout_cache.get(&key) {
            l.clone()
        } else {
            let l = area.create_pango_layout(Some(icon_name));
            let mut font_desc = gtk::pango::FontDescription::new();
            if state_ref.bold_single_chars {
                font_desc.set_weight(gtk::pango::Weight::Bold);
            }
            font_desc.set_family("Sans");
            let emoji_scale = if icon_name.chars().all(is_emoji_char) {
                EMOJI_SIZE_SCALE
            } else {
                1.0
            };
            font_desc.set_absolute_size(icon_size * emoji_scale * gtk::pango::SCALE as f64);
            l.set_font_description(Some(&font_desc));
            state_ref.text_layout_cache.insert(key, l.clone());
            l
        };
        let (iw, ih) = l.pixel_size();
        let _ = cr.save();
        cr.translate(cx, cy);
        cr.move_to(-(iw as f64 / 2.0), -(ih as f64 / 2.0));
        pangocairo::functions::show_layout(cr, &l);
        let _ = cr.restore();
    } else if let Some(&codepoint) = state_ref.codepoints.get(icon_name) {
        let layout = state_ref.material_glyph_layout(area, codepoint, icon_size);
        let (ink, _logical) = layout.pixel_extents();
        let _ = cr.save();
        cr.translate(cx, cy);
        cr.move_to(
            -(ink.x() as f64 + ink.width() as f64 / 2.0),
            -(ink.y() as f64 + ink.height() as f64 / 2.0),
        );
        let _ = pangocairo::functions::show_layout(cr, &layout);
        let _ = cr.restore();
    } else if let Some(surf) = state_ref.icon_frame_surface(icon_name) {
        let cw = surf.width() as f64;
        let ch = surf.height() as f64;
        let scale = icon_size / cw.max(ch).max(1.0);
        let _ = cr.save();
        cr.translate(cx - cw * scale / 2.0, cy - ch * scale / 2.0);
        cr.scale(scale, scale);
        let _ = cr.set_source_surface(&surf, 0.0, 0.0);
        let _ = cr.paint();
        let _ = cr.restore();
    }
}

/// Completes the close sequence: hides the window and rewinds to the root
/// menu. Must run synchronously (from both the animation tick and the
/// trigger): a tick callback scheduled while the widget is unmapped never
/// fires, which would strand `is_closing`/`is_animating` and desync the
/// app's notion of window visibility from the compositor forever.
fn complete_close(
    state: &Rc<RefCell<MenuState>>,
    win: &gtk::ApplicationWindow,
    root_menu: &launcher_core::MenuConfig,
    anim_flag: &std::cell::Cell<bool>,
) {
    if let Ok(mut s) = state.try_borrow_mut() {
        s.is_closing = false;
        win.hide();
        // Drop any hotspot hand cursor so the next open starts clean even
        // if the pointer never leaves the surface between sessions.
        win.set_cursor_from_name(Some("default"));

        // Reset to root menu
        s.root_items = root_menu.menu.clone();
        s.root_icon = root_menu.icon.clone();
        s.reset_to_root();
        s.arm_cursor_capture();
        s.end_marking();
        if let Some(display) = gdk::Display::default() {
            s.preload_icons(&display);
        }
    }
    anim_flag.set(false);
}

fn go_back(state: &mut MenuState, area: &gtk::DrawingArea) -> bool {
    if let Some(prev) = state.history.pop() {
        let prev_icon = state.history_icons.pop().unwrap_or(None);
        let parent_offset = state.history_offsets.pop().unwrap_or((0.0, 0.0));

        // Push current state to forward history
        state.forward_history.push(state.current_items.clone());
        state.forward_history_icons.push(state.current_icon.clone());
        state.forward_history_offsets.push(state.nav_offset);

        if let Some(icon) = prev_icon.clone() {
            state.current_icon = Some(icon);
        } else if prev_icon.is_none() && state.history.is_empty() {
            state.current_icon = state.root_icon.clone();
        } else {
            state.current_icon = None; // fallback
        }

        state.nav_offset = parent_offset;
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
        let next_offset = state.forward_history_offsets.pop().unwrap_or((0.0, 0.0));

        // Push current state to history
        state.history.push(state.current_items.clone());
        state.history_icons.push(state.current_icon.clone());
        state.history_offsets.push(state.nav_offset);

        state.current_icon = next_icon;
        state.nav_offset = next_offset;
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

    let is_back_button =
        !state.history.is_empty() && !state.hide_back_entry && index == display_items_count - 1;
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
            state.forward_history_offsets.clear();

            state.history.push(current_items);
            state.history_icons.push(state.current_icon.clone());
            state.history_offsets.push(state.nav_offset);

            // Non-anchored mode: drift the submenu away from the parent
            // along the direction of the wedge that opened it.
            if state.submenu_shift {
                let (dx, dy) = state.slice_direction(index, display_items_count);
                let dist = state.visual_radius_base(display_items_count) * SUBMENU_SHIFT_FACTOR;
                state.nav_offset = (
                    state.nav_offset.0 + dx * dist,
                    state.nav_offset.1 + dy * dist,
                );
            }

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

            // Fast-path: launching another rmwk menu from within this launcher.
            // Swap the menu contents in place (same as an IPC OpenMenu while
            // visible) instead of closing and waiting for a spawned process
            // to boot and tell us to reopen.
            if let launcher_core::Action::Command { cmd, .. } = &action {
                if let Some(new_path) = resolve_self_menu_command(cmd) {
                    if new_path.exists() {
                        let same_menu = state.current_menu_path == new_path;
                        state.current_menu_path = new_path.clone();
                        if same_menu {
                            // Matches the IPC OpenMenu toggle behaviour
                            state.is_closing = true;
                        } else {
                            match launcher_core::load_menu(&new_path) {
                                Ok(m) => {
                                    state.root_items = m.menu.clone();
                                    state.root_icon = m.icon.clone();
                                    state.reset_to_root();
                                    // Surface stays mapped, so no new enter
                                    // event will arrive: re-anchor directly
                                    // to the live pointer position.
                                    state.origin = state.pointer_pos;
                                    if let Some(display) = gdk::Display::default() {
                                        state.preload_icons(&display);
                                    }
                                    area.queue_draw();
                                }
                                Err(e) => {
                                    error!("Failed to load menu from {:?}: {}", new_path, e);
                                }
                            }
                        }
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
            .application_id("rmwk.launcher")
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
            spawn_at_cursor: ui_config.spawn_at_cursor,
            origin: None,
            pointer_pos: None,
            reveal_pending: false,
            reveal_seq: 0,
            marking_mode: ui_config.marking_mode,
            marking_pressed: false,
            marking_active: false,
            marking_press_pos: None,
            marking_press_time: None,
            marking_dwell: None,
            marking_dwell_ms: ui_config.marking_dwell_ms.max(30),
            submenu_shift: ui_config.submenu_shift,
            show_breadcrumbs: ui_config.show_breadcrumbs,
            settings_hotspot_corner: ui_config.settings_hotspot_corner.clone(),
            hotspot_hovered: false,
            nav_offset: (0.0, 0.0),
            history_offsets: vec![],
            forward_history_offsets: vec![],
            is_closing: false,
            hover_progresses: vec![],
            icon_cache: HashMap::new(),
            anim_cache: HashMap::new(),
            text_layout_cache: HashMap::new(),
            material_layout_cache: HashMap::new(),
            label_layout_cache: HashMap::new(),
            extra_radius: ui_config.extra_radius,
            scale: ui_config.scale,
            enable_pie_spacing: ui_config.enable_pie_spacing,
            pill_roundness: ui_config.pill_roundness,
            use_symbolic_icons: ui_config.use_symbolic_icons,
            bold_single_chars: ui_config.bold_single_chars,
            menu_style: ui_config.menu_style.clone(),
            center_layout: ui_config.center_layout,
            disable_hover_animation: ui_config.disable_hover_animation,
            hover_visual_cue: ui_config.hover_visual_cue.clone(),
            enable_blur: ui_config.enable_blur
                && ui_config.menu_style != "floating"
                && ui_config.menu_style != "floating-icons",
            last_cx: 0.0,
            last_cy: 0.0,
            last_blur_key: None,
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
                let st = state_realize.borrow();
                let (cx, cy) = st.menu_center(width, height, st.get_display_items().len());
                let spacing = st.effective_pie_spacing(st.get_display_items().len());
                let regions = if st.reveal_pending {
                    Vec::new()
                } else {
                    target_blur_regions(st.enable_blur, false, spacing, st.scale)
                };
                blur.update_sectioned_region(cx, cy, &regions);
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
                let mut state = state_resize.borrow_mut();
                let (cx, cy) = state.menu_center(win_w, win_h, state.get_display_items().len());
                state.last_cx = cx;
                state.last_cy = cy;
                let regions = if state.reveal_pending {
                    Vec::new()
                } else {
                    target_blur_regions(
                        state.enable_blur,
                        state.is_closing,
                        state.effective_pie_spacing(state.get_display_items().len()),
                        state.scale,
                    )
                };
                blur.update_sectioned_region(cx, cy, &regions);
            }
        });

        let draw_state = state.clone();
        let blur_draw = wayland_blur.clone();
        drawing_area.set_draw_func(move |area, cr, width, height| {
            let mut state_ref = match draw_state.try_borrow_mut() {
                Ok(s) => s,
                Err(_) => return,
            };

            let display_items = state_ref.get_display_items();
            let n = display_items.len();

            // Keep hover_progresses in lockstep with the slice count so the
            // modulo lookups below can index freely: a menu swap can queue a
            // draw before the animation tick gets a chance to resize (e.g.
            // marking-mode auto-descend / auto-back).
            if state_ref.hover_progresses.len() != n {
                state_ref.hover_progresses.resize(n, 0.0);
            }

            let (cx, cy) = state_ref.menu_center(width as f64, height as f64, n);

            // Hold the menu transparent until the cursor position is
            // known, so it never appears at the monitor center and then
            // jumps to the pointer.
            if state_ref.reveal_pending {
                if let Some(blur) = blur_draw.borrow().as_ref() {
                    if state_ref.last_blur_key != Some(BLUR_PENDING_KEY) {
                        blur.update_sectioned_region(cx, cy, &[]);
                        state_ref.last_blur_key = Some(BLUR_PENDING_KEY);
                    }
                }
                return;
            }

            // Update blur region based on animation progress
            if let Some(blur) = blur_draw.borrow().as_ref() {
                // Only update Wayland region if it has actually changed to avoid IPC overhead
                let spacing = state_ref.effective_pie_spacing(n);
                let scale_bits = state_ref.scale.to_bits();
                let key = (
                    state_ref.enable_blur,
                    state_ref.is_closing,
                    spacing,
                    scale_bits,
                );
                let center_changed =
                    (cx - state_ref.last_cx).abs() > 0.5 || (cy - state_ref.last_cy).abs() > 0.5;
                if state_ref.last_blur_key != Some(key) || center_changed {
                    let regions = target_blur_regions(
                        state_ref.enable_blur,
                        state_ref.is_closing,
                        spacing,
                        state_ref.scale,
                    );
                    blur.update_sectioned_region(cx, cy, &regions);
                    state_ref.last_blur_key = Some(key);
                    state_ref.last_cx = cx;
                    state_ref.last_cy = cy;
                }
            }

            let ease_progress = 1.0;

            // Clear surface (ensure transparent background is clean)
            let scale = state_ref.scale.max(0.01);
            let max_interactive_dist = (BASE_R
                + state_ref.effective_pie_spacing(n)
                + SLICE_WIDTH
                + HOVER_GROW
                + state_ref.extra_radius
                + 80.0)
                * scale;
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

            let _ = cr.save();
            cr.translate(cx, cy);
            cr.scale(scale, scale);
            cr.translate(-cx, -cy);

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
                // Pie and floating-icons: show the hovered entry's label in
                // the hub, falling back to the current menu icon
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
                // Draw center hub if visible. All modes share the pie hub
                // size (BASE_R); floating modes additionally round it via
                // the pill_roundness setting
                let hub_rad = BASE_R * ease_progress;
                let hub_round = if state_ref.is_floating() {
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
                    // Hub icon size in px (material glyph / system image / single-char),
                    // shared by all modes since the hub size is standardized
                    let icon_size = 92.0 * ease_progress;
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
                            let emoji_scale = if icon_name.chars().all(is_emoji_char) {
                                EMOJI_SIZE_SCALE
                            } else {
                                1.0
                            };
                            font_desc.set_absolute_size(
                                icon_size * emoji_scale * gtk::pango::SCALE as f64,
                            );
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
                    } else if let Some(surf) = state_ref.icon_frame_surface(icon_name) {
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

                    // Threshold 10 keeps normal words intact (they wrap
                    // whole at spaces); only longer runs get break points
                    let soft = soft_break_long_runs(text, 10);
                    let mut center_layout = if let Some(l) = state_ref.label_layout_cache.get(text)
                    {
                        l.clone()
                    } else {
                        // Pre-split long unbreakable runs with zero-width
                        // spaces: Pango can then wrap them at those points
                        // with WORD wrapping, which never inserts the visible
                        // hyphens that mid-word (WordChar) breaks do. Normal
                        // words stay whole and wrap at spaces.
                        let l = area.create_pango_layout(Some(&soft));
                        let mut font_desc = gtk::pango::FontDescription::new();
                        font_desc.set_family("Sans");
                        font_desc.set_weight(gtk::pango::Weight::Bold);
                        font_desc.set_absolute_size(16.0 * gtk::pango::SCALE as f64);
                        l.set_font_description(Some(&font_desc));
                        state_ref.label_layout_cache.insert(text.clone(), l.clone());
                        l
                    };
                    // Text box: a wide horizontal rectangle through the
                    // hub's middle. Its width follows the hub's actual
                    // shape: the full chord of the inscribed circle for
                    // round hubs (pie), widening to the full square side as
                    // pill_roundness decreases (floating-icons), using the
                    // rounded-square corner geometry.
                    let pad = 8.0;
                    let line_h = 22.0; // approx line height at 16px font
                    let box_h = 4.0 * line_h; // hard cap: at most 4 lines
                    let corner = if state_ref.is_floating() {
                        hub_rad * state_ref.pill_roundness.clamp(0.0, 1.0)
                    } else {
                        hub_rad // pie hubs are circles
                    };
                    let half_h = box_h * 0.5;
                    let half_w = if corner < 0.5 || half_h <= hub_rad - corner {
                        // Box vertical extent stays within the straight
                        // edges: full width available
                        hub_rad
                    } else {
                        // Corner region: widest width whose corner point
                        // stays inside the corner's rounding circle
                        let d = corner * corner - (half_h - (hub_rad - corner)).powi(2);
                        (hub_rad - corner) + d.max(0.0).sqrt()
                    };
                    let box_w = (half_w * 2.0 - 2.0 * pad).max(40.0);
                    // Word wrap (never hyphenates); the zero-width spaces
                    // pre-inserted into the layout text handle unbreakable
                    // runs
                    center_layout.set_wrap(gtk::pango::WrapMode::Word);
                    center_layout.set_alignment(gtk::pango::Alignment::Center);
                    center_layout.set_width((box_w * gtk::pango::SCALE as f64) as i32);

                    let (mut pango_w, mut pango_h) = center_layout.pixel_size();
                    // Over 4 lines at full size? Re-layout at a reduced font
                    // size so the text genuinely re-wraps into fewer lines
                    // (slight overflow -> slight decrease, exaggerated
                    // overflow -> considerable decrease)
                    if pango_h as f64 > box_h {
                        let font_scale = (box_h / pango_h as f64).max(0.2);
                        let l = area.create_pango_layout(Some(&soft));
                        let mut font_desc = gtk::pango::FontDescription::new();
                        font_desc.set_family("Sans");
                        font_desc.set_weight(gtk::pango::Weight::Bold);
                        font_desc.set_absolute_size(16.0 * font_scale * gtk::pango::SCALE as f64);
                        l.set_font_description(Some(&font_desc));
                        l.set_wrap(gtk::pango::WrapMode::Word);
                        l.set_alignment(gtk::pango::Alignment::Center);
                        l.set_width((box_w * gtk::pango::SCALE as f64) as i32);
                        center_layout = l;
                        let (w2, h2) = center_layout.pixel_size();
                        pango_w = w2;
                        pango_h = h2;
                    }
                    cr.save().unwrap();
                    cr.translate(cx, cy);
                    if ease_progress > 0.001 {
                        cr.scale(ease_progress, ease_progress);
                    }
                    // Center on the layout's allocated width (not pixel_size,
                    // which ignores the center alignment's offset)
                    cr.move_to(-box_w / 2.0, -(pango_h as f64) / 2.0);
                    pangocairo::functions::show_layout(&cr, &center_layout);
                    cr.restore().unwrap();
                }
            };

            if state_ref.is_floating() {
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
                        let icon_only = state_ref.floating_icon_only();
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

                        // Dynamic radius scaling (roundness-aware: square
                        // shapes need a roomier ring than circles)
                        let arc_per_entry = state_ref.floating_arc_per_entry();
                        let required_r = n as f64 * arc_per_entry / (2.0 * std::f64::consts::PI);
                        let base_dist = BASE_R + state_ref.floating_base_gap();
                        let pill_dist =
                            base_dist.max(required_r) + (hp * HOVER_GROW) * ease_progress;

                        let icon_center_x = cx + pill_dist * mid_angle.cos();
                        let icon_center_y = cy + pill_dist * mid_angle.sin();

                        // Measure text
                        let text = &item.label;
                        let text_layout = if icon_only {
                            None
                        } else if let Some(l) = state_ref.label_layout_cache.get(text) {
                            Some(l.clone())
                        } else {
                            let l = area.create_pango_layout(Some(text));
                            let mut font_desc = gtk::pango::FontDescription::new();
                            font_desc.set_family("Sans");
                            font_desc.set_size(gtk::pango::units_from_double(14.0));
                            l.set_font_description(Some(&font_desc));
                            state_ref.label_layout_cache.insert(text.clone(), l.clone());
                            Some(l)
                        };
                        let (tw, th) = if icon_only {
                            (0, 0)
                        } else {
                            text_layout.as_ref().unwrap().pixel_size()
                        };
                        let (tw_f, th_f) = (tw as f64 * ease_progress, th as f64 * ease_progress);

                        // Measure icon
                        let mut icon_w = 0.0;
                        let mut icon_h = 0.0;
                        // Icon-only entries scale like pie slices: the icon
                        // grows with the arc width available at its distance
                        let icon_size = if icon_only {
                            (pill_dist * angle_per_slice * 0.5).clamp(16.0, 64.0) * ease_progress
                        } else {
                            FLOATING_PILL_ICON_SIZE * ease_progress
                        };
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
                                    let emoji_scale = if icon_name.chars().all(is_emoji_char) {
                                        EMOJI_SIZE_SCALE
                                    } else {
                                        1.0
                                    };
                                    font_desc.set_size(gtk::pango::units_from_double(
                                        64.0 * 0.75 * emoji_scale,
                                    ));
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
                                    state_ref.material_glyph_layout(&area, codepoint, icon_size);
                                let (ink, _logical) = layout.pixel_extents();
                                icon_w = ink.width() as f64 * ease_progress;
                                icon_h = ink.height() as f64 * ease_progress;
                            } else if let Some(surf) = state_ref.icon_frame_surface(icon_name) {
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
                        let r = if icon_only {
                            (icon_size / 2.0 + FLOATING_ICONS_TILE_PADDING) * ease_progress
                        } else {
                            (icon_size / 2.0 + 8.0) * ease_progress
                        };

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
                                // Bare tile: shape follows pill_roundness
                                // (circle at 1.0, rounded square below it)
                                cr.new_path();
                                if round >= r - 0.001 {
                                    cr.arc(
                                        icon_center_x,
                                        icon_center_y,
                                        r,
                                        0.0,
                                        2.0 * std::f64::consts::PI,
                                    );
                                } else {
                                    rounded_rect_path(
                                        cr,
                                        icon_center_x - r,
                                        icon_center_y - r,
                                        2.0 * r,
                                        2.0 * r,
                                        round,
                                    );
                                }
                                return;
                            }
                            match mode {
                                PillMode::Right | PillMode::Left => {
                                    rounded_rect_path(cr, bx0, by0, bw, bh, round)
                                }
                                PillMode::Top | PillMode::Bottom => pill_capsule_union(cr),
                            }
                        };

                        // Icon-only entries keep a single shape (the tile,
                        // rounded per pill_roundness) but tint it with the
                        // slice colors: entry-surface fill + entry-border
                        // stroke, skipping the opaque floating-icon tile
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

                        if !icon_only {
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
                        }

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
                            if let Some(tl) = text_layout.as_ref() {
                                pangocairo::functions::show_layout(&cr, tl);
                            }
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
                                        // Em-based uniform scale (like pie
                                        // mode): normalizing by glyph width
                                        // made narrow letters render much
                                        // larger than wide emojis
                                        let scale = (icon_size * 0.90) / 64.0;
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
                                    let layout = state_ref
                                        .material_glyph_layout(&area, codepoint, icon_size);
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
                                } else if let Some(surf) = state_ref.icon_frame_surface(icon_name) {
                                    let _ = cr.save();
                                    cr.translate(icon_x + icon_w / 2.0, icon_y + icon_h / 2.0);
                                    let scale = icon_size / surf.width().max(surf.height()) as f64;
                                    let (sw, sh) = (surf.width() as f64, surf.height() as f64);
                                    cr.scale(scale, scale);
                                    let _ = cr.set_source_surface(surf, -sw / 2.0, -sh / 2.0);
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
                // The hub stays at BASE_R while the whole entry donut is pushed
                // outwards by the effective spacing (animated with ease_progress);
                // it auto-grows with entry count like floating mode
                let spacing = state_ref.effective_pie_spacing(n) * ease_progress;
                let base_outer_radius = (BASE_R + SLICE_WIDTH - 0.5) * ease_progress + spacing;
                let base_inner_radius = BASE_R * ease_progress + spacing;
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

                        let mut fill_outer_radius =
                            (BASE_R + SLICE_WIDTH - 0.5) * ease_progress + spacing;
                        let mut stroke_outer_radius =
                            (BASE_R + SLICE_WIDTH - 0.5) * ease_progress + spacing;

                        match state_ref.hover_visual_cue.as_str() {
                            "sides" => {
                                let hover_angle_grow = HOVER_GROW / (BASE_R + SLICE_WIDTH);
                                start_angle += (hp_prev - hp_curr) * hover_angle_grow;
                                end_angle += (hp_curr - hp_next) * hover_angle_grow;
                            }
                            "outwards" => {
                                stroke_outer_radius =
                                    (BASE_R + SLICE_WIDTH + (hp_curr * HOVER_GROW) - 0.5)
                                        * ease_progress
                                        + spacing;

                                if is_hovered {
                                    fill_outer_radius = stroke_outer_radius;
                                } else {
                                    // Instantly retreat the fill when unhovering so it doesn't leave an invisible trail
                                    fill_outer_radius =
                                        (BASE_R + SLICE_WIDTH - 0.5) * ease_progress + spacing;
                                }
                            }
                            _ => { // "none"
                                 // keep default values
                            }
                        }

                        let stroke_inner_radius = BASE_R * ease_progress + spacing;

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
                                    let emoji_scale = if icon_name.chars().all(is_emoji_char) {
                                        EMOJI_SIZE_SCALE
                                    } else {
                                        1.0
                                    };
                                    font_desc.set_size(gtk::pango::units_from_double(
                                        64.0 * 0.75 * emoji_scale,
                                    ));
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
                            } else if let Some(surf) = state_ref.icon_frame_surface(icon_name) {
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
                    let base_outer = (BASE_R + SLICE_WIDTH - 0.5) * ease_progress + spacing;

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

                // Once the entry ring separates from the hub, stroke its inner
                // edge with the outer ring color so the detached donut keeps a
                // defined edge (and the jagged inner edge of the blurred
                // annulus hides behind it). Hovering only expands slices
                // outwards, so the inner circle is always drawn in full.
                if spacing > 0.5 {
                    cr.new_path();
                    cr.arc(cx, cy, BASE_R * ease_progress + spacing, 0.0, 2.0 * PI);
                    cr.set_source_rgba(
                        outer_border_color.red() as f64,
                        outer_border_color.green() as f64,
                        outer_border_color.blue() as f64,
                        outer_border_color.alpha() as f64 * ease_progress,
                    );
                    cr.set_line_width(2.0);
                    cr.stroke().unwrap();
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

                        let mut stroke_outer_radius =
                            (BASE_R + SLICE_WIDTH - 0.5) * ease_progress + spacing;
                        if state_ref.hover_visual_cue == "outwards" {
                            stroke_outer_radius = (BASE_R + SLICE_WIDTH + (hp_curr * HOVER_GROW)
                                - 0.5)
                                * ease_progress
                                + spacing;
                        }
                        let stroke_inner_radius = BASE_R * ease_progress + spacing;

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
            // Non-anchored mode: breadcrumb trail of ancestor badges, each
            // showing that level's hub icon and pointing back along the
            // direction it was reached from. hit_test() keeps them out of
            // the wedge logic; the event handlers use breadcrumb_hit().
            if state_ref.submenu_shift && state_ref.show_breadcrumbs && !state_ref.history_offsets.is_empty() {
                let layout = state_ref.breadcrumb_layout(n);
                if !layout.is_empty() {
                    // Discs farthest-first, so the immediate parent ends up
                    // on top. Shape mirrors the hub: circles in pie mode,
                    // pill_roundness-rounded squares in the floating modes.
                    for (depth, &((dx, dy), r)) in layout.iter().enumerate().rev() {
                        let fade = BREADCRUMB_FADE.powf(depth as f64);
                        let bx = cx + dx;
                        let by = cy + dy;
                        let badge_round = if state_ref.is_floating() {
                            r * state_ref.pill_roundness.clamp(0.0, 1.0)
                        } else {
                            r
                        };
                        let badge_path = |cr: &cairo::Context| {
                            cr.new_path();
                            if badge_round >= r - 0.001 {
                                cr.arc(bx, by, r, 0.0, 2.0 * std::f64::consts::PI);
                            } else {
                                rounded_rect_path(
                                    cr,
                                    bx - r,
                                    by - r,
                                    2.0 * r,
                                    2.0 * r,
                                    badge_round,
                                );
                            }
                        };
                        if hub_fill.alpha() > 0.001 {
                            badge_path(cr);
                            cr.set_source_rgba(
                                hub_fill.red() as f64,
                                hub_fill.green() as f64,
                                hub_fill.blue() as f64,
                                hub_fill.alpha() as f64 * fade,
                            );
                            let _ = cr.fill();
                        }
                        if hub_border.alpha() > 0.001 {
                            badge_path(cr);
                            cr.set_source_rgba(
                                hub_border.red() as f64,
                                hub_border.green() as f64,
                                hub_border.blue() as f64,
                                hub_border.alpha() as f64 * fade,
                            );
                            cr.set_line_width(2.0);
                            let _ = cr.stroke();
                        }

                        let icon_idx = state_ref.history_icons.len().saturating_sub(depth + 1);
                        let badge_icon = state_ref
                            .history_icons
                            .get(icon_idx)
                            .cloned()
                            .flatten()
                            .or_else(|| {
                                if depth == 0 {
                                    Some("arrow_back".to_string())
                                } else {
                                    None
                                }
                            });
                        if let Some(ref icon_name) = badge_icon {
                            cr.set_source_rgba(
                                hub_icon_color.red() as f64,
                                hub_icon_color.green() as f64,
                                hub_icon_color.blue() as f64,
                                hub_icon_color.alpha() as f64 * fade,
                            );
                            draw_small_icon(&mut state_ref, area, cr, icon_name, r * 1.3, bx, by);
                        }
                    }
                }
            }
            let _ = cr.restore();
        });
        window.set_child(Some(&drawing_area));

        // Pausable frame clock controller: ensures 0.0% CPU when stationary/closed
        let is_animating = Rc::new(std::cell::Cell::new(false));
        let last_frame_time = Rc::new(RefCell::new(None));
        let anim_gen = Rc::new(std::cell::Cell::new(0u64));

        let trigger_anim: Rc<dyn Fn()> = {
            let is_animating = is_animating.clone();
            let anim_gen = anim_gen.clone();
            let last_frame_time = last_frame_time.clone();
            let tick_state = state.clone();
            let area_clone_tick = drawing_area.clone();
            let window_clone_tick = window.clone();
            let menu_config_tick = menu_config.clone();

            Rc::new(move || {
                // Always request an immediate redraw so state changes made just
                // before triggering (e.g. swapping menus via IPC while visible)
                // are painted even when no hover animation ends up running.
                area_clone_tick.queue_draw();

                // A pending close must not wait on the paint-clock tick: if
                // that tick was scheduled while the widget was unmapped it
                // never fires, leaving is_closing and is_animating stranded
                // forever (and every later open toggling the wrong way).
                if tick_state.try_borrow().map(|s| s.is_closing).unwrap_or(false) {
                    complete_close(
                        &tick_state,
                        &window_clone_tick,
                        &menu_config_tick,
                        &is_animating,
                    );
                    return;
                }

                if is_animating.get() {
                    return;
                }
                is_animating.set(true);
                *last_frame_time.borrow_mut() = None;
                let my_gen = {
                    let g = anim_gen.get() + 1;
                    anim_gen.set(g);
                    g
                };

                let state_tick = tick_state.clone();
                let area_tick = area_clone_tick.clone();
                let win_tick = window_clone_tick.clone();
                let config_tick = menu_config_tick.clone();
                let anim_flag = is_animating.clone();
                let gen_flag = anim_gen.clone();
                let lft = last_frame_time.clone();

                area_clone_tick.add_tick_callback(move |_widget, frame_clock| {
                    // A newer trigger owns the animation now: retire
                    // silently without touching the shared flag.
                    if gen_flag.get() != my_gen {
                        return glib::ControlFlow::Break;
                    }

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
                        drop(state);
                        complete_close(&state_tick, &win_tick, &config_tick, &anim_flag);
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

                    // Animated icons (GIFs) keep the frame clock ticking and
                    // force redraws so their frames advance while visible.
                    if state.has_visible_animation() {
                        still_animating = true;
                        needs_redraw = true;
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
        let motion_window = window.clone();
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

            // Settings-hotspot hover feedback: swap to the "hand" cursor
            // while the pointer sits in the invisible corner zone.
            let over_hotspot = state.settings_hotspot_hit(x, y, width, height);
            let cursor_changed = over_hotspot != state.hotspot_hovered;
            state.hotspot_hovered = over_hotspot;

            let origin_set = state.note_pointer(x, y);

            let n_disp = state.get_display_items().len();
            let (cx, cy) = state.menu_center(width, height, n_disp);
            let hovered = match state.breadcrumb_hit(x, y, cx, cy, n_disp) {
                // Hovering a breadcrumb disc cues the Back wedge (it also
                // backs up one step via the marking dwell); with the Back
                // entry hidden there is nothing to cue, just suppress.
                Some(_) if !state.hide_back_entry && n_disp > 0 => Some(n_disp - 1),
                Some(_) => None,
                None => state.hit_test(x, y, cx, cy),
            };
            let mut hovered_changed = false;

            if state.hovered_index != hovered {
                state.hovered_index = hovered;
                hovered_changed = true;
            }

            // Marking mode: promote a held press into a marking session
            // once the pointer has travelled far enough (or been held long
            // enough), then (re)arm the submenu dwell on each hover change.
            let mut dwell_for: Option<usize> = None;
            if state.marking_mode && state.marking_pressed && !state.is_closing {
                if !state.marking_active {
                    if let (Some((px, py)), Some(t0)) =
                        (state.marking_press_pos, state.marking_press_time)
                    {
                        let s = state.scale.max(0.01);
                        let dist = ((x - px).powi(2) + (y - py).powi(2)).sqrt();
                        if dist >= MARKING_TRIGGER_DIST * s
                            || t0.elapsed() >= std::time::Duration::from_millis(MARKING_TRIGGER_MS)
                        {
                            state.marking_active = true;
                        }
                    }
                }
                if state.marking_active && hovered_changed {
                    if let Some(id) = state.marking_dwell.take() {
                        id.remove();
                    }
                    if let Some(i) = hovered {
                        if state.marking_dwell_target(i) {
                            dwell_for = Some(i);
                        }
                    }
                }
            }

            drop(state);
            if cursor_changed {
                motion_window
                    .set_cursor_from_name(if over_hotspot { Some("pointer") } else { Some("default") });
            }
            if let Some(i) = dwell_for {
                schedule_marking_dwell(&motion_state, &area_clone, i, trigger_anim_motion.clone());
            }
            if hovered_changed || origin_set {
                trigger_anim_motion();
            }
        });

        // The wl_pointer.enter delivered when the overlay maps carries the
        // cursor position at the exact moment the menu was launched.
        let enter_state = state.clone();
        let area_clone_enter = drawing_area.clone();
        let enter_window = window.clone();
        motion_controller.connect_enter(move |_ctrl, x, y| {
            let mut state = match enter_state.try_borrow_mut() {
                Ok(s) => s,
                Err(_) => return,
            };
            if state.is_closing {
                return;
            }
            let over_hotspot = state.settings_hotspot_hit(
                x,
                y,
                area_clone_enter.width() as f64,
                area_clone_enter.height() as f64,
            );
            state.hotspot_hovered = over_hotspot;
            let reveal = state.note_pointer(x, y);
            drop(state);
            // Always apply, not just on hover: the hand cursor may have
            // survived a hide/show cycle from a previous session.
            enter_window.set_cursor_from_name(if over_hotspot {
                Some("pointer")
            } else {
                Some("default")
            });
            if reveal {
                area_clone_enter.queue_draw();
            }
        });

        let leave_state = state.clone();
        let leave_window = window.clone();
        let trigger_anim_leave = trigger_anim.clone();
        motion_controller.connect_leave(move |_ctrl| {
            let mut hovered_changed = false;
            let mut cursor_reset = false;
            if let Ok(mut state) = leave_state.try_borrow_mut() {
                state.end_marking();
                if state.hotspot_hovered {
                    state.hotspot_hovered = false;
                    cursor_reset = true;
                }
                if state.hovered_index.is_some() {
                    state.hovered_index = None;
                    hovered_changed = true;
                }
            }
            if cursor_reset {
                leave_window.set_cursor_from_name(Some("default"));
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

        let press_state = state.clone();
        click_controller.connect_pressed(move |gesture, _n_press, x, y| {
            if gesture.current_button() != 1 {
                return;
            }
            if let Ok(mut state) = press_state.try_borrow_mut() {
                if !state.marking_mode || state.is_closing {
                    return;
                }
                // Start tracking a potential marking session; it only
                // becomes one once the pointer travels MARKING_TRIGGER_DIST
                // or the button has been held long enough (motion handler).
                state.end_marking();
                state.marking_pressed = true;
                state.marking_press_pos = Some((x, y));
                state.marking_press_time = Some(std::time::Instant::now());
            }
        });

        click_controller.connect_released(move |gesture, _n_press, x, y| {
            let button = gesture.current_button();

            let mut state = match click_state.try_borrow_mut() {
                Ok(s) => s,
                Err(_) => return,
            };

            if button == 3 {
                // Right click goes back, or dismisses launcher if at root
                state.end_marking();
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

                // Ending a marking session: the commit-at-release logic
                // below is exactly what marking wants (hovered slice at
                // the release point is activated, hub = back, outside =
                // close), so nothing else changes.
                state.end_marking();

                let width = area_clone_click.width() as f64;
                let height = area_clone_click.height() as f64;
                let (cx, cy) = state.menu_center(width, height, state.get_display_items().len());

                let s = state.scale.max(0.01);
                let mx = (x - cx) / s;
                let my = (y - cy) / s;
                let dist = (mx * mx + my * my).sqrt();

                let mut activated = false;

                // Invisible corner hotspot: re-executes this same binary
                // with the `settings` subcommand (no path assumptions; the
                // settings GApplication id keeps it single-instance) and
                // closes the overlay.
                if state.settings_hotspot_hit(x, y, width, height) {
                    match std::env::current_exe() {
                        Ok(exe) => {
                            let _ = std::process::Command::new(exe).arg("settings").spawn();
                        }
                        Err(e) => {
                            warn!("Could not resolve own executable path for settings: {}", e);
                        }
                    }
                    state.is_closing = true;
                    activated = true;
                } else if let Some(depth) =
                    state.breadcrumb_hit(x, y, cx, cy, state.get_display_items().len())
                {
                    // Breadcrumb disc: jump straight back that many levels
                    for _ in 0..=depth {
                        if !go_back(&mut state, &area_clone_click) {
                            break;
                        }
                    }
                    activated = true;
                } else if dist < BASE_R {
                    // Center hub click - goes back in history if not at root
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

        // System tray (StatusNotifierItem). Lifetime tied to this process:
        // while the daemon runs the icon shows; on exit the bus name drops
        // and the host removes it. Left-click opens settings; the menu has
        // Open Settings / Exit.
        tray::spawn_tray(ipc_tx.clone());

        // Monitor config and menu files using exact same logic as theme_editor.
        // Both share one debounce window: a settings save touches the menu
        // file *and* config.toml and each write emits several monitor events;
        // without this the daemon ran the (expensive) full reload once per
        // event, re-decoding every icon back-to-back.
        let pending_file_reload = Rc::new(std::cell::Cell::new(false));

        let config_file = gtk::gio::File::for_path(&config_path);
        let ipc_tx_config = ipc_tx.clone();
        let pending_cfg = pending_file_reload.clone();
        if let Ok(monitor) = config_file.monitor_file(
            gtk::gio::FileMonitorFlags::NONE,
            gtk::gio::Cancellable::NONE,
        ) {
            monitor.connect_changed(move |_, _, _, event| {
                if event == gtk::gio::FileMonitorEvent::ChangesDoneHint
                    || event == gtk::gio::FileMonitorEvent::Created
                {
                    if !pending_cfg.get() {
                        pending_cfg.set(true);
                        let tx = ipc_tx_config.clone();
                        let pending = pending_cfg.clone();
                        gtk::glib::timeout_add_local(
                            std::time::Duration::from_millis(150),
                            move || {
                                pending.set(false);
                                let _ = tx.send(IpcMessage::ReloadConfig);
                                glib::ControlFlow::Break
                            },
                        );
                    }
                }
            });
            if let Ok(mut s) = state.try_borrow_mut() {
                s._config_monitor = Some(monitor);
            }
        }

        let menu_file = gtk::gio::File::for_path(&menu_path);
        let ipc_tx_menu = ipc_tx.clone();
        let pending_menu = pending_file_reload.clone();
        if let Ok(monitor) = menu_file.monitor_file(
            gtk::gio::FileMonitorFlags::NONE,
            gtk::gio::Cancellable::NONE,
        ) {
            monitor.connect_changed(move |_, _, _, event| {
                if event == gtk::gio::FileMonitorEvent::ChangesDoneHint
                    || event == gtk::gio::FileMonitorEvent::Created
                {
                    if !pending_menu.get() {
                        pending_menu.set(true);
                        let tx = ipc_tx_menu.clone();
                        let pending = pending_menu.clone();
                        gtk::glib::timeout_add_local(
                            std::time::Duration::from_millis(150),
                            move || {
                                pending.set(false);
                                let _ = tx.send(IpcMessage::ReloadConfig);
                                glib::ControlFlow::Break
                            },
                        );
                    }
                }
            });
            if let Ok(mut s) = state.try_borrow_mut() {
                s._menu_monitor = Some(monitor);
            }
        }

        let ipc_state = state.clone();
        let ipc_area = drawing_area.clone();
        let theme_provider_clone = theme_provider.clone();
        let user_provider_clone = user_provider.clone();
        let config_path_clone = config_path.clone();
        let trigger_anim_ipc = trigger_anim.clone();

        let window_clone_ipc = window.clone();
        let app_clone_ipc = app.clone();
        let server_handle_ipc = server_handle_wrapper.clone();
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
                            let reveal_seq = state.arm_cursor_capture();
                            *state.theme_colors.borrow_mut() = None;
                            drop(state);
                            schedule_reveal_fallback(&ipc_state, &ipc_area, reveal_seq);

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
                    IpcMessage::Quit => {
                        info!("Quitting via IPC (tray Exit)");
                        window_clone_ipc.hide();
                        // Release the socket first so a fresh instance can
                        // bind immediately even if teardown lags.
                        if let Some(handle) = server_handle_ipc.lock().unwrap().take() {
                            handle.shutdown();
                        }
                        app_clone_ipc.quit();
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
                            let reveal_seq = state.arm_cursor_capture();
                            *state.theme_colors.borrow_mut() = None;
                            drop(state);
                            schedule_reveal_fallback(&ipc_state, &ipc_area, reveal_seq);

                            load_and_apply_theme(
                                &config_path_clone,
                                &theme_provider_clone,
                                &user_provider_clone,
                            );
                            window_clone_ipc.present();
                            trigger_anim_ipc();
                        } else if !same_menu {
                            // Swapping menus while visible: the surface never
                            // unmapped, so re-anchor to the live cursor.
                            state.origin = state.pointer_pos;
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
                            let mut reveal_seq = None;
                            if let Ok(mut state) = ipc_state.try_borrow_mut() {
                                state.reset_to_root();
                                if let Some(display) = gdk::Display::default() {
                                    state.preload_icons(&display);
                                }
                                state.is_closing = false;
                                reveal_seq = Some(state.arm_cursor_capture());
                                *state.theme_colors.borrow_mut() = None;
                            }
                            if let Some(seq) = reveal_seq {
                                schedule_reveal_fallback(&ipc_state, &ipc_area, seq);
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
                            state.scale = cfg.ui.scale;
                            state.extra_radius = cfg.ui.extra_radius;
                            state.enable_pie_spacing = cfg.ui.enable_pie_spacing;
                            state.pill_roundness = cfg.ui.pill_roundness;
                            // Only drop caches that the changed setting can
                            // actually invalidate. Wiping icon_cache on every
                            // reload forced a synchronous re-decode of every
                            // image icon (very slow on image-heavy menus).
                            if state.use_symbolic_icons != cfg.ui.use_symbolic_icons {
                                state.use_symbolic_icons = cfg.ui.use_symbolic_icons;
                                state.icon_cache.clear();
                            }
                            if state.bold_single_chars != cfg.ui.bold_single_chars {
                                state.bold_single_chars = cfg.ui.bold_single_chars;
                                state.text_layout_cache.clear();
                            }
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
                            state.hide_back_entry = cfg.ui.hide_back_entry;
                            state.spawn_at_cursor = cfg.ui.spawn_at_cursor;
                            if state.marking_mode != cfg.ui.marking_mode {
                                state.marking_mode = cfg.ui.marking_mode;
                                state.end_marking();
                            }
                            state.marking_dwell_ms = cfg.ui.marking_dwell_ms.max(30);
                            state.submenu_shift = cfg.ui.submenu_shift;
                            state.show_breadcrumbs = cfg.ui.show_breadcrumbs;
                            state.settings_hotspot_corner =
                                cfg.ui.settings_hotspot_corner.clone();
                            let new_blur = cfg.ui.enable_blur
                                && cfg.ui.menu_style != "floating"
                                && cfg.ui.menu_style != "floating-icons";
                            if state.enable_blur != new_blur {
                                state.enable_blur = new_blur;
                                blur_needs_update = true;
                            }
                            // CSS colors may have changed; text/label layout
                            // caches are keyed by content+size and survive.
                            // Icon surfaces are re-preloaded below, after the
                            // menu file itself has been re-read.
                            *state.theme_colors.borrow_mut() = None;
                            info!(
                                "Reloaded scale: {}, extra_radius: {}",
                                state.scale, state.extra_radius
                            );
                        }

                        if blur_needs_update {
                            let regions = target_blur_regions(
                                state.enable_blur,
                                state.is_closing,
                                state.effective_pie_spacing(state.get_display_items().len()),
                                state.scale,
                            );
                            // The WaylandBlur is normally created at realize time,
                            // only when blur is already enabled. If it was enabled
                            // at runtime (e.g. switching to pie mode), create it now.
                            if state.enable_blur && wayland_blur.borrow().is_none() {
                                if let Some(blur) = wayland::WaylandBlur::new(&window_clone_ipc) {
                                    *wayland_blur.borrow_mut() = Some(blur);
                                }
                            }
                            let (cx, cy) = state.menu_center(
                                window_clone_ipc.width() as f64,
                                window_clone_ipc.height() as f64,
                                state.get_display_items().len(),
                            );
                            state.last_cx = cx;
                            state.last_cy = cy;
                            if let Some(blur) = wayland_blur.borrow().as_ref() {
                                blur.update_sectioned_region(cx, cy, &regions);
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
            let mut reveal_seq = None;
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                state_mut.is_closing = false;
                reveal_seq = Some(state_mut.arm_cursor_capture());
            }
            if let Some(seq) = reveal_seq {
                schedule_reveal_fallback(&state, &drawing_area, seq);
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
