use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::comm::CommunicationModule;
use crate::config::Config;
use crate::dashboard::DashboardCommand;
use crate::db::Database;
use crate::module::ModuleStatus;

pub struct ModuleInit {
    pub config: Arc<Config>,
    pub db: Arc<Database>,
    pub dash_tx: mpsc::Sender<DashboardCommand>,
    pub base_dir: PathBuf,
}

pub struct ModuleRegistration {
    pub id: &'static str,
    pub display_name: &'static str,
    pub initial_status: ModuleStatus,
    pub ready: fn(&ModuleInit) -> ModuleReady,
    pub build: fn(&mut ModuleInit) -> Box<dyn CommunicationModule>,
}

pub struct ModuleReady {
    pub ready: bool,
    pub reason: Option<String>,
}

inventory::collect!(ModuleRegistration);

#[macro_export]
macro_rules! register_module {
    ($id:expr, $display_name:expr, $status:expr, $ready:expr, $build:expr) => {
        inventory::submit! {
            $crate::registry::ModuleRegistration {
                id: $id,
                display_name: $display_name,
                initial_status: $status,
                ready: $ready,
                build: $build,
            }
        }
    };
}
