use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::fs;
use tokio::net::{UnixListener, UnixStream};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use anyhow::{Context, Result};
use tracing::{info, warn, debug, error};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "cmd", rename_all = "kebab-case")]
pub enum IpcMessage {
    Open,
    Close,
    Toggle,
    ReloadConfig,
}

pub fn get_socket_path() -> PathBuf {
    if let Ok(runtime_dir) = std::env::var("XDG_RUNTIME_DIR") {
        Path::new(&runtime_dir).join("rmwk.sock")
    } else {
        Path::new("/tmp").join("rmwk.sock")
    }
}

/// Send a single message to a running instance and close connection.
pub async fn send_message<P: AsRef<Path>>(socket_path: P, msg: &IpcMessage) -> Result<()> {
    let mut stream = UnixStream::connect(socket_path).await?;
    let mut serialized = serde_json::to_vec(msg)?;
    serialized.push(b'\n');
    stream.write_all(&serialized).await?;
    stream.flush().await?;
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
/// Whenever a message is received, it is sent to `event_tx`.
pub fn start_server<P, F>(socket_path: P, mut on_message: F) -> Result<ServerHandle>
where
    P: AsRef<Path> + Send + 'static,
    F: FnMut(IpcMessage) + Send + 'static,
{
    let path = socket_path.as_ref().to_path_buf();

    // Remove existing stale socket file
    if path.exists() {
        debug!("Cleaning up stale socket file: {:?}", path);
        let _ = fs::remove_file(&path);
    }

    let (shutdown_tx, mut shutdown_rx) = tokio::sync::oneshot::channel::<()>();

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
                                    // Handle connection asynchronously
                                    let mut reader = BufReader::new(stream);
                                    let mut line = String::new();
                                    match reader.read_line(&mut line).await {
                                        Ok(0) => {}, // EOF
                                        Ok(_) => {
                                            match serde_json::from_str::<IpcMessage>(&line.trim()) {
                                                Ok(msg) => {
                                                    debug!("Received IPC command: {:?}", msg);
                                                    on_message(msg);
                                                }
                                                Err(e) => {
                                                    warn!("Received invalid IPC message JSON: {} (raw: {:?})", e, line);
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            warn!("Error reading from IPC stream: {}", e);
                                        }
                                    }
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
