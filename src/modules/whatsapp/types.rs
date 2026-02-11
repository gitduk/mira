use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisteredGroup {
    pub jid: String,
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
