//! Product-owned orchestration primitives for Mahayana.
//!
//! These types intentionally do not expose Codex or Grok Build protocol types.
//! They provide the durable behaviors Mahayana needs from prompt queues,
//! memory, workflow DAGs, hooks, and subagent scheduling.

use mahayana_kernel::CapabilitySet;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

fn generated_id(prefix: &str) -> String {
    format!("{prefix}:{}", Uuid::new_v4())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptPriority {
    Background,
    Normal,
    UserBlocking,
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptState {
    Pending,
    Running,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptEntry {
    pub id: String,
    pub text: String,
    pub priority: PromptPriority,
    pub state: PromptState,
    pub created_at_ms: i64,
    pub dedupe_key: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PromptQueue {
    entries: VecDeque<PromptEntry>,
}

impl PromptQueue {
    pub fn enqueue(
        &mut self,
        text: impl Into<String>,
        priority: PromptPriority,
        dedupe_key: Option<String>,
        metadata: Value,
    ) -> Result<String, OrchestratorError> {
        let text = text.into();
        if text.trim().is_empty() {
            return Err(OrchestratorError::InvalidInput(
                "prompt text must not be empty".into(),
            ));
        }

        if let Some(key) = dedupe_key.as_deref()
            && let Some(existing) = self.entries.iter_mut().find(|entry| {
                entry.state == PromptState::Pending && entry.dedupe_key.as_deref() == Some(key)
            })
        {
            existing.text = text;
            existing.priority = existing.priority.max(priority);
            existing.metadata = metadata;
            return Ok(existing.id.clone());
        }

        let id = generated_id("prompt");
        self.entries.push_back(PromptEntry {
            id: id.clone(),
            text,
            priority,
            state: PromptState::Pending,
            created_at_ms: now_ms(),
            dedupe_key,
            metadata,
        });
        Ok(id)
    }

    pub fn take_next(&mut self) -> Option<PromptEntry> {
        let index = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.state == PromptState::Pending)
            .max_by(|(_, left), (_, right)| {
                left.priority
                    .cmp(&right.priority)
                    .then_with(|| right.created_at_ms.cmp(&left.created_at_ms))
            })
            .map(|(index, _)| index)?;
        let entry = self.entries.get_mut(index)?;
        entry.state = PromptState::Running;
        Some(entry.clone())
    }

    pub fn complete(&mut self, id: &str) -> Result<(), OrchestratorError> {
        self.transition_prompt(id, PromptState::Completed)
    }

    pub fn cancel(&mut self, id: &str) -> Result<(), OrchestratorError> {
        self.transition_prompt(id, PromptState::Cancelled)
    }

    fn transition_prompt(&mut self, id: &str, state: PromptState) -> Result<(), OrchestratorError> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| entry.id == id)
            .ok_or_else(|| OrchestratorError::PromptNotFound(id.to_string()))?;
        entry.state = state;
        Ok(())
    }

    pub fn pending(&self) -> Vec<PromptEntry> {
        self.entries
            .iter()
            .filter(|entry| entry.state == PromptState::Pending)
            .cloned()
            .collect()
    }

    pub fn combine_pending(
        &mut self,
        separator: &str,
        priority: PromptPriority,
    ) -> Result<Option<String>, OrchestratorError> {
        let pending = self
            .entries
            .iter()
            .filter(|entry| entry.state == PromptState::Pending)
            .cloned()
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return Ok(None);
        }
        for entry in &mut self.entries {
            if entry.state == PromptState::Pending {
                entry.state = PromptState::Cancelled;
            }
        }
        let text = pending
            .iter()
            .map(|entry| entry.text.as_str())
            .collect::<Vec<_>>()
            .join(separator);
        let source_ids = pending
            .iter()
            .map(|entry| Value::String(entry.id.clone()))
            .collect::<Vec<_>>();
        let id = self.enqueue(
            text,
            priority,
            None,
            serde_json::json!({"combinedPromptIds": source_ids}),
        )?;
        Ok(Some(id))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub namespace: String,
    pub key: String,
    pub value: Value,
    pub tags: BTreeSet<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub expires_at_ms: Option<i64>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct MemoryStore {
    records: HashMap<(String, String), MemoryRecord>,
}

impl MemoryStore {
    pub fn upsert(
        &mut self,
        namespace: impl Into<String>,
        key: impl Into<String>,
        value: Value,
        tags: impl IntoIterator<Item = String>,
        expires_at_ms: Option<i64>,
    ) -> Result<MemoryRecord, OrchestratorError> {
        let namespace = namespace.into();
        let key = key.into();
        if namespace.trim().is_empty() || key.trim().is_empty() {
            return Err(OrchestratorError::InvalidInput(
                "memory namespace and key must not be empty".into(),
            ));
        }
        let timestamp = now_ms();
        let map_key = (namespace.clone(), key.clone());
        let created_at_ms = self
            .records
            .get(&map_key)
            .map(|record| record.created_at_ms)
            .unwrap_or(timestamp);
        let record = MemoryRecord {
            id: self
                .records
                .get(&map_key)
                .map(|record| record.id.clone())
                .unwrap_or_else(|| generated_id("memory")),
            namespace,
            key,
            value,
            tags: tags.into_iter().collect(),
            created_at_ms,
            updated_at_ms: timestamp,
            expires_at_ms,
        };
        self.records.insert(map_key, record.clone());
        Ok(record)
    }

    pub fn get(&mut self, namespace: &str, key: &str) -> Option<MemoryRecord> {
        self.prune_expired(now_ms());
        self.records
            .get(&(namespace.to_string(), key.to_string()))
            .cloned()
    }

    pub fn remove(&mut self, namespace: &str, key: &str) -> Option<MemoryRecord> {
        self.records
            .remove(&(namespace.to_string(), key.to_string()))
    }

    pub fn search(
        &mut self,
        namespace: Option<&str>,
        query: Option<&str>,
        required_tags: &[String],
        limit: usize,
    ) -> Vec<MemoryRecord> {
        self.prune_expired(now_ms());
        let query = query.map(str::to_lowercase);
        let mut records = self
            .records
            .values()
            .filter(|record| namespace.is_none_or(|namespace| record.namespace == namespace))
            .filter(|record| {
                required_tags
                    .iter()
                    .all(|tag| record.tags.contains(tag.as_str()))
            })
            .filter(|record| {
                query.as_ref().is_none_or(|query| {
                    record.key.to_lowercase().contains(query)
                        || record.value.to_string().to_lowercase().contains(query)
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        records.sort_by_key(|record| std::cmp::Reverse(record.updated_at_ms));
        records.truncate(limit.max(1));
        records
    }

    pub fn prune_expired(&mut self, timestamp_ms: i64) -> usize {
        let before = self.records.len();
        self.records.retain(|_, record| {
            record
                .expires_at_ms
                .is_none_or(|expires_at_ms| expires_at_ms > timestamp_ms)
        });
        before.saturating_sub(self.records.len())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowTaskState {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkflowTask {
    pub id: String,
    pub title: String,
    pub depends_on: BTreeSet<String>,
    pub state: WorkflowTaskState,
    pub result: Option<Value>,
    pub error: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub title: String,
    tasks: BTreeMap<String, WorkflowTask>,
}

impl Workflow {
    pub fn new(title: impl Into<String>) -> Result<Self, OrchestratorError> {
        let title = title.into();
        if title.trim().is_empty() {
            return Err(OrchestratorError::InvalidInput(
                "workflow title must not be empty".into(),
            ));
        }
        Ok(Self {
            id: generated_id("workflow"),
            title,
            tasks: BTreeMap::new(),
        })
    }

    pub fn add_task(
        &mut self,
        id: impl Into<String>,
        title: impl Into<String>,
        depends_on: impl IntoIterator<Item = String>,
        metadata: Value,
    ) -> Result<(), OrchestratorError> {
        let id = id.into();
        let title = title.into();
        if id.trim().is_empty() || title.trim().is_empty() {
            return Err(OrchestratorError::InvalidInput(
                "workflow task id and title must not be empty".into(),
            ));
        }
        if self.tasks.contains_key(&id) {
            return Err(OrchestratorError::DuplicateWorkflowTask(id));
        }
        let depends_on = depends_on.into_iter().collect::<BTreeSet<_>>();
        if depends_on.contains(&id) {
            return Err(OrchestratorError::WorkflowCycle(id));
        }
        self.tasks.insert(
            id.clone(),
            WorkflowTask {
                id: id.clone(),
                title,
                depends_on,
                state: WorkflowTaskState::Pending,
                result: None,
                error: None,
                metadata,
            },
        );
        if let Err(error) = self.validate() {
            self.tasks.remove(&id);
            return Err(error);
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), OrchestratorError> {
        for task in self.tasks.values() {
            for dependency in &task.depends_on {
                if !self.tasks.contains_key(dependency) {
                    return Err(OrchestratorError::MissingWorkflowDependency {
                        task_id: task.id.clone(),
                        dependency: dependency.clone(),
                    });
                }
            }
        }
        let mut visiting = BTreeSet::new();
        let mut visited = BTreeSet::new();
        for task_id in self.tasks.keys() {
            self.visit(task_id, &mut visiting, &mut visited)?;
        }
        Ok(())
    }

    fn visit(
        &self,
        task_id: &str,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeSet<String>,
    ) -> Result<(), OrchestratorError> {
        if visited.contains(task_id) {
            return Ok(());
        }
        if !visiting.insert(task_id.to_string()) {
            return Err(OrchestratorError::WorkflowCycle(task_id.to_string()));
        }
        let task = self
            .tasks
            .get(task_id)
            .ok_or_else(|| OrchestratorError::WorkflowTaskNotFound(task_id.to_string()))?;
        for dependency in &task.depends_on {
            self.visit(dependency, visiting, visited)?;
        }
        visiting.remove(task_id);
        visited.insert(task_id.to_string());
        Ok(())
    }

    pub fn runnable_tasks(&self) -> Vec<WorkflowTask> {
        self.tasks
            .values()
            .filter(|task| task.state == WorkflowTaskState::Pending)
            .filter(|task| {
                task.depends_on.iter().all(|dependency| {
                    self.tasks
                        .get(dependency)
                        .is_some_and(|dependency| dependency.state == WorkflowTaskState::Completed)
                })
            })
            .cloned()
            .collect()
    }

    pub fn start(&mut self, task_id: &str) -> Result<(), OrchestratorError> {
        let runnable = self.runnable_tasks().iter().any(|task| task.id == task_id);
        if !runnable {
            return Err(OrchestratorError::WorkflowTaskNotRunnable(
                task_id.to_string(),
            ));
        }
        self.task_mut(task_id)?.state = WorkflowTaskState::Running;
        Ok(())
    }

    pub fn complete(&mut self, task_id: &str, result: Value) -> Result<(), OrchestratorError> {
        let task = self.task_mut(task_id)?;
        if task.state != WorkflowTaskState::Running {
            return Err(OrchestratorError::WorkflowTaskNotRunning(
                task_id.to_string(),
            ));
        }
        task.state = WorkflowTaskState::Completed;
        task.result = Some(result);
        task.error = None;
        Ok(())
    }

    pub fn fail(
        &mut self,
        task_id: &str,
        error: impl Into<String>,
    ) -> Result<(), OrchestratorError> {
        let task = self.task_mut(task_id)?;
        if task.state != WorkflowTaskState::Running {
            return Err(OrchestratorError::WorkflowTaskNotRunning(
                task_id.to_string(),
            ));
        }
        task.state = WorkflowTaskState::Failed;
        task.error = Some(error.into());
        Ok(())
    }

    pub fn cancel(&mut self, task_id: &str) -> Result<(), OrchestratorError> {
        let task = self.task_mut(task_id)?;
        task.state = WorkflowTaskState::Cancelled;
        Ok(())
    }

    pub fn progress(&self) -> (usize, usize) {
        let completed = self
            .tasks
            .values()
            .filter(|task| task.state == WorkflowTaskState::Completed)
            .count();
        (completed, self.tasks.len())
    }

    pub fn task(&self, task_id: &str) -> Option<&WorkflowTask> {
        self.tasks.get(task_id)
    }

    fn task_mut(&mut self, task_id: &str) -> Result<&mut WorkflowTask, OrchestratorError> {
        self.tasks
            .get_mut(task_id)
            .ok_or_else(|| OrchestratorError::WorkflowTaskNotFound(task_id.to_string()))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPoint {
    BeforeModel,
    AfterModel,
    BeforeTool,
    AfterTool,
    BeforeCheckpoint,
    AfterCheckpoint,
    BeforeWorkflowTask,
    AfterWorkflowTask,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookEffect {
    pub inject_context: Option<String>,
    pub require_approval: bool,
    pub block_reason: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookDefinition {
    pub id: String,
    pub point: HookPoint,
    pub priority: i32,
    pub enabled: bool,
    pub matcher: Option<String>,
    pub effect: HookEffect,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct HookRegistry {
    hooks: BTreeMap<String, HookDefinition>,
}

impl HookRegistry {
    pub fn register(
        &mut self,
        point: HookPoint,
        priority: i32,
        matcher: Option<String>,
        effect: HookEffect,
    ) -> String {
        let id = generated_id("hook");
        self.hooks.insert(
            id.clone(),
            HookDefinition {
                id: id.clone(),
                point,
                priority,
                enabled: true,
                matcher,
                effect,
            },
        );
        id
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), OrchestratorError> {
        let hook = self
            .hooks
            .get_mut(id)
            .ok_or_else(|| OrchestratorError::HookNotFound(id.to_string()))?;
        hook.enabled = enabled;
        Ok(())
    }

    pub fn dispatch(&self, point: HookPoint, subject: &str) -> Vec<HookDefinition> {
        let subject = subject.to_lowercase();
        let mut hooks = self
            .hooks
            .values()
            .filter(|hook| hook.enabled && hook.point == point)
            .filter(|hook| {
                hook.matcher
                    .as_deref()
                    .is_none_or(|matcher| subject.contains(&matcher.to_lowercase()))
            })
            .cloned()
            .collect::<Vec<_>>();
        hooks.sort_by(|left, right| {
            right
                .priority
                .cmp(&left.priority)
                .then_with(|| left.id.cmp(&right.id))
        });
        hooks
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubagentState {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubagentTask {
    pub id: String,
    pub parent_id: Option<String>,
    pub name: String,
    pub goal: String,
    pub required_capabilities: CapabilitySet,
    pub state: SubagentState,
    pub created_at_ms: i64,
    pub result: Option<Value>,
    pub error: Option<String>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubagentScheduler {
    max_concurrency: usize,
    tasks: BTreeMap<String, SubagentTask>,
}

impl SubagentScheduler {
    pub fn new(max_concurrency: usize) -> Result<Self, OrchestratorError> {
        if max_concurrency == 0 {
            return Err(OrchestratorError::InvalidInput(
                "subagent concurrency must be at least one".into(),
            ));
        }
        Ok(Self {
            max_concurrency,
            tasks: BTreeMap::new(),
        })
    }

    pub fn spawn(
        &mut self,
        parent_id: Option<String>,
        name: impl Into<String>,
        goal: impl Into<String>,
        required_capabilities: CapabilitySet,
        metadata: Value,
    ) -> Result<String, OrchestratorError> {
        if let Some(parent_id) = parent_id.as_deref()
            && !self.tasks.contains_key(parent_id)
        {
            return Err(OrchestratorError::SubagentNotFound(parent_id.to_string()));
        }
        let name = name.into();
        let goal = goal.into();
        if name.trim().is_empty() || goal.trim().is_empty() {
            return Err(OrchestratorError::InvalidInput(
                "subagent name and goal must not be empty".into(),
            ));
        }
        let id = generated_id("subagent");
        self.tasks.insert(
            id.clone(),
            SubagentTask {
                id: id.clone(),
                parent_id,
                name,
                goal,
                required_capabilities,
                state: SubagentState::Pending,
                created_at_ms: now_ms(),
                result: None,
                error: None,
                metadata,
            },
        );
        Ok(id)
    }

    pub fn start(&mut self, id: &str) -> Result<SubagentTask, OrchestratorError> {
        if self.running_count() >= self.max_concurrency {
            return Err(OrchestratorError::SubagentConcurrencyExhausted);
        }
        let task = self.task_mut(id)?;
        if task.state != SubagentState::Pending {
            return Err(OrchestratorError::SubagentNotPending(id.to_string()));
        }
        task.state = SubagentState::Running;
        Ok(task.clone())
    }

    pub fn start_next(&mut self) -> Option<SubagentTask> {
        if self.running_count() >= self.max_concurrency {
            return None;
        }
        let id = self
            .tasks
            .values()
            .filter(|task| task.state == SubagentState::Pending)
            .min_by_key(|task| task.created_at_ms)
            .map(|task| task.id.clone())?;
        let task = self.tasks.get_mut(&id)?;
        task.state = SubagentState::Running;
        Some(task.clone())
    }

    pub fn complete(&mut self, id: &str, result: Value) -> Result<(), OrchestratorError> {
        let task = self.task_mut(id)?;
        if task.state != SubagentState::Running {
            return Err(OrchestratorError::SubagentNotRunning(id.to_string()));
        }
        task.state = SubagentState::Completed;
        task.result = Some(result);
        task.error = None;
        Ok(())
    }

    pub fn fail(&mut self, id: &str, error: impl Into<String>) -> Result<(), OrchestratorError> {
        let task = self.task_mut(id)?;
        if task.state != SubagentState::Running {
            return Err(OrchestratorError::SubagentNotRunning(id.to_string()));
        }
        task.state = SubagentState::Failed;
        task.error = Some(error.into());
        Ok(())
    }

    pub fn cancel(&mut self, id: &str) -> Result<(), OrchestratorError> {
        let task = self.task_mut(id)?;
        task.state = SubagentState::Cancelled;
        Ok(())
    }

    pub fn running_count(&self) -> usize {
        self.tasks
            .values()
            .filter(|task| task.state == SubagentState::Running)
            .count()
    }

    pub fn children(&self, parent_id: &str) -> Vec<SubagentTask> {
        self.tasks
            .values()
            .filter(|task| task.parent_id.as_deref() == Some(parent_id))
            .cloned()
            .collect()
    }

    pub fn task(&self, id: &str) -> Option<&SubagentTask> {
        self.tasks.get(id)
    }

    fn task_mut(&mut self, id: &str) -> Result<&mut SubagentTask, OrchestratorError> {
        self.tasks
            .get_mut(id)
            .ok_or_else(|| OrchestratorError::SubagentNotFound(id.to_string()))
    }
}

impl Default for SubagentScheduler {
    fn default() -> Self {
        Self {
            max_concurrency: 4,
            tasks: BTreeMap::new(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum OrchestratorError {
    #[error("invalid orchestration input: {0}")]
    InvalidInput(String),
    #[error("prompt not found: {0}")]
    PromptNotFound(String),
    #[error("workflow task already exists: {0}")]
    DuplicateWorkflowTask(String),
    #[error("workflow task not found: {0}")]
    WorkflowTaskNotFound(String),
    #[error("workflow dependency is missing for {task_id}: {dependency}")]
    MissingWorkflowDependency { task_id: String, dependency: String },
    #[error("workflow contains a dependency cycle at: {0}")]
    WorkflowCycle(String),
    #[error("workflow task is not runnable: {0}")]
    WorkflowTaskNotRunnable(String),
    #[error("workflow task is not running: {0}")]
    WorkflowTaskNotRunning(String),
    #[error("hook not found: {0}")]
    HookNotFound(String),
    #[error("subagent not found: {0}")]
    SubagentNotFound(String),
    #[error("subagent is not running: {0}")]
    SubagentNotRunning(String),
    #[error("subagent is not pending: {0}")]
    SubagentNotPending(String),
    #[error("subagent concurrency is exhausted")]
    SubagentConcurrencyExhausted,
}

#[cfg(test)]
mod tests {
    use super::*;
    use mahayana_kernel::Capability;

    #[test]
    fn prompt_queue_prioritizes_user_blocking_work() {
        let mut queue = PromptQueue::default();
        queue
            .enqueue("background", PromptPriority::Background, None, Value::Null)
            .expect("enqueue background");
        queue
            .enqueue("urgent", PromptPriority::UserBlocking, None, Value::Null)
            .expect("enqueue urgent");
        assert_eq!(queue.take_next().expect("next prompt").text, "urgent");
    }

    #[test]
    fn prompt_queue_dedupes_pending_updates() {
        let mut queue = PromptQueue::default();
        let first = queue
            .enqueue(
                "first",
                PromptPriority::Normal,
                Some("same".into()),
                Value::Null,
            )
            .expect("enqueue first");
        let second = queue
            .enqueue(
                "second",
                PromptPriority::Critical,
                Some("same".into()),
                Value::Null,
            )
            .expect("enqueue second");
        assert_eq!(first, second);
        let next = queue.take_next().expect("next prompt");
        assert_eq!(next.text, "second");
        assert_eq!(next.priority, PromptPriority::Critical);
    }

    #[test]
    fn memory_prunes_expired_records() {
        let mut memory = MemoryStore::default();
        memory
            .upsert(
                "session",
                "expired",
                serde_json::json!({"value": true}),
                Vec::<String>::new(),
                Some(0),
            )
            .expect("write memory");
        assert!(memory.get("session", "expired").is_none());
    }

    #[test]
    fn workflow_runs_only_after_dependencies_complete() {
        let mut workflow = Workflow::new("release").expect("workflow");
        workflow
            .add_task("build", "Build", Vec::<String>::new(), Value::Null)
            .expect("add build");
        workflow
            .add_task("test", "Test", ["build".to_string()], Value::Null)
            .expect("add test");
        assert_eq!(workflow.runnable_tasks()[0].id, "build");
        workflow.start("build").expect("start build");
        workflow
            .complete("build", serde_json::json!({"ok": true}))
            .expect("complete build");
        assert_eq!(workflow.runnable_tasks()[0].id, "test");
    }

    #[test]
    fn hook_dispatch_is_priority_ordered_and_filtered() {
        let mut hooks = HookRegistry::default();
        hooks.register(
            HookPoint::BeforeTool,
            1,
            None,
            HookEffect {
                inject_context: None,
                require_approval: false,
                block_reason: None,
                metadata: Value::Null,
            },
        );
        let high = hooks.register(
            HookPoint::BeforeTool,
            10,
            Some("write".into()),
            HookEffect {
                inject_context: None,
                require_approval: true,
                block_reason: None,
                metadata: Value::Null,
            },
        );
        let matched = hooks.dispatch(HookPoint::BeforeTool, "filesystem_write");
        assert_eq!(matched[0].id, high);
    }

    #[test]
    fn subagent_scheduler_enforces_concurrency() {
        let mut scheduler = SubagentScheduler::new(1).expect("scheduler");
        let caps = CapabilitySet::new([Capability::Model]);
        scheduler
            .spawn(None, "one", "first goal", caps.clone(), Value::Null)
            .expect("spawn one");
        scheduler
            .spawn(None, "two", "second goal", caps, Value::Null)
            .expect("spawn two");
        assert!(scheduler.start_next().is_some());
        assert!(scheduler.start_next().is_none());
    }
}
