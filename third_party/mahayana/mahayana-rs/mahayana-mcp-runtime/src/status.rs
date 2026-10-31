use crate::{McpTransport, NativeMcpClient};

/// Observable state for the Mahayana-owned MCP client.
///
/// `NativeMcpClient` is intentionally connectionless between requests, so a
/// configured transport is the only durable lifecycle state. Request failures
/// are returned by the request that observed them instead of mutating status or
/// reconnecting as a side effect of inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpRuntimeState {
    Configured,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpTransportKind {
    Stdio,
    Http,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct McpRuntimeStatus {
    pub state: McpRuntimeState,
    pub transport: McpTransportKind,
}

impl NativeMcpClient {
    /// Returns a passive runtime snapshot.
    ///
    /// This method never starts a process, opens a socket, sends a request, or
    /// reconnects a transport. It is safe for UI/status polling.
    pub fn status(&self) -> McpRuntimeStatus {
        let transport = match &self.transport {
            McpTransport::Stdio { .. } => McpTransportKind::Stdio,
            McpTransport::Http { .. } => McpTransportKind::Http,
        };
        McpRuntimeStatus {
            state: McpRuntimeState::Configured,
            transport,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn stdio_status_is_passive_even_when_command_does_not_exist() {
        let client = NativeMcpClient::new(McpTransport::Stdio {
            command: PathBuf::from("/definitely/not/a/real/mahayana-mcp-server"),
            args: Vec::new(),
            cwd: PathBuf::from("."),
            env: BTreeMap::new(),
        });
        assert_eq!(
            client.status(),
            McpRuntimeStatus {
                state: McpRuntimeState::Configured,
                transport: McpTransportKind::Stdio,
            }
        );
    }

    #[test]
    fn http_status_does_not_contact_the_endpoint() {
        let client = NativeMcpClient::new(McpTransport::Http {
            url: "https://127.0.0.1:1/never-contact".into(),
            headers: BTreeMap::new(),
        });
        assert_eq!(client.status().transport, McpTransportKind::Http);
    }
}
