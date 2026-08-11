use std::collections::BTreeMap;

use crate::capture::claude::detection;
use crate::models::DetectedProcess;
use sysinfo::System;

pub fn find_claude_processes() -> Vec<DetectedProcess> {
    let mut system = System::new_all();
    system.refresh_all();

    let mut processes = BTreeMap::new();
    for (pid, process) in system.processes() {
        let name = process.name().to_string_lossy().to_string();
        let path = process.exe().map(|path| path.display().to_string());
        if detection::is_likely_claude_desktop_process(&name, path.as_deref()) {
            processes.insert(
                pid.as_u32(),
                DetectedProcess {
                    pid: pid.as_u32(),
                    name,
                    path,
                },
            );
        }
    }

    processes.into_values().collect()
}
