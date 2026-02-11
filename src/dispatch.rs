use std::path::PathBuf;

use crate::comm::ChannelAddr;

pub struct ModuleWorkItem {
    pub prompt: String,
    pub workspace: String,
    pub addr: ChannelAddr,
    pub is_main: bool,
    pub is_scheduled_task: bool,
    pub workspace_dir: PathBuf,
    pub global_claude_md: Option<String>,
}
