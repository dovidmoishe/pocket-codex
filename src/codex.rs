use crate::config::Config;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{collections::VecDeque, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    process::{Child, Command},
};

/// A private JSONL connection. Only the worker owns it: no socket exposed to ngrok.
pub struct Codex {
    _child: Option<Child>,
    input: Box<dyn AsyncWrite + Send + Unpin>,
    output: BufReader<Box<dyn AsyncRead + Send + Unpin>>,
    next_id: u64,
    notifications: VecDeque<Value>,
}

impl Codex {
    pub async fn start(config: &Config, directory: &std::path::Path) -> Result<Self> {
        let mut command = Command::new(config.executable());
        command
            .args(["app-server", "--listen", "stdio://"])
            .current_dir(directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .kill_on_drop(true);
        // Keep this creative client from becoming a remote shell or browser controller.
        for feature in [
            "shell_tool",
            "unified_exec",
            "browser_use",
            "browser_use_external",
            "computer_use",
            "apps",
            "multi_agent",
            "in_app_browser",
        ] {
            command.args(["--disable", feature]);
        }
        command.args(["--enable", "image_generation"]);
        if let Some(home) = &config.codex_home {
            command.env("CODEX_HOME", home);
        }
        #[cfg(windows)]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        let mut child = command
            .spawn()
            .context("could not start Codex; install the CLI or set --codex-bin")?;
        let input = child.stdin.take().context("Codex stdin is unavailable")?;
        let output = child.stdout.take().context("Codex stdout is unavailable")?;
        let mut connection = Self {
            _child: Some(child),
            input: Box::new(input),
            output: BufReader::new(Box::new(output)),
            next_id: 1,
            notifications: VecDeque::new(),
        };
        connection.rpc("initialize", json!({
            "clientInfo": { "name": "pocket_codex", "title": "Pocket Codex", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "experimentalApi": true }
        })).await?;
        connection
            .write(&json!({ "method": "initialized" }))
            .await?;
        Ok(connection)
    }

    async fn write(&mut self, value: &Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(value)?;
        bytes.push(b'\n');
        self.input.write_all(&bytes).await?;
        self.input.flush().await?;
        Ok(())
    }

    async fn read_wire(&mut self) -> Result<Value> {
        let mut line = String::new();
        loop {
            line.clear();
            if self.output.read_line(&mut line).await? == 0 {
                bail!("Codex disconnected; the next job will reconnect");
            }
            if line.len() > 24 * 1024 * 1024 {
                bail!("Codex response exceeded the image limit");
            }
            if !line.trim().is_empty() {
                break;
            }
        }
        serde_json::from_str(&line).context("Codex sent invalid JSON")
    }

    async fn reject_request(&mut self, message: &Value) -> Result<()> {
        // Fail closed for approvals, elicitation, and dynamically executed tools.
        // This app never silently grants an agent extra laptop permissions.
        self.write(&json!({"id": message["id"], "error": {
            "code": -32601, "message": "This creative web client does not grant approvals or execute client tools."
        }})).await
    }

    pub async fn rpc(&mut self, method: &str, params: Value) -> Result<Value> {
        tokio::time::timeout(Duration::from_secs(60), self.rpc_inner(method, params))
            .await
            .context("Codex request timed out")?
    }

    async fn rpc_inner(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(&json!({ "id": id, "method": method, "params": params }))
            .await?;
        loop {
            let message = self.read_wire().await?;
            if message.get("method").is_some() {
                if message.get("id").is_some() {
                    self.reject_request(&message).await?;
                } else {
                    if self.notifications.len() >= 256 {
                        bail!("too many buffered Codex events");
                    }
                    self.notifications.push_back(message);
                }
            } else if message["id"].as_u64() == Some(id) {
                if let Some(error) = message.get("error") {
                    bail!(
                        "Codex: {}",
                        error["message"].as_str().unwrap_or("request failed")
                    );
                }
                return message
                    .get("result")
                    .cloned()
                    .context("Codex reply has no result");
            }
            // Responses to an interrupted old request can safely be ignored.
        }
    }

    pub async fn event(&mut self) -> Result<Value> {
        if let Some(message) = self.notifications.pop_front() {
            return Ok(message);
        }
        loop {
            let message = self.read_wire().await?;
            if message.get("method").is_some() {
                if message.get("id").is_some() {
                    self.reject_request(&message).await?;
                } else {
                    return Ok(message);
                }
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn fixture(stream: tokio::io::DuplexStream) -> Self {
        let (read, write) = tokio::io::split(stream);
        Self {
            _child: None,
            input: Box::new(write),
            output: BufReader::new(Box::new(read)),
            next_id: 1,
            notifications: VecDeque::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rpc_preserves_early_events_and_rejects_server_tool_requests() {
        let (client, server) = tokio::io::duplex(8192);
        let fake = tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(server);
            let mut lines = BufReader::new(read).lines();
            let request: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            for message in [
                json!({"method":"item/started","params":{"item":{"type":"imageGeneration"}}}),
                json!({"id":99,"method":"item/commandExecution/requestApproval","params":{}}),
            ] {
                write
                    .write_all(format!("{message}\n").as_bytes())
                    .await
                    .unwrap();
            }
            let rejection: Value =
                serde_json::from_str(&lines.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(rejection["id"], 99);
            assert_eq!(rejection["error"]["code"], -32601);
            let response = json!({"id":request["id"],"result":{"turn":{"id":"turn-1"}}});
            write
                .write_all(format!("{response}\n").as_bytes())
                .await
                .unwrap();
        });
        let mut codex = Codex::fixture(client);
        let reply = codex.rpc("turn/start", json!({})).await.unwrap();
        assert_eq!(reply["turn"]["id"], "turn-1");
        assert_eq!(codex.event().await.unwrap()["method"], "item/started");
        fake.await.unwrap();
    }
}
