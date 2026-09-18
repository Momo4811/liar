//! The language server.

use crate::workspace::Workspace;
use liar_core::messages::Tone;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use tokio::sync::Mutex;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

pub struct Backend {
    client: Client,
    workspace: Mutex<Workspace>,
}

impl Backend {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            workspace: Mutex::new(Workspace::new(Tone::Dry)),
        }
    }

    /// Re-analyses the workspace and publishes diagnostics for every file.
    ///
    /// Every file, not only the one just saved: editing `helpers.py` can create
    /// or clear a finding in `app.py`, which the editor is also showing.
    async fn republish(&self) {
        let workspace = self.workspace.lock().await;

        // An analysis panic must not take the editor down with it. A linter
        // that kills your editor when it meets an unmodelled construct is worse
        // than no linter, so the failure is reported and the session continues.
        let result = std::panic::catch_unwind(AssertUnwindSafe(|| workspace.diagnostics()));

        let diagnostics = match result {
            Ok(diagnostics) => diagnostics,
            Err(_) => {
                self.client
                    .log_message(
                        MessageType::ERROR,
                        "liar: analysis failed on this workspace; diagnostics are stale",
                    )
                    .await;
                return;
            }
        };

        for (uri, items) in diagnostics {
            self.client.publish_diagnostics(uri, items, None).await;
        }
    }

    /// Reads `liar.tone` out of whatever the editor sent, ignoring anything it
    /// does not understand.
    fn tone_from(options: Option<&serde_json::Value>) -> Option<Tone> {
        options?
            .get("liar")
            .and_then(|liar| liar.get("tone"))
            .or_else(|| options?.get("tone"))
            .and_then(|tone| tone.as_str())
            .and_then(|tone| tone.parse().ok())
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if let Some(tone) = Self::tone_from(params.initialization_options.as_ref()) {
            self.workspace.lock().await.set_tone(tone);
        }

        if let Some(root) = workspace_root(&params) {
            self.workspace.lock().await.open(&root);
        }

        Ok(InitializeResult {
            server_info: Some(ServerInfo {
                name: "liar".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
            capabilities: ServerCapabilities {
                // Full sync rather than incremental: the analysis re-derives
                // everything from text anyway, so tracking deltas would add a
                // way to be wrong without making anything faster.
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
                ..Default::default()
            },
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        let count = self.workspace.lock().await.len();
        self.client
            .log_message(MessageType::INFO, format!("liar: watching {count} files"))
            .await;
        self.republish().await;
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        if let Ok(path) = params.text_document.uri.to_file_path() {
            self.workspace
                .lock()
                .await
                .update(&path, params.text_document.text);
            self.republish().await;
        }
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        // The text is tracked so the buffer stays current, but analysis waits
        // for a save. Re-indexing a whole workspace on every keystroke is a
        // performance problem this does not take on, and half-typed code
        // produces findings that are noise.
        let Ok(path) = params.text_document.uri.to_file_path() else {
            return;
        };
        if let Some(change) = params.content_changes.into_iter().next_back() {
            self.workspace.lock().await.update(&path, change.text);
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let Ok(path) = params.text_document.uri.to_file_path() else {
            return;
        };
        // The editor may or may not include the text, depending on how it was
        // configured. Falling back to the file on disk means the server is
        // correct either way.
        let text = params.text.or_else(|| std::fs::read_to_string(&path).ok());
        if let Some(text) = text {
            self.workspace.lock().await.update(&path, text);
        }
        self.republish().await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        // The file stays in the workspace: it is still part of the project and
        // still affects what resolves in its neighbours. Only its unsaved
        // buffer is gone, and the copy on disk is what the analysis wants.
        if let Ok(path) = params.text_document.uri.to_file_path()
            && let Ok(text) = std::fs::read_to_string(&path)
        {
            self.workspace.lock().await.update(&path, text);
        }
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let Ok(path) = params.text_document.uri.to_file_path() else {
            return Ok(None);
        };

        let workspace = self.workspace.lock().await;
        let mut actions = Vec::new();

        for diagnostic in params.context.diagnostics {
            if !matches!(&diagnostic.code, Some(NumberOrString::String(code)) if code == "C1") {
                continue;
            }

            // Offered only inside an async def. Inserting await in a plain def
            // produces a syntax error, which is a worse outcome than the bug.
            let Some(offset) = workspace.offset_of(&path, diagnostic.range.start) else {
                continue;
            };
            if !workspace.is_inside_async_function(&path, offset) {
                continue;
            }

            let edit = TextEdit {
                range: Range {
                    start: diagnostic.range.start,
                    end: diagnostic.range.start,
                },
                new_text: "await ".to_string(),
            };

            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Add await".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(
                        [(params.text_document.uri.clone(), vec![edit])]
                            .into_iter()
                            .collect(),
                    ),
                    ..Default::default()
                }),
                is_preferred: Some(true),
                ..Default::default()
            }));
        }

        Ok((!actions.is_empty()).then_some(actions))
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }
}

fn workspace_root(params: &InitializeParams) -> Option<PathBuf> {
    #[allow(deprecated)]
    let from_root_uri = params
        .root_uri
        .as_ref()
        .and_then(|uri| uri.to_file_path().ok());

    from_root_uri.or_else(|| {
        params
            .workspace_folders
            .as_ref()?
            .first()?
            .uri
            .to_file_path()
            .ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tone_is_read_from_initialization_options() {
        let options = serde_json::json!({ "liar": { "tone": "brutal" } });
        assert_eq!(Backend::tone_from(Some(&options)), Some(Tone::Brutal));
    }

    #[test]
    fn a_bare_tone_key_also_works() {
        let options = serde_json::json!({ "tone": "professional" });
        assert_eq!(Backend::tone_from(Some(&options)), Some(Tone::Professional));
    }

    #[test]
    fn an_unrecognised_tone_is_ignored_rather_than_fatal() {
        // An editor sending nonsense should not stop the server starting.
        let options = serde_json::json!({ "liar": { "tone": "sarcastic" } });
        assert_eq!(Backend::tone_from(Some(&options)), None);
    }

    #[test]
    fn absent_options_mean_the_default() {
        assert_eq!(Backend::tone_from(None), None);
        let options = serde_json::json!({ "something": "else" });
        assert_eq!(Backend::tone_from(Some(&options)), None);
    }

    #[test]
    fn the_workspace_root_comes_from_root_uri() {
        let dir = tempfile::tempdir().unwrap();
        let uri = Url::from_directory_path(dir.path()).unwrap();

        #[allow(deprecated)]
        let params = InitializeParams {
            root_uri: Some(uri),
            ..Default::default()
        };

        let found = workspace_root(&params).expect("a root");
        assert_eq!(
            std::fs::canonicalize(found).unwrap(),
            std::fs::canonicalize(dir.path()).unwrap()
        );
    }

    #[test]
    fn the_workspace_root_falls_back_to_workspace_folders() {
        // Newer clients send folders and omit the deprecated root_uri.
        let dir = tempfile::tempdir().unwrap();
        let uri = Url::from_directory_path(dir.path()).unwrap();

        let params = InitializeParams {
            workspace_folders: Some(vec![WorkspaceFolder {
                uri,
                name: "test".to_string(),
            }]),
            ..Default::default()
        };

        assert!(workspace_root(&params).is_some());
    }

    #[test]
    fn no_root_at_all_is_not_an_error() {
        assert!(workspace_root(&InitializeParams::default()).is_none());
    }
}
