pub mod process;
pub mod protocol;

use std::path::PathBuf;

use tokio::sync::mpsc;
use tracing::{error, info, warn};

use process::BridgeProcess;
use protocol::{BridgeCommand, BridgeEvent};

use crate::error::Result;

const RESTART_DELAY_SECS: u64 = 5;

pub struct BridgeManager {
    bridge_dir: PathBuf,
    store_dir: PathBuf,
    event_tx: mpsc::Sender<BridgeEvent>,
    cmd_tx: Option<mpsc::Sender<BridgeCommand>>,
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
            cmd_tx: None,
            shutdown: false,
        }
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
                    self.cmd_tx = Some(cmd_tx);

                    // Forward events from bridge to main loop
                    let event_tx = self.event_tx.clone();
                    let event_handle = tokio::spawn(async move {
                        while let Some(event) = event_rx.recv().await {
                            if event_tx.send(event).await.is_err() {
                                break;
                            }
                        }
                    });

                    // Forward commands from main loop to bridge
                    let cmd_handle = tokio::spawn(async move {
                        while let Some(cmd) = cmd_rx.recv().await {
                            if let Err(e) = process.send_command(&cmd).await {
                                error!("Failed to send command to bridge: {}", e);
                                break;
                            }
                        }
                        let _ = process.shutdown().await;
                    });

                    // Wait for either task to finish (bridge died)
                    tokio::select! {
                        _ = event_handle => {
                            warn!("Bridge event stream ended");
                        }
                        _ = cmd_handle => {
                            warn!("Bridge command stream ended");
                        }
                    }

                    self.cmd_tx = None;
                }
                Err(e) => {
                    error!("Failed to start bridge: {}", e);
                }
            }

            if self.shutdown {
                break;
            }

            info!("Bridge process ended, restarting in {}s...", RESTART_DELAY_SECS);
            tokio::time::sleep(tokio::time::Duration::from_secs(RESTART_DELAY_SECS)).await;
        }

        Ok(())
    }

    pub async fn send_command(&self, cmd: BridgeCommand) -> Result<()> {
        if let Some(tx) = &self.cmd_tx {
            tx.send(cmd)
                .await
                .map_err(|e| crate::error::MiraError::Bridge(format!("Send failed: {}", e)))?;
        }
        Ok(())
    }

    pub fn request_shutdown(&mut self) {
        self.shutdown = true;
    }
}
