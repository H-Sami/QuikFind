use crate::error::{QuikFindError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{error, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    pub author: Option<String>,
    pub description: Option<String>,
    pub entry: String,
    pub permissions: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginResponse {
    pub success: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

/// Trait that all plugins must implement
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;

    /// Initializes the plugin.
    ///
    /// # Errors
    /// Returns an error if initialization fails.
    fn initialize(&mut self) -> Result<()>;

    /// Handles a query.
    ///
    /// # Errors
    /// Returns an error if query handling fails.
    fn handle_query(&self, query: &str) -> Result<PluginResponse>;

    /// Shuts down the plugin.
    ///
    /// # Errors
    /// Returns an error if shutdown fails.
    fn shutdown(&mut self) -> Result<()>;
}

/// Registry of all loaded plugins
pub struct PluginRegistry {
    plugins: HashMap<String, Box<dyn Plugin>>,
    plugin_dir: PathBuf,
}

impl PluginRegistry {
    #[must_use]
    pub fn new(config_dir: &Path) -> Self {
        let plugin_dir = config_dir.join("plugins");
        Self {
            plugins: HashMap::new(),
            plugin_dir,
        }
    }

    /// Initializes the plugin directory.
    ///
    /// # Errors
    /// Returns an error if the directory cannot be created.
    pub fn init(&mut self) -> Result<()> {
        std::fs::create_dir_all(&self.plugin_dir).map_err(QuikFindError::Io)?;
        info!("Plugin directory: {:?}", self.plugin_dir);
        Ok(())
    }

    pub fn register(&mut self, plugin: Box<dyn Plugin>) {
        let name = plugin.name().to_string();
        info!("Registered plugin: {} v{}", name, plugin.version());
        self.plugins.insert(name, plugin);
    }

    #[must_use]
    pub fn get_plugin(&self, name: &str) -> Option<&dyn Plugin> {
        self.plugins.get(name).map(AsRef::as_ref)
    }

    pub fn unregister(&mut self, name: &str) -> Option<Box<dyn Plugin>> {
        let plugin = self.plugins.remove(name)?;
        info!("Unregistered plugin: {}", name);
        Some(plugin)
    }

    #[must_use]
    pub fn list_plugins(&self) -> Vec<(&str, &str)> {
        self.plugins
            .iter()
            .map(|(name, plugin)| (name.as_str(), plugin.version()))
            .collect()
    }

    pub fn shutdown_all(&mut self) {
        for (name, mut plugin) in self.plugins.drain() {
            if let Err(e) = plugin.shutdown() {
                error!("Error shutting down plugin '{}': {}", name, e);
            }
        }
        info!("All plugins shut down");
    }
}


