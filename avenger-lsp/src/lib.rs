//! Native Language Server Protocol transport for Avenger.
//!
//! Language intelligence belongs in `avenger-lang-analysis`; this crate owns
//! only protocol lifecycle, capability negotiation, and wire conversion.

use tower_lsp_server::{LanguageServer, LspService, Server, jsonrpc, ls_types::*};

const SERVER_NAME: &str = "avenger-lsp";

#[derive(Debug, Default)]
struct Backend;

impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> jsonrpc::Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                position_encoding: Some(negotiate_position_encoding(&params.capabilities)),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: SERVER_NAME.to_owned(),
                version: Some(env!("CARGO_PKG_VERSION").to_owned()),
            }),
            ..InitializeResult::default()
        })
    }

    async fn shutdown(&self) -> jsonrpc::Result<()> {
        Ok(())
    }
}

/// Run the native Avenger language server over standard input and output.
///
/// Standard output is reserved exclusively for LSP framing. Callers must send
/// human-readable logs to standard error or through LSP client notifications.
pub async fn run_stdio() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let (service, socket) = LspService::new(|_| Backend);
    Server::new(stdin, stdout, socket).serve(service).await;
}

fn negotiate_position_encoding(capabilities: &ClientCapabilities) -> PositionEncodingKind {
    capabilities
        .general
        .as_ref()
        .and_then(|general| general.position_encodings.as_ref())
        .filter(|encodings| {
            encodings
                .iter()
                .any(|encoding| encoding == &PositionEncodingKind::UTF8)
        })
        .map_or(PositionEncodingKind::UTF16, |_| PositionEncodingKind::UTF8)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tower::{Service, ServiceExt};
    use tower_lsp_server::{LspService, jsonrpc::Request, ls_types::*};

    use super::Backend;

    #[tokio::test]
    async fn initializes_and_shuts_down_in_memory() {
        let (mut service, _socket) = LspService::new(|_| Backend);
        let initialize = Request::build("initialize")
            .id(1)
            .params(json!({
                "capabilities": {
                    "general": { "positionEncodings": ["utf-8", "utf-16"] }
                }
            }))
            .finish();
        let response = service
            .ready()
            .await
            .expect("initialize service ready")
            .call(initialize)
            .await
            .expect("initialize service call")
            .expect("initialize response");
        let result: InitializeResult = serde_json::from_value(
            serde_json::to_value(response.result().expect("initialize result"))
                .expect("serialize initialize result"),
        )
        .expect("deserialize initialize result");
        assert_eq!(
            result.capabilities.position_encoding,
            Some(PositionEncodingKind::UTF8)
        );
        assert_eq!(
            result.server_info.as_ref().map(|info| info.name.as_str()),
            Some("avenger-lsp")
        );

        let shutdown = Request::build("shutdown").id(2).finish();
        let response = service
            .ready()
            .await
            .expect("shutdown service ready")
            .call(shutdown)
            .await
            .expect("shutdown service call")
            .expect("shutdown response");
        assert!(response.is_ok());
    }

    #[tokio::test]
    async fn defaults_to_utf16_when_client_omits_position_encodings() {
        let (mut service, _socket) = LspService::new(|_| Backend);
        let initialize = Request::build("initialize")
            .id(1)
            .params(json!({ "capabilities": {} }))
            .finish();
        let response = service
            .ready()
            .await
            .expect("initialize service ready")
            .call(initialize)
            .await
            .expect("initialize service call")
            .expect("initialize response");
        let result: InitializeResult = serde_json::from_value(
            serde_json::to_value(response.result().expect("initialize result"))
                .expect("serialize initialize result"),
        )
        .expect("deserialize initialize result");
        assert_eq!(
            result.capabilities.position_encoding,
            Some(PositionEncodingKind::UTF16)
        );
    }
}
