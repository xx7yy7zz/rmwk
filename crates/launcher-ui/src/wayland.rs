use gdk4_wayland::prelude::*;
use gdk4_wayland::{WaylandDisplay, WaylandSurface};
use gtk4::prelude::*;
use wayland_backend::sys::client::Backend as SysBackend;
use wayland_backend::sys::client::ObjectId as SysObjectId;
use wayland_client::backend::Backend;
use wayland_client::protocol::{
    wl_compositor::WlCompositor, wl_region::WlRegion, wl_registry, wl_surface::WlSurface,
};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
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
        let region = self.compositor.create_region(&self.qh, ());

        // Shrink the blur region by 1 pixel so the jagged aliased edges
        // hide safely behind the anti-aliased Cairo stroke
        let r = (radius - 1.0).max(0.0) as i32;
        let cx = center_x as i32;
        let cy = center_y as i32;
        
        for y in -r..=r {
            let x = (((r * r - y * y) as f64).sqrt().round()) as i32;
            if x > 0 {
                let rect_x = cx - x;
                let rect_y = cy + y;
                let width = x * 2;
                let height = 1;
                region.add(rect_x, rect_y, width, height);
            }
        }

        self.effect_surface.set_blur_region(Some(&region));
        region.destroy();
        // Since we are using a foreign display, we should flush the connection
        let _ = self.conn.flush();
        tracing::debug!("Wayland: Updated circular blur region to radius {}", radius);
    }
}
