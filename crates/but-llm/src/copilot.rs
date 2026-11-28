use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex as StdMutex, OnceLock, mpsc},
    time::Duration,
};

use anyhow::{Context as _, Result, anyhow, bail};
use but_tools::tool::Toolset;
use copilot_sdk::{
    Client, ClientMode,
    tool::{define_tool, try_tool_parameters},
    types::{
        CreateSessionOptions, InfiniteSessionConfig, MemoryConfiguration, MessageOptions,
        SessionEvent, SystemMessageConfig, Tool, ToolInvocation, ToolResult, ToolResultExpanded,
    },
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    sync::{Mutex as AsyncMutex, oneshot},
};

use crate::{
    ChatMessage, ToolCallContent, ToolResponseContent, client::LLMClient,
    config::DEFAULT_COPILOT_MODEL,
};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_STDERR_LINES: usize = 20;
const DEFAULT_SYSTEM_MESSAGE: &str =
    "You are a helpful assistant. Follow the user's instructions directly and concisely.";
const DISABLED_BUILTIN_MCP_SERVERS: &[&str] = &["github-mcp-server", "githubiq"];
const COPILOT_SERVER_ARGS: &[&str] = &[
    "--server",
    "--stdio",
    "--no-auto-update",
    "--sync-session=off",
];

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone)]
struct ClientHandle {
    client: Arc<Client>,
    stderr_tail: Arc<StdMutex<VecDeque<String>>>,
    session_config_dir: Arc<tempfile::TempDir>,
}

struct ManagedClient {
    client: Arc<Client>,
    child: Child,
    stderr_tail: Arc<StdMutex<VecDeque<String>>>,
    session_config_dir: Arc<tempfile::TempDir>,
}

impl ManagedClient {
    fn handle(&self) -> ClientHandle {
        ClientHandle {
            client: self.client.clone(),
            stderr_tail: self.stderr_tail.clone(),
            session_config_dir: self.session_config_dir.clone(),
        }
    }
}

struct CopilotRuntime {
    tokio_rt: tokio::runtime::Runtime,
    cli_path_override: Option<PathBuf>,
    managed: AsyncMutex<Option<ManagedClient>>,
}

impl std::fmt::Debug for CopilotRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CopilotRuntime")
            .field("cli_path_override", &self.cli_path_override)
            .finish_non_exhaustive()
    }
}

impl CopilotRuntime {
    fn new(cli_path_override: Option<PathBuf>) -> Result<Self> {
        let tokio_rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("but-copilot-sdk")
            .enable_all()
            .build()
            .context("Failed to initialize Tokio runtime for Copilot SDK")?;
        Ok(Self {
            tokio_rt,
            cli_path_override,
            managed: AsyncMutex::new(None),
        })
    }

    fn resolve_cli_path(&self) -> Result<PathBuf> {
        if let Some(path) = &self.cli_path_override {
            return Ok(path.clone());
        }
        resolve_copilot_cli_path()
    }

    async fn get_or_start_client(&self) -> Result<ClientHandle> {
        let mut guard = self.managed.lock().await;
        if let Some(existing) = guard.as_mut() {
            match existing.child.try_wait() {
                Ok(None) => {
                    return Ok(existing.handle());
                }
                Ok(Some(status)) => {
                    tracing::debug!(?status, "Copilot CLI server exited; restarting");
                    *guard = None;
                }
                Err(err) => {
                    tracing::debug!(
                        ?err,
                        "Failed checking Copilot CLI server status; restarting"
                    );
                    *guard = None;
                }
            }
        }

        let cli_path = self.resolve_cli_path()?;
        let started = spawn_copilot_server(&cli_path).await?;
        let handle = started.handle();
        *guard = Some(started);
        Ok(handle)
    }

    async fn invalidate_client(&self, failed_client: &Arc<Client>) {
        let mut guard = self.managed.lock().await;
        if guard
            .as_ref()
            .is_some_and(|managed| Arc::ptr_eq(&managed.client, failed_client))
        {
            *guard = None;
        }
    }
}

fn shared_runtime() -> Result<Arc<CopilotRuntime>> {
    static SHARED: OnceLock<Arc<CopilotRuntime>> = OnceLock::new();
    if let Some(rt) = SHARED.get() {
        return Ok(rt.clone());
    }
    let created = Arc::new(CopilotRuntime::new(None)?);
    let _ = SHARED.set(created.clone());
    Ok(SHARED.get().cloned().unwrap_or(created))
}

fn resolve_copilot_cli_path() -> Result<PathBuf> {
    if let Some(explicit) = std::env::var_os("COPILOT_CLI_PATH").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(explicit));
    }
    which::which("copilot").map_err(|_| {
        anyhow!(
            "GitHub Copilot CLI ('copilot') was not found on PATH. Install the GitHub Copilot CLI and run 'copilot login' (or set COPILOT_CLI_PATH)."
        )
    })
}

fn format_with_stderr(
    base: anyhow::Error,
    stderr_tail: &StdMutex<VecDeque<String>>,
) -> anyhow::Error {
    let lines = stderr_tail
        .lock()
        .ok()
        .map(|buf| buf.iter().cloned().collect::<Vec<_>>().join("\n"))
        .unwrap_or_default();
    let trimmed = lines.trim();
    if trimmed.is_empty() {
        base
    } else {
        base.context(format!("Copilot CLI error: {trimmed}"))
    }
}

fn build_copilot_command(cli_path: &Path) -> Command {
    #[cfg(windows)]
    if let Some((node_exe, loader_script)) = resolve_windows_npm_loader(cli_path) {
        let mut cmd = Command::new(node_exe);
        cmd.arg(loader_script);
        return cmd;
    }

    Command::new(cli_path)
}

#[cfg(windows)]
fn resolve_windows_npm_loader(cli_path: &Path) -> Option<(PathBuf, PathBuf)> {
    let ext = cli_path.extension()?.to_str()?;
    if !ext.eq_ignore_ascii_case("cmd") && !ext.eq_ignore_ascii_case("bat") {
        return None;
    }
    let parent = cli_path.parent()?;
    let loader_script = parent
        .join("node_modules")
        .join("@github")
        .join("copilot")
        .join("npm-loader.js");
    if !loader_script.is_file() {
        return None;
    }
    let sibling_node = parent.join("node.exe");
    let node_exe = if sibling_node.is_file() {
        sibling_node
    } else {
        which::which("node").ok()?
    };
    Some((node_exe, loader_script))
}

async fn spawn_copilot_server(cli_path: &Path) -> Result<ManagedClient> {
    let session_config_dir = Arc::new(
        tempfile::Builder::new()
            .prefix("gitbutler-copilot-")
            .tempdir()
            .context("Failed to create isolated temporary directory for Copilot SDK sessions")?,
    );
    let working_dir = session_config_dir.path().to_path_buf();
    let mut cmd = build_copilot_command(cli_path);
    cmd.current_dir(&working_dir)
        .args(COPILOT_SERVER_ARGS)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);

    let mut child = cmd.spawn().with_context(|| {
        format!(
            "Failed to start GitHub Copilot CLI at '{}'. Ensure 'copilot' is installed and executable.",
            cli_path.display()
        )
    })?;

    let stdout = child
        .stdout
        .take()
        .context("Failed to capture stdout from Copilot CLI server")?;
    let stdin = child
        .stdin
        .take()
        .context("Failed to capture stdin for Copilot CLI server")?;
    let stderr_tail: Arc<StdMutex<VecDeque<String>>> =
        Arc::new(StdMutex::new(VecDeque::with_capacity(MAX_STDERR_LINES)));

    if let Some(stderr) = child.stderr.take() {
        let tail = stderr_tail.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "but_llm::copilot_stderr", "{line}");
                if let Ok(mut guard) = tail.lock() {
                    if guard.len() >= MAX_STDERR_LINES {
                        guard.pop_front();
                    }
                    guard.push_back(line);
                }
            }
        });
    }

    let client = Client::from_streams(ClientMode::Empty, stdout, stdin, working_dir)
        .context("Failed to initialize Copilot SDK JSON-RPC stream")?;

    if let Err(err) = client.verify_protocol_version().await {
        let _ = child.kill().await;
        return Err(format_with_stderr(
            anyhow!(err).context(
                "Incompatible GitHub Copilot CLI version; please update the 'copilot' CLI",
            ),
            &stderr_tail,
        ));
    }

    let auth = match client.get_auth_status().await {
        Ok(auth) => auth,
        Err(err) => {
            let _ = child.kill().await;
            return Err(format_with_stderr(
                anyhow!(err).context("Failed to check GitHub Copilot CLI authentication status"),
                &stderr_tail,
            ));
        }
    };

    if !auth.is_authenticated {
        let _ = child.kill().await;
        let detail = auth
            .status_message
            .as_deref()
            .map(str::trim)
            .filter(|msg| !msg.is_empty());
        match detail {
            Some(msg) => bail!(
                "GitHub Copilot CLI is not authenticated ({msg}). Run 'copilot login' in your terminal and try again."
            ),
            None => bail!(
                "GitHub Copilot CLI is not authenticated. Run 'copilot login' in your terminal and try again."
            ),
        }
    }

    Ok(ManagedClient {
        client: Arc::new(client),
        child,
        stderr_tail,
        session_config_dir,
    })
}

async fn cleanup_session(session: copilot_sdk::session::Session, config_dir: &Path) {
    let session_id = session.id().as_str().to_string();
    let _ = session.disconnect().await;
    if !session_id.is_empty()
        && !session_id.contains('/')
        && !session_id.contains('\\')
        && session_id != "."
        && session_id != ".."
    {
        let session_dir = config_dir.join("session-state").join(&session_id);
        let _ = tokio::fs::remove_dir_all(session_dir).await;
    }
}

fn normalize_model(model: &str) -> Option<String> {
    let trimmed = model.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(DEFAULT_COPILOT_MODEL) {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn build_session_prompts(
    system_message: &str,
    chat_messages: &[ChatMessage],
) -> (Option<String>, String) {
    let system_message = {
        let trimmed = system_message.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_string())
    };

    if let [ChatMessage::User(single)] = chat_messages {
        return (system_message, single.clone());
    }

    let mut sections = Vec::with_capacity(chat_messages.len());
    for msg in chat_messages {
        match msg {
            ChatMessage::User(content) => sections.push(format!("User:\n{content}")),
            ChatMessage::Assistant(content) => sections.push(format!("Assistant:\n{content}")),
            ChatMessage::ToolCall(call) => sections.push(format!(
                "Assistant tool call ({}, id={}):\n{}",
                call.name, call.id, call.arguments
            )),
            ChatMessage::ToolResponse(resp) => {
                sections.push(format!("Tool response (id={}):\n{}", resp.id, resp.result))
            }
        }
    }

    (system_message, sections.join("\n\n"))
}

fn build_session_options(
    system_message: Option<String>,
    model: &str,
    streaming: bool,
    tools: Option<Vec<Tool>>,
    config_directory: &Path,
) -> CreateSessionOptions {
    // Always use "replace" mode so prompts without an explicit system message
    // do not inherit the multi-thousand-token Copilot Coding Agent prompt.
    let effective_system_message =
        system_message.unwrap_or_else(|| DEFAULT_SYSTEM_MESSAGE.to_string());

    let mut options = CreateSessionOptions::default()
        .with_client_name("gitbutler")
        .with_working_directory(config_directory)
        .with_config_directory(config_directory)
        .with_permissions(|p| p.deny_all_permissions())
        .with_system_message(
            SystemMessageConfig::new()
                .with_mode("replace")
                .with_content(effective_system_message),
        );

    if let Some(model) = normalize_model(model) {
        options = options.with_model(model);
    }

    options.enable_config_discovery = Some(false);
    options.filesystem.track_file_changes = Some(false);
    options.instructions.on_demand_discovery = Some(false);
    options.instructions.skip_custom_instructions = Some(true);
    options.skills.enabled = Some(false);
    options.extensions.enabled = Some(false);
    options.hooks.enable_repository_hooks = Some(false);
    options.git.enable_host_git_operations = Some(false);
    options.mcp.servers = Some(Default::default());
    options.mcp.disabled_servers = Some(
        DISABLED_BUILTIN_MCP_SERVERS
            .iter()
            .map(|name| (*name).to_string())
            .collect(),
    );
    options.state.enable_cross_session_store = Some(false);
    options.state.infinite_sessions = Some(InfiniteSessionConfig::new().with_enabled(false));
    options.state.memory = Some(MemoryConfiguration::disabled());
    options.events.streaming = Some(streaming);

    match tools {
        Some(defs) => {
            let names: Vec<String> = defs.iter().map(|t| t.name.clone()).collect();
            options.with_tools(|t| t.with_definitions(defs).with_available_tools(names))
        }
        None => options.with_tools(|t| t.with_available_tools(std::iter::empty::<String>())),
    }
}

fn extract_assistant_text(event: Option<SessionEvent>) -> Option<String> {
    let event = event?;
    let text = event.data.get("content").and_then(|c| c.as_str())?;
    (!text.is_empty()).then(|| text.to_string())
}

struct ToolSpec {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

struct ToolExecutionOutcome {
    text: String,
    is_error: bool,
}

enum LoopEvent {
    ToolCall {
        id: String,
        name: String,
        arguments: String,
        reply: oneshot::Sender<ToolExecutionOutcome>,
    },
    Finished(Result<Option<String>>),
}

fn build_sdk_tool(spec: ToolSpec, loop_tx: mpsc::Sender<LoopEvent>) -> Result<Tool> {
    let tool_name_for_call = spec.name.clone();
    let mut tool = define_tool::<serde_json::Map<String, serde_json::Value>, _, _>(
        spec.name.clone(),
        spec.description,
        move |inv: ToolInvocation, params: serde_json::Map<String, serde_json::Value>| {
            let tx = loop_tx.clone();
            let name = tool_name_for_call.clone();
            async move {
                let arguments = serde_json::Value::Object(params).to_string();
                let (reply_tx, reply_rx) = oneshot::channel();
                if tx
                    .send(LoopEvent::ToolCall {
                        id: inv.tool_call_id,
                        name,
                        arguments,
                        reply: reply_tx,
                    })
                    .is_err()
                {
                    return Ok(ToolResult::Expanded(
                        ToolResultExpanded::new(
                            "Tool execution loop closed unexpectedly",
                            "failure",
                        )
                        .with_error("Tool execution loop closed"),
                    ));
                }
                match reply_rx.await {
                    Ok(outcome) => {
                        let status = if outcome.is_error {
                            "failure"
                        } else {
                            "success"
                        };
                        Ok(ToolResult::Expanded(ToolResultExpanded::new(
                            outcome.text,
                            status,
                        )))
                    }
                    Err(_) => Ok(ToolResult::Expanded(
                        ToolResultExpanded::new("Tool execution cancelled", "failure")
                            .with_error("Tool execution cancelled"),
                    )),
                }
            }
        },
    )
    .with_skip_permission(true);

    let schema = if spec.parameters.is_object() {
        spec.parameters
    } else {
        serde_json::json!({ "type": "object" })
    };
    tool.parameters = try_tool_parameters(schema)
        .with_context(|| format!("Invalid JSON schema parameters for tool '{}'", spec.name))?;
    Ok(tool)
}

#[derive(Debug, Clone)]
pub struct CopilotProvider {
    model: Option<String>,
    runtime: Arc<CopilotRuntime>,
}

impl CopilotProvider {
    pub fn with(model: Option<String>) -> Option<Self> {
        let runtime = match shared_runtime() {
            Ok(rt) => rt,
            Err(err) => {
                tracing::error!(?err, "Failed to initialize Copilot runtime");
                return None;
            }
        };
        Some(Self { model, runtime })
    }

    #[cfg(test)]
    pub(crate) fn with_cli_path(model: Option<String>, cli_path: PathBuf) -> Result<Self> {
        Ok(Self {
            model,
            runtime: Arc::new(CopilotRuntime::new(Some(cli_path))?),
        })
    }

    fn run_async<T: Send + 'static>(
        &self,
        fut: impl std::future::Future<Output = Result<T>> + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.runtime.tokio_rt.spawn(async move {
            let _ = tx.send(fut.await);
        });
        rx.recv()
            .context("Copilot SDK background task terminated unexpectedly")?
    }
}

impl LLMClient for CopilotProvider {
    fn model(&self) -> Option<String> {
        self.model.clone()
    }

    fn tool_calling_loop_stream(
        &self,
        system_message: &str,
        chat_messages: Vec<ChatMessage>,
        tool_set: &mut impl Toolset,
        model: &str,
        on_token: impl Fn(&str) + Send + Sync + 'static,
    ) -> Result<(String, Vec<ChatMessage>)> {
        let specs: Vec<ToolSpec> = tool_set
            .list()
            .into_iter()
            .map(|t| ToolSpec {
                name: t.name(),
                description: t.description(),
                parameters: t.parameters(),
            })
            .collect();

        let (system_prompt, prompt) = build_session_prompts(system_message, &chat_messages);
        let model = model.to_string();
        let runtime = self.runtime.clone();
        let (loop_tx, loop_rx) = mpsc::channel::<LoopEvent>();

        let mut sdk_tools = Vec::with_capacity(specs.len());
        for spec in specs {
            sdk_tools.push(build_sdk_tool(spec, loop_tx.clone())?);
        }

        self.runtime.tokio_rt.spawn(async move {
            let outcome: Result<Option<String>> = async {
                let handle = runtime.get_or_start_client().await?;
                let options = build_session_options(
                    system_prompt,
                    &model,
                    true,
                    (!sdk_tools.is_empty()).then_some(sdk_tools),
                    handle.session_config_dir.path(),
                );
                let session = match handle.client.create_session(options).await {
                    Ok(session) => session,
                    Err(err) => {
                        runtime.invalidate_client(&handle.client).await;
                        return Err(format_with_stderr(
                            anyhow!(err).context("Failed to create Copilot SDK session"),
                            &handle.stderr_tail,
                        ));
                    }
                };

                let mut events = session.subscribe();
                let on_token = Arc::new(on_token);
                let token_cb = on_token.clone();
                let stream_task = tokio::spawn(async move {
                    loop {
                        match events.recv().await {
                            Ok(event) => {
                                if event.event_type == "assistant.message_delta"
                                    && let Some(delta) =
                                        event.data.get("deltaContent").and_then(|v| v.as_str())
                                    && !delta.is_empty()
                                {
                                    token_cb(delta);
                                }
                            }
                            Err(err) => match err.kind() {
                                copilot_sdk::subscription::RecvErrorKind::Lagged(_) => continue,
                                _ => break,
                            },
                        }
                    }
                });

                let send_res = session
                    .send_and_wait(MessageOptions::new(prompt).with_wait_timeout(REQUEST_TIMEOUT))
                    .await;
                tokio::task::yield_now().await;
                stream_task.abort();
                cleanup_session(session, handle.session_config_dir.path()).await;

                match send_res {
                    Ok(event) => Ok(extract_assistant_text(event)),
                    Err(err) => {
                        runtime.invalidate_client(&handle.client).await;
                        Err(format_with_stderr(
                            anyhow!(err).context("Copilot SDK request failed"),
                            &handle.stderr_tail,
                        ))
                    }
                }
            }
            .await;

            let _ = loop_tx.send(LoopEvent::Finished(outcome));
        });

        let mut history = chat_messages;
        while let Ok(event) = loop_rx.recv() {
            match event {
                LoopEvent::ToolCall {
                    id,
                    name,
                    arguments,
                    reply,
                } => {
                    history.push(ChatMessage::ToolCall(ToolCallContent {
                        id: id.clone(),
                        name: name.clone(),
                        arguments: arguments.clone(),
                    }));
                    let value = tool_set.call_tool(&name, &arguments);
                    let is_error = value.get("error").is_some();
                    let result_text = serde_json::to_string(&value).unwrap_or_else(|e| {
                        format!("{{\"error\":\"Failed to serialize tool output: {e}\"}}")
                    });
                    history.push(ChatMessage::ToolResponse(ToolResponseContent {
                        id,
                        result: result_text.clone(),
                    }));
                    let _ = reply.send(ToolExecutionOutcome {
                        text: result_text,
                        is_error,
                    });
                }
                LoopEvent::Finished(result) => {
                    let final_response = result?.unwrap_or_default();
                    if !final_response.is_empty() {
                        history.push(ChatMessage::Assistant(final_response.clone()));
                    }
                    return Ok((final_response, history));
                }
            }
        }

        bail!("Copilot SDK tool-calling loop ended without producing a result")
    }

    fn tool_calling_loop(
        &self,
        system_message: &str,
        chat_messages: Vec<ChatMessage>,
        tool_set: &mut impl Toolset,
        model: &str,
    ) -> Result<String> {
        let (response, _) =
            self.tool_calling_loop_stream(system_message, chat_messages, tool_set, model, |_| {})?;
        Ok(response)
    }

    fn stream_response(
        &self,
        system_message: &str,
        chat_messages: Vec<ChatMessage>,
        model: &str,
        on_token: impl Fn(&str) + Send + Sync + 'static,
    ) -> Result<Option<String>> {
        let (system_prompt, prompt) = build_session_prompts(system_message, &chat_messages);
        let model = model.to_string();
        let runtime = self.runtime.clone();

        self.run_async(async move {
            let handle = runtime.get_or_start_client().await?;
            let options = build_session_options(
                system_prompt,
                &model,
                true,
                None,
                handle.session_config_dir.path(),
            );
            let session = match handle.client.create_session(options).await {
                Ok(session) => session,
                Err(err) => {
                    runtime.invalidate_client(&handle.client).await;
                    return Err(format_with_stderr(
                        anyhow!(err).context("Failed to create Copilot SDK session"),
                        &handle.stderr_tail,
                    ));
                }
            };

            let mut events = session.subscribe();
            let stream_task = tokio::spawn(async move {
                loop {
                    match events.recv().await {
                        Ok(event) => {
                            if event.event_type == "assistant.message_delta"
                                && let Some(delta) =
                                    event.data.get("deltaContent").and_then(|v| v.as_str())
                                && !delta.is_empty()
                            {
                                on_token(delta);
                            }
                        }
                        Err(err) => match err.kind() {
                            copilot_sdk::subscription::RecvErrorKind::Lagged(_) => continue,
                            _ => break,
                        },
                    }
                }
            });

            let send_res = session
                .send_and_wait(MessageOptions::new(prompt).with_wait_timeout(REQUEST_TIMEOUT))
                .await;
            tokio::task::yield_now().await;
            stream_task.abort();
            cleanup_session(session, handle.session_config_dir.path()).await;

            match send_res {
                Ok(event) => Ok(extract_assistant_text(event)),
                Err(err) => {
                    runtime.invalidate_client(&handle.client).await;
                    Err(format_with_stderr(
                        anyhow!(err).context("Copilot SDK streaming request failed"),
                        &handle.stderr_tail,
                    ))
                }
            }
        })
    }

    fn response(
        &self,
        system_message: &str,
        chat_messages: Vec<ChatMessage>,
        model: &str,
    ) -> Result<Option<String>> {
        let (system_prompt, prompt) = build_session_prompts(system_message, &chat_messages);
        let model = model.to_string();
        let runtime = self.runtime.clone();

        self.run_async(async move {
            let handle = runtime.get_or_start_client().await?;
            let options = build_session_options(
                system_prompt,
                &model,
                false,
                None,
                handle.session_config_dir.path(),
            );
            let session = match handle.client.create_session(options).await {
                Ok(session) => session,
                Err(err) => {
                    runtime.invalidate_client(&handle.client).await;
                    return Err(format_with_stderr(
                        anyhow!(err).context("Failed to create Copilot SDK session"),
                        &handle.stderr_tail,
                    ));
                }
            };

            let send_res = session
                .send_and_wait(MessageOptions::new(prompt).with_wait_timeout(REQUEST_TIMEOUT))
                .await;
            cleanup_session(session, handle.session_config_dir.path()).await;

            match send_res {
                Ok(event) => Ok(extract_assistant_text(event)),
                Err(err) => {
                    runtime.invalidate_client(&handle.client).await;
                    Err(format_with_stderr(
                        anyhow!(err).context("Copilot SDK request failed"),
                        &handle.stderr_tail,
                    ))
                }
            }
        })
    }

    fn structured_output<
        T: serde::Serialize + DeserializeOwned + JsonSchema + std::marker::Send + 'static,
    >(
        &self,
        system_message: &str,
        chat_messages: Vec<ChatMessage>,
        model: &str,
    ) -> Result<Option<T>> {
        let (system_prompt, prompt) = build_session_prompts(system_message, &chat_messages);
        let model = model.to_string();
        let runtime = self.runtime.clone();

        self.run_async(async move {
            let handle = runtime.get_or_start_client().await?;
            let options = build_session_options(
                system_prompt,
                &model,
                false,
                None,
                handle.session_config_dir.path(),
            );
            let session = match handle.client.create_session(options).await {
                Ok(session) => session,
                Err(err) => {
                    runtime.invalidate_client(&handle.client).await;
                    return Err(format_with_stderr(
                        anyhow!(err).context("Failed to create Copilot SDK session"),
                        &handle.stderr_tail,
                    ));
                }
            };

            let send_res = session
                .send_and_wait_typed::<T>(
                    MessageOptions::new(prompt).with_wait_timeout(REQUEST_TIMEOUT),
                )
                .await;
            cleanup_session(session, handle.session_config_dir.path()).await;

            match send_res {
                Ok(parsed) => Ok(Some(parsed)),
                Err(err) => {
                    runtime.invalidate_client(&handle.client).await;
                    Err(format_with_stderr(
                        anyhow!(err).context("Copilot SDK structured output request failed"),
                        &handle.stderr_tail,
                    ))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_default_and_explicit_models() {
        assert_eq!(normalize_model(""), None);
        assert_eq!(normalize_model("  default "), None);
        assert_eq!(normalize_model("DEFAULT"), None);
        assert_eq!(normalize_model(" gpt-4.1 "), Some("gpt-4.1".to_string()));
        assert_eq!(normalize_model("auto"), Some("auto".to_string()));
    }

    #[test]
    fn formats_single_and_multi_turn_prompts() {
        let (sys, user) =
            build_session_prompts("  Be concise. ", &[ChatMessage::User("Hello".into())]);
        assert_eq!(sys.as_deref(), Some("Be concise."));
        assert_eq!(user, "Hello");

        let (sys_none, transcript) = build_session_prompts(
            "   ",
            &[
                ChatMessage::User("First".into()),
                ChatMessage::Assistant("Reply".into()),
                ChatMessage::User("Second".into()),
            ],
        );
        assert!(sys_none.is_none());
        assert!(transcript.contains("User:\nFirst"));
        assert!(transcript.contains("Assistant:\nReply"));
        assert!(transcript.contains("User:\nSecond"));
    }

    #[test]
    fn builds_isolated_session_options_and_custom_tool_definitions() {
        let (tx, _rx) = mpsc::channel();
        let tool = build_sdk_tool(
            ToolSpec {
                name: "inspect_commit".into(),
                description: "Inspect a commit".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "commit_id": { "type": "string" }
                    },
                    "required": ["commit_id"]
                }),
            },
            tx,
        )
        .unwrap();

        assert_eq!(tool.name, "inspect_commit");
        assert!(tool.skip_permission);
        assert!(tool.parameters.contains_key("properties"));

        let temp_dir = tempfile::tempdir().unwrap();
        let opts = build_session_options(
            Some("System instructions".into()),
            "default",
            true,
            Some(vec![tool]),
            temp_dir.path(),
        );
        assert!(opts.model.is_none());
        assert_eq!(opts.config_directory.as_deref(), Some(temp_dir.path()));
        assert_eq!(opts.working_directory.as_deref(), Some(temp_dir.path()));
        assert_eq!(opts.enable_config_discovery, Some(false));
        assert_eq!(opts.filesystem.track_file_changes, Some(false));
        assert_eq!(opts.skills.enabled, Some(false));
        assert_eq!(opts.extensions.enabled, Some(false));
        assert_eq!(opts.hooks.enable_repository_hooks, Some(false));
        assert_eq!(opts.git.enable_host_git_operations, Some(false));
        assert_eq!(opts.state.enable_cross_session_store, Some(false));
        assert_eq!(opts.state.memory, Some(MemoryConfiguration::disabled()));
        assert!(
            COPILOT_SERVER_ARGS.contains(&"--sync-session=off"),
            "server args must disable remote session sync so SDK calls do not appear in user session history"
        );
        assert_eq!(
            opts.tools.available_tools,
            Some(vec!["inspect_commit".to_string()])
        );

        let no_tool_opts = build_session_options(None, "gpt-4.1", false, None, temp_dir.path());
        assert_eq!(no_tool_opts.model.as_deref(), Some("gpt-4.1"));
        assert_eq!(no_tool_opts.tools.available_tools, Some(Vec::new()));
        let sys_cfg = no_tool_opts
            .system_message
            .expect("always sets system_message");
        assert_eq!(sys_cfg.mode.as_deref(), Some("replace"));
        assert_eq!(sys_cfg.content.as_deref(), Some(DEFAULT_SYSTEM_MESSAGE));
        assert_eq!(
            no_tool_opts.mcp.disabled_servers,
            Some(vec![
                "github-mcp-server".to_string(),
                "githubiq".to_string()
            ])
        );
        assert_eq!(
            no_tool_opts
                .state
                .infinite_sessions
                .and_then(|cfg| cfg.enabled),
            Some(false)
        );
    }

    #[test]
    fn missing_cli_binary_surfaces_actionable_error() {
        let provider = CopilotProvider::with_cli_path(
            Some("default".into()),
            PathBuf::from("definitely-nonexistent-copilot-binary-xyz-12345"),
        )
        .unwrap();
        let err = provider
            .response("System", vec![ChatMessage::User("Hi".into())], "default")
            .unwrap_err();
        let message = format!("{err:#}");
        assert!(
            message.contains("Failed to start GitHub Copilot CLI"),
            "unexpected error: {message}"
        );
    }

    #[test]
    #[ignore = "requires an authenticated local copilot CLI"]
    fn live_copilot_provider_leaves_no_session_in_temp_store() {
        let cli_path = resolve_copilot_cli_path().expect("copilot CLI installed");
        let provider = CopilotProvider::with_cli_path(Some("default".into()), cli_path).unwrap();

        let reply = provider
            .response(
                "Reply with OK only.",
                vec![ChatMessage::User("EPHEMERAL_VERIFY_TOKEN_98765".into())],
                "default",
            )
            .expect("live response succeeds")
            .expect("non-empty response");
        assert!(
            !reply.trim().is_empty(),
            "Copilot provider returned non-empty reply"
        );

        provider
            .run_async({
                let runtime = provider.runtime.clone();
                async move {
                    let guard = runtime.managed.lock().await;
                    let managed = guard.as_ref().expect("managed client started");
                    let state_dir = managed.session_config_dir.path().join("session-state");
                    if state_dir.exists() {
                        let remaining: Vec<_> = std::fs::read_dir(&state_dir)?
                            .filter_map(|e| e.ok())
                            .map(|e| e.file_name())
                            .filter(|name| name != ".session-operation-locks")
                            .collect();
                        assert!(
                            remaining.is_empty(),
                            "cleanup_session removes per-session state directories: {remaining:?}"
                        );
                    }
                    assert!(
                        !managed
                            .session_config_dir
                            .path()
                            .join("session-store.db")
                            .exists(),
                        "cross-session SQLite store remains disabled"
                    );
                    Ok(())
                }
            })
            .unwrap();
    }
}
