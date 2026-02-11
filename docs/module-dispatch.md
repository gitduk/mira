Module Dispatch Template

Purpose
This is a minimal template for module-side scheduling. Mira will call `drain_work_items()` when the module is scheduled. The module is responsible for batching, ordering, and deciding what work to emit.

Steps
1. Buffer incoming messages internally.
2. In `drain_work_items()`, drain the buffer into one or more `ModuleWorkItem`.
3. Return an empty list if there is no work.

Example
```rust
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::comm::CommunicationModule;
use crate::dispatch::ModuleWorkItem;
use crate::prompt::xml_escape;
use crate::comm::ChannelAddr;

struct MyModule {
    pending: Arc<Mutex<VecDeque<MyMessage>>>,
}

#[async_trait::async_trait]
impl CommunicationModule for MyModule {
    async fn drain_work_items(&self) -> crate::error::Result<Vec<ModuleWorkItem>> {
        let mut pending = self.pending.lock().await;
        if pending.is_empty() {
            return Ok(Vec::new());
        }

        let mut prompt = String::from("<messages>\\n");
        while let Some(msg) = pending.pop_front() {
            prompt.push_str(&format!(
                "<message sender=\\"{}\\" channel=\\"{}\\" time=\\"{}\\">{}</message>\\n",
                xml_escape(&msg.sender_name),
                xml_escape(&msg.channel_id),
                msg.timestamp,
                xml_escape(&msg.content),
            ));
        }
        prompt.push_str("</messages>");

        Ok(vec![ModuleWorkItem {
            prompt,
            group_folder: "my-module".into(),
            addr: ChannelAddr::new("my-module", "default"),
            is_main: false,
            is_scheduled_task: false,
            workspace_dir: std::path::PathBuf::from("groups/my-module"),
            global_claude_md: None,
        }])
    }
}
```
