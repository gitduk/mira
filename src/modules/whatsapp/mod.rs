pub mod bridge;
pub mod module;
pub mod store;
pub mod types;

pub use module::WhatsAppModule;

use crate::module::ModuleStatus;
use crate::register_module;
use crate::registry::{ModuleInit, ModuleReady};

fn ready_whatsapp(init: &ModuleInit) -> ModuleReady {
    let bridge_dir = init.base_dir.join("bridge");
    if !bridge_dir.exists() {
        return ModuleReady {
            ready: false,
            reason: Some("bridge directory missing".into()),
        };
    }

    let store_dir = init.config.store_dir.join("whatsapp");
    match std::fs::read_dir(&store_dir) {
        Ok(mut entries) => {
            let has_files = entries.next().is_some();
            if has_files {
                ModuleReady {
                    ready: true,
                    reason: None,
                }
            } else {
                ModuleReady {
                    ready: false,
                    reason: Some("store empty; login required".into()),
                }
            }
        }
        Err(_) => ModuleReady {
            ready: false,
            reason: Some("store missing; login required".into()),
        },
    }
}

fn build_whatsapp(init: &mut ModuleInit) -> Box<dyn crate::comm::CommunicationModule> {
    let bridge_dir = init.base_dir.join("bridge");
    let store_dir = init.config.store_dir.clone();
    Box::new(WhatsAppModule::new(
        bridge_dir,
        store_dir,
        init.db.clone(),
        init.config.clone(),
    ))
}

register_module!(
    "whatsapp",
    "WhatsApp",
    ModuleStatus::Connecting,
    ready_whatsapp,
    build_whatsapp
);
