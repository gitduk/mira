use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredGroup {
    pub name: String,
    pub folder: String,
    pub trigger: String,
    pub added_at: String,
    #[serde(default = "default_requires_trigger")]
    pub requires_trigger: Option<bool>,
}

fn default_requires_trigger() -> Option<bool> {
    None
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewMessage {
    pub id: String,
    pub chat_jid: String,
    pub sender: String,
    pub sender_name: String,
    pub content: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    pub group_folder: String,
    pub chat_jid: String,
    pub prompt: String,
    pub schedule_type: ScheduleType,
    pub schedule_value: String,
    pub context_mode: ContextMode,
    pub next_run: Option<String>,
    pub last_run: Option<String>,
    pub last_result: Option<String>,
    pub status: TaskStatus,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleType {
    Cron,
    Interval,
    Once,
}

impl ScheduleType {
    pub fn as_str(&self) -> &str {
        match self {
            ScheduleType::Cron => "cron",
            ScheduleType::Interval => "interval",
            ScheduleType::Once => "once",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "cron" => Some(ScheduleType::Cron),
            "interval" => Some(ScheduleType::Interval),
            "once" => Some(ScheduleType::Once),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ContextMode {
    Group,
    Isolated,
}

impl ContextMode {
    pub fn as_str(&self) -> &str {
        match self {
            ContextMode::Group => "group",
            ContextMode::Isolated => "isolated",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "group" => ContextMode::Group,
            _ => ContextMode::Isolated,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Active,
    Paused,
    Completed,
}

impl TaskStatus {
    pub fn as_str(&self) -> &str {
        match self {
            TaskStatus::Active => "active",
            TaskStatus::Paused => "paused",
            TaskStatus::Completed => "completed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "active" => TaskStatus::Active,
            "paused" => TaskStatus::Paused,
            "completed" => TaskStatus::Completed,
            _ => TaskStatus::Active,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskRunLog {
    pub task_id: String,
    pub run_at: String,
    pub duration_ms: i64,
    pub status: RunStatus,
    pub result: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Success,
    Error,
}

impl RunStatus {
    pub fn as_str(&self) -> &str {
        match self {
            RunStatus::Success => "success",
            RunStatus::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatInfo {
    pub jid: String,
    pub name: String,
    pub last_message_time: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AvailableGroup {
    pub jid: String,
    pub name: String,
    pub last_activity: String,
    pub is_registered: bool,
}

/// IPC commands sent from agent tools to main loop via mpsc channel
#[derive(Debug)]
pub enum IpcCommand {
    SendMessage {
        chat_jid: String,
        text: String,
    },
    ScheduleTask {
        prompt: String,
        schedule_type: String,
        schedule_value: String,
        context_mode: String,
        target_jid: String,
        source_group: String,
    },
    PauseTask {
        task_id: String,
        source_group: String,
    },
    ResumeTask {
        task_id: String,
        source_group: String,
    },
    CancelTask {
        task_id: String,
        source_group: String,
    },
    RegisterGroup {
        jid: String,
        name: String,
        folder: String,
        trigger: String,
    },
    RefreshGroups,
}
