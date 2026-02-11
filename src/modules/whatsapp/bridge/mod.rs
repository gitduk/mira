pub mod process;
pub mod protocol;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tracing::{error, info, warn};

use process::BridgeProcess;
use protocol::{BridgeCommand, BridgeEvent};

use crate::error::Result;

const RESTART_DELAY_SECS: u64 = 5;

/// Shared handle for sending commands to the bridge process.
/// Updated each time a new bridge process spawns.
#[derive(Clone)]
pub struct BridgeCommandSender {
    inner: Arc<Mutex<Option<mpsc::Sender<BridgeCommand>>>>,
}

impl BridgeCommandSender {
    fn new() -> Self {
        BridgeCommandSender {
            inner: Arc::new(Mutex::new(None)),
        }
    }

    fn set(&self, tx: mpsc::Sender<BridgeCommand>) {
        *self.inner.lock().unwrap() = Some(tx);
    }

    fn clear(&self) {
        *self.inner.lock().unwrap() = None;
    }

    pub async fn send(&self, cmd: BridgeCommand) -> Result<()> {
        let tx = self.inner.lock().unwrap().clone();
        if let Some(tx) = tx {
            tx.send(cmd)
                .await
                .map_err(|e| crate::error::MiraError::Bridge(format!("Send failed: {}", e)))?;
        } else {
            warn!("No active bridge connection, command dropped");
        }
        Ok(())
    }
}

pub struct BridgeManager {
    bridge_dir: PathBuf,
    store_dir: PathBuf,
    event_tx: mpsc::Sender<BridgeEvent>,
    cmd_sender: BridgeCommandSender,
    shutdown: bool,
}

impl BridgeManager {
    pub fn new(
        bridge_dir: PathBuf,
        store_dir: PathBuf,
        event_tx: mpsc::Sender<BridgeEvent>,
    ) -> Self {
        BridgeManager {
            bridge_dir,
            store_dir,
            event_tx,
            cmd_sender: BridgeCommandSender::new(),
            shutdown: false,
        }
    }

    /// Get a cloneable command sender handle.
    /// Can be called before `run()` — commands will be forwarded once connected.
    pub fn command_sender(&self) -> BridgeCommandSender {
        self.cmd_sender.clone()
    }

    /// Start the bridge and manage its lifecycle (auto-restart on crash)
    pub async fn run(&mut self) -> Result<()> {
        loop {
            if self.shutdown {
                break;
            }

            info!("Starting bridge process...");
            match BridgeProcess::spawn(&self.bridge_dir, &self.store_dir).await {
                Ok((mut process, mut event_rx)) => {
                    // Set up command forwarding
                    let (cmd_tx, mut cmd_rx) = mpsc::channel::<BridgeCommand>(32);
                    self.cmd_sender.set(cmd_tx);

                    // Forward events from bridge to main loop
                    let event_tx = self.event_tx.clone();
                    let mut event_handle = tokio::spawn(async move {
                        while let Some(event) = event_rx.recv().await {
                            if event_tx.send(event).await.is_err() {
                                break;
                            }
                        }
                    });

                    // Forward commands from main loop to bridge
                    let mut cmd_handle = tokio::spawn(async move {
                        while let Some(cmd) = cmd_rx.recv().await {
                            if let Err(e) = process.send_command(&cmd).await {
                                error!("Failed to send command to bridge: {}", e);
                                break;
                            }
                        }
                        let _ = process.shutdown().await;
                    });

                    // Wait for either task to finish (bridge died), then abort the other
                    tokio::select! {
                        _ = &mut event_handle => {
                            warn!("Bridge event stream ended");
                            cmd_handle.abort();
                        }
                        _ = &mut cmd_handle => {
                            warn!("Bridge command stream ended");
                            event_handle.abort();
                        }
                    }

                    self.cmd_sender.clear();
                }
                Err(e) => {
                    error!("Failed to start bridge: {}", e);
                }
            }

            if self.shutdown {
                break;
            }

            info!(
                "Bridge process ended, restarting in {}s...",
                RESTART_DELAY_SECS
            );
            tokio::time::sleep(tokio::time::Duration::from_secs(RESTART_DELAY_SECS)).await;
        }

        Ok(())
    }

    pub async fn send_command(&self, cmd: BridgeCommand) -> Result<()> {
        self.cmd_sender.send(cmd).await
    }

    pub fn request_shutdown(&mut self) {
        self.shutdown = true;
    }
}
