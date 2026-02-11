use std::path::Path;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

use super::protocol::{BridgeCommand, BridgeEvent};
use crate::error::{MiraError, Result};

pub struct BridgeProcess {
    child: Child,
    stdin_tx: mpsc::Sender<String>,
}

impl BridgeProcess {
    pub async fn spawn(
        bridge_dir: &Path,
        store_dir: &Path,
    ) -> Result<(Self, mpsc::Receiver<BridgeEvent>)> {
        let bridge_script = bridge_dir.join("src/index.ts");

        // Use npx tsx or bun to run TypeScript directly
        let mut child = Command::new("npx")
            .args(["tsx", bridge_script.to_str().unwrap()])
            .current_dir(bridge_dir)
            .env("STORE_DIR", store_dir.to_str().unwrap())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| MiraError::Bridge(format!("Failed to spawn bridge: {}", e)))?;

        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let stdin = child.stdin.take().unwrap();

        // Channel for sending commands to bridge stdin
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(32);

        // Channel for receiving events from bridge stdout
        let (event_tx, event_rx) = mpsc::channel::<BridgeEvent>(100);

        // Stdin writer task
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(line) = stdin_rx.recv().await {
                if let Err(e) = stdin.write_all(line.as_bytes()).await {
                    error!("Failed to write to bridge stdin: {}", e);
                    break;
                }
                if let Err(e) = stdin.write_all(b"\n").await {
                    error!("Failed to write newline to bridge stdin: {}", e);
                    break;
                }
                if let Err(e) = stdin.flush().await {
                    error!("Failed to flush bridge stdin: {}", e);
                    break;
                }
            }
        });

        // Stdout reader task
        let event_tx_clone = event_tx.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim().to_string();
                if line.is_empty() {
                    continue;
                }
                match serde_json::from_str::<BridgeEvent>(&line) {
                    Ok(event) => {
                        if event_tx_clone.send(event).await.is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        warn!(
                            "Failed to parse bridge event: {} - line: {}",
                            e,
                            &line[..line.len().min(200)]
                        );
                    }
                }
            }
            info!("Bridge stdout reader ended");
        });

        // Stderr reader task (log only)
        tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if !line.trim().is_empty() {
                    warn!("[bridge stderr] {}", line);
                }
            }
        });

        Ok((BridgeProcess { child, stdin_tx }, event_rx))
    }

    pub async fn send_command(&self, cmd: &BridgeCommand) -> Result<()> {
        let json = serde_json::to_string(cmd)?;
        self.stdin_tx
            .send(json)
            .await
            .map_err(|e| MiraError::Bridge(format!("Failed to send command: {}", e)))?;
        Ok(())
    }

    pub async fn shutdown(&mut self) -> Result<()> {
        let _ = self.send_command(&BridgeCommand::Shutdown).await;
        // Give the process a moment to shut down gracefully
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        let _ = self.child.kill().await;
        Ok(())
    }

    pub fn try_wait(&mut self) -> Option<std::process::ExitStatus> {
        self.child.try_wait().ok().flatten()
    }
}
