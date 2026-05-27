//! Shared self-update plumbing for kittui/kittwm binaries.

use std::path::PathBuf;

use anyhow::{anyhow, Error, Result};
use mcp_cli::{McpServer, StdioServerConfig, ToolRouter};
use serde_json::Value;
use updatable_cli::{AssetStrategy, Updater, UpdaterConfig};

const DEFAULT_REPOSITORY: &str = "harryaskham/kittui";

/// Which update action to run from the CLI.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum UpdateAction {
    /// Print local install/staged status.
    Status,
    /// Check GitHub releases for the latest release.
    Check,
    /// Stage and promote the latest release.
    Run,
}

/// CLI options shared by kittui and kittwm update commands.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateOptions {
    /// Action to run. Defaults to [`UpdateAction::Run`].
    pub action: UpdateAction,
    /// Emit mcp-cli JSON envelope.
    pub json: bool,
    /// Override owner/repo release source.
    pub repository: Option<String>,
    /// Override install directory.
    pub install_dir: Option<PathBuf>,
}

impl Default for UpdateOptions {
    fn default() -> Self {
        Self {
            action: UpdateAction::Run,
            json: false,
            repository: None,
            install_dir: None,
        }
    }
}

/// Build an updatable-cli config for one kittui-family binary.
pub fn updater_config(tool_name: &str) -> UpdaterConfig {
    let mut config = UpdaterConfig::new(tool_name, env!("CARGO_PKG_VERSION"), DEFAULT_REPOSITORY);
    config.asset_strategy = AssetStrategy::TendrilStyle;
    config
}

fn configured_updater(tool_name: &str, options: &UpdateOptions) -> Updater {
    let mut config = updater_config(tool_name);
    if let Some(repository) = &options.repository {
        config.repo_slug = repository.clone();
    }
    if let Some(install_dir) = &options.install_dir {
        config.install_dir = Some(install_dir.clone());
    }
    Updater::new(config)
}

/// Apply any staged `<tool>_next` update at process entry.
pub fn maybe_apply_staged_update(tool_name: &str) {
    let _ = updatable_cli::maybe_apply_staged_update(tool_name);
}

/// Execute `kittui update` / `kittwm update`.
pub fn run_update_command(tool_name: &str, options: &UpdateOptions) -> Result<()> {
    let updater = configured_updater(tool_name, options);
    match options.action {
        UpdateAction::Status => {
            let status = updater.current_status()?;
            if options.json {
                print_json_envelope("update status", serde_json::to_value(status)?)?;
            } else {
                println!(
                    "{tool_name} update status\ncurrent version: {}\ninstall dir: {}\ninstalled: {} ({})\nstaged next: {} ({})",
                    status.current_version,
                    status.install_dir,
                    status.installed_exists,
                    status.installed_path,
                    status.next_staged,
                    status.next_path,
                );
            }
        }
        UpdateAction::Check => {
            let latest = updater
                .check_latest()
                .map_err(|err| explain_update_error(tool_name, options, err))?;
            if options.json {
                print_json_envelope("update check", serde_json::to_value(latest)?)?;
            } else {
                println!(
                    "{tool_name} latest release\ntag: {}\nversion: {}\nnewer than current: {}\nassets: {}",
                    latest.tag,
                    latest.version,
                    latest.newer_than_current,
                    latest.assets.join(", "),
                );
            }
        }
        UpdateAction::Run => {
            let outcome = updater
                .run_update()
                .map_err(|err| explain_update_error(tool_name, options, err))?;
            if options.json {
                print_json_envelope("update", serde_json::to_value(outcome)?)?;
            } else {
                println!(
                    "{tool_name} update\ncurrent version: {}\nlatest version: {}\nstaged: {}\npromoted: {}\nnext path: {}\ninstalled path: {}{}",
                    outcome.current_version,
                    outcome.latest_version,
                    outcome.staged,
                    outcome.promoted,
                    outcome.next_path,
                    outcome.installed_path,
                    outcome
                        .note
                        .as_deref()
                        .map(|note| format!("\nnote: {note}"))
                        .unwrap_or_default(),
                );
            }
        }
    }
    Ok(())
}

fn explain_update_error(tool_name: &str, options: &UpdateOptions, err: Error) -> Error {
    let details = format!("{err:#}");
    if is_missing_latest_release_error(&details) {
        let repo = options.repository.as_deref().unwrap_or(DEFAULT_REPOSITORY);
        anyhow!(
            "{tool_name} update could not find a published GitHub Release for {repo}. \
             The updater installs release assets; it does not update directly from main. \
             Publish a semver tag/release with {tool_name} assets, or build/install from source, then retry.\n\nCaused by:\n{details}"
        )
    } else {
        err
    }
}

fn is_missing_latest_release_error(details: &str) -> bool {
    details.contains("/releases/latest") && details.contains("status code 404")
}

fn print_json_envelope(command: &str, data: Value) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&mcp_cli::JsonEnvelope::success_for(command, data))?
    );
    Ok(())
}

/// Serve the shared updatable-cli self-update tools over MCP stdio.
pub fn serve_update_mcp(tool_name: &'static str) -> Result<()> {
    let mut router = ToolRouter::new();
    updatable_cli::register_update_tool(&mut router, move |_context: &()| {
        updater_config(tool_name)
    });
    let server = McpServer::new(
        StdioServerConfig {
            server_name: tool_name.to_string(),
            server_version: env!("CARGO_PKG_VERSION").to_string(),
        },
        router,
    );
    server.serve_stdio(&())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updater_config_uses_kittui_release_repository_and_tendril_assets() {
        let config = updater_config("kittui");
        assert_eq!(config.tool_name, "kittui");
        assert_eq!(config.repo_slug, DEFAULT_REPOSITORY);
        assert!(matches!(config.asset_strategy, AssetStrategy::TendrilStyle));
    }

    #[test]
    fn update_errors_explain_missing_latest_release() {
        assert!(is_missing_latest_release_error(
            "GET https://api.github.com/repos/harryaskham/kittui/releases/latest: status code 404"
        ));
        let err = explain_update_error(
            "kittwm",
            &UpdateOptions::default(),
            anyhow!(
                "GET https://api.github.com/repos/harryaskham/kittui/releases/latest: status code 404"
            ),
        );
        let message = format!("{err:#}");
        assert!(message.contains("could not find a published GitHub Release"));
        assert!(message.contains("does not update directly from main"));
        assert!(message.contains("semver tag/release"));
    }
}
