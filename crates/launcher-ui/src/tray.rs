//! StatusNotifierItem (system tray) support, implemented directly on zbus:
//! the daemon exports an `org.kde.StatusNotifierItem` plus a
//! `com.canonical.dbusmenu` object and registers them with the watcher
//! provided by the desktop's tray host (KDE/waybar/etc). The icon's
//! lifetime is the process' lifetime: when the daemon exits, the bus name
//! is dropped and the host removes the icon automatically.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use launcher_ipc::IpcMessage;
use tokio::sync::mpsc::UnboundedSender;
use tracing::{info, warn};
use zbus::zvariant::{Array, Dict, ObjectPath, OwnedValue, Signature, Structure, Value};

const MENU_PATH: &str = "/MenuBar";
const ITEM_PATH: &str = "/StatusNotifierItem";

const ITEM_SETTINGS: i32 = 1;
const ITEM_EXIT: i32 = 2;

fn sig(s: &str) -> Signature {
    s.parse::<Signature>().unwrap()
}

fn variant(v: Value<'static>) -> Value<'static> {
    Value::Value(Box::new(v))
}

fn owned<T: Into<Value<'static>>>(v: T) -> OwnedValue {
    OwnedValue::try_from(v.into()).unwrap()
}

fn leaf(id: i32, label: &'static str) -> OwnedValue {
    let mut props = Dict::new(&sig("s"), &sig("v"));
    props
        .append(Value::from("label"), variant(Value::from(label)))
        .unwrap();
    props
        .append(Value::from("enabled"), variant(Value::from(true)))
        .unwrap();
    props
        .append(Value::from("visible"), variant(Value::from(true)))
        .unwrap();
    let children = Array::new(&sig("v"));
    let st = zbus::zvariant::StructureBuilder::new()
        .add_field(id)
        .append_field(Value::Dict(props))
        .append_field(Value::Array(children))
        .build()
        .unwrap();
    OwnedValue::try_from(Value::Structure(st)).unwrap()
}

fn root_layout(revision: u32) -> (u32, MenuLayout) {
    let mut props = HashMap::new();
    props.insert(
        "children-display".to_string(),
        OwnedValue::try_from(Value::from("submenu")).unwrap(),
    );
    (
        revision,
        MenuLayout {
            id: 0,
            props,
            children: vec![leaf(ITEM_SETTINGS, "Open Settings"), leaf(ITEM_EXIT, "Exit")],
        },
    )
}

#[derive(Debug, serde::Serialize, serde::Deserialize, zbus::zvariant::Type)]
struct MenuLayout {
    id: i32,
    props: HashMap<String, OwnedValue>,
    children: Vec<OwnedValue>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, zbus::zvariant::Type)]
struct IconPixmap {
    width: i32,
    height: i32,
    data: Vec<u8>,
}

/// (s(sa(iiay)ss))
#[derive(Debug, serde::Serialize, serde::Deserialize, zbus::zvariant::Type)]
struct ToolTip {
    service: String,
    icon: (String, Vec<IconPixmap>, String, String),
}

// The zbus property derive converts struct-typed properties through
// Structure, so provide the (s(sa(iiay)ss)) assembly by hand.
impl From<ToolTip> for Structure<'static> {
    fn from(t: ToolTip) -> Self {
        let mut pixmaps = Array::new(&sig("(iiay)"));
        for p in t.icon.1 {
            let pm = zbus::zvariant::StructureBuilder::new()
                .add_field(p.width)
                .add_field(p.height)
                .append_field(Value::from(p.data))
                .build()
                .unwrap();
            pixmaps.append(Value::Structure(pm)).unwrap();
        }
        let icon = zbus::zvariant::StructureBuilder::new()
            .add_field(t.icon.0)
            .append_field(Value::Array(pixmaps))
            .add_field(t.icon.2)
            .add_field(t.icon.3)
            .build()
            .unwrap();
        zbus::zvariant::StructureBuilder::new()
            .add_field(t.service)
            .append_field(Value::Structure(icon))
            .build()
            .unwrap()
    }
}

#[derive(Clone)]
struct Shared {
    ipc_tx: UnboundedSender<IpcMessage>,
    revision: Arc<AtomicU32>,
    theme_path: Option<PathBuf>,
}

fn open_settings() {
    match std::env::current_exe() {
        Ok(exe) => {
            let _ = std::process::Command::new(exe).arg("settings").spawn();
        }
        Err(e) => warn!("tray: could not resolve own executable path: {}", e),
    }
}

// ---------------------------------------------------------------------------
// org.kde.StatusNotifierItem
// ---------------------------------------------------------------------------

struct SniItem {
    shared: Shared,
}

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl SniItem {
    fn activate(&self, _x: i32, _y: i32) {
        open_settings();
    }

    fn secondary_activate(&self, _x: i32, _y: i32) {}

    fn context_menu(&self, _x: i32, _y: i32) {}

    fn scroll(&self, _delta: i32, _orientation: &str) {}

    #[zbus(property)]
    fn category(&self) -> String {
        "ApplicationStatus".to_string()
    }

    #[zbus(property)]
    fn id(&self) -> String {
        "rmwk".to_string()
    }

    #[zbus(property)]
    fn title(&self) -> String {
        "rmwk launcher".to_string()
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "Active".to_string()
    }

    #[zbus(property)]
    fn icon_name(&self) -> String {
        "rmwk".to_string()
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        self.shared
            .theme_path
            .as_ref()
            .and_then(|p| p.to_str())
            .map(|s| vec![s.to_string()])
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn menu(&self) -> ObjectPath<'static> {
        ObjectPath::from_str_unchecked(MENU_PATH)
    }

    #[zbus(property)]
    fn tool_tip(&self) -> ToolTip {
        ToolTip {
            service: String::new(),
            icon: (
                "rmwk".to_string(),
                Vec::new(),
                "rmwk launcher".to_string(),
                "Running — click to open settings".to_string(),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// com.canonical.dbusmenu
// ---------------------------------------------------------------------------

struct TrayMenu {
    shared: Shared,
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl TrayMenu {
    fn get_layout(
        &self,
        _parent_id: i32,
        _recursion_depth: i32,
        _property_names: Vec<String>,
    ) -> (u32, MenuLayout) {
        root_layout(self.shared.revision.load(Ordering::SeqCst))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let (_, layout) = root_layout(0);
        let mut out = Vec::new();
        for id in ids {
            let props = if id == 0 {
                layout.props.clone()
            } else {
                let mut m = HashMap::new();
                let label = if id == ITEM_SETTINGS {
                    "Open Settings"
                } else {
                    "Exit"
                };
                m.insert("label".to_string(), owned(label));
                m.insert("enabled".to_string(), owned(true));
                m.insert("visible".to_string(), owned(true));
                m
            };
            let _ = &property_names;
            out.push((id, props));
        }
        out
    }

    fn get_property(&self, _id: i32, _name: &str) -> (bool, OwnedValue) {
        (false, owned(""))
    }

    fn event(&self, id: i32, event_id: &str, _data: OwnedValue, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        match id {
            ITEM_SETTINGS => open_settings(),
            ITEM_EXIT => {
                info!("tray: Exit requested, shutting down daemon");
                let _ = self.shared.ipc_tx.send(IpcMessage::Quit);
            }
            _ => {}
        }
    }

    fn event_send(&self, _id: i32, _event_id: &str, _data: OwnedValue, _timestamp: u32) {}

    fn about_show(&self) {}

    fn about_to_show(&self) -> bool {
        true
    }

    fn ping(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn status(&self) -> String {
        "normal".to_string()
    }

    #[zbus(property)]
    fn text_direction(&self) -> String {
        "ltr".to_string()
    }

    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        self.shared
            .theme_path
            .as_ref()
            .and_then(|p| p.to_str())
            .map(|s| vec![s.to_string()])
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn version(&self) -> i32 {
        3
    }
}

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Spawns the tray thread. The thread owns a blocking zbus connection: it
/// exports the SNI item + dbusmenu, registers with the watcher (retrying
/// until a tray host shows up), and then parks. Method calls are served by
/// zbus' internal connection thread.
pub fn spawn_tray(ipc_tx: UnboundedSender<IpcMessage>) {
    let handle = std::thread::Builder::new()
        .name("tray".to_string())
        .spawn(move || {
            let theme_path = launcher_core::install_user_icon();
            let shared = Shared {
                ipc_tx,
                revision: Arc::new(AtomicU32::new(1)),
                theme_path,
            };

            let conn = match zbus::blocking::Connection::session() {
                Ok(c) => c,
                Err(e) => {
                    warn!("tray: no session bus: {}", e);
                    return;
                }
            };

            let bus_name = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
            if let Err(e) = conn.request_name(bus_name.as_str()) {
                warn!("tray: failed to claim bus name: {}", e);
                return;
            }
            if let Err(e) = conn
                .object_server()
                .at(ITEM_PATH, SniItem { shared: shared.clone() })
            {
                warn!("tray: failed to export SNI item: {}", e);
                return;
            }
            if let Err(e) = conn.object_server().at(MENU_PATH, TrayMenu { shared }) {
                warn!("tray: failed to export dbusmenu: {}", e);
                return;
            }

            // Keep retrying until a tray host (watcher) is present; it may
            // start long after the daemon.
            loop {
                match conn.call_method(
                    Some("org.kde.StatusNotifierWatcher"),
                    "/StatusNotifierWatcher",
                    Some("org.kde.StatusNotifierWatcher"),
                    "RegisterStatusNotifierItem",
                    &bus_name.as_str(),
                ) {
                    Ok(_) => {
                        info!("tray: registered with StatusNotifierWatcher");
                        break;
                    }
                    Err(_) => std::thread::sleep(std::time::Duration::from_secs(5)),
                }
            }

            // Park forever; the connection thread serves the interfaces and
            // the OS tears everything down when the process exits.
            loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            }
        });
    if let Err(e) = handle {
        warn!("tray: failed to spawn tray thread: {}", e);
    }
}
