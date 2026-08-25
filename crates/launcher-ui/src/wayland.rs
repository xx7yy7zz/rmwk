use gdk4_wayland::prelude::*;
use gdk4_wayland::{WaylandDisplay, WaylandSurface};
use gtk4::prelude::*;
use wayland_backend::sys::client::Backend as SysBackend;
use wayland_backend::sys::client::ObjectId as SysObjectId;
use wayland_client::protocol::{
    wl_compositor::WlCompositor, wl_region::WlRegion, wl_registry, wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_manager_v1::ExtBackgroundEffectManagerV1;
use wayland_protocols::ext::background_effect::v1::client::ext_background_effect_surface_v1::ExtBackgroundEffectSurfaceV1;

pub struct WaylandState {
    pub manager: Option<ExtBackgroundEffectManagerV1>,
    pub compositor: Option<WlCompositor>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for WaylandState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "ext_background_effect_manager_v1" {
                state.manager = Some(registry.bind::<ExtBackgroundEffectManagerV1, _, _>(
                    name,
                    version.min(1),
                    qh,
                    (),
                ));
            } else if interface == "wl_compositor" {
                state.compositor =
                    Some(registry.bind::<WlCompositor, _, _>(name, version.min(4), qh, ()));
            }
        }
    }
}

impl Dispatch<ExtBackgroundEffectManagerV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &ExtBackgroundEffectManagerV1,
        _: <ExtBackgroundEffectManagerV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlCompositor, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &WlCompositor,
        _: <WlCompositor as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ExtBackgroundEffectSurfaceV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &ExtBackgroundEffectSurfaceV1,
        _: <ExtBackgroundEffectSurfaceV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlRegion, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &WlRegion,
        _: <WlRegion as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

pub struct WaylandBlur {
    compositor: WlCompositor,
    effect_surface: ExtBackgroundEffectSurfaceV1,
    qh: QueueHandle<WaylandState>,
    conn: Connection,
}

impl WaylandBlur {
    pub fn new(window: &gtk4::ApplicationWindow) -> Option<Self> {
        let display_binding = gtk4::prelude::WidgetExt::display(window);
        let display = display_binding.downcast_ref::<WaylandDisplay>()?;
        let surface = window
            .surface()
            .and_then(|s| s.downcast::<WaylandSurface>().ok())?;

        let wl_display_ptr = display.wl_display_raw()?.as_ptr();
        let wl_surface_ptr = surface.wl_surface_raw()?.as_ptr();

        unsafe {
            let backend = SysBackend::from_foreign_display(wl_display_ptr as *mut _);
            let conn = Connection::from_backend(backend.into());

            let mut event_queue = conn.new_event_queue();
            let qh = event_queue.handle();

            let display_proxy = conn.display();
            display_proxy.get_registry(&qh, ());

            let mut state = WaylandState {
                manager: None,
                compositor: None,
            };

            event_queue.roundtrip(&mut state).ok()?;

            if state.manager.is_none() {
                tracing::warn!("Wayland: ext_background_effect_manager_v1 not found!");
            }
            if state.compositor.is_none() {
                tracing::warn!("Wayland: wl_compositor not found!");
            }

            let manager = state.manager?;
            let compositor = state.compositor?;

            tracing::info!("Wayland: Successfully bound ext_background_effect_manager_v1");

            let surface_id =
                SysObjectId::from_ptr(WlSurface::interface(), wl_surface_ptr as *mut _).unwrap();
            let wl_surface = WlSurface::from_id(&conn, surface_id).unwrap();

            let effect_surface = manager.get_background_effect(&wl_surface, &qh, ());

            Some(Self {
                compositor,
                effect_surface,
                qh,
                conn,
            })
        }
    }

    pub fn update_circular_region(&self, radius: f64, center_x: f64, center_y: f64) {
        self.update_sectioned_region(center_x, center_y, &[(radius, None)]);
    }

    /// Sets the blur region as a union of concentric sections around
    /// (center_x, center_y). Each section is (outer_radius, inner_radius):
    /// `None` inner radius yields a solid disc, `Some(r)` an annulus.
    pub fn update_sectioned_region(
        &self,
        center_x: f64,
        center_y: f64,
        sections: &[(f64, Option<f64>)],
    ) {
        let max_outer = sections
            .iter()
            .map(|s| s.0)
            .fold(0.0f64, f64::max);
        if max_outer <= 0.0 {
            self.effect_surface.set_blur_region(None);
            let _ = self.conn.flush();
            tracing::debug!("Wayland: Cleared blur region");
            return;
        }

        let region = self.compositor.create_region(&self.qh, ());

        // Shrink each boundary by 1 pixel so jagged aliased edges hide
        // safely behind the anti-aliased Cairo strokes
        let cx = center_x as i32;
        let cy = center_y as i32;

        for &(outer, inner) in sections {
            if outer <= 0.0 {
                continue;
            }
            let r = (outer - 1.0).max(0.0);
            // Inner hole boundary grows by 1px for the same reason
            let r_hole = match inner {
                Some(ri) => (ri + 1.0).max(0.0),
                None => 0.0,
            };

            // Only the largest section keeps the elliptical top stretch
            let top_stretch = if (outer - max_outer).abs() < 0.01 { 1 } else { 0 };
            let r_top = r + top_stretch as f64;

            for y in -r_top as i32..=r as i32 {
                let y_f = y as f64;
                let half_w = if y < 0 {
                    // Top half: Elliptical stretch
                    let b = r_top;
                    (r * (1.0 - (y_f * y_f) / (b * b)).sqrt()).round()
                } else {
                    // Bottom half: Perfect circle
                    ((r * r - y_f * y_f).sqrt()).round()
                };

                if half_w <= 0.0 {
                    continue;
                }

                let hw = half_w as i32;
                let row_y = cy + y;

                let hole_w = if y_f.abs() < r_hole {
                    (r_hole * r_hole - y_f * y_f).sqrt().round()
                } else {
                    0.0
                };

                if hole_w > 0.0 {
                    // Annulus row: two horizontal spans flanking the hole
                    let hole = hole_w as i32;
                    if hole < hw {
                        let span = hw - hole;
                        region.add(cx - hw, row_y, span, 1);
                        region.add(cx + hole, row_y, span, 1);
                    }
                } else {
                    region.add(cx - hw, row_y, hw * 2, 1);
                }
            }
        }

        self.effect_surface.set_blur_region(Some(&region));
        region.destroy();
        // Since we are using a foreign display, flush the connection
        let _ = self.conn.flush();
        tracing::debug!(
            "Wayland: Updated sectioned blur region ({} sections)",
            sections.len()
        );
    }
}
