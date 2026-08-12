use crate::instance_setup;
use crate::instances::Instance;
use anyhow::{Result, bail};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallationStatus {
    pub instance_id: String,
    pub instance_name: String,
    pub phase: String,
    pub progress: f32,
    pub message: String,
}

#[derive(Clone, Default)]
pub struct InstallationManager {
    statuses: Arc<Mutex<HashMap<String, InstallationStatus>>>,
}

impl InstallationManager {
    pub fn snapshot(&self) -> Result<Vec<InstallationStatus>> {
        let statuses = self
            .statuses
            .lock()
            .map_err(|_| anyhow::anyhow!("Could not read instance installation status"))?;
        Ok(statuses.values().cloned().collect())
    }

    pub fn is_installing(&self, instance_id: &str) -> Result<bool> {
        let statuses = self
            .statuses
            .lock()
            .map_err(|_| anyhow::anyhow!("Could not read instance installation status"))?;
        Ok(statuses
            .get(instance_id)
            .is_some_and(|status| status.phase == "installing"))
    }

    pub fn remove(&self, instance_id: &str) {
        if let Ok(mut statuses) = self.statuses.lock() {
            statuses.remove(instance_id);
        }
    }

    pub fn start(&self, app: AppHandle, root: PathBuf, instance: Instance) -> Result<()> {
        let operation_instance = instance.clone();
        self.start_job(
            app,
            instance,
            format!("Preparing Minecraft {}...", operation_instance.version),
            move |progress| {
                instance_setup::prepare(&root, &operation_instance, progress.as_ref())?;
                Ok(())
            },
        )
    }

    pub fn start_job<F>(
        &self,
        app: AppHandle,
        instance: Instance,
        initial_message: String,
        operation: F,
    ) -> Result<()>
    where
        F: FnOnce(Arc<dyn Fn(f32, String) + Send + Sync>) -> Result<()> + Send + 'static,
    {
        let initial = InstallationStatus {
            instance_id: instance.id.clone(),
            instance_name: instance.name.clone(),
            phase: "installing".to_owned(),
            progress: 0.0,
            message: initial_message,
        };
        {
            let mut statuses = self
                .statuses
                .lock()
                .map_err(|_| anyhow::anyhow!("Could not update instance installation status"))?;
            if statuses
                .get(&instance.id)
                .is_some_and(|status| status.phase == "installing")
            {
                bail!("This instance is already being installed");
            }
            statuses.insert(instance.id.clone(), initial.clone());
        }
        let _ = app.emit("instance-installation-progress", &initial);

        let manager = self.clone();
        tauri::async_runtime::spawn_blocking(move || {
            let progress_manager = manager.clone();
            let progress_app = app.clone();
            let progress_instance = instance.clone();
            let progress: Arc<dyn Fn(f32, String) + Send + Sync> =
                Arc::new(move |value: f32, message: String| {
                    let status = InstallationStatus {
                        instance_id: progress_instance.id.clone(),
                        instance_name: progress_instance.name.clone(),
                        phase: "installing".to_owned(),
                        progress: value.clamp(0.0, 1.0),
                        message,
                    };
                    progress_manager.set_and_emit(&progress_app, status);
                });

            match operation(progress) {
                Ok(_) => {
                    let completed = InstallationStatus {
                        instance_id: instance.id.clone(),
                        instance_name: instance.name.clone(),
                        phase: "completed".to_owned(),
                        progress: 1.0,
                        message: format!("{} is ready to play.", instance.name),
                    };
                    manager.set_and_emit(&app, completed);
                    manager.remove(&instance.id);
                }
                Err(error) => {
                    manager.set_and_emit(
                        &app,
                        InstallationStatus {
                            instance_id: instance.id.clone(),
                            instance_name: instance.name.clone(),
                            phase: "failed".to_owned(),
                            progress: 0.0,
                            message: format!("Installation failed: {error:#}"),
                        },
                    );
                }
            }
        });
        Ok(())
    }

    fn set_and_emit(&self, app: &AppHandle, status: InstallationStatus) {
        if let Ok(mut statuses) = self.statuses.lock() {
            statuses.insert(status.instance_id.clone(), status.clone());
        }
        let _ = app.emit("instance-installation-progress", status);
    }
}
