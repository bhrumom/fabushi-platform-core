//! Product-level feature controller over the direct Mahayana Runtime Host.
//!
//! `HostMode::Test` is a deterministic in-process backend for fast E2E. It uses
//! real Rust commands, state, approvals, event ordering, and lifecycle without
//! network or simulator dependencies. Production provider mappings are added
//! feature-by-feature and must never silently fall back to test behavior.

use chrono::Datelike;
use chrono::Timelike;
use chrono::Utc;

#[cfg(feature = "production")]
use mahayana_core::ApprovalDecision as RuntimeApprovalDecision;
#[cfg(feature = "production")]
use mahayana_core::ApprovalId;
#[cfg(feature = "production")]
use mahayana_core::CODEX_ASSISTANT_CONVERSATION_ID;
#[cfg(feature = "production")]
use mahayana_core::ConversationId;
#[cfg(feature = "production")]
use mahayana_core::MessageRole as RuntimeMessageRole;
#[cfg(feature = "production")]
use mahayana_core::OperationId;
#[cfg(feature = "production")]
use mahayana_core::RuntimeActivityStatus;
#[cfg(feature = "production")]
use mahayana_core::RuntimeCommand;
#[cfg(feature = "production")]
use mahayana_core::RuntimeEvent;
#[cfg(feature = "production")]
use mahayana_core::RuntimeResponse;
#[cfg(feature = "production")]
use mahayana_core::capability::CapabilityAvailability;
#[cfg(feature = "production")]
use mahayana_core::capability::CapabilityKind;
#[cfg(feature = "production")]
use mahayana_host::HostCreateConfig;
#[cfg(feature = "production")]
use mahayana_host::MahayanaHost;
use mahayana_host_protocol::AgentMode;
use mahayana_host_protocol::AgentStepStatus;
#[cfg(feature = "production")]
use mahayana_host_protocol::ApprovalDecision;
use mahayana_host_protocol::ApprovalResolution;
use mahayana_host_protocol::AttachmentContext;
use mahayana_host_protocol::AutomationSummary;
use mahayana_host_protocol::AutomationTrigger;
use mahayana_host_protocol::BotSummary;
use mahayana_host_protocol::CapabilitySummary;
use mahayana_host_protocol::CommandAccepted;
use mahayana_host_protocol::ConnectorAccountSummary;
use mahayana_host_protocol::ConnectorStatus;
use mahayana_host_protocol::ConnectorSummary;
use mahayana_host_protocol::ConnectorToolSummary;
use mahayana_host_protocol::ConnectorTransport;
use mahayana_host_protocol::ConversationMessage;
use mahayana_host_protocol::ConversationSummary;
use mahayana_host_protocol::DraftAction;
use mahayana_host_protocol::DraftSendState;
use mahayana_host_protocol::EventCard;
use mahayana_host_protocol::EventField;
use mahayana_host_protocol::FeatureCommand;
use mahayana_host_protocol::HOST_PROTOCOL_VERSION;
use mahayana_host_protocol::HostConfig;
use mahayana_host_protocol::HostEvent;
use mahayana_host_protocol::HostInfo;
use mahayana_host_protocol::HostMode;
use mahayana_host_protocol::ListenerIntegrationSummary;
use mahayana_host_protocol::ListenerPlatform;
use mahayana_host_protocol::MessageDraft;
use mahayana_host_protocol::MessageRole;
use mahayana_host_protocol::SkillPublishState;
use mahayana_host_protocol::SkillSource;
use mahayana_host_protocol::SkillSummary;
use mahayana_host_protocol::SkillTeamSummary;
use mahayana_host_protocol::SurfacePlatform;
use mahayana_host_protocol::TranscriptCard;
use mahayana_host_protocol::UpdateState;
#[cfg(feature = "production")]
use serde::de::DeserializeOwned;
use serde_json::Value;
use serde_json::json;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::MutexGuard;
#[cfg(feature = "production")]
use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

#[derive(Debug, thiserror::Error)]
pub enum FeatureHostError {
    #[cfg(feature = "production")]
    #[error(transparent)]
    Runtime(#[from] mahayana_host::HostError),
    #[error("production Runtime support is not compiled into this Host")]
    ProductionUnavailable,
    #[error("feature Host state mutex is poisoned")]
    StatePoisoned,
    #[error("feature Host is closed")]
    Closed,
    #[error("{0}")]
    Contract(String),
}

#[cfg_attr(not(feature = "production"), allow(dead_code))]
#[derive(Debug)]
struct PendingApproval {
    mini_app_id: String,
    capability: String,
    runtime_approval_id: Option<String>,
}

#[derive(Debug)]
struct FeatureState {
    events: VecDeque<HostEvent>,
    installed: BTreeMap<String, String>,
    pending_approvals: BTreeMap<String, PendingApproval>,
    operations: BTreeSet<String>,
    sequence: u64,
    closed: bool,
    session_active: bool,
    auth_user: Option<Value>,
    automations: BTreeMap<String, AutomationSummary>,
    connectors: BTreeMap<String, ConnectorSummary>,
    skills: BTreeMap<String, SkillSummary>,
    bots: BTreeMap<String, BotSummary>,
    listeners: BTreeMap<ListenerPlatform, ListenerIntegrationSummary>,
    update_state: UpdateState,
}

impl Default for FeatureState {
    fn default() -> Self {
        Self {
            events: VecDeque::new(),
            installed: BTreeMap::new(),
            pending_approvals: BTreeMap::new(),
            operations: BTreeSet::new(),
            sequence: 0,
            closed: false,
            session_active: true,
            auth_user: None,
            automations: BTreeMap::new(),
            connectors: default_connectors(),
            skills: default_skills(),
            bots: default_bots(),
            listeners: default_listeners(),
            update_state: UpdateState::UpToDate {
                version: env!("CARGO_PKG_VERSION").into(),
            },
        }
    }
}

pub struct FeatureHostController {
    config: HostConfig,
    info: HostInfo,
    #[cfg(feature = "production")]
    runtime: Option<MahayanaHost>,
    automation_path: Option<PathBuf>,
    state: Mutex<FeatureState>,
}

impl FeatureHostController {
    pub fn create(config: HostConfig, platform: SurfacePlatform) -> Result<Self, FeatureHostError> {
        validate_config(&config)?;
        match config.mode {
            HostMode::Test => Ok(Self::create_test_backend(config, platform)),
            HostMode::Production => {
                #[cfg(feature = "production")]
                {
                    Self::create_with_host_config(config, platform, HostCreateConfig::default())
                }
                #[cfg(not(feature = "production"))]
                {
                    Err(FeatureHostError::ProductionUnavailable)
                }
            }
        }
    }

    fn create_test_backend(config: HostConfig, platform: SurfacePlatform) -> Self {
        let info = HostInfo {
            runtime_version: "mahayana-test-backend".to_string(),
            protocol_version: HOST_PROTOCOL_VERSION.to_string(),
            platform,
        };
        let mut state = FeatureState::default();
        state.events.push_back(HostEvent::HostReady {
            timestamp: timestamp(),
            info: info.clone(),
        });
        Self {
            config,
            info,
            #[cfg(feature = "production")]
            runtime: None,
            automation_path: None,
            state: Mutex::new(state),
        }
    }

    #[cfg(feature = "production")]
    pub fn create_with_host_config(
        config: HostConfig,
        platform: SurfacePlatform,
        host_config: HostCreateConfig,
    ) -> Result<Self, FeatureHostError> {
        validate_config(&config)?;
        if config.mode == HostMode::Test {
            return Ok(Self::create_test_backend(config, platform));
        }
        let automation_path = host_config.automation_path.clone().or_else(|| {
            host_config
                .runtime
                .data_dir
                .as_ref()
                .map(|data_dir| data_dir.join("automations.json"))
        });
        let runtime = MahayanaHost::create(host_config)?;
        let info = HostInfo {
            runtime_version: format!("mahayana-abi-{}", runtime.status().runtime_abi_version),
            protocol_version: HOST_PROTOCOL_VERSION.to_string(),
            platform,
        };
        let mut state = FeatureState::default();
        if let Some(path) = automation_path.as_deref() {
            state.automations = load_automations(path);
        }
        state.events.push_back(HostEvent::HostReady {
            timestamp: timestamp(),
            info: info.clone(),
        });
        Ok(Self {
            config,
            info,
            runtime: Some(runtime),
            automation_path,
            state: Mutex::new(state),
        })
    }

    pub fn info(&self) -> HostInfo {
        self.info.clone()
    }

    /// Return UI-safe account state. Credentials stay inside the Rust product
    /// client and are never serialized across the presentation boundary.
    pub fn auth_status(&self) -> Result<Value, FeatureHostError> {
        match self.config.mode {
            HostMode::Test => {
                let state = self.state()?;
                Ok(match state.auth_user.as_ref() {
                    Some(user) => json!({
                        "@type": "mahayana.auth.status",
                        "loggedIn": true,
                        "provider": "test",
                        "user": user,
                    }),
                    None => json!({
                        "@type": "mahayana.auth.status",
                        "loggedIn": false,
                        "provider": "test",
                    }),
                })
            }
            HostMode::Production => {
                #[cfg(feature = "production")]
                {
                    // First paint must be local-first. Restoring the UI-safe
                    // account from the Rust-owned session is immediate and
                    // lets an offline macOS launch remain signed in. Network
                    // operations still validate the token at their boundary.
                    if let Ok(session) = self
                        .runtime()?
                        .product_execute("mahayana.auth.session.restore", &json!({}))
                    {
                        return Ok(session);
                    }
                    self.runtime()?
                        .product_execute("mahayana.auth.status", &json!({}))
                        .map_err(FeatureHostError::from)
                }
                #[cfg(not(feature = "production"))]
                Err(FeatureHostError::ProductionUnavailable)
            }
        }
    }

    pub fn password_login(
        &self,
        username: String,
        password: String,
    ) -> Result<Value, FeatureHostError> {
        let username = required(username, "username")?;
        let password = required(password, "password")?;
        #[cfg(not(feature = "production"))]
        let _ = &password;
        match self.config.mode {
            HostMode::Test => {
                let user = json!({
                    "id": "fast-e2e-user",
                    "username": username,
                    "nickname": "本地测试用户",
                });
                self.state()?.auth_user = Some(user.clone());
                Ok(json!({
                    "@type": "mahayana.auth.session",
                    "loggedIn": true,
                    "provider": "test",
                    "sessionStored": true,
                    "user": user,
                }))
            }
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self
                    .runtime()?
                    .product_execute(
                        "mahayana.auth.password.login",
                        &json!({"username": username, "password": password}),
                    )
                    .map_err(FeatureHostError::from);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    pub fn auth_providers(&self) -> Result<Value, FeatureHostError> {
        match self.config.mode {
            HostMode::Test => Ok(json!([
                {"id": "google", "displayName": "Google", "enabled": true},
                {"id": "apple", "displayName": "Apple", "enabled": true},
                {"id": "microsoft", "displayName": "Microsoft", "enabled": true},
                {"id": "github", "displayName": "GitHub", "enabled": true}
            ])),
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self
                    .runtime()?
                    .product_execute("mahayana.auth.oauth.providers", &json!({}))
                    .map_err(FeatureHostError::from);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    pub fn oauth_start(&self, provider: String) -> Result<Value, FeatureHostError> {
        let provider = required(provider, "provider")?;
        match self.config.mode {
            HostMode::Test => Ok(json!({
                "attemptId": format!("test-oauth-{provider}"),
                "provider": provider,
                "authorizationUrl": format!("about:blank#fabushi-test-oauth-{provider}"),
            })),
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self
                    .runtime()?
                    .product_execute(
                        "mahayana.auth.oauth.start",
                        &json!({"provider": provider, "platform": "macos"}),
                    )
                    .map_err(FeatureHostError::from);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    pub fn oauth_poll(&self, attempt_id: String) -> Result<Value, FeatureHostError> {
        let attempt_id = required(attempt_id, "attemptId")?;
        match self.config.mode {
            HostMode::Test => {
                let user = json!({
                    "id": "fast-e2e-oauth-user",
                    "email": "oauth@example.test",
                    "nickname": "OAuth 测试用户",
                });
                self.state()?.auth_user = Some(user.clone());
                Ok(json!({
                    "attemptId": attempt_id,
                    "status": "completed",
                    "auth": {
                        "loggedIn": true,
                        "provider": "google",
                        "user": user,
                    }
                }))
            }
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self
                    .runtime()?
                    .product_execute(
                        "mahayana.auth.oauth.poll",
                        &json!({"attemptId": attempt_id}),
                    )
                    .map_err(FeatureHostError::from);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    pub fn logout(&self) -> Result<Value, FeatureHostError> {
        match self.config.mode {
            HostMode::Test => {
                self.state()?.auth_user = None;
                Ok(json!({
                    "@type": "mahayana.auth.session",
                    "loggedIn": false,
                    "revoked": true,
                }))
            }
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self
                    .runtime()?
                    .clear_session()
                    .map_err(FeatureHostError::from);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    pub fn execute(&self, command: FeatureCommand) -> Result<CommandAccepted, FeatureHostError> {
        if matches!(
            &command,
            FeatureCommand::AutomationList { .. }
                | FeatureCommand::AutomationUpsert { .. }
                | FeatureCommand::AutomationSetEnabled { .. }
                | FeatureCommand::AutomationDelete { .. }
                | FeatureCommand::AutomationRun { .. }
        ) {
            return self.execute_automation(command);
        }
        if is_product_surface_command(&command) {
            return self.execute_product_surface(command);
        }
        match self.config.mode {
            HostMode::Test => self.execute_test(command),
            HostMode::Production => self.execute_production(command),
        }
    }

    fn execute_automation(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        match command {
            FeatureCommand::AutomationList { .. } => {
                let mut automations = self
                    .state()?
                    .automations
                    .values()
                    .cloned()
                    .collect::<Vec<_>>();
                automations.sort_by_key(|item| item.created_at_ms);
                self.state()?.events.push_back(HostEvent::AutomationListed {
                    timestamp: timestamp(),
                    automations,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::AutomationUpsert {
                id,
                name,
                prompt,
                schedule,
                trigger,
                enabled,
                ..
            } => {
                let name = required(name, "automation name")?;
                let prompt = required(prompt, "automation prompt")?;
                let trigger = trigger.unwrap_or_else(|| AutomationTrigger::Schedule {
                    schedule: schedule.clone(),
                });
                let (schedule, trigger) = match trigger {
                    AutomationTrigger::Schedule { schedule } => {
                        let schedule = normalize_automation_schedule(&schedule)?;
                        (schedule.clone(), AutomationTrigger::Schedule { schedule })
                    }
                    AutomationTrigger::Event {
                        source,
                        event,
                        filter,
                    } => {
                        let event = required(event, "automation event")?;
                        (
                            format!("event:{}:{event}", listener_platform_slug(source)),
                            AutomationTrigger::Event {
                                source,
                                event,
                                filter: filter.filter(|value| !value.trim().is_empty()),
                            },
                        )
                    }
                };
                let now = now_millis();
                let mut state = self.state()?;
                let id = id
                    .filter(|id| is_safe_automation_id(id))
                    .unwrap_or_else(|| {
                        state.sequence += 1;
                        format!("routine-{}-{}", now, state.sequence)
                    });
                let action = if state.automations.contains_key(&id) {
                    "updated"
                } else {
                    "created"
                };
                let previous = state.automations.get(&id);
                let automation = AutomationSummary {
                    id: id.clone(),
                    name,
                    prompt,
                    schedule: schedule.clone(),
                    trigger: Some(trigger.clone()),
                    enabled,
                    created_at_ms: previous.map_or(now, |item| item.created_at_ms),
                    last_run_at_ms: previous.and_then(|item| item.last_run_at_ms),
                    next_run_at_ms: automation_next_run(&trigger, &schedule, enabled, now),
                };
                state.automations.insert(id, automation.clone());
                self.persist_automations(&state.automations)?;
                state.events.push_back(HostEvent::AutomationChanged {
                    timestamp: timestamp(),
                    action: action.into(),
                    automation,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::AutomationSetEnabled { id, enabled, .. } => {
                let mut state = self.state()?;
                let automation = state.automations.get_mut(&id).ok_or_else(|| {
                    FeatureHostError::Contract(format!("unknown automation: {id}"))
                })?;
                automation.enabled = enabled;
                let trigger =
                    automation
                        .trigger
                        .clone()
                        .unwrap_or_else(|| AutomationTrigger::Schedule {
                            schedule: automation.schedule.clone(),
                        });
                automation.next_run_at_ms =
                    automation_next_run(&trigger, &automation.schedule, enabled, now_millis());
                let automation = automation.clone();
                self.persist_automations(&state.automations)?;
                state.events.push_back(HostEvent::AutomationChanged {
                    timestamp: timestamp(),
                    action: if enabled { "resumed" } else { "paused" }.into(),
                    automation,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::AutomationDelete { id, .. } => {
                let mut state = self.state()?;
                let automation = state.automations.remove(&id).ok_or_else(|| {
                    FeatureHostError::Contract(format!("unknown automation: {id}"))
                })?;
                self.persist_automations(&state.automations)?;
                state.events.push_back(HostEvent::AutomationChanged {
                    timestamp: timestamp(),
                    action: "deleted".into(),
                    automation,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::AutomationRun { id, .. } => {
                let automation =
                    {
                        let mut state = self.state()?;
                        let automation = state.automations.get_mut(&id).ok_or_else(|| {
                            FeatureHostError::Contract(format!("unknown automation: {id}"))
                        })?;
                        let now = now_millis();
                        automation.last_run_at_ms = Some(now);
                        let trigger = automation.trigger.clone().unwrap_or_else(|| {
                            AutomationTrigger::Schedule {
                                schedule: automation.schedule.clone(),
                            }
                        });
                        automation.next_run_at_ms = automation_next_run(
                            &trigger,
                            &automation.schedule,
                            automation.enabled,
                            now,
                        );
                        let automation = automation.clone();
                        self.persist_automations(&state.automations)?;
                        state.events.push_back(HostEvent::AutomationChanged {
                            timestamp: timestamp(),
                            action: "running".into(),
                            automation: automation.clone(),
                        });
                        automation
                    };
                if let Some(AutomationTrigger::Event {
                    source,
                    event,
                    filter,
                }) = automation.trigger.as_ref()
                {
                    self.state()?.events.push_back(HostEvent::TranscriptCard {
                        timestamp: timestamp(),
                        entry_id: format!("event-{}-{}", automation.id, now_millis()),
                        operation_id: None,
                        card: TranscriptCard::Event {
                            event: EventCard {
                                source: *source,
                                event: event.clone(),
                                title: format!("{} event", listener_platform_display(*source)),
                                summary: format!("{} woke routine “{}”.", event, automation.name),
                                url: None,
                                actor: None,
                                fields: filter.as_ref().map(|filter| {
                                    vec![EventField {
                                        label: "Filter".into(),
                                        value: filter.clone(),
                                    }]
                                }),
                                occurred_at_ms: Some(now_millis()),
                            },
                        },
                    });
                }
                let trigger_context = match automation.trigger.as_ref() {
                    Some(AutomationTrigger::Event { source, event, .. }) => format!(
                        "\n触发事件：{} / {}",
                        listener_platform_display(*source),
                        event
                    ),
                    _ => String::new(),
                };
                let text = format!(
                    "[自动化例程：{}]{}\n这是用户保存的 standing instruction。请立即执行并报告结果。\n\n{}",
                    automation.name, trigger_context, automation.prompt
                );
                match self.config.mode {
                    HostMode::Test => self.execute_test(FeatureCommand::ChatSend {
                        request_id,
                        text,
                        agent_id: Some("mahayana-assistant".into()),
                        conversation_id: None,
                        mode: AgentMode::Agent,
                        mode_statement: None,
                        model: None,
                        attachments: Vec::new(),
                    }),
                    HostMode::Production => {
                        #[cfg(feature = "production")]
                        return self.production_chat(
                            request_id,
                            text,
                            Some("mahayana-assistant".into()),
                            None,
                            AgentMode::Agent,
                            None,
                            None,
                            Vec::new(),
                        );
                        #[cfg(not(feature = "production"))]
                        return Err(FeatureHostError::ProductionUnavailable);
                    }
                }
            }
            _ => unreachable!("non-automation command routed to automation executor"),
        }
    }

    fn execute_product_surface(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        match self.config.mode {
            HostMode::Test => self.execute_product_surface_test(command),
            HostMode::Production => {
                #[cfg(feature = "production")]
                return self.execute_product_surface_production(command);
                #[cfg(not(feature = "production"))]
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }
    }

    fn execute_product_surface_test(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        let mut state = self.state()?;
        ensure_open(&state)?;
        match command {
            FeatureCommand::ConnectorList { .. } => {
                let connectors = state.connectors.values().cloned().collect();
                state.events.push_back(HostEvent::ConnectorListed {
                    timestamp: timestamp(),
                    connectors,
                });
            }
            FeatureCommand::ConnectorConnect {
                connector_id,
                account_label,
                ..
            } => {
                let connector_id = required(connector_id, "connectorId")?;
                let account_id = next_id(&mut state, "account");
                let connector = state.connectors.get_mut(&connector_id).ok_or_else(|| {
                    FeatureHostError::Contract(format!("unknown connector: {connector_id}"))
                })?;
                connector.accounts.push(ConnectorAccountSummary {
                    id: account_id,
                    label: account_label
                        .clone()
                        .filter(|label| !label.trim().is_empty())
                        .unwrap_or_else(|| "Personal".into()),
                    status: ConnectorStatus::Connected,
                    email: None,
                    team_managed: Some(false),
                    error: None,
                });
                connector.status = ConnectorStatus::Connected;
                let connector = connector.clone();
                state.events.push_back(HostEvent::ConnectorChanged {
                    timestamp: timestamp(),
                    action: "connected".into(),
                    connector,
                });
                if let Some(platform) = listener_platform_for_connector(&connector_id) {
                    if let Some(integration) = state.listeners.get_mut(&platform) {
                        integration.is_connected = true;
                        integration.account_label = account_label;
                        let integration = integration.clone();
                        state.events.push_back(HostEvent::ListenerChanged {
                            timestamp: timestamp(),
                            integration,
                        });
                    }
                }
            }
            FeatureCommand::ConnectorRenameAccount {
                connector_id,
                account_id,
                label,
                ..
            } => {
                let label = required(label, "account label")?;
                let connector = state.connectors.get_mut(&connector_id).ok_or_else(|| {
                    FeatureHostError::Contract(format!("unknown connector: {connector_id}"))
                })?;
                let account = connector
                    .accounts
                    .iter_mut()
                    .find(|account| account.id == account_id)
                    .ok_or_else(|| {
                        FeatureHostError::Contract(format!(
                            "unknown connector account: {account_id}"
                        ))
                    })?;
                if account.team_managed == Some(true) {
                    return Err(FeatureHostError::Contract(
                        "team-managed accounts cannot be renamed".into(),
                    ));
                }
                account.label = label;
                let connector = connector.clone();
                state.events.push_back(HostEvent::ConnectorChanged {
                    timestamp: timestamp(),
                    action: "updated".into(),
                    connector,
                });
            }
            FeatureCommand::ConnectorRemoveAccount {
                connector_id,
                account_id,
                ..
            } => {
                let connector = state.connectors.get_mut(&connector_id).ok_or_else(|| {
                    FeatureHostError::Contract(format!("unknown connector: {connector_id}"))
                })?;
                if connector
                    .accounts
                    .iter()
                    .any(|account| account.id == account_id && account.team_managed == Some(true))
                {
                    return Err(FeatureHostError::Contract(
                        "team-managed accounts cannot be removed".into(),
                    ));
                }
                let before = connector.accounts.len();
                connector
                    .accounts
                    .retain(|account| account.id != account_id);
                if before == connector.accounts.len() {
                    return Err(FeatureHostError::Contract(format!(
                        "unknown connector account: {account_id}"
                    )));
                }
                if connector.accounts.is_empty() {
                    connector.status = ConnectorStatus::Disconnected;
                }
                let connector = connector.clone();
                state.events.push_back(HostEvent::ConnectorChanged {
                    timestamp: timestamp(),
                    action: "removed".into(),
                    connector,
                });
            }
            FeatureCommand::ConnectorSetToolEnabled {
                connector_id,
                tool_id,
                enabled,
                ..
            } => {
                let connector = state.connectors.get_mut(&connector_id).ok_or_else(|| {
                    FeatureHostError::Contract(format!("unknown connector: {connector_id}"))
                })?;
                let tool = connector
                    .tools
                    .iter_mut()
                    .find(|tool| tool.id == tool_id)
                    .ok_or_else(|| {
                        FeatureHostError::Contract(format!("unknown connector tool: {tool_id}"))
                    })?;
                tool.enabled = enabled;
                let connector = connector.clone();
                state.events.push_back(HostEvent::ConnectorChanged {
                    timestamp: timestamp(),
                    action: "toolChanged".into(),
                    connector,
                });
            }
            FeatureCommand::SkillList { agent_id, .. } => {
                let skills = state
                    .skills
                    .values()
                    .filter(|skill| {
                        agent_id
                            .as_ref()
                            .is_none_or(|agent_id| skill.owner_agent_id.as_ref() == Some(agent_id))
                    })
                    .cloned()
                    .collect();
                state.events.push_back(HostEvent::SkillListed {
                    timestamp: timestamp(),
                    skills,
                    teams: default_skill_teams(),
                });
            }
            FeatureCommand::SkillUpsert {
                id,
                name,
                description,
                use_when,
                instructions,
                owner_agent_id,
                ..
            } => {
                let name = required(name, "skill name")?;
                let use_when = required(use_when, "skill useWhen")?;
                let instructions = required(instructions, "skill instructions")?;
                let id = id.unwrap_or_else(|| next_id(&mut state, "skill"));
                let action = if state.skills.contains_key(&id) {
                    "updated"
                } else {
                    "created"
                };
                let previous = state.skills.get(&id);
                if previous.is_some_and(|skill| skill.read_only == Some(true)) {
                    return Err(FeatureHostError::Contract(
                        "managed skills cannot be edited".into(),
                    ));
                }
                let skill = SkillSummary {
                    id: id.clone(),
                    name,
                    description,
                    use_when,
                    instructions,
                    source: previous.map_or(SkillSource::Private, |skill| skill.source),
                    publish_state: previous
                        .map_or(SkillPublishState::Local, |skill| skill.publish_state),
                    owner_agent_id,
                    team_id: previous.and_then(|skill| skill.team_id.clone()),
                    team_name: previous.and_then(|skill| skill.team_name.clone()),
                    read_only: previous.and_then(|skill| skill.read_only),
                    updated_at_ms: now_millis(),
                };
                state.skills.insert(id, skill.clone());
                state.events.push_back(HostEvent::SkillChanged {
                    timestamp: timestamp(),
                    action: action.into(),
                    skill,
                });
            }
            FeatureCommand::SkillDelete { id, .. } => {
                let skill = state
                    .skills
                    .get(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown skill: {id}")))?;
                if skill.read_only == Some(true) || skill.source == SkillSource::Team {
                    return Err(FeatureHostError::Contract(
                        "team-managed skills cannot be deleted".into(),
                    ));
                }
                let skill = state.skills.remove(&id).expect("skill checked above");
                state.events.push_back(HostEvent::SkillChanged {
                    timestamp: timestamp(),
                    action: "deleted".into(),
                    skill,
                });
            }
            FeatureCommand::SkillPublish { id, team_id, .. } => {
                let team = default_skill_teams()
                    .into_iter()
                    .find(|team| team.id == team_id)
                    .ok_or_else(|| {
                        FeatureHostError::Contract(format!("unknown skill team: {team_id}"))
                    })?;
                let skill = state
                    .skills
                    .get_mut(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown skill: {id}")))?;
                if skill.description.trim().is_empty() {
                    return Err(FeatureHostError::Contract(
                        "add a skill description before publishing".into(),
                    ));
                }
                skill.source = SkillSource::Team;
                skill.publish_state = SkillPublishState::Published;
                skill.team_id = Some(team.id);
                skill.team_name = Some(team.name);
                skill.updated_at_ms = now_millis();
                let skill = skill.clone();
                state.events.push_back(HostEvent::SkillChanged {
                    timestamp: timestamp(),
                    action: "published".into(),
                    skill,
                });
            }
            FeatureCommand::SkillUnpublish { id, .. } => {
                let skill = state
                    .skills
                    .get_mut(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown skill: {id}")))?;
                if skill.publish_state == SkillPublishState::Managed {
                    return Err(FeatureHostError::Contract(
                        "managed skills cannot be unpublished".into(),
                    ));
                }
                skill.source = SkillSource::Private;
                skill.publish_state = SkillPublishState::Local;
                skill.team_id = None;
                skill.team_name = None;
                skill.updated_at_ms = now_millis();
                let skill = skill.clone();
                state.events.push_back(HostEvent::SkillChanged {
                    timestamp: timestamp(),
                    action: "unpublished".into(),
                    skill,
                });
            }
            FeatureCommand::SkillSync { id, .. } => {
                let skill = state
                    .skills
                    .get_mut(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown skill: {id}")))?;
                if skill.team_id.is_none() {
                    return Err(FeatureHostError::Contract(
                        "only published skills can be synced".into(),
                    ));
                }
                skill.publish_state = SkillPublishState::Synced;
                skill.updated_at_ms = now_millis();
                let skill = skill.clone();
                state.events.push_back(HostEvent::SkillChanged {
                    timestamp: timestamp(),
                    action: "synced".into(),
                    skill,
                });
            }
            FeatureCommand::BotList { .. } => {
                let bots = state.bots.values().cloned().collect();
                state.events.push_back(HostEvent::BotListed {
                    timestamp: timestamp(),
                    bots,
                });
            }
            FeatureCommand::BotSetHidden { id, hidden, .. } => {
                let bot = state
                    .bots
                    .get_mut(&id)
                    .ok_or_else(|| FeatureHostError::Contract(format!("unknown bot: {id}")))?;
                bot.hidden = hidden;
                let bot = bot.clone();
                state.events.push_back(HostEvent::BotChanged {
                    timestamp: timestamp(),
                    bot,
                });
            }
            FeatureCommand::DraftResolve { draft, action, .. } => {
                let draft_id = draft.id().to_string();
                match action {
                    DraftAction::Discard => {
                        state.events.push_back(HostEvent::DraftChanged {
                            timestamp: timestamp(),
                            draft_id,
                            status: DraftSendState::Discarded,
                            error: None,
                        });
                    }
                    DraftAction::Send => {
                        validate_draft(&draft)?;
                        state.events.push_back(HostEvent::DraftChanged {
                            timestamp: timestamp(),
                            draft_id: draft_id.clone(),
                            status: DraftSendState::Sending,
                            error: None,
                        });
                        state.events.push_back(HostEvent::DraftChanged {
                            timestamp: timestamp(),
                            draft_id,
                            status: DraftSendState::Sent,
                            error: None,
                        });
                    }
                }
            }
            FeatureCommand::SecretProvide {
                secret_request_id,
                value,
                ..
            } => {
                if value.is_empty() {
                    return Err(FeatureHostError::Contract(
                        "secret value must not be empty".into(),
                    ));
                }
                state.events.push_back(HostEvent::SecretProvided {
                    timestamp: timestamp(),
                    secret_request_id,
                });
            }
            FeatureCommand::ListenerList { .. } => {
                let integrations = state.listeners.values().cloned().collect();
                state.events.push_back(HostEvent::ListenerListed {
                    timestamp: timestamp(),
                    integrations,
                });
            }
            FeatureCommand::ListenerConnect { platform, .. } => {
                let integration = state.listeners.get_mut(&platform).ok_or_else(|| {
                    FeatureHostError::Contract(format!(
                        "unsupported listener platform: {platform:?}"
                    ))
                })?;
                integration.is_connected = true;
                integration.error = None;
                let integration = integration.clone();
                state.events.push_back(HostEvent::ListenerChanged {
                    timestamp: timestamp(),
                    integration,
                });
                if let Some(connector_id) = connector_for_listener_platform(platform) {
                    if let Some(connector) = state.connectors.get_mut(connector_id) {
                        connector.status = ConnectorStatus::Connected;
                        let connector = connector.clone();
                        state.events.push_back(HostEvent::ConnectorChanged {
                            timestamp: timestamp(),
                            action: "connected".into(),
                            connector,
                        });
                    }
                }
            }
            FeatureCommand::UpdateStatus { .. } => {
                let update_state = state.update_state.clone();
                state.events.push_back(HostEvent::UpdateChanged {
                    timestamp: timestamp(),
                    state: update_state,
                });
            }
            FeatureCommand::UpdateCheck { .. } => {
                state.update_state = UpdateState::Checking;
                let checking = state.update_state.clone();
                state.events.push_back(HostEvent::UpdateChanged {
                    timestamp: timestamp(),
                    state: checking,
                });
                state.update_state = UpdateState::UpToDate {
                    version: env!("CARGO_PKG_VERSION").into(),
                };
                let update_state = state.update_state.clone();
                state.events.push_back(HostEvent::UpdateChanged {
                    timestamp: timestamp(),
                    state: update_state,
                });
            }
            FeatureCommand::UpdateInstall { .. } => {
                let version = match &state.update_state {
                    UpdateState::Available { version, .. }
                    | UpdateState::Ready { version }
                    | UpdateState::Downloading { version, .. }
                    | UpdateState::Staging { version } => version.clone(),
                    _ => {
                        return Err(FeatureHostError::Contract(
                            "no update is available to install".into(),
                        ));
                    }
                };
                state.update_state = UpdateState::Downloading {
                    version: version.clone(),
                    progress: Some(100),
                };
                let downloading = state.update_state.clone();
                state.events.push_back(HostEvent::UpdateChanged {
                    timestamp: timestamp(),
                    state: downloading,
                });
                state.update_state = UpdateState::Ready { version };
                let ready = state.update_state.clone();
                state.events.push_back(HostEvent::UpdateChanged {
                    timestamp: timestamp(),
                    state: ready,
                });
            }
            _ => unreachable!("non-product-surface command routed to product executor"),
        }
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn production_live_connector_sources(
        &self,
    ) -> Result<(Vec<Value>, Vec<Value>), FeatureHostError> {
        let servers = match self.runtime()?.execute(RuntimeCommand::McpServers)? {
            RuntimeResponse::McpServers { data } => data,
            other => return Err(unexpected_response("mahayana.mcp.servers", other)),
        };
        let apps = match self.runtime()?.execute(RuntimeCommand::McpApps) {
            Ok(RuntimeResponse::McpApps { data }) => data,
            Ok(other) => return Err(unexpected_response("mahayana.mcp.apps", other)),
            // Connector directory discovery is feature-gated in some Codex
            // builds. Live MCP tool metadata is still authoritative for linked
            // connectors, so an unavailable directory should not hide them.
            Err(_) => Vec::new(),
        };
        Ok((servers, apps))
    }

    #[cfg(feature = "production")]
    fn production_connector_snapshot(
        &self,
    ) -> Result<
        (
            Vec<ConnectorSummary>,
            BTreeMap<String, LiveConnectorProjection>,
        ),
        FeatureHostError,
    > {
        let payload = json!({"type": "connector.list", "requestId": "connector-snapshot"});
        let response = self
            .runtime()?
            .product_execute("mahayana.connector.list", &payload)?;
        let base: Vec<ConnectorSummary> =
            decode_product_field(response, "connectors", "mahayana.connector.list")?;
        let (servers, apps) = self.production_live_connector_sources()?;
        let live = live_connector_projections(&servers, &apps);
        Ok((merge_live_connectors(base, &live), live))
    }

    #[cfg(feature = "production")]
    fn emit_connector_snapshot_change(
        &self,
        connector_id: &str,
        action: &str,
    ) -> Result<(), FeatureHostError> {
        let (connectors, _) = self.production_connector_snapshot()?;
        let connector = connectors
            .into_iter()
            .find(|connector| connector.id == connector_id)
            .ok_or_else(|| {
                FeatureHostError::Contract(format!("unknown connector: {connector_id}"))
            })?;
        self.state()?.events.push_back(HostEvent::ConnectorChanged {
            timestamp: timestamp(),
            action: action.into(),
            connector,
        });
        Ok(())
    }

    #[cfg(feature = "production")]
    fn execute_live_product_surface_production(
        &self,
        command: &FeatureCommand,
    ) -> Result<Option<CommandAccepted>, FeatureHostError> {
        let request_id = command.request_id().to_string();
        match command {
            FeatureCommand::ConnectorList { .. } => {
                let (connectors, _) = self.production_connector_snapshot()?;
                self.state()?.events.push_back(HostEvent::ConnectorListed {
                    timestamp: timestamp(),
                    connectors,
                });
                Ok(Some(CommandAccepted {
                    request_id,
                    operation_id: None,
                }))
            }
            FeatureCommand::ConnectorConnect {
                connector_id,
                account_label: _,
                ..
            } => {
                if connector_id == "git" {
                    let payload = serde_json::to_value(command).map_err(|error| {
                        FeatureHostError::Contract(format!("encode connector.connect: {error}"))
                    })?;
                    self.runtime()?
                        .product_execute("mahayana.connector.connect", &payload)?;
                    self.emit_connector_snapshot_change(connector_id, "connected")?;
                    return Ok(Some(CommandAccepted {
                        request_id,
                        operation_id: None,
                    }));
                }
                let (_, live) = self.production_connector_snapshot()?;
                let projection = live.get(connector_id).ok_or_else(|| {
                    FeatureHostError::Contract(format!(
                        "{connector_id} is not installed or discoverable in the Codex connector runtime; add its plugin from Plugins first"
                    ))
                })?;
                if let Some(url) = projection.install_url.clone() {
                    if !url.starts_with("https://") {
                        return Err(FeatureHostError::Contract(
                            "connector authorization URL must use HTTPS".into(),
                        ));
                    }
                    self.state()?
                        .events
                        .push_back(HostEvent::ConnectorOauthRequested {
                            timestamp: timestamp(),
                            connector_id: connector_id.clone(),
                            authorization_url: url,
                        });
                    return Ok(Some(CommandAccepted {
                        request_id,
                        operation_id: None,
                    }));
                }
                let server = projection.server_name.clone().ok_or_else(|| {
                    FeatureHostError::Contract(format!(
                        "{connector_id} does not expose an OAuth-capable MCP server"
                    ))
                })?;
                if projection.status == Some(ConnectorStatus::Connected) {
                    self.emit_connector_snapshot_change(connector_id, "connected")?;
                    return Ok(Some(CommandAccepted {
                        request_id,
                        operation_id: None,
                    }));
                }
                let authorization_url =
                    match self.runtime()?.execute(RuntimeCommand::McpOauthLogin {
                        server: server.clone(),
                    })? {
                        RuntimeResponse::McpOauth {
                            authorization_url: Some(url),
                            ..
                        } => url,
                        other => {
                            return Err(unexpected_response("mahayana.mcp.oauth.login", other));
                        }
                    };
                self.state()?
                    .events
                    .push_back(HostEvent::ConnectorOauthRequested {
                        timestamp: timestamp(),
                        connector_id: connector_id.clone(),
                        authorization_url,
                    });
                Ok(Some(CommandAccepted {
                    request_id,
                    operation_id: None,
                }))
            }
            FeatureCommand::ConnectorRenameAccount { connector_id, .. }
            | FeatureCommand::ConnectorSetToolEnabled { connector_id, .. } => {
                let method = product_surface_method(command);
                let payload = serde_json::to_value(command).map_err(|error| {
                    FeatureHostError::Contract(format!("encode {method}: {error}"))
                })?;
                self.runtime()?.product_execute(method, &payload)?;
                self.emit_connector_snapshot_change(
                    connector_id,
                    if matches!(command, FeatureCommand::ConnectorSetToolEnabled { .. }) {
                        "toolChanged"
                    } else {
                        "updated"
                    },
                )?;
                Ok(Some(CommandAccepted {
                    request_id,
                    operation_id: None,
                }))
            }
            FeatureCommand::ConnectorRemoveAccount {
                connector_id,
                account_id,
                ..
            } => {
                let (_, live) = self.production_connector_snapshot()?;
                if let Some(projection) = live.get(connector_id) {
                    if projection.server_name.as_deref() == Some("codex_apps") {
                        if let Some(url) = projection.install_url.clone() {
                            self.state()?
                                .events
                                .push_back(HostEvent::ConnectorOauthRequested {
                                    timestamp: timestamp(),
                                    connector_id: connector_id.clone(),
                                    authorization_url: url,
                                });
                        }
                        return Err(FeatureHostError::Contract(
                            "this linked ChatGPT App account is server-managed; its account page was opened because the current Codex connector API does not expose unlink"
                                .into(),
                        ));
                    }
                    if let Some(server) = projection.server_name.clone() {
                        match self
                            .runtime()?
                            .execute(RuntimeCommand::McpOauthLogout { server })?
                        {
                            RuntimeResponse::McpOauth { .. } => {}
                            other => {
                                return Err(unexpected_response(
                                    "mahayana.mcp.oauth.logout",
                                    other,
                                ));
                            }
                        }
                    }
                }
                let method = product_surface_method(command);
                let payload = serde_json::to_value(command).map_err(|error| {
                    FeatureHostError::Contract(format!("encode {method}: {error}"))
                })?;
                self.runtime()?.product_execute(method, &payload)?;
                let _ = account_id;
                self.emit_connector_snapshot_change(connector_id, "removed")?;
                Ok(Some(CommandAccepted {
                    request_id,
                    operation_id: None,
                }))
            }
            FeatureCommand::DraftResolve { draft, action, .. } => {
                let draft_id = draft.id().to_string();
                if *action == DraftAction::Discard {
                    self.state()?.events.push_back(HostEvent::DraftChanged {
                        timestamp: timestamp(),
                        draft_id,
                        status: DraftSendState::Discarded,
                        error: None,
                    });
                    return Ok(Some(CommandAccepted {
                        request_id,
                        operation_id: None,
                    }));
                }
                validate_draft(draft)?;
                let connector_id = match draft {
                    MessageDraft::Email { .. } => "gmail",
                    MessageDraft::Slack { .. } => "slack",
                };
                let (_, live) = self.production_connector_snapshot()?;
                let projection = live.get(connector_id).ok_or_else(|| {
                    FeatureHostError::Contract(format!(
                        "{connector_id} connector is not installed; install and authorize it before sending"
                    ))
                })?;
                if projection.status != Some(ConnectorStatus::Connected) {
                    return Err(FeatureHostError::Contract(format!(
                        "{connector_id} connector is not authorized"
                    )));
                }
                let server = projection.server_name.clone().ok_or_else(|| {
                    FeatureHostError::Contract(format!(
                        "{connector_id} connector does not expose an MCP server"
                    ))
                })?;
                let (tool, schema) =
                    projection_send_tool(projection, connector_id).ok_or_else(|| {
                        FeatureHostError::Contract(format!(
                            "{connector_id} connector has no compatible send tool"
                        ))
                    })?;
                let arguments = draft_tool_arguments(draft, schema)?;
                self.state()?.events.push_back(HostEvent::DraftChanged {
                    timestamp: timestamp(),
                    draft_id: draft_id.clone(),
                    status: DraftSendState::Sending,
                    error: None,
                });
                let result = self.runtime()?.execute(RuntimeCommand::McpToolCall {
                    server,
                    tool: tool.to_string(),
                    arguments,
                });
                match result {
                    Ok(RuntimeResponse::McpToolResult { .. }) => {
                        self.state()?.events.push_back(HostEvent::DraftChanged {
                            timestamp: timestamp(),
                            draft_id,
                            status: DraftSendState::Sent,
                            error: None,
                        });
                        Ok(Some(CommandAccepted {
                            request_id,
                            operation_id: None,
                        }))
                    }
                    Ok(other) => Err(unexpected_response("mahayana.mcp.tool.call", other)),
                    Err(error) => {
                        let message = error.to_string();
                        self.state()?.events.push_back(HostEvent::DraftChanged {
                            timestamp: timestamp(),
                            draft_id,
                            status: DraftSendState::Failed,
                            error: Some(message.clone()),
                        });
                        Err(error.into())
                    }
                }
            }
            FeatureCommand::ListenerList { .. } => {
                let payload = serde_json::to_value(command).map_err(|error| {
                    FeatureHostError::Contract(format!("encode listener.list: {error}"))
                })?;
                let response = self
                    .runtime()?
                    .product_execute("mahayana.listener.list", &payload)?;
                let mut integrations: Vec<ListenerIntegrationSummary> =
                    decode_product_field(response, "integrations", "mahayana.listener.list")?;
                let (connectors, _) = self.production_connector_snapshot()?;
                for integration in &mut integrations {
                    if integration.platform == ListenerPlatform::Git {
                        integration.is_connected = true;
                        integration.account_label = Some("Local repository".into());
                        continue;
                    }
                    if let Some(connector_id) =
                        connector_for_listener_platform(integration.platform)
                    {
                        if let Some(connector) = connectors
                            .iter()
                            .find(|connector| connector.id == connector_id)
                        {
                            integration.is_connected =
                                connector.status == ConnectorStatus::Connected;
                            integration.account_label = connector
                                .accounts
                                .first()
                                .map(|account| account.label.clone());
                        }
                    }
                }
                self.state()?.events.push_back(HostEvent::ListenerListed {
                    timestamp: timestamp(),
                    integrations,
                });
                Ok(Some(CommandAccepted {
                    request_id,
                    operation_id: None,
                }))
            }
            FeatureCommand::ListenerConnect { platform, .. } => {
                if *platform == ListenerPlatform::Git {
                    let payload = serde_json::to_value(command).map_err(|error| {
                        FeatureHostError::Contract(format!("encode listener.connect: {error}"))
                    })?;
                    let response = self
                        .runtime()?
                        .product_execute("mahayana.listener.connect", &payload)?;
                    let integration =
                        decode_product_field(response, "integration", "mahayana.listener.connect")?;
                    self.state()?.events.push_back(HostEvent::ListenerChanged {
                        timestamp: timestamp(),
                        integration,
                    });
                    return Ok(Some(CommandAccepted {
                        request_id,
                        operation_id: None,
                    }));
                }
                let connector_id = connector_for_listener_platform(*platform).ok_or_else(|| {
                    FeatureHostError::Contract(format!(
                        "no connector exists for listener platform {platform:?}"
                    ))
                })?;
                let synthetic = FeatureCommand::ConnectorConnect {
                    request_id: request_id.clone(),
                    connector_id: connector_id.into(),
                    account_label: None,
                };
                self.execute_live_product_surface_production(&synthetic)?;
                Ok(Some(CommandAccepted {
                    request_id,
                    operation_id: None,
                }))
            }
            _ => Ok(None),
        }
    }

    #[cfg(feature = "production")]
    fn execute_product_surface_production(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        if let Some(accepted) = self.execute_live_product_surface_production(&command)? {
            return Ok(accepted);
        }
        let request_id = command.request_id().to_string();
        let method = product_surface_method(&command);
        let payload = serde_json::to_value(&command).map_err(|error| {
            FeatureHostError::Contract(format!("encode {method} payload: {error}"))
        })?;

        if let FeatureCommand::DraftResolve {
            draft,
            action: DraftAction::Send,
            ..
        } = &command
        {
            validate_draft(draft)?;
            self.state()?.events.push_back(HostEvent::DraftChanged {
                timestamp: timestamp(),
                draft_id: draft.id().to_string(),
                status: DraftSendState::Sending,
                error: None,
            });
        }

        let response = self
            .runtime()?
            .product_execute(method, &payload)
            .map_err(FeatureHostError::from)?;
        let mut state = self.state()?;
        match command {
            FeatureCommand::ConnectorList { .. } => {
                let connectors = decode_product_field(response, "connectors", method)?;
                state.events.push_back(HostEvent::ConnectorListed {
                    timestamp: timestamp(),
                    connectors,
                });
            }
            FeatureCommand::ConnectorConnect { connector_id, .. } => {
                if let Some(url) = response
                    .get("authorizationUrl")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                {
                    state.events.push_back(HostEvent::ConnectorOauthRequested {
                        timestamp: timestamp(),
                        connector_id,
                        authorization_url: url,
                    });
                }
                if response.get("connector").is_some() || response.get("id").is_some() {
                    let connector = decode_product_field(response, "connector", method)?;
                    state.events.push_back(HostEvent::ConnectorChanged {
                        timestamp: timestamp(),
                        action: "connected".into(),
                        connector,
                    });
                }
            }
            FeatureCommand::ConnectorRenameAccount { .. }
            | FeatureCommand::ConnectorRemoveAccount { .. }
            | FeatureCommand::ConnectorSetToolEnabled { .. } => {
                let action = match command {
                    FeatureCommand::ConnectorRemoveAccount { .. } => "removed",
                    FeatureCommand::ConnectorSetToolEnabled { .. } => "toolChanged",
                    _ => "updated",
                };
                let connector = decode_product_field(response, "connector", method)?;
                state.events.push_back(HostEvent::ConnectorChanged {
                    timestamp: timestamp(),
                    action: action.into(),
                    connector,
                });
            }
            FeatureCommand::SkillList { .. } => {
                let skills = decode_product_field(response.clone(), "skills", method)?;
                let teams = response
                    .get("teams")
                    .cloned()
                    .map(|value| decode_product_value(value, method))
                    .transpose()?
                    .unwrap_or_default();
                state.events.push_back(HostEvent::SkillListed {
                    timestamp: timestamp(),
                    skills,
                    teams,
                });
            }
            FeatureCommand::SkillUpsert { .. }
            | FeatureCommand::SkillDelete { .. }
            | FeatureCommand::SkillPublish { .. }
            | FeatureCommand::SkillUnpublish { .. }
            | FeatureCommand::SkillSync { .. } => {
                let action = match command {
                    FeatureCommand::SkillDelete { .. } => "deleted".to_string(),
                    FeatureCommand::SkillPublish { .. } => "published".to_string(),
                    FeatureCommand::SkillUnpublish { .. } => "unpublished".to_string(),
                    FeatureCommand::SkillSync { .. } => "synced".to_string(),
                    _ => response
                        .get("action")
                        .and_then(Value::as_str)
                        .unwrap_or("updated")
                        .to_string(),
                };
                let skill = decode_product_field(response, "skill", method)?;
                state.events.push_back(HostEvent::SkillChanged {
                    timestamp: timestamp(),
                    action,
                    skill,
                });
            }
            FeatureCommand::BotList { .. } => {
                let bots = decode_product_field(response, "bots", method)?;
                state.events.push_back(HostEvent::BotListed {
                    timestamp: timestamp(),
                    bots,
                });
            }
            FeatureCommand::BotSetHidden { .. } => {
                let bot = decode_product_field(response, "bot", method)?;
                state.events.push_back(HostEvent::BotChanged {
                    timestamp: timestamp(),
                    bot,
                });
            }
            FeatureCommand::DraftResolve { draft, action, .. } => {
                let status = response
                    .get("status")
                    .cloned()
                    .map(|value| decode_product_value(value, method))
                    .transpose()?
                    .unwrap_or(match action {
                        DraftAction::Send => DraftSendState::Sent,
                        DraftAction::Discard => DraftSendState::Discarded,
                    });
                state.events.push_back(HostEvent::DraftChanged {
                    timestamp: timestamp(),
                    draft_id: draft.id().to_string(),
                    status,
                    error: response
                        .get("error")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                });
            }
            FeatureCommand::SecretProvide {
                secret_request_id, ..
            } => {
                state.events.push_back(HostEvent::SecretProvided {
                    timestamp: timestamp(),
                    secret_request_id,
                });
            }
            FeatureCommand::ListenerList { .. } => {
                let integrations = decode_product_field(response, "integrations", method)?;
                state.events.push_back(HostEvent::ListenerListed {
                    timestamp: timestamp(),
                    integrations,
                });
            }
            FeatureCommand::ListenerConnect { .. } => {
                let integration = decode_product_field(response, "integration", method)?;
                state.events.push_back(HostEvent::ListenerChanged {
                    timestamp: timestamp(),
                    integration,
                });
            }
            FeatureCommand::UpdateStatus { .. }
            | FeatureCommand::UpdateCheck { .. }
            | FeatureCommand::UpdateInstall { .. } => {
                let update_state = decode_product_field(response, "state", method)?;
                state.events.push_back(HostEvent::UpdateChanged {
                    timestamp: timestamp(),
                    state: update_state,
                });
            }
            _ => unreachable!("non-product-surface command routed to product executor"),
        }
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    /// Delivers a verified listener event into the automation engine. This is
    /// intentionally not a renderer command: only the backend relay/native
    /// listener bridge may call it, so web content cannot forge trigger events.
    pub fn ingest_listener_event(&self, event: EventCard) -> Result<usize, FeatureHostError> {
        let serialized = serde_json::to_string(&event).map_err(|error| {
            FeatureHostError::Contract(format!("encode listener event: {error}"))
        })?;
        let matching_ids = {
            let state = self.state()?;
            state
                .automations
                .values()
                .filter(|automation| {
                    if !automation.enabled {
                        return false;
                    }
                    let Some(AutomationTrigger::Event {
                        source,
                        event: expected_event,
                        filter,
                    }) = automation.trigger.as_ref()
                    else {
                        return false;
                    };
                    *source == event.source
                        && (expected_event == "*" || expected_event == &event.event)
                        && filter.as_ref().is_none_or(|filter| {
                            serialized
                                .to_ascii_lowercase()
                                .contains(&filter.to_ascii_lowercase())
                        })
                })
                .map(|automation| automation.id.clone())
                .collect::<Vec<_>>()
        };

        for id in &matching_ids {
            let automation = {
                let mut state = self.state()?;
                let automation = state.automations.get_mut(id).ok_or_else(|| {
                    FeatureHostError::Contract(format!("unknown automation: {id}"))
                })?;
                automation.last_run_at_ms = Some(event.occurred_at_ms.unwrap_or_else(now_millis));
                let automation = automation.clone();
                self.persist_automations(&state.automations)?;
                state.events.push_back(HostEvent::AutomationChanged {
                    timestamp: timestamp(),
                    action: "triggered".into(),
                    automation: automation.clone(),
                });
                state.events.push_back(HostEvent::TranscriptCard {
                    timestamp: timestamp(),
                    entry_id: format!("relay-{}-{}", automation.id, now_millis()),
                    operation_id: None,
                    card: TranscriptCard::Event {
                        event: event.clone(),
                    },
                });
                automation
            };
            let details = event
                .fields
                .as_ref()
                .map(|fields| {
                    fields
                        .iter()
                        .map(|field| format!("{}: {}", field.label, field.value))
                        .collect::<Vec<_>>()
                        .join("\n")
                })
                .unwrap_or_default();
            let text = format!(
                "[事件自动化：{}]\n来源：{}\n事件：{}\n标题：{}\n摘要：{}{}\n\n这是用户保存的 standing instruction。请基于上面的真实事件立即执行并报告结果。\n\n{}",
                automation.name,
                listener_platform_display(event.source),
                event.event,
                event.title,
                event.summary,
                if details.is_empty() {
                    String::new()
                } else {
                    format!("\n{details}")
                },
                automation.prompt
            );
            let request_id = format!("listener-{}-{}", automation.id, now_millis());
            match self.config.mode {
                HostMode::Test => {
                    self.execute_test(FeatureCommand::ChatSend {
                        request_id,
                        text,
                        agent_id: Some("mahayana-assistant".into()),
                        conversation_id: None,
                        mode: AgentMode::Agent,
                        mode_statement: None,
                        model: None,
                        attachments: Vec::new(),
                    })?;
                }
                HostMode::Production => {
                    #[cfg(feature = "production")]
                    {
                        self.production_chat(
                            request_id,
                            text,
                            Some("mahayana-assistant".into()),
                            None,
                            AgentMode::Agent,
                            None,
                            None,
                            Vec::new(),
                        )?;
                    }
                    #[cfg(not(feature = "production"))]
                    return Err(FeatureHostError::ProductionUnavailable);
                }
            }
        }
        Ok(matching_ids.len())
    }

    pub fn receive(&self) -> Result<Option<HostEvent>, FeatureHostError> {
        self.fire_due_automation()?;
        if let Some(event) = self.state()?.events.pop_front() {
            return Ok(Some(event));
        }
        match self.config.mode {
            HostMode::Test => Ok(None),
            HostMode::Production => self.receive_production(),
        }
    }

    fn fire_due_automation(&self) -> Result<(), FeatureHostError> {
        let now = now_millis();
        let due = self
            .state()?
            .automations
            .values()
            .find(|automation| {
                automation.enabled && automation.next_run_at_ms.is_some_and(|next| next <= now)
            })
            .map(|automation| automation.id.clone());
        if let Some(id) = due {
            let _ = self.execute_automation(FeatureCommand::AutomationRun {
                request_id: format!("scheduled-{id}-{now}"),
                id,
            })?;
        }
        Ok(())
    }

    fn persist_automations(
        &self,
        automations: &BTreeMap<String, AutomationSummary>,
    ) -> Result<(), FeatureHostError> {
        let Some(path) = self.automation_path.as_deref() else {
            return Ok(());
        };
        persist_automations(path, automations)
    }

    pub fn resolve_approval(&self, resolution: ApprovalResolution) -> Result<(), FeatureHostError> {
        let pending = {
            let mut state = self.state()?;
            ensure_open(&state)?;
            state
                .pending_approvals
                .remove(&resolution.approval_id)
                .ok_or_else(|| {
                    FeatureHostError::Contract(format!(
                        "unknown approval: {}",
                        resolution.approval_id
                    ))
                })?
        };

        #[cfg(not(feature = "production"))]
        let _ = &pending;

        if self.config.mode == HostMode::Production {
            #[cfg(feature = "production")]
            {
                if let Some(runtime_approval_id) = pending.runtime_approval_id {
                    let decision = match resolution.decision {
                        ApprovalDecision::AllowOnce => RuntimeApprovalDecision::Accept,
                        ApprovalDecision::AllowSession => RuntimeApprovalDecision::AcceptForSession,
                        ApprovalDecision::Deny => RuntimeApprovalDecision::Decline,
                    };
                    self.runtime()?.resolve_approval(
                        ApprovalId(runtime_approval_id),
                        decision,
                        json!({
                            "miniAppId": pending.mini_app_id,
                            "capability": pending.capability,
                        }),
                    )?;
                }
            }
            #[cfg(not(feature = "production"))]
            {
                return Err(FeatureHostError::ProductionUnavailable);
            }
        }

        self.state()?.events.push_back(HostEvent::ApprovalResolved {
            timestamp: timestamp(),
            approval_id: resolution.approval_id,
            decision: resolution.decision,
        });
        Ok(())
    }

    pub fn interrupt(&self, operation_id: &str) -> Result<(), FeatureHostError> {
        {
            let state = self.state()?;
            ensure_open(&state)?;
            if !state.operations.contains(operation_id) {
                return Err(FeatureHostError::Contract(format!(
                    "unknown operation: {operation_id}"
                )));
            }
        }

        if self.config.mode == HostMode::Production {
            #[cfg(feature = "production")]
            if !operation_id.starts_with("host-task-") {
                self.runtime()?
                    .interrupt(OperationId(operation_id.to_string()))?;
            }
            #[cfg(not(feature = "production"))]
            return Err(FeatureHostError::ProductionUnavailable);
        }

        let mut state = self.state()?;
        state.operations.remove(operation_id);
        state.events.push_back(HostEvent::OperationInterrupted {
            timestamp: timestamp(),
            operation_id: operation_id.to_string(),
        });
        Ok(())
    }

    pub fn close(&self) -> Result<(), FeatureHostError> {
        let mut state = self.state()?;
        if state.closed {
            return Ok(());
        }
        state.closed = true;
        state.events.push_back(HostEvent::HostClosed {
            timestamp: timestamp(),
        });
        Ok(())
    }

    #[cfg(feature = "production")]
    fn runtime(&self) -> Result<&MahayanaHost, FeatureHostError> {
        self.runtime
            .as_ref()
            .ok_or(FeatureHostError::ProductionUnavailable)
    }

    #[cfg(not(feature = "production"))]
    fn execute_production(
        &self,
        _command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        Err(FeatureHostError::ProductionUnavailable)
    }

    #[cfg(not(feature = "production"))]
    fn receive_production(&self) -> Result<Option<HostEvent>, FeatureHostError> {
        Err(FeatureHostError::ProductionUnavailable)
    }

    #[cfg(feature = "production")]
    fn translate_runtime_event(
        &self,
        event: RuntimeEvent,
    ) -> Result<Option<HostEvent>, FeatureHostError> {
        let event = match event {
            RuntimeEvent::Ready { .. } => None,
            RuntimeEvent::MessageDelta {
                operation_id,
                delta,
                ..
            } => Some(HostEvent::ChatDelta {
                timestamp: timestamp(),
                operation_id: operation_id.to_string(),
                delta,
            }),
            RuntimeEvent::MessageCompleted {
                operation_id,
                message,
                ..
            } => {
                let operation_id = operation_id.to_string();
                let mut cards = transcript_cards_from_metadata(&message.metadata);
                let message_id = message.id.to_string();
                if message.text.trim().is_empty() && !cards.is_empty() {
                    let first = cards.remove(0);
                    let mut state = self.state()?;
                    for (index, card) in cards.into_iter().enumerate() {
                        state.events.push_back(HostEvent::TranscriptCard {
                            timestamp: timestamp(),
                            entry_id: format!("{message_id}-card-{}", index + 1),
                            operation_id: Some(operation_id.clone()),
                            card,
                        });
                    }
                    Some(HostEvent::TranscriptCard {
                        timestamp: timestamp(),
                        entry_id: format!("{message_id}-card-0"),
                        operation_id: Some(operation_id),
                        card: first,
                    })
                } else {
                    if !cards.is_empty() {
                        let mut state = self.state()?;
                        for (index, card) in cards.into_iter().enumerate() {
                            state.events.push_back(HostEvent::TranscriptCard {
                                timestamp: timestamp(),
                                entry_id: format!("{message_id}-card-{index}"),
                                operation_id: Some(operation_id.clone()),
                                card,
                            });
                        }
                    }
                    let role = match message.role {
                        RuntimeMessageRole::User => MessageRole::User,
                        RuntimeMessageRole::Assistant
                        | RuntimeMessageRole::Contact
                        | RuntimeMessageRole::MiniApp
                        | RuntimeMessageRole::System => MessageRole::Assistant,
                    };
                    Some(HostEvent::ChatMessage {
                        timestamp: timestamp(),
                        role,
                        text: message.text,
                        operation_id: Some(operation_id),
                    })
                }
            }
            RuntimeEvent::ApprovalRequested {
                approval_id,
                title,
                details,
                ..
            } => Some(self.translate_runtime_approval(approval_id, title, details)?),
            RuntimeEvent::OperationCompleted { operation_id } => {
                let operation_id = operation_id.to_string();
                self.state()?.operations.remove(&operation_id);
                Some(HostEvent::OperationCompleted {
                    timestamp: timestamp(),
                    operation_id,
                })
            }
            RuntimeEvent::OperationFailed {
                operation_id,
                code,
                message,
            } => {
                let operation_id = operation_id.to_string();
                self.state()?.operations.remove(&operation_id);
                Some(HostEvent::OperationFailed {
                    timestamp: timestamp(),
                    operation_id,
                    code,
                    message,
                })
            }
            RuntimeEvent::ModelUsageUpdated {
                operation_id,
                usage,
            } => {
                let tokens = usage.total.unwrap_or_else(|| usage.last.clone());
                Some(HostEvent::UsageUpdated {
                    timestamp: timestamp(),
                    operation_id: operation_id.to_string(),
                    input_tokens: tokens.input_tokens,
                    cached_input_tokens: tokens.cached_input_tokens,
                    output_tokens: tokens.output_tokens,
                    reasoning_tokens: tokens.reasoning_output_tokens,
                    total_tokens: tokens.total_tokens,
                    context_window: usage.model_context_window,
                })
            }
            RuntimeEvent::PluginProgress {
                operation_id,
                plugin_id,
                tool,
                progress,
                total,
                message,
            } => Some(HostEvent::AgentStep {
                timestamp: timestamp(),
                operation_id: Some(operation_id.to_string()),
                step_id: format!("{plugin_id}:{tool}"),
                kind: "tool".into(),
                title: tool,
                detail: Some(message),
                status: if total > 0 && progress >= total {
                    AgentStepStatus::Completed
                } else {
                    AgentStepStatus::Running
                },
                progress: Some(progress),
                total: Some(total),
            }),
            RuntimeEvent::AgentActivity {
                operation_id,
                step_id,
                kind,
                title,
                detail,
                status,
            } => Some(HostEvent::AgentStep {
                timestamp: timestamp(),
                operation_id: Some(operation_id.to_string()),
                step_id,
                kind,
                title,
                detail,
                status: match status {
                    RuntimeActivityStatus::Running => AgentStepStatus::Running,
                    RuntimeActivityStatus::Completed => AgentStepStatus::Completed,
                    RuntimeActivityStatus::Failed => AgentStepStatus::Failed,
                },
                progress: None,
                total: None,
            }),
            RuntimeEvent::ProviderDegraded { provider, message } => Some(HostEvent::AgentStep {
                timestamp: timestamp(),
                operation_id: None,
                step_id: format!("provider:{provider}"),
                kind: "provider".into(),
                title: format!("{provider} 服务降级"),
                detail: Some(message),
                status: AgentStepStatus::Failed,
                progress: None,
                total: None,
            }),
            RuntimeEvent::Lagged { skipped } => Some(HostEvent::AgentStep {
                timestamp: timestamp(),
                operation_id: None,
                step_id: "runtime:event-lag".into(),
                kind: "runtime".into(),
                title: "事件流正在追赶".into(),
                detail: Some(format!("跳过 {skipped} 个过期事件")),
                status: AgentStepStatus::Failed,
                progress: None,
                total: None,
            }),
        };
        Ok(event)
    }

    #[cfg(feature = "production")]
    fn translate_runtime_approval(
        &self,
        approval_id: ApprovalId,
        title: String,
        details: serde_json::Value,
    ) -> Result<HostEvent, FeatureHostError> {
        let approval_key = approval_id.to_string();
        let mini_app_id = details
            .get("pluginId")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("runtime")
            .to_string();
        let capability = details
            .get("capability")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(title.as_str())
            .to_string();
        let reason = details
            .get("reason")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| details.to_string());
        self.state()?.pending_approvals.insert(
            approval_key.clone(),
            PendingApproval {
                mini_app_id: mini_app_id.clone(),
                capability: capability.clone(),
                runtime_approval_id: Some(approval_id.to_string()),
            },
        );
        Ok(HostEvent::ApprovalRequested {
            timestamp: timestamp(),
            approval_id: approval_key,
            mini_app_id,
            capability,
            reason,
            kind: details
                .get("kind")
                .and_then(Value::as_str)
                .map(str::to_string),
            subject: details
                .get("subject")
                .or_else(|| details.get("command"))
                .and_then(Value::as_str)
                .map(str::to_string),
            detail: details
                .get("detail")
                .and_then(Value::as_str)
                .map(str::to_string),
            proposed_rule: details
                .get("proposedRule")
                .and_then(Value::as_str)
                .map(str::to_string),
            location: details
                .get("location")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    #[cfg(feature = "production")]
    fn receive_production(&self) -> Result<Option<HostEvent>, FeatureHostError> {
        for _ in 0..16 {
            let Some(event) = self.runtime()?.receive(Duration::from_millis(1))? else {
                return Ok(None);
            };
            if let Some(event) = self.translate_runtime_event(event)? {
                return Ok(Some(event));
            }
        }
        Ok(None)
    }

    #[cfg(feature = "production")]
    fn production_long_task(
        &self,
        request_id: String,
        label: String,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let label = required(label, "operation label")?;
        let mut state = self.state()?;
        let operation_id = next_id(&mut state, "host-task");
        state.operations.insert(operation_id.clone());
        state.events.push_back(HostEvent::OperationStarted {
            timestamp: timestamp(),
            operation_id: operation_id.clone(),
            label,
            interruptible: true,
        });
        Ok(CommandAccepted {
            request_id,
            operation_id: Some(operation_id),
        })
    }

    #[cfg(feature = "production")]
    fn production_clear_session(
        &self,
        request_id: String,
    ) -> Result<CommandAccepted, FeatureHostError> {
        self.runtime()?.clear_session()?;
        let mut state = self.state()?;
        state.session_active = false;
        state.events.push_back(HostEvent::SessionCleared {
            timestamp: timestamp(),
        });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn production_capability_request(
        &self,
        request_id: String,
        mini_app_id: String,
        capability: String,
        reason: String,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let mini_app_id = required(mini_app_id, "miniAppId")?;
        let capability = required(capability, "capability")?;
        let reason = required(reason, "reason")?;
        let mut state = self.state()?;
        if !state.installed.contains_key(&mini_app_id) {
            return Err(FeatureHostError::Contract(format!(
                "MiniApp is not installed: {mini_app_id}"
            )));
        }
        let approval_id = next_id(&mut state, "approval");
        state.pending_approvals.insert(
            approval_id.clone(),
            PendingApproval {
                mini_app_id: mini_app_id.clone(),
                capability: capability.clone(),
                runtime_approval_id: None,
            },
        );
        state.events.push_back(HostEvent::ApprovalRequested {
            timestamp: timestamp(),
            approval_id,
            mini_app_id,
            capability,
            reason,
            kind: Some("capability".into()),
            subject: None,
            detail: None,
            proposed_rule: None,
            location: None,
        });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn production_open(
        &self,
        request_id: String,
        mini_app_id: String,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let mini_app_id = required(mini_app_id, "miniAppId")?;
        if !self.state()?.installed.contains_key(&mini_app_id) {
            return Err(FeatureHostError::Contract(format!(
                "MiniApp is not installed: {mini_app_id}"
            )));
        }
        let html = match self.runtime()?.execute(RuntimeCommand::PluginUi {
            plugin_id: mini_app_id.clone(),
        })? {
            RuntimeResponse::PluginUi { html, .. } => html,
            other => return Err(unexpected_response("miniapp.open", other)),
        };
        self.state()?.events.push_back(HostEvent::MiniAppOpened {
            timestamp: timestamp(),
            mini_app_id,
            html: Some(html),
        });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn production_install(
        &self,
        request_id: String,
        mini_app_id: String,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let mini_app_id = required(mini_app_id, "miniAppId")?;
        let response = self.runtime()?.execute(RuntimeCommand::ListCapabilities {
            query: Some(format!("miniapp.{mini_app_id}")),
        })?;
        let available = match response {
            RuntimeResponse::Capabilities { data } => data.into_iter().any(|capability| {
                capability.plugin_id.as_deref() == Some(mini_app_id.as_str())
                    && capability.is_invokable()
            }),
            other => return Err(unexpected_response("marketplace.install", other)),
        };
        if !available {
            return Err(FeatureHostError::Contract(format!(
                "MiniApp is unavailable in the production Runtime: {mini_app_id}"
            )));
        }
        let version = "bundled".to_string();
        let mut state = self.state()?;
        state.installed.insert(mini_app_id.clone(), version.clone());
        state.events.push_back(HostEvent::MarketplaceInstalled {
            timestamp: timestamp(),
            mini_app_id,
            version,
        });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn production_chat(
        &self,
        request_id: String,
        text: String,
        agent_id: Option<String>,
        requested_conversation_id: Option<String>,
        mode: AgentMode,
        mode_statement: Option<String>,
        model: Option<String>,
        attachments: Vec<AttachmentContext>,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let text = required(text, "chat text")?;
        if let Some(mini_app_id) = agent_id.as_deref().filter(|id| *id != "mahayana-assistant") {
            match self
                .runtime()?
                .execute(RuntimeCommand::ApproveLocalPluginTool {
                    plugin_id: mini_app_id.to_string(),
                    tool: "chat".to_string(),
                })? {
                RuntimeResponse::LocalPluginToolApproved { .. } => {}
                other => return Err(unexpected_response("miniapp.chat.approve", other)),
            }
            let response = self
                .runtime()?
                .execute(RuntimeCommand::CallLocalPluginTool {
                    plugin_id: mini_app_id.to_string(),
                    tool: "chat".to_string(),
                    arguments: json!({"message": text}),
                })?;
            let reply = match response {
                RuntimeResponse::LocalPluginToolResult { result, .. } => result
                    .pointer("/content/0/text")
                    .and_then(Value::as_str)
                    .filter(|reply| !reply.is_empty())
                    .unwrap_or("已收到。请选择应用内的快捷操作继续。")
                    .to_string(),
                other => return Err(unexpected_response("miniapp.chat", other)),
            };
            let mut state = self.state()?;
            state.events.push_back(HostEvent::ChatMessage {
                timestamp: timestamp(),
                role: MessageRole::User,
                text,
                operation_id: None,
            });
            state.events.push_back(HostEvent::ChatMessage {
                timestamp: timestamp(),
                role: MessageRole::Assistant,
                text: reply,
                operation_id: None,
            });
            return Ok(CommandAccepted {
                request_id,
                operation_id: None,
            });
        }
        let conversation_id = requested_conversation_id
            .map(ConversationId)
            .or_else(|| {
                agent_id
                    .filter(|id| id != "mahayana-assistant")
                    .map(|id| ConversationId(format!("miniapp:{id}")))
            })
            .unwrap_or_else(|| ConversationId(CODEX_ASSISTANT_CONVERSATION_ID.to_string()));
        let (provider, routed_model) = match self.runtime()?.execute(RuntimeCommand::Status)? {
            RuntimeResponse::Status(status) => (
                format!("{:?}", status.model_provider).to_lowercase(),
                status.model,
            ),
            other => return Err(unexpected_response("runtime.status", other)),
        };
        let runtime_text =
            compose_agent_input(&text, mode, mode_statement.as_deref(), &attachments);
        let response = self.runtime()?.execute(RuntimeCommand::SendMessage {
            conversation_id,
            text: runtime_text,
            client_message_id: Some(request_id.clone()),
        })?;
        let operation_id = match response {
            RuntimeResponse::Accepted { operation_id } => operation_id.to_string(),
            other => return Err(unexpected_response("chat.send", other)),
        };
        let mut state = self.state()?;
        state.operations.insert(operation_id.clone());
        state.events.push_back(HostEvent::ModelRouted {
            timestamp: timestamp(),
            operation_id: operation_id.clone(),
            provider,
            model: routed_model.clone(),
            mode,
        });
        if let Some(preferred_model) = model.filter(|preferred| preferred != &routed_model) {
            state.events.push_back(HostEvent::AgentStep {
                timestamp: timestamp(),
                operation_id: Some(operation_id.clone()),
                step_id: format!("{operation_id}:model-preference"),
                kind: "model".into(),
                title: format!("使用已配置模型 {routed_model}"),
                detail: Some(format!(
                    "本次偏好 {preferred_model}；当前 Runtime 不支持会话中热切换"
                )),
                status: AgentStepStatus::Completed,
                progress: None,
                total: None,
            });
        }
        state.events.push_back(HostEvent::ChatMessage {
            timestamp: timestamp(),
            role: MessageRole::User,
            text,
            operation_id: None,
        });
        state.events.push_back(HostEvent::OperationStarted {
            timestamp: timestamp(),
            operation_id: operation_id.clone(),
            label: "chat-response".into(),
            interruptible: true,
        });
        Ok(CommandAccepted {
            request_id,
            operation_id: Some(operation_id),
        })
    }

    #[cfg(feature = "production")]
    fn production_list_conversations(
        &self,
        request_id: String,
        query: Option<String>,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let conversations = match self.runtime()?.execute(RuntimeCommand::ListConversations)? {
            RuntimeResponse::Conversations { data } => data,
            other => return Err(unexpected_response("conversation.list", other)),
        };
        let query = query.map(|query| query.to_lowercase());
        let conversations = conversations
            .into_iter()
            .filter(|conversation| {
                query.as_ref().is_none_or(|query| {
                    conversation.title.to_lowercase().contains(query)
                        || conversation.id.0.to_lowercase().contains(query)
                })
            })
            .map(|conversation| ConversationSummary {
                id: conversation.id.0,
                title: conversation.title,
                kind: conversation.peer.provider_key().into(),
                pinned: conversation.pinned,
                unread_count: conversation.unread_count,
                updated_at_ms: conversation.updated_at_ms,
            })
            .collect();
        self.state()?
            .events
            .push_back(HostEvent::ConversationListed {
                timestamp: timestamp(),
                conversations,
            });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn production_open_conversation(
        &self,
        request_id: String,
        conversation_id: String,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let conversation_id = required(conversation_id, "conversationId")?;
        let messages = match self
            .runtime()?
            .execute(RuntimeCommand::ConversationHistory {
                conversation_id: ConversationId(conversation_id.clone()),
                limit: Some(200),
            })? {
            RuntimeResponse::History { data } => data,
            other => return Err(unexpected_response("conversation.open", other)),
        };
        let messages = messages
            .into_iter()
            .map(|message| ConversationMessage {
                id: message.id.0,
                role: match message.role {
                    RuntimeMessageRole::User => MessageRole::User,
                    _ => MessageRole::Assistant,
                },
                text: message.text,
                created_at_ms: message.created_at_ms,
            })
            .collect();
        self.state()?
            .events
            .push_back(HostEvent::ConversationOpened {
                timestamp: timestamp(),
                conversation_id,
                messages,
            });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn production_list_capabilities(
        &self,
        request_id: String,
        query: Option<String>,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let response = self
            .runtime()?
            .execute(RuntimeCommand::ListCapabilities { query })?;
        let data = match response {
            RuntimeResponse::Capabilities { data } => data,
            other => return Err(unexpected_response("capability.list", other)),
        };
        let capabilities = data
            .into_iter()
            .map(|capability| CapabilitySummary {
                id: capability.id,
                title: capability.title,
                kind: match capability.kind {
                    CapabilityKind::Agent => "agent",
                    CapabilityKind::Bot => "bot",
                    CapabilityKind::Plugin => "plugin",
                    CapabilityKind::MiniApp => "miniApp",
                    CapabilityKind::Application => "application",
                    CapabilityKind::Contact => "contact",
                }
                .into(),
                mention: capability.mention,
                conversation_id: capability.conversation_id.to_string(),
                provider: capability.provider,
                plugin_id: capability.plugin_id,
                description: capability.description,
                required_permissions: capability.required_permissions,
                availability: match capability.availability {
                    CapabilityAvailability::Ready => "ready",
                    CapabilityAvailability::PermissionRequired => "permissionRequired",
                    CapabilityAvailability::Unavailable => "unavailable",
                }
                .into(),
                unavailable_reason: capability.unavailable_reason,
            })
            .collect();
        self.state()?.events.push_back(HostEvent::CapabilityListed {
            timestamp: timestamp(),
            capabilities,
        });
        Ok(CommandAccepted {
            request_id,
            operation_id: None,
        })
    }

    #[cfg(feature = "production")]
    fn execute_production(
        &self,
        command: FeatureCommand,
    ) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        {
            let state = self.state()?;
            ensure_open(&state)?;
        }
        match command {
            FeatureCommand::ChatSend {
                text,
                agent_id,
                conversation_id,
                mode,
                mode_statement,
                model,
                attachments,
                ..
            } => self.production_chat(
                request_id,
                text,
                agent_id,
                conversation_id,
                mode,
                mode_statement,
                model,
                attachments,
            ),
            FeatureCommand::ConversationList { query, .. } => {
                self.production_list_conversations(request_id, query)
            }
            FeatureCommand::ConversationOpen {
                conversation_id, ..
            } => self.production_open_conversation(request_id, conversation_id),
            FeatureCommand::CapabilityList { query, .. } => {
                self.production_list_capabilities(request_id, query)
            }
            FeatureCommand::MarketplaceInstall { mini_app_id, .. } => {
                self.production_install(request_id, mini_app_id)
            }
            FeatureCommand::MiniAppOpen { mini_app_id, .. } => {
                self.production_open(request_id, mini_app_id)
            }
            FeatureCommand::CapabilityRequest {
                mini_app_id,
                capability,
                reason,
                ..
            } => self.production_capability_request(request_id, mini_app_id, capability, reason),
            FeatureCommand::RuntimeLongTask { label, .. } => {
                self.production_long_task(request_id, label)
            }
            FeatureCommand::SessionClear { .. } => self.production_clear_session(request_id),
            _ => unreachable!(
                "automation and product-surface commands are intercepted before production dispatch"
            ),
        }
    }

    fn execute_test(&self, command: FeatureCommand) -> Result<CommandAccepted, FeatureHostError> {
        let request_id = command.request_id().to_string();
        let mut state = self.state()?;
        ensure_open(&state)?;
        match command {
            FeatureCommand::ChatSend {
                text,
                agent_id,
                mode,
                model,
                attachments,
                ..
            } => {
                let text = required(text, "chat text")?;
                let operation_id = next_id(&mut state, "chat");
                state.events.push_back(HostEvent::ChatMessage {
                    timestamp: timestamp(),
                    role: MessageRole::User,
                    text: text.clone(),
                    operation_id: None,
                });
                state.events.push_back(HostEvent::OperationStarted {
                    timestamp: timestamp(),
                    operation_id: operation_id.clone(),
                    label: "chat-response".into(),
                    interruptible: false,
                });
                state.events.push_back(HostEvent::ModelRouted {
                    timestamp: timestamp(),
                    operation_id: operation_id.clone(),
                    provider: "mahayana-test".into(),
                    model: model.unwrap_or_else(|| "auto".into()),
                    mode,
                });
                state.events.push_back(HostEvent::AgentStep {
                    timestamp: timestamp(),
                    operation_id: Some(operation_id.clone()),
                    step_id: format!("{operation_id}:context"),
                    kind: "context".into(),
                    title: if attachments.is_empty() {
                        "分析请求".into()
                    } else {
                        format!("读取 {} 个附件", attachments.len())
                    },
                    detail: None,
                    status: AgentStepStatus::Completed,
                    progress: Some(1),
                    total: Some(1),
                });
                state.events.push_back(HostEvent::ChatMessage {
                    timestamp: timestamp(),
                    role: MessageRole::Assistant,
                    text: agent_id
                        .filter(|id| id != "mahayana-assistant")
                        .map(|id| format!("{id}机器人收到：{text}"))
                        .unwrap_or_else(|| format!("收到：{text}")),
                    operation_id: Some(operation_id.clone()),
                });
                state.events.push_back(HostEvent::UsageUpdated {
                    timestamp: timestamp(),
                    operation_id: operation_id.clone(),
                    input_tokens: text.chars().count() as i64,
                    cached_input_tokens: 0,
                    output_tokens: 8,
                    reasoning_tokens: 0,
                    total_tokens: text.chars().count() as i64 + 8,
                    context_window: Some(128_000),
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: Some(operation_id),
                })
            }
            FeatureCommand::ConversationList { query, .. } => {
                let mut conversations = vec![ConversationSummary {
                    id: "codex:agent:assistant".into(),
                    title: "大乘助手".into(),
                    kind: "codex".into(),
                    pinned: true,
                    unread_count: 0,
                    updated_at_ms: 0,
                }];
                if let Some(query) = query {
                    conversations.retain(|item| item.title.contains(&query));
                }
                state.events.push_back(HostEvent::ConversationListed {
                    timestamp: timestamp(),
                    conversations,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::ConversationOpen {
                conversation_id, ..
            } => {
                state.events.push_back(HostEvent::ConversationOpened {
                    timestamp: timestamp(),
                    conversation_id,
                    messages: Vec::new(),
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::CapabilityList { query, .. } => {
                let mut capabilities = vec![CapabilitySummary {
                    id: "agent.mahayana".into(),
                    title: "大乘助手".into(),
                    kind: "agent".into(),
                    mention: "@agent.mahayana".into(),
                    conversation_id: "codex:agent:assistant".into(),
                    provider: "codex".into(),
                    plugin_id: None,
                    description: "大乘共享智能代理".into(),
                    required_permissions: Vec::new(),
                    availability: "ready".into(),
                    unavailable_reason: None,
                }];
                capabilities.extend(state.installed.keys().map(|plugin_id| CapabilitySummary {
                    id: format!("miniapp.{plugin_id}"),
                    title: plugin_id.clone(),
                    kind: "miniApp".into(),
                    mention: format!("@miniapp.{plugin_id}"),
                    conversation_id: format!("miniapp:{plugin_id}"),
                    provider: "miniapp".into(),
                    plugin_id: Some(plugin_id.clone()),
                    description: "大乘共享插件、小程序、应用或机器人能力".into(),
                    required_permissions: Vec::new(),
                    availability: "ready".into(),
                    unavailable_reason: None,
                }));
                if let Some(query) = query {
                    let query = query.to_lowercase();
                    capabilities.retain(|item| {
                        item.id.to_lowercase().contains(&query)
                            || item.title.to_lowercase().contains(&query)
                            || item.description.to_lowercase().contains(&query)
                    });
                }
                state.events.push_back(HostEvent::CapabilityListed {
                    timestamp: timestamp(),
                    capabilities,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::MarketplaceInstall { mini_app_id, .. } => {
                let mini_app_id = required(mini_app_id, "miniAppId")?;
                let version = "1.0.0".to_string();
                state.installed.insert(mini_app_id.clone(), version.clone());
                state.events.push_back(HostEvent::MarketplaceInstalled {
                    timestamp: timestamp(),
                    mini_app_id,
                    version,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::MiniAppOpen { mini_app_id, .. } => {
                let mini_app_id = required(mini_app_id, "miniAppId")?;
                if !state.installed.contains_key(&mini_app_id) {
                    return Err(FeatureHostError::Contract(format!(
                        "MiniApp is not installed: {mini_app_id}"
                    )));
                }
                state.events.push_back(HostEvent::MiniAppOpened {
                    timestamp: timestamp(),
                    mini_app_id,
                    html: Some(
                        "<!doctype html><html><body><h1>测试 MiniApp</h1></body></html>".into(),
                    ),
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::CapabilityRequest {
                mini_app_id,
                capability,
                reason,
                ..
            } => {
                let mini_app_id = required(mini_app_id, "miniAppId")?;
                let capability = required(capability, "capability")?;
                let reason = required(reason, "reason")?;
                let approval_id = next_id(&mut state, "approval");
                state.pending_approvals.insert(
                    approval_id.clone(),
                    PendingApproval {
                        mini_app_id: mini_app_id.clone(),
                        capability: capability.clone(),
                        runtime_approval_id: None,
                    },
                );
                state.events.push_back(HostEvent::ApprovalRequested {
                    timestamp: timestamp(),
                    approval_id,
                    mini_app_id,
                    capability,
                    reason,
                    kind: Some("capability".into()),
                    subject: None,
                    detail: None,
                    proposed_rule: None,
                    location: None,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            FeatureCommand::RuntimeLongTask { label, .. } => {
                let label = required(label, "operation label")?;
                let operation_id = next_id(&mut state, "operation");
                state.operations.insert(operation_id.clone());
                state.events.push_back(HostEvent::OperationStarted {
                    timestamp: timestamp(),
                    operation_id: operation_id.clone(),
                    label,
                    interruptible: true,
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: Some(operation_id),
                })
            }
            FeatureCommand::SessionClear { .. } => {
                state.session_active = false;
                state.events.push_back(HostEvent::SessionCleared {
                    timestamp: timestamp(),
                });
                Ok(CommandAccepted {
                    request_id,
                    operation_id: None,
                })
            }
            _ => unreachable!(
                "automation and product-surface commands are intercepted before test dispatch"
            ),
        }
    }

    fn state(&self) -> Result<MutexGuard<'_, FeatureState>, FeatureHostError> {
        self.state
            .lock()
            .map_err(|_| FeatureHostError::StatePoisoned)
    }
}

fn transcript_cards_from_metadata(metadata: &Value) -> Vec<TranscriptCard> {
    if let Some(cards) = metadata.get("cards").and_then(Value::as_array) {
        return cards.iter().filter_map(decode_transcript_card).collect();
    }
    for field in ["transcriptCard", "card", "artifact"] {
        if let Some(card) = metadata.get(field).and_then(decode_transcript_card) {
            return vec![card];
        }
    }
    decode_transcript_card(metadata).into_iter().collect()
}

fn decode_transcript_card(value: &Value) -> Option<TranscriptCard> {
    let mut value = value.clone();
    let object = value.as_object_mut()?;
    if !object.contains_key("kind") {
        let kind = object.get("type").cloned()?;
        object.insert("kind".into(), kind);
    }
    serde_json::from_value(value).ok()
}

fn is_product_surface_command(command: &FeatureCommand) -> bool {
    matches!(
        command,
        FeatureCommand::ConnectorList { .. }
            | FeatureCommand::ConnectorConnect { .. }
            | FeatureCommand::ConnectorRenameAccount { .. }
            | FeatureCommand::ConnectorRemoveAccount { .. }
            | FeatureCommand::ConnectorSetToolEnabled { .. }
            | FeatureCommand::SkillList { .. }
            | FeatureCommand::SkillUpsert { .. }
            | FeatureCommand::SkillDelete { .. }
            | FeatureCommand::SkillPublish { .. }
            | FeatureCommand::SkillUnpublish { .. }
            | FeatureCommand::SkillSync { .. }
            | FeatureCommand::BotList { .. }
            | FeatureCommand::BotSetHidden { .. }
            | FeatureCommand::DraftResolve { .. }
            | FeatureCommand::SecretProvide { .. }
            | FeatureCommand::ListenerList { .. }
            | FeatureCommand::ListenerConnect { .. }
            | FeatureCommand::UpdateStatus { .. }
            | FeatureCommand::UpdateCheck { .. }
            | FeatureCommand::UpdateInstall { .. }
    )
}

#[cfg(feature = "production")]
#[derive(Debug, Clone, Default)]
struct LiveConnectorProjection {
    server_name: Option<String>,
    connector_id: Option<String>,
    install_url: Option<String>,
    status: Option<ConnectorStatus>,
    accounts: BTreeMap<String, ConnectorAccountSummary>,
    tools: BTreeMap<String, ConnectorToolSummary>,
    tool_schemas: BTreeMap<String, Value>,
}

#[cfg(feature = "production")]
fn connector_key_from_name(value: &str) -> Option<&'static str> {
    let normalized = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    if normalized.contains("gmail") || normalized.contains("googlemail") {
        Some("gmail")
    } else if normalized.contains("github") {
        Some("github")
    } else if normalized.contains("slack") {
        Some("slack")
    } else if normalized.contains("microsoftteams") || normalized == "teams" {
        Some("teams")
    } else if normalized.contains("linear") {
        Some("linear")
    } else if normalized.contains("sentry") {
        Some("sentry")
    } else if normalized.contains("pagerduty") {
        Some("pagerduty")
    } else if normalized == "git" {
        Some("git")
    } else {
        None
    }
}

#[cfg(feature = "production")]
fn connector_slug(name: &str) -> String {
    let mut slug = String::new();
    let mut needs_dash = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            if needs_dash && !slug.is_empty() {
                slug.push('-');
            }
            needs_dash = false;
            slug.push(character.to_ascii_lowercase());
        } else {
            needs_dash = true;
        }
    }
    if slug.is_empty() { "app".into() } else { slug }
}

#[cfg(feature = "production")]
fn connector_status_from_auth(auth_status: Option<&str>, has_tools: bool) -> ConnectorStatus {
    match auth_status.unwrap_or_default() {
        "notLoggedIn" => ConnectorStatus::AuthRequired,
        "oAuth" | "bearerToken" => ConnectorStatus::Connected,
        "unsupported" if has_tools => ConnectorStatus::Connected,
        "unknown" if has_tools => ConnectorStatus::Connected,
        _ if has_tools => ConnectorStatus::Connected,
        _ => ConnectorStatus::Disconnected,
    }
}

#[cfg(feature = "production")]
fn live_connector_projections(
    servers: &[Value],
    apps: &[Value],
) -> BTreeMap<String, LiveConnectorProjection> {
    let mut live = BTreeMap::<String, LiveConnectorProjection>::new();
    for app in apps {
        let name = app.get("name").and_then(Value::as_str).unwrap_or_default();
        let id = app.get("id").and_then(Value::as_str).unwrap_or_default();
        let Some(key) = connector_key_from_name(name).or_else(|| connector_key_from_name(id))
        else {
            continue;
        };
        let entry = live.entry(key.to_string()).or_default();
        entry.connector_id = (!id.is_empty()).then(|| id.to_string());
        entry.install_url = app
            .get("installUrl")
            .and_then(Value::as_str)
            .map(str::to_string);
        if app.get("isAccessible").and_then(Value::as_bool) == Some(true) {
            entry.status = Some(ConnectorStatus::Connected);
        }
    }
    for server in servers {
        let server_name = server
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let auth_status = server.get("authStatus").and_then(Value::as_str);
        let tools = server
            .get("tools")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let direct_key = connector_key_from_name(server_name);
        if let Some(key) = direct_key {
            let entry = live.entry(key.to_string()).or_default();
            entry.server_name = Some(server_name.to_string());
            entry.status = Some(connector_status_from_auth(auth_status, !tools.is_empty()));
        }
        for (wire_name, tool) in tools {
            let meta = tool.get("_meta").and_then(Value::as_object);
            let connector_name = meta
                .and_then(|meta| meta.get("connector_name"))
                .and_then(Value::as_str);
            let connector_id = meta
                .and_then(|meta| meta.get("connector_id"))
                .and_then(Value::as_str);
            let Some(key) = connector_name
                .and_then(connector_key_from_name)
                .or_else(|| connector_id.and_then(connector_key_from_name))
                .or(direct_key)
            else {
                continue;
            };
            let entry = live.entry(key.to_string()).or_default();
            entry.server_name = Some(server_name.to_string());
            entry.status = Some(ConnectorStatus::Connected);
            if let Some(connector_id) = connector_id {
                entry.connector_id = Some(connector_id.to_string());
                if entry.install_url.is_none() {
                    let display = connector_name.unwrap_or(key);
                    entry.install_url = Some(format!(
                        "https://chatgpt.com/apps/{}/{}",
                        connector_slug(display),
                        connector_id
                    ));
                }
            }
            let tool_id = tool
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(wire_name.as_str())
                .to_string();
            let description = tool
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let read_only = tool
                .pointer("/annotations/readOnlyHint")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if let Some(schema) = tool
                .get("inputSchema")
                .or_else(|| tool.get("input_schema"))
                .cloned()
            {
                entry.tool_schemas.insert(tool_id.clone(), schema);
            }
            entry.tools.insert(
                tool_id.clone(),
                ConnectorToolSummary {
                    id: tool_id.clone(),
                    name: tool
                        .get("title")
                        .and_then(Value::as_str)
                        .filter(|title| !title.trim().is_empty())
                        .unwrap_or(tool_id.as_str())
                        .to_string(),
                    description,
                    enabled: true,
                    requires_approval: Some(!read_only),
                },
            );
            let link_id = meta
                .and_then(|meta| meta.get("link_id"))
                .and_then(Value::as_str);
            let owner = meta
                .and_then(|meta| meta.get("link_owner_profile"))
                .and_then(Value::as_object);
            if link_id.is_some() || owner.is_some() {
                let account_id = link_id
                    .map(|link_id| format!("mcp:{key}:{link_id}"))
                    .unwrap_or_else(|| format!("mcp:{key}"));
                let email = owner
                    .and_then(|owner| owner.get("email"))
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let label = owner
                    .and_then(|owner| {
                        owner
                            .get("name")
                            .or_else(|| owner.get("nickname"))
                            .and_then(Value::as_str)
                    })
                    .map(str::to_string)
                    .or_else(|| email.clone())
                    .or_else(|| connector_name.map(str::to_string))
                    .unwrap_or_else(|| key.to_string());
                entry.accounts.insert(
                    account_id.clone(),
                    ConnectorAccountSummary {
                        id: account_id,
                        label,
                        status: ConnectorStatus::Connected,
                        email,
                        team_managed: Some(false),
                        error: None,
                    },
                );
            }
        }
    }
    live
}

#[cfg(feature = "production")]
fn projection_send_tool<'a>(
    projection: &'a LiveConnectorProjection,
    connector_id: &str,
) -> Option<(&'a str, Option<&'a Value>)> {
    let preferred: &[&str] = match connector_id {
        "gmail" => &[
            "gmail.send_email",
            "send_email",
            "gmail.send_draft",
            "send_draft",
        ],
        "slack" => &[
            "slack.send_message",
            "slack.post_message",
            "send_message",
            "post_message",
        ],
        _ => &[],
    };
    for candidate in preferred {
        if let Some((tool_id, _)) = projection
            .tools
            .iter()
            .find(|(tool_id, _)| tool_id.as_str() == *candidate || tool_id.ends_with(candidate))
        {
            return Some((tool_id.as_str(), projection.tool_schemas.get(tool_id)));
        }
    }
    None
}

#[cfg(feature = "production")]
fn schema_property_is_array(schema: Option<&Value>, name: &str) -> bool {
    schema
        .and_then(|schema| schema.pointer(&format!("/properties/{name}/type")))
        .and_then(Value::as_str)
        == Some("array")
}

#[cfg(feature = "production")]
fn draft_tool_arguments(
    draft: &MessageDraft,
    schema: Option<&Value>,
) -> Result<Value, FeatureHostError> {
    let properties = schema
        .and_then(|schema| schema.get("properties"))
        .and_then(Value::as_object);
    match draft {
        MessageDraft::Email {
            from,
            to,
            cc,
            subject,
            body,
            ..
        } => {
            if to.is_empty() {
                return Err(FeatureHostError::Contract(
                    "email draft requires at least one recipient".into(),
                ));
            }
            let mut arguments = serde_json::Map::new();
            if schema_property_is_array(schema, "to") {
                arguments.insert("to".into(), json!(to));
            } else {
                arguments.insert("to".into(), Value::String(to.join(", ")));
            }
            arguments.insert("subject".into(), Value::String(subject.clone()));
            if properties.is_some_and(|properties| properties.contains_key("payload")) {
                arguments.insert(
                    "payload".into(),
                    json!({
                        "mime_type": "text/plain",
                        "charset": "UTF-8",
                        "body": {"content": body}
                    }),
                );
            } else if properties.is_some_and(|properties| properties.contains_key("message")) {
                arguments.insert("message".into(), Value::String(body.clone()));
            } else {
                arguments.insert("body".into(), Value::String(body.clone()));
            }
            if let Some(cc) = cc.as_ref().filter(|cc| !cc.is_empty()) {
                if schema_property_is_array(schema, "cc") {
                    arguments.insert("cc".into(), json!(cc));
                } else {
                    arguments.insert("cc".into(), Value::String(cc.join(", ")));
                }
            }
            if let Some(from) = from.as_ref().filter(|from| !from.trim().is_empty()) {
                let key = if properties
                    .is_some_and(|properties| properties.contains_key("from_address"))
                {
                    "from_address"
                } else {
                    "from"
                };
                if properties.is_none_or(|properties| properties.contains_key(key)) {
                    arguments.insert(key.into(), Value::String(from.clone()));
                }
            }
            Ok(Value::Object(arguments))
        }
        MessageDraft::Slack {
            target,
            thread,
            body,
            ..
        } => {
            if target.trim().is_empty() || body.trim().is_empty() {
                return Err(FeatureHostError::Contract(
                    "Slack draft requires a target and message".into(),
                ));
            }
            let properties = properties.ok_or_else(|| {
                FeatureHostError::Contract(
                    "Slack connector did not expose an input schema for its send tool".into(),
                )
            })?;
            let target_key = [
                "channel",
                "channel_id",
                "target",
                "conversation",
                "conversation_id",
            ]
            .into_iter()
            .find(|key| properties.contains_key(*key))
            .ok_or_else(|| {
                FeatureHostError::Contract(
                    "Slack send tool has no supported channel/target parameter".into(),
                )
            })?;
            let body_key = ["text", "message", "body"]
                .into_iter()
                .find(|key| properties.contains_key(*key))
                .ok_or_else(|| {
                    FeatureHostError::Contract(
                        "Slack send tool has no supported message parameter".into(),
                    )
                })?;
            let mut arguments = serde_json::Map::new();
            arguments.insert(target_key.into(), Value::String(target.clone()));
            arguments.insert(body_key.into(), Value::String(body.clone()));
            if let Some(thread) = thread.as_ref().filter(|thread| !thread.trim().is_empty())
                && let Some(thread_key) = ["thread_ts", "thread", "thread_id"]
                    .into_iter()
                    .find(|key| properties.contains_key(*key))
            {
                arguments.insert(thread_key.into(), Value::String(thread.clone()));
            }
            Ok(Value::Object(arguments))
        }
    }
}

#[cfg(feature = "production")]
fn merge_live_connectors(
    mut connectors: Vec<ConnectorSummary>,
    live: &BTreeMap<String, LiveConnectorProjection>,
) -> Vec<ConnectorSummary> {
    for connector in &mut connectors {
        let Some(projection) = live.get(&connector.id) else {
            if connector.id == "git" {
                connector.status = ConnectorStatus::Connected;
                connector.can_add_account = false;
            }
            continue;
        };
        if let Some(status) = projection.status {
            connector.status = status;
        }
        connector.can_add_account = projection.install_url.is_some()
            || projection
                .server_name
                .as_deref()
                .is_some_and(|name| name != "codex_apps");
        if let Some(source) = projection
            .connector_id
            .as_ref()
            .or(projection.server_name.as_ref())
        {
            connector.source = Some(source.clone());
        }
        let enabled_preferences = connector
            .tools
            .iter()
            .map(|tool| (tool.id.clone(), tool.enabled))
            .collect::<BTreeMap<_, _>>();
        if !projection.tools.is_empty() {
            connector.tools = projection
                .tools
                .values()
                .cloned()
                .map(|mut tool| {
                    if let Some(enabled) = enabled_preferences.get(&tool.id) {
                        tool.enabled = *enabled;
                    }
                    tool
                })
                .collect();
        }
        if !projection.accounts.is_empty() {
            let labels = connector
                .accounts
                .iter()
                .map(|account| (account.id.clone(), account.label.clone()))
                .collect::<BTreeMap<_, _>>();
            connector.accounts = projection
                .accounts
                .values()
                .cloned()
                .map(|mut account| {
                    if let Some(label) = labels.get(&account.id) {
                        account.label = label.clone();
                    }
                    account
                })
                .collect();
        } else if connector.status == ConnectorStatus::Connected
            && connector.accounts.is_empty()
            && projection.server_name.as_deref() != Some("codex_apps")
        {
            connector.accounts.push(ConnectorAccountSummary {
                id: format!("mcp:{}", connector.id),
                label: projection
                    .server_name
                    .clone()
                    .unwrap_or_else(|| connector.display_name.clone()),
                status: ConnectorStatus::Connected,
                email: None,
                team_managed: Some(false),
                error: None,
            });
        }
    }
    connectors.sort_by(|left, right| left.display_name.cmp(&right.display_name));
    connectors
}

fn listener_platform_slug(platform: ListenerPlatform) -> &'static str {
    match platform {
        ListenerPlatform::Slack => "slack",
        ListenerPlatform::Github => "github",
        ListenerPlatform::Git => "git",
        ListenerPlatform::Teams => "teams",
        ListenerPlatform::Linear => "linear",
        ListenerPlatform::Sentry => "sentry",
        ListenerPlatform::Pagerduty => "pagerduty",
    }
}

fn listener_platform_display(platform: ListenerPlatform) -> &'static str {
    match platform {
        ListenerPlatform::Slack => "Slack",
        ListenerPlatform::Github => "GitHub",
        ListenerPlatform::Git => "Git",
        ListenerPlatform::Teams => "Microsoft Teams",
        ListenerPlatform::Linear => "Linear",
        ListenerPlatform::Sentry => "Sentry",
        ListenerPlatform::Pagerduty => "PagerDuty",
    }
}

fn automation_next_run(
    trigger: &AutomationTrigger,
    schedule: &str,
    enabled: bool,
    after_ms: i64,
) -> Option<i64> {
    if !enabled || matches!(trigger, AutomationTrigger::Event { .. }) {
        None
    } else {
        next_automation_run(schedule, after_ms)
    }
}

fn connector_tool(id: &str, name: &str, description: &str) -> ConnectorToolSummary {
    ConnectorToolSummary {
        id: id.into(),
        name: name.into(),
        description: description.into(),
        enabled: true,
        requires_approval: Some(true),
    }
}

fn connector_summary(
    id: &str,
    display_name: &str,
    description: &str,
    transport: ConnectorTransport,
    tools: Vec<ConnectorToolSummary>,
) -> ConnectorSummary {
    ConnectorSummary {
        id: id.into(),
        display_name: display_name.into(),
        description: description.into(),
        status: ConnectorStatus::Disconnected,
        is_team: false,
        can_add_account: true,
        transport,
        source: Some("Built in".into()),
        teammate_count: None,
        accounts: Vec::new(),
        tools,
    }
}

fn default_connectors() -> BTreeMap<String, ConnectorSummary> {
    [
        connector_summary(
            "github",
            "GitHub",
            "Repositories, pull requests, issues, comments and CI.",
            ConnectorTransport::Http,
            vec![
                connector_tool(
                    "read_repository",
                    "Read repository",
                    "Read repository files and metadata.",
                ),
                connector_tool(
                    "create_issue",
                    "Create issue",
                    "Create and update GitHub issues.",
                ),
                connector_tool(
                    "comment_pull_request",
                    "Comment on pull request",
                    "Post review comments on pull requests.",
                ),
            ],
        ),
        connector_summary(
            "slack",
            "Slack",
            "Messages, mentions, reactions and approved drafts.",
            ConnectorTransport::Http,
            vec![
                connector_tool(
                    "search_messages",
                    "Search messages",
                    "Search workspace messages and threads.",
                ),
                connector_tool(
                    "post_message",
                    "Post message",
                    "Send an approved message or thread reply.",
                ),
                connector_tool(
                    "add_reaction",
                    "Add reaction",
                    "Add a reaction to a message.",
                ),
            ],
        ),
        connector_summary(
            "teams",
            "Microsoft Teams",
            "Teams messages, mentions, channels and approved drafts.",
            ConnectorTransport::Http,
            vec![
                connector_tool(
                    "search_messages",
                    "Search messages",
                    "Search Teams channels and chats.",
                ),
                connector_tool(
                    "post_message",
                    "Post message",
                    "Send an approved Teams message.",
                ),
            ],
        ),
        connector_summary(
            "linear",
            "Linear",
            "Issues, comments, status changes and projects.",
            ConnectorTransport::Http,
            vec![
                connector_tool(
                    "read_issues",
                    "Read issues",
                    "Read Linear issues and projects.",
                ),
                connector_tool(
                    "update_issue",
                    "Update issue",
                    "Update issue state, assignee and fields.",
                ),
            ],
        ),
        connector_summary(
            "sentry",
            "Sentry",
            "Errors, regressions, releases and issue ownership.",
            ConnectorTransport::Http,
            vec![
                connector_tool(
                    "read_issues",
                    "Read issues",
                    "Read Sentry issues and events.",
                ),
                connector_tool(
                    "resolve_issue",
                    "Resolve issue",
                    "Resolve or assign a Sentry issue.",
                ),
            ],
        ),
        connector_summary(
            "pagerduty",
            "PagerDuty",
            "Incidents, acknowledgements, responders and escalation.",
            ConnectorTransport::Http,
            vec![
                connector_tool(
                    "read_incidents",
                    "Read incidents",
                    "Read incident details and timelines.",
                ),
                connector_tool(
                    "acknowledge_incident",
                    "Acknowledge incident",
                    "Acknowledge an incident after approval.",
                ),
            ],
        ),
        connector_summary(
            "git",
            "Git",
            "Local commits, branches and repository state.",
            ConnectorTransport::Command,
            vec![
                connector_tool(
                    "read_status",
                    "Read status",
                    "Read local repository status.",
                ),
                connector_tool(
                    "read_history",
                    "Read history",
                    "Read commit and branch history.",
                ),
            ],
        ),
    ]
    .into_iter()
    .map(|connector| (connector.id.clone(), connector))
    .collect()
}

fn default_skill_teams() -> Vec<SkillTeamSummary> {
    vec![SkillTeamSummary {
        id: "team-mahayana".into(),
        name: "Mahayana Team".into(),
    }]
}

fn default_skills() -> BTreeMap<String, SkillSummary> {
    [
        SkillSummary {
            id: "skill-research-brief".into(),
            name: "Research brief".into(),
            description: "Turn verified sources into a concise research brief.".into(),
            use_when: "Use when a task needs sourced research and a decision-ready summary.".into(),
            instructions: "Verify sources, distinguish facts from inference, and end with actionable conclusions.".into(),
            source: SkillSource::Private,
            publish_state: SkillPublishState::Local,
            owner_agent_id: Some("mahayana-assistant".into()),
            team_id: None,
            team_name: None,
            read_only: Some(false),
            updated_at_ms: 0,
        },
        SkillSummary {
            id: "skill-incident-response".into(),
            name: "Incident response".into(),
            description: "Coordinate incident triage across monitoring and communication tools.".into(),
            use_when: "Use when an alert or incident needs coordinated triage.".into(),
            instructions: "Establish severity, collect evidence, propose actions, and request approval before external changes.".into(),
            source: SkillSource::Team,
            publish_state: SkillPublishState::Managed,
            owner_agent_id: None,
            team_id: Some("team-mahayana".into()),
            team_name: Some("Mahayana Team".into()),
            read_only: Some(true),
            updated_at_ms: 0,
        },
    ]
    .into_iter()
    .map(|skill| (skill.id.clone(), skill))
    .collect()
}

fn default_bots() -> BTreeMap<String, BotSummary> {
    [
        BotSummary {
            id: "mahayana-assistant".into(),
            name: "大乘助手".into(),
            description: "General-purpose Mahayana assistant.".into(),
            hidden: false,
            avatar: None,
            conversation_id: Some("codex:agent:assistant".into()),
        },
        BotSummary {
            id: "research-bot".into(),
            name: "Research Bot".into(),
            description: "Source verification and research synthesis.".into(),
            hidden: false,
            avatar: None,
            conversation_id: Some("codex:agent:research".into()),
        },
        BotSummary {
            id: "incident-bot".into(),
            name: "Incident Bot".into(),
            description: "Incident triage and operational coordination.".into(),
            hidden: true,
            avatar: None,
            conversation_id: Some("codex:agent:incident".into()),
        },
    ]
    .into_iter()
    .map(|bot| (bot.id.clone(), bot))
    .collect()
}

fn listener_summary(
    platform: ListenerPlatform,
    display_name: &str,
    blurb: &str,
) -> ListenerIntegrationSummary {
    ListenerIntegrationSummary {
        platform,
        display_name: display_name.into(),
        blurb: blurb.into(),
        is_connected: false,
        account_label: None,
        error: None,
    }
}

fn default_listeners() -> BTreeMap<ListenerPlatform, ListenerIntegrationSummary> {
    [
        listener_summary(
            ListenerPlatform::Github,
            "GitHub",
            "Let automations watch a repo's PRs, comments, issues, and CI.",
        ),
        listener_summary(
            ListenerPlatform::Git,
            "Git",
            "Wake automations on local commits, branches, tags, and repository changes.",
        ),
        listener_summary(
            ListenerPlatform::Slack,
            "Slack",
            "Wake automations on Slack messages, mentions, and reactions.",
        ),
        listener_summary(
            ListenerPlatform::Teams,
            "Microsoft Teams",
            "Wake automations on Teams messages, mentions, and reactions.",
        ),
        listener_summary(
            ListenerPlatform::Linear,
            "Linear",
            "Wake automations on issues, comments, status changes, and assignments.",
        ),
        listener_summary(
            ListenerPlatform::Sentry,
            "Sentry",
            "Wake automations on new, regressed, assigned, and resolved issues.",
        ),
        listener_summary(
            ListenerPlatform::Pagerduty,
            "PagerDuty",
            "Wake automations when incidents are triggered, acknowledged, escalated, or resolved.",
        ),
    ]
    .into_iter()
    .map(|integration| (integration.platform, integration))
    .collect()
}

fn listener_platform_for_connector(connector_id: &str) -> Option<ListenerPlatform> {
    match connector_id {
        "github" => Some(ListenerPlatform::Github),
        "git" => Some(ListenerPlatform::Git),
        "slack" => Some(ListenerPlatform::Slack),
        "teams" => Some(ListenerPlatform::Teams),
        "linear" => Some(ListenerPlatform::Linear),
        "sentry" => Some(ListenerPlatform::Sentry),
        "pagerduty" => Some(ListenerPlatform::Pagerduty),
        _ => None,
    }
}

fn connector_for_listener_platform(platform: ListenerPlatform) -> Option<&'static str> {
    match platform {
        ListenerPlatform::Github => Some("github"),
        ListenerPlatform::Git => Some("git"),
        ListenerPlatform::Slack => Some("slack"),
        ListenerPlatform::Teams => Some("teams"),
        ListenerPlatform::Linear => Some("linear"),
        ListenerPlatform::Sentry => Some("sentry"),
        ListenerPlatform::Pagerduty => Some("pagerduty"),
    }
}

fn validate_draft(draft: &MessageDraft) -> Result<(), FeatureHostError> {
    match draft {
        MessageDraft::Email {
            to, subject, body, ..
        } => {
            if to.is_empty()
                || to
                    .iter()
                    .any(|recipient| !recipient.contains('@') || recipient.trim().is_empty())
            {
                return Err(FeatureHostError::Contract(
                    "email draft requires valid recipients".into(),
                ));
            }
            required(subject.clone(), "email subject")?;
            required(body.clone(), "email body")?;
        }
        MessageDraft::Slack { target, body, .. } => {
            required(target.clone(), "Slack target")?;
            required(body.clone(), "Slack body")?;
        }
    }
    Ok(())
}

#[cfg(feature = "production")]
fn product_surface_method(command: &FeatureCommand) -> &'static str {
    match command {
        FeatureCommand::ConnectorList { .. } => "mahayana.connector.list",
        FeatureCommand::ConnectorConnect { .. } => "mahayana.connector.connect",
        FeatureCommand::ConnectorRenameAccount { .. } => "mahayana.connector.account.rename",
        FeatureCommand::ConnectorRemoveAccount { .. } => "mahayana.connector.account.remove",
        FeatureCommand::ConnectorSetToolEnabled { .. } => "mahayana.connector.tool.setEnabled",
        FeatureCommand::SkillList { .. } => "mahayana.skill.list",
        FeatureCommand::SkillUpsert { .. } => "mahayana.skill.upsert",
        FeatureCommand::SkillDelete { .. } => "mahayana.skill.delete",
        FeatureCommand::SkillPublish { .. } => "mahayana.skill.publish",
        FeatureCommand::SkillUnpublish { .. } => "mahayana.skill.unpublish",
        FeatureCommand::SkillSync { .. } => "mahayana.skill.sync",
        FeatureCommand::BotList { .. } => "mahayana.bot.list",
        FeatureCommand::BotSetHidden { .. } => "mahayana.bot.setHidden",
        FeatureCommand::DraftResolve { .. } => "mahayana.draft.resolve",
        FeatureCommand::SecretProvide { .. } => "mahayana.secret.provide",
        FeatureCommand::ListenerList { .. } => "mahayana.listener.list",
        FeatureCommand::ListenerConnect { .. } => "mahayana.listener.connect",
        FeatureCommand::UpdateStatus { .. } => "mahayana.update.status",
        FeatureCommand::UpdateCheck { .. } => "mahayana.update.check",
        FeatureCommand::UpdateInstall { .. } => "mahayana.update.install",
        _ => unreachable!("non-product command has no product method"),
    }
}

#[cfg(feature = "production")]
fn decode_product_value<T: DeserializeOwned>(
    value: Value,
    method: &str,
) -> Result<T, FeatureHostError> {
    serde_json::from_value(value)
        .map_err(|error| FeatureHostError::Contract(format!("decode {method} response: {error}")))
}

#[cfg(feature = "production")]
fn decode_product_field<T: DeserializeOwned>(
    value: Value,
    field: &str,
    method: &str,
) -> Result<T, FeatureHostError> {
    let value = value.get(field).cloned().unwrap_or(value);
    decode_product_value(value, method)
}

fn ensure_open(state: &FeatureState) -> Result<(), FeatureHostError> {
    if state.closed {
        Err(FeatureHostError::Closed)
    } else {
        Ok(())
    }
}

fn next_id(state: &mut FeatureState, prefix: &str) -> String {
    state.sequence += 1;
    format!("{prefix}-{}", state.sequence)
}

fn validate_config(config: &HostConfig) -> Result<(), FeatureHostError> {
    if config.profile_id.trim().is_empty() {
        Err(FeatureHostError::Contract(
            "profileId must not be empty".into(),
        ))
    } else {
        Ok(())
    }
}

#[cfg(feature = "production")]
fn unexpected_response(command: &str, response: RuntimeResponse) -> FeatureHostError {
    FeatureHostError::Contract(format!(
        "unexpected Runtime response for {command}: {response:?}"
    ))
}

fn required(value: String, name: &str) -> Result<String, FeatureHostError> {
    let value = value.trim();
    if value.is_empty() {
        Err(FeatureHostError::Contract(format!(
            "{name} must not be empty"
        )))
    } else {
        Ok(value.to_string())
    }
}

fn compose_agent_input(
    text: &str,
    mode: AgentMode,
    mode_statement: Option<&str>,
    attachments: &[AttachmentContext],
) -> String {
    if mode == AgentMode::Agent && attachments.is_empty() {
        return text.to_string();
    }
    let mode_instruction = match mode {
        AgentMode::Agent => "请自主使用可用工具完成任务，并明确报告结果。",
        AgentMode::Ask => {
            "请只分析并回答问题；未经用户明确要求，不要修改文件或执行有副作用的操作。"
        }
        AgentMode::Plan => "请先形成可执行计划，列出依赖、风险和验证方式；暂不执行有副作用的操作。",
    };
    let mut input = format!(
        "[Agent 模式]\n{}\n{mode_instruction}\n\n[用户请求]\n{text}",
        mode_statement.unwrap_or("")
    );
    for attachment in attachments {
        input.push_str("\n\n[附件: ");
        input.push_str(&attachment.name);
        input.push_str("]\n");
        input.push_str(attachment.text.as_deref().unwrap_or("（仅提供文件元数据）"));
    }
    input
}

fn is_safe_automation_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 96
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn load_automations(path: &Path) -> BTreeMap<String, AutomationSummary> {
    let Ok(bytes) = std::fs::read(path) else {
        return BTreeMap::new();
    };
    let Ok(items) = serde_json::from_slice::<Vec<AutomationSummary>>(&bytes) else {
        return BTreeMap::new();
    };
    let now = now_millis();
    items
        .into_iter()
        .filter_map(|mut item| {
            if !is_safe_automation_id(&item.id)
                || item.name.trim().is_empty()
                || item.prompt.trim().is_empty()
                || normalize_automation_schedule(&item.schedule).is_err()
            {
                return None;
            }
            item.next_run_at_ms = item
                .enabled
                .then(|| {
                    item.next_run_at_ms
                        .filter(|next| *next > now)
                        .or_else(|| next_automation_run(&item.schedule, now))
                })
                .flatten();
            Some((item.id.clone(), item))
        })
        .collect()
}

fn persist_automations(
    path: &Path,
    automations: &BTreeMap<String, AutomationSummary>,
) -> Result<(), FeatureHostError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| {
            FeatureHostError::Contract(format!("create automation directory: {error}"))
        })?;
    }
    let temp = path.with_extension("json.tmp");
    let data = serde_json::to_vec_pretty(&automations.values().cloned().collect::<Vec<_>>())
        .map_err(|error| FeatureHostError::Contract(format!("serialize automations: {error}")))?;
    std::fs::write(&temp, data)
        .map_err(|error| FeatureHostError::Contract(format!("write automation store: {error}")))?;
    std::fs::rename(&temp, path)
        .map_err(|error| FeatureHostError::Contract(format!("commit automation store: {error}")))?;
    Ok(())
}

fn normalize_automation_schedule(raw: &str) -> Result<String, FeatureHostError> {
    let schedule = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if schedule.is_empty() || next_automation_run(&schedule, now_millis()).is_none() {
        return Err(FeatureHostError::Contract(
            "invalid automation schedule; use a 5-field cron, @hourly/@daily/@weekly/@monthly/@yearly, or @every <n><s|m|h|d>"
                .into(),
        ));
    }
    Ok(schedule)
}

fn parse_every_interval_ms(schedule: &str) -> Option<i64> {
    let rest = schedule.strip_prefix("@every ")?.trim();
    let split = rest
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(rest.len());
    let amount = rest[..split].parse::<i64>().ok()?;
    let unit = rest[split..].trim().to_ascii_lowercase();
    let unit_ms = match unit.as_str() {
        "s" => 1_000,
        "m" => 60_000,
        "h" => 3_600_000,
        "d" => 86_400_000,
        _ => return None,
    };
    amount.checked_mul(unit_ms).filter(|value| *value > 0)
}

fn expand_cron_alias(schedule: &str) -> &str {
    match schedule.to_ascii_lowercase().as_str() {
        "@hourly" => "0 * * * *",
        "@daily" | "@midnight" => "0 0 * * *",
        "@weekly" => "0 0 * * 0",
        "@monthly" => "0 0 1 * *",
        "@yearly" | "@annually" => "0 0 1 1 *",
        _ => schedule,
    }
}

fn parse_cron_field(field: &str, min: u32, max: u32) -> Option<BTreeSet<u32>> {
    let mut values = BTreeSet::new();
    for part in field.split(',') {
        let mut step_parts = part.split('/');
        let range = step_parts.next()?;
        let step: u32 = step_parts
            .next()
            .map_or(Some(1), |value| value.parse().ok())?;
        if step_parts.next().is_some() || step == 0 {
            return None;
        }
        let (start, end) = if range == "*" || range.is_empty() {
            (min, max)
        } else if let Some((start, end)) = range.split_once('-') {
            (start.parse().ok()?, end.parse().ok()?)
        } else {
            let start = range.parse().ok()?;
            (start, if part.contains('/') { max } else { start })
        };
        if start < min || end > max || start > end {
            return None;
        }
        for value in (start..=end).step_by(step as usize) {
            values.insert(if max == 7 && value == 7 { 0 } else { value });
        }
    }
    (!values.is_empty()).then_some(values)
}

fn next_automation_run(schedule: &str, after_ms: i64) -> Option<i64> {
    if let Some(interval) = parse_every_interval_ms(schedule) {
        return after_ms.checked_add(interval);
    }
    let expression = expand_cron_alias(schedule);
    let fields = expression.split_whitespace().collect::<Vec<_>>();
    if fields.len() != 5 {
        return None;
    }
    let minute = parse_cron_field(fields[0], 0, 59)?;
    let hour = parse_cron_field(fields[1], 0, 23)?;
    let day_of_month = parse_cron_field(fields[2], 1, 31)?;
    let month = parse_cron_field(fields[3], 1, 12)?;
    let day_of_week = parse_cron_field(fields[4], 0, 7)?;
    let dom_restricted = fields[2] != "*";
    let dow_restricted = fields[4] != "*";
    let mut candidate = after_ms.div_euclid(60_000) * 60_000 + 60_000;
    for _ in 0..(366 * 24 * 60) {
        let date = chrono::DateTime::<Utc>::from_timestamp_millis(candidate)?;
        let dom_ok = day_of_month.contains(&date.day());
        let dow_ok = day_of_week.contains(&date.weekday().num_days_from_sunday());
        let day_ok = if dom_restricted && dow_restricted {
            dom_ok || dow_ok
        } else {
            (!dom_restricted || dom_ok) && (!dow_restricted || dow_ok)
        };
        if minute.contains(&date.minute())
            && hour.contains(&date.hour())
            && month.contains(&date.month())
            && day_ok
        {
            return Some(candidate);
        }
        candidate = candidate.checked_add(60_000)?;
    }
    None
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

fn timestamp() -> String {
    now_millis().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use mahayana_host_protocol::ApprovalDecision;

    #[cfg(feature = "production")]
    fn isolated_host_config(profile: &str) -> HostCreateConfig {
        let root = std::env::temp_dir().join(format!(
            "fabushi-feature-host-{profile}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("create isolated Host root");
        HostCreateConfig {
            runtime: mahayana_core::RuntimeConfig {
                data_dir: Some(root.join("runtime")),
                ..Default::default()
            },
            product_session_path: Some(root.join("product-session.json")),
            inherit_installed_plugins: Some(false),
            ..Default::default()
        }
    }

    fn controller() -> FeatureHostController {
        FeatureHostController::create(
            HostConfig {
                profile_id: "fast-e2e".into(),
                mode: HostMode::Test,
            },
            SurfacePlatform::Tauri,
        )
        .expect("create feature Host")
    }

    fn drain(controller: &FeatureHostController) -> Vec<HostEvent> {
        let mut events = Vec::new();
        while let Some(event) = controller.receive().expect("receive event") {
            events.push(event);
        }
        events
    }

    #[test]
    fn automation_schedule_supports_the_recovered_grok_grammar() {
        let base = 1_750_000_000_000_i64;
        assert_eq!(parse_every_interval_ms("@every 5m"), Some(300_000));
        assert_eq!(next_automation_run("@every 5m", base), Some(base + 300_000));
        assert!(next_automation_run("@daily", base).is_some());
        assert!(next_automation_run("*/15 9-17 * * 1-5", base).is_some());
        assert!(normalize_automation_schedule("not a schedule").is_err());
    }

    #[test]
    fn automation_crud_and_manual_run_use_the_host_event_contract() {
        let controller = controller();
        drain(&controller);
        controller
            .execute(FeatureCommand::AutomationUpsert {
                request_id: "automation-create".into(),
                id: Some("morning-review".into()),
                name: "晨间复盘".into(),
                prompt: "总结昨天的进展并给出今天的三个行动。".into(),
                schedule: "@daily".into(),
                trigger: Some(AutomationTrigger::Schedule {
                    schedule: "@daily".into(),
                }),
                enabled: true,
            })
            .expect("create automation");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::AutomationChanged { action, automation, .. }
                if action == "created" && automation.id == "morning-review"
        )));

        controller
            .execute(FeatureCommand::AutomationList {
                request_id: "automation-list".into(),
            })
            .expect("list automations");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::AutomationListed { automations, .. }
                if automations.len() == 1 && automations[0].name == "晨间复盘"
        )));

        controller
            .execute(FeatureCommand::AutomationSetEnabled {
                request_id: "automation-pause".into(),
                id: "morning-review".into(),
                enabled: false,
            })
            .expect("pause automation");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::AutomationChanged { action, automation, .. }
                if action == "paused" && !automation.enabled
        )));

        controller
            .execute(FeatureCommand::AutomationRun {
                request_id: "automation-run".into(),
                id: "morning-review".into(),
            })
            .expect("run automation");
        let kinds = drain(&controller)
            .into_iter()
            .map(|event| event.kind())
            .collect::<Vec<_>>();
        assert!(kinds.contains(&"automation.changed"));
        assert!(kinds.contains(&"chat.message"));

        controller
            .execute(FeatureCommand::AutomationDelete {
                request_id: "automation-delete".into(),
                id: "morning-review".into(),
            })
            .expect("delete automation");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::AutomationChanged { action, .. } if action == "deleted"
        )));
    }

    #[test]
    fn product_surfaces_emit_stateful_events_without_leaking_secrets() {
        let controller = controller();
        drain(&controller);

        controller
            .execute(FeatureCommand::ConnectorList {
                request_id: "connector-list".into(),
            })
            .expect("list connectors");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::ConnectorListed { connectors, .. }
                if connectors.iter().any(|connector| connector.id == "github")
        )));

        controller
            .execute(FeatureCommand::ConnectorConnect {
                request_id: "connector-connect".into(),
                connector_id: "github".into(),
                account_label: Some("Work".into()),
            })
            .expect("connect GitHub");
        let events = drain(&controller);
        assert!(events.iter().any(|event| matches!(
            event,
            HostEvent::ConnectorChanged { connector, .. }
                if connector.id == "github"
                    && connector.status == ConnectorStatus::Connected
                    && connector.accounts.iter().any(|account| account.label == "Work")
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            HostEvent::ListenerChanged { integration, .. }
                if integration.platform == ListenerPlatform::Github
                    && integration.is_connected
        )));

        controller
            .execute(FeatureCommand::ConnectorSetToolEnabled {
                request_id: "connector-tool".into(),
                connector_id: "github".into(),
                tool_id: "create_issue".into(),
                enabled: false,
            })
            .expect("disable connector tool");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::ConnectorChanged { action, connector, .. }
                if action == "toolChanged"
                    && connector.tools.iter().any(|tool| tool.id == "create_issue" && !tool.enabled)
        )));

        controller
            .execute(FeatureCommand::SkillUpsert {
                request_id: "skill-create".into(),
                id: Some("skill-release-check".into()),
                name: "Release check".into(),
                description: "Verify a release candidate before publishing.".into(),
                use_when: "Use before a production release.".into(),
                instructions: "Run tests, inspect diffs, and summarize risks.".into(),
                owner_agent_id: Some("mahayana-assistant".into()),
            })
            .expect("create skill");
        drain(&controller);
        controller
            .execute(FeatureCommand::SkillPublish {
                request_id: "skill-publish".into(),
                id: "skill-release-check".into(),
                team_id: "team-mahayana".into(),
            })
            .expect("publish skill");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::SkillChanged { action, skill, .. }
                if action == "published"
                    && skill.publish_state == SkillPublishState::Published
                    && skill.team_id.as_deref() == Some("team-mahayana")
        )));

        controller
            .execute(FeatureCommand::BotSetHidden {
                request_id: "bot-hide".into(),
                id: "research-bot".into(),
                hidden: true,
            })
            .expect("hide bot");
        assert!(drain(&controller).into_iter().any(|event| matches!(
            event,
            HostEvent::BotChanged { bot, .. }
                if bot.id == "research-bot" && bot.hidden
        )));

        controller
            .execute(FeatureCommand::DraftResolve {
                request_id: "draft-send".into(),
                draft: MessageDraft::Email {
                    id: "draft-1".into(),
                    from: None,
                    to: vec!["person@example.com".into()],
                    cc: None,
                    subject: "Release ready".into(),
                    body: "The release candidate passed validation.".into(),
                    status: DraftSendState::Editable,
                    error: None,
                },
                action: DraftAction::Send,
            })
            .expect("send draft");
        let events = drain(&controller);
        assert!(events.iter().any(|event| matches!(
            event,
            HostEvent::DraftChanged {
                status: DraftSendState::Sending,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            HostEvent::DraftChanged {
                status: DraftSendState::Sent,
                ..
            }
        )));

        controller
            .execute(FeatureCommand::SecretProvide {
                request_id: "secret-provide".into(),
                secret_request_id: "deployment-token".into(),
                value: "super-secret-value".into(),
            })
            .expect("provide secret");
        let secret_events = drain(&controller);
        let serialized = serde_json::to_string(&secret_events).expect("serialize events");
        assert!(!serialized.contains("super-secret-value"));
        assert!(secret_events.into_iter().any(|event| matches!(
            event,
            HostEvent::SecretProvided { secret_request_id, .. }
                if secret_request_id == "deployment-token"
        )));

        controller
            .execute(FeatureCommand::AutomationUpsert {
                request_id: "event-routine-create".into(),
                id: Some("regression-triage".into()),
                name: "Regression triage".into(),
                prompt: "Inspect the regression and summarize impact.".into(),
                schedule: "event:sentry:issue.regressed".into(),
                trigger: Some(AutomationTrigger::Event {
                    source: ListenerPlatform::Sentry,
                    event: "issue.regressed".into(),
                    filter: Some("web".into()),
                }),
                enabled: true,
            })
            .expect("create event routine");
        drain(&controller);
        assert_eq!(
            controller
                .ingest_listener_event(EventCard {
                    source: ListenerPlatform::Sentry,
                    event: "issue.regressed".into(),
                    title: "Checkout regression".into(),
                    summary: "A production regression was detected.".into(),
                    url: Some("https://sentry.example.invalid/issues/42".into()),
                    actor: Some("sentry".into()),
                    fields: Some(vec![EventField {
                        label: "Project".into(),
                        value: "web".into(),
                    }]),
                    occurred_at_ms: Some(now_millis()),
                })
                .expect("ingest listener event"),
            1
        );
        let event_events = drain(&controller);
        assert!(event_events.iter().any(|event| matches!(
            event,
            HostEvent::TranscriptCard {
                card: TranscriptCard::Event { event },
                ..
            } if event.source == ListenerPlatform::Sentry
                && event.event == "issue.regressed"
                && event.title == "Checkout regression"
        )));
        assert!(event_events.iter().any(|event| matches!(
            event,
            HostEvent::AutomationChanged { action, automation, .. }
                if action == "triggered" && automation.id == "regression-triage"
        )));

        controller
            .execute(FeatureCommand::UpdateCheck {
                request_id: "update-check".into(),
            })
            .expect("check updates");
        let events = drain(&controller);
        assert!(events.iter().any(|event| matches!(
            event,
            HostEvent::UpdateChanged {
                state: UpdateState::Checking,
                ..
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            HostEvent::UpdateChanged {
                state: UpdateState::UpToDate { .. },
                ..
            }
        )));
    }

    #[test]
    fn automation_store_round_trips_atomically() {
        let root = std::env::temp_dir().join(format!(
            "fabushi-automation-store-{}-{}",
            std::process::id(),
            now_millis()
        ));
        let path = root.join("automations.json");
        let mut items = BTreeMap::new();
        items.insert(
            "weekly-review".into(),
            AutomationSummary {
                id: "weekly-review".into(),
                name: "每周复盘".into(),
                prompt: "整理本周工作。".into(),
                schedule: "@weekly".into(),
                trigger: Some(AutomationTrigger::Schedule {
                    schedule: "@weekly".into(),
                }),
                enabled: false,
                created_at_ms: 1,
                last_run_at_ms: None,
                next_run_at_ms: None,
            },
        );
        persist_automations(&path, &items).expect("persist automation store");
        let loaded = load_automations(&path);
        assert_eq!(loaded.get("weekly-review"), items.get("weekly-review"));
        std::fs::remove_dir_all(&root).expect("remove isolated automation store");
    }

    #[test]
    fn deterministic_rust_backend_executes_every_declared_feature_journey() {
        let controller = controller();
        assert_eq!(drain(&controller)[0].kind(), "host.ready");

        controller
            .execute(FeatureCommand::ChatSend {
                request_id: "chat-1".into(),
                text: "验证极速自动化测试".into(),
                agent_id: None,
                conversation_id: None,
                mode: AgentMode::Agent,
                mode_statement: None,
                model: None,
                attachments: Vec::new(),
            })
            .expect("chat");
        controller
            .execute(FeatureCommand::MarketplaceInstall {
                request_id: "install-1".into(),
                mini_app_id: "global-dharma".into(),
            })
            .expect("install");
        controller
            .execute(FeatureCommand::MiniAppOpen {
                request_id: "open-1".into(),
                mini_app_id: "global-dharma".into(),
            })
            .expect("open");
        controller
            .execute(FeatureCommand::CapabilityRequest {
                request_id: "capability-1".into(),
                mini_app_id: "global-dharma".into(),
                capability: "camera".into(),
                reason: "scan scripture".into(),
            })
            .expect("capability");
        let approval_id = drain(&controller)
            .into_iter()
            .find_map(|event| match event {
                HostEvent::ApprovalRequested { approval_id, .. } => Some(approval_id),
                _ => None,
            })
            .expect("approval id");
        controller
            .resolve_approval(ApprovalResolution {
                approval_id,
                decision: ApprovalDecision::AllowOnce,
            })
            .expect("resolve approval");
        let operation = controller
            .execute(FeatureCommand::RuntimeLongTask {
                request_id: "operation-1".into(),
                label: "index scriptures".into(),
            })
            .expect("long task")
            .operation_id
            .expect("operation id");
        controller.interrupt(&operation).expect("interrupt");
        controller
            .execute(FeatureCommand::SessionClear {
                request_id: "session-1".into(),
            })
            .expect("clear session");
        controller.close().expect("close");

        let kinds = drain(&controller)
            .into_iter()
            .map(|event| event.kind())
            .collect::<Vec<_>>();
        assert!(kinds.contains(&"approval.resolved"));
        assert!(kinds.contains(&"operation.started"));
        assert!(kinds.contains(&"operation.interrupted"));
        assert!(kinds.contains(&"session.cleared"));
        assert!(kinds.contains(&"host.closed"));
    }

    #[test]
    fn deterministic_oauth_journey_matches_the_cross_platform_ui_contract() {
        let controller = controller();
        let providers = controller.auth_providers().expect("OAuth providers");
        assert_eq!(providers.as_array().map(Vec::len), Some(4));
        assert_eq!(providers[0]["id"], "google");

        let attempt = controller
            .oauth_start("google".into())
            .expect("start OAuth");
        assert_eq!(attempt["provider"], "google");
        let completed = controller
            .oauth_poll(
                attempt["attemptId"]
                    .as_str()
                    .expect("attempt id")
                    .to_string(),
            )
            .expect("complete OAuth");
        assert_eq!(completed["status"], "completed");
        assert_eq!(completed["auth"]["loggedIn"], true);
        assert_eq!(controller.auth_status().unwrap()["loggedIn"], true);
    }

    #[cfg(not(feature = "production"))]
    #[test]
    fn production_mode_requires_the_explicit_runtime_feature() {
        let error = FeatureHostController::create(
            HostConfig {
                profile_id: "production".into(),
                mode: HostMode::Production,
            },
            SurfacePlatform::Tauri,
        )
        .err()
        .expect("production must not fall back to the test backend");
        assert!(matches!(error, FeatureHostError::ProductionUnavailable));
    }

    #[cfg(feature = "production")]
    #[test]
    fn production_uses_the_real_runtime_and_rust_owned_session_store() {
        let controller = FeatureHostController::create_with_host_config(
            HostConfig {
                profile_id: "production".into(),
                mode: HostMode::Production,
            },
            SurfacePlatform::Tauri,
            isolated_host_config("production"),
        )
        .expect("create feature Host");
        let ready = drain(&controller);
        assert_eq!(ready[0].kind(), "host.ready");
        assert!(
            controller
                .info()
                .runtime_version
                .starts_with("mahayana-abi-")
        );

        controller
            .execute(FeatureCommand::MarketplaceInstall {
                request_id: "install-1".into(),
                mini_app_id: "global-dharma".into(),
            })
            .expect("verify bundled production MiniApp");
        controller
            .execute(FeatureCommand::SessionClear {
                request_id: "session-1".into(),
            })
            .expect("clear isolated Rust session");

        let kinds = drain(&controller)
            .into_iter()
            .map(|event| event.kind())
            .collect::<Vec<_>>();
        assert!(kinds.contains(&"marketplace.installed"));
        assert!(kinds.contains(&"session.cleared"));
    }

    #[cfg(feature = "production")]
    #[test]
    fn draft_tool_arguments_match_live_gmail_and_slack_schemas() {
        let email = MessageDraft::Email {
            id: "email-1".into(),
            from: Some("sender@example.com".into()),
            to: vec!["one@example.com".into(), "two@example.com".into()],
            cc: Some(vec!["copy@example.com".into()]),
            subject: "Subject".into(),
            body: "Plain text body".into(),
            status: DraftSendState::Editable,
            error: None,
        };
        let gmail_schema = json!({
            "type": "object",
            "properties": {
                "to": {"type": "string"},
                "cc": {"type": "string"},
                "subject": {"type": "string"},
                "from_address": {"type": ["string", "null"]},
                "payload": {"type": "object"}
            }
        });
        let email_args = draft_tool_arguments(&email, Some(&gmail_schema)).expect("email args");
        assert_eq!(email_args["to"], "one@example.com, two@example.com");
        assert_eq!(email_args["cc"], "copy@example.com");
        assert_eq!(email_args["from_address"], "sender@example.com");
        assert_eq!(email_args["payload"]["mime_type"], "text/plain");
        assert_eq!(email_args["payload"]["body"]["content"], "Plain text body");

        let slack = MessageDraft::Slack {
            id: "slack-1".into(),
            workspace: None,
            target: "C012345".into(),
            thread: Some("1234.56".into()),
            body: "hello".into(),
            status: DraftSendState::Editable,
            error: None,
        };
        let slack_schema = json!({
            "type": "object",
            "properties": {
                "channel_id": {"type": "string"},
                "text": {"type": "string"},
                "thread_ts": {"type": "string"}
            }
        });
        let slack_args = draft_tool_arguments(&slack, Some(&slack_schema)).expect("Slack args");
        assert_eq!(slack_args["channel_id"], "C012345");
        assert_eq!(slack_args["text"], "hello");
        assert_eq!(slack_args["thread_ts"], "1234.56");
    }

    #[cfg(feature = "production")]
    #[test]
    fn production_runtime_events_preserve_streaming_and_terminal_states() {
        let controller = FeatureHostController::create_with_host_config(
            HostConfig {
                profile_id: "production-events".into(),
                mode: HostMode::Production,
            },
            SurfacePlatform::Tauri,
            isolated_host_config("production-events"),
        )
        .expect("create production event Host");
        let operation_id = OperationId("operation-1".into());
        let conversation_id = ConversationId(CODEX_ASSISTANT_CONVERSATION_ID.into());

        let delta = controller
            .translate_runtime_event(RuntimeEvent::MessageDelta {
                operation_id: operation_id.clone(),
                conversation_id: conversation_id.clone(),
                delta: "般若".into(),
            })
            .expect("translate delta")
            .expect("delta event");
        assert!(matches!(
            delta,
            HostEvent::ChatDelta {
                operation_id: ref current,
                ref delta,
                ..
            } if current == "operation-1" && delta == "般若"
        ));

        let completed = controller
            .translate_runtime_event(RuntimeEvent::MessageCompleted {
                operation_id: operation_id.clone(),
                message: mahayana_core::Message {
                    id: mahayana_core::MessageId("message-1".into()),
                    conversation_id,
                    role: RuntimeMessageRole::Assistant,
                    text: "般若波罗蜜多".into(),
                    created_at_ms: 1,
                    metadata: serde_json::json!({}),
                },
            })
            .expect("translate completed message")
            .expect("completed message event");
        assert!(matches!(
            completed,
            HostEvent::ChatMessage {
                operation_id: Some(ref current),
                role: MessageRole::Assistant,
                ref text,
                ..
            } if current == "operation-1" && text == "般若波罗蜜多"
        ));

        let terminal = controller
            .translate_runtime_event(RuntimeEvent::OperationCompleted {
                operation_id: operation_id.clone(),
            })
            .expect("translate completion")
            .expect("completion event");
        assert!(matches!(
            terminal,
            HostEvent::OperationCompleted {
                operation_id: ref current,
                ..
            } if current == "operation-1"
        ));

        let failed = controller
            .translate_runtime_event(RuntimeEvent::OperationFailed {
                operation_id,
                code: "provider_error".into(),
                message: "provider unavailable".into(),
            })
            .expect("translate failure")
            .expect("failure event");
        assert!(matches!(
            failed,
            HostEvent::OperationFailed {
                operation_id: ref current,
                ref code,
                ref message,
                ..
            } if current == "operation-1"
                && code == "provider_error"
                && message == "provider unavailable"
        ));
    }
}
