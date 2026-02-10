#[derive(Debug, Clone, PartialEq)]
pub enum ModuleStatus {
    Idle,
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
}

impl ModuleStatus {
    pub fn display_symbol(&self) -> &'static str {
        match self {
            ModuleStatus::Idle => "○",
            ModuleStatus::Connecting => "◌",
            ModuleStatus::Connected => "●",
            ModuleStatus::Reconnecting => "⟳",
            ModuleStatus::Disconnected => "●",
        }
    }

    pub fn display_text(&self) -> &'static str {
        match self {
            ModuleStatus::Idle => "Idle",
            ModuleStatus::Connecting => "Connecting",
            ModuleStatus::Connected => "Connected",
            ModuleStatus::Reconnecting => "Reconnecting",
            ModuleStatus::Disconnected => "Disconnected",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Module {
    pub id: String,
    pub display_name: String,
    pub mounted: bool,
    pub status: ModuleStatus,
}

impl Module {
    pub fn new(id: &str, display_name: &str) -> Self {
        Module {
            id: id.to_string(),
            display_name: display_name.to_string(),
            mounted: false,
            status: ModuleStatus::Idle,
        }
    }
}
