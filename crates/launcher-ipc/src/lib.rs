use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{debug, error, info, warn};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum IpcMessage {
    Open,
    Close,
    Toggle,
    ReloadConfig,
    OpenMenu { menu_path: PathBuf },
}

pub fn get_socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        Path::new(&runtime_dir).join("rmwk.sock")
    } else {
        #[cfg(unix)]
        let uid = unsafe { libc::getuid() };
        #[cfg(not(unix))]
        let uid = 1000;

        let run_user = format!("/run/user/{}", uid);
        if Path::new(&run_user).exists() {
            Path::new(&run_user).join("rmwk.sock")
        } else {
            Path::new("/tmp").join(format!("rmwk-{}.sock", uid))
        }
    }
}

/// Send a single message to a running instance asynchronously.
pub async fn send_message<P: AsRef<Path>>(socket_path: P, msg: &IpcMessage) -> Result<()> {
    let mut stream = UnixStream::connect(socket_path).await?;
    let mut serialized = serde_json::to_vec(msg)?;
    serialized.push(b'\n');
    stream.write_all(&serialized).await?;
    stream.flush().await?;
    Ok(())
}

/// Send a single message to a running instance synchronously with a timeout.
/// Avoids the overhead of spinning up an ephemeral Tokio runtime from CLI commands.
pub fn send_message_sync<P: AsRef<Path>>(socket_path: P, msg: &IpcMessage) -> Result<()> {
    use std::io::Write;
    use std::os::unix::net::UnixStream as StdUnixStream;

    let mut stream = StdUnixStream::connect(socket_path)?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    let mut serialized = serde_json::to_vec(msg)?;
    serialized.push(b'\n');
    stream.write_all(&serialized)?;
    stream.flush()?;
    Ok(())
}

/// A handle to shut down the server.
pub struct ServerHandle {
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

impl ServerHandle {
    pub fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
    }
}

/// Start an IPC listener server on the Unix socket.
///
/// Whenever a message is received, it is forwarded via `on_message`.
pub fn start_server<P, F>(socket_path: P, on_message: F) -> Result<ServerHandle>
where
    P: AsRef<Path> + Send + 'static,
    F: Fn(IpcMessage) + Send + Sync + 'static,
{
    let path = socket_path.as_ref().to_path_buf();

    // Check for stale socket file
    if path.exists() {
        debug!("Cleaning up stale socket file: {:?}", path);
        let _ = fs::remove_file(&path);
    }

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let on_message = Arc::new(on_message);

    // Start a background thread to run the Tokio event loop for IPC
    std::thread::Builder::new()
        .name("ipc-server".to_string())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    error!("Failed to build Tokio runtime for IPC server: {}", e);
                    return;
                }
            };

            rt.block_on(async {
                let listener = match UnixListener::bind(&path) {
                    Ok(l) => l,
                    Err(e) => {
                        error!("Failed to bind to Unix socket at {:?}: {}", path, e);
                        return;
                    }
                };
                info!("IPC server listening on {:?}", path);

                loop {
                    tokio::select! {
                        accept_res = listener.accept() => {
                            match accept_res {
                                Ok((stream, _addr)) => {
                                    let on_message_handler = on_message.clone();
                                    tokio::spawn(async move {
                                        let mut reader = BufReader::new(stream);
                                        let mut line = String::new();
                                        let read_future = reader.read_line(&mut line);

                                        match tokio::time::timeout(Duration::from_secs(2), read_future).await {
                                            Ok(Ok(0)) => {}, // EOF
                                            Ok(Ok(_)) => {
                                                match serde_json::from_str::<IpcMessage>(line.trim()) {
                                                    Ok(msg) => {
                                                        debug!("Received IPC command: {:?}", msg);
                                                        on_message_handler(msg);
                                                    }
                                                    Err(e) => {
                                                        warn!("Received invalid IPC message JSON: {} (raw: {:?})", e, line);
                                                    }
                                                }
                                            }
                                            Ok(Err(e)) => {
                                                warn!("Error reading from IPC stream: {}", e);
                                            }
                                            Err(_) => {
                                                warn!("IPC stream read timed out after 2s");
                                            }
                                        }
                                    });
                                }
                                Err(e) => {
                                    error!("Failed to accept Unix socket connection: {}", e);
                                }
                            }
                        }
                        _ = &mut shutdown_rx => {
                            info!("IPC server shutdown signal received");
                            break;
                        }
                    }
                }

                // Cleanup socket on exit
                if path.exists() {
                    let _ = fs::remove_file(&path);
                }
            });
        })
        .context("Failed to spawn IPC server thread")?;

    Ok(ServerHandle { shutdown_tx })
}
