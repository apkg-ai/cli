mod api;
mod commands;
mod config;
mod error;
mod resolver;
mod setup;
mod util;

use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};

use crate::error::AppError;

#[derive(Parser)]
#[command(
    name = "qpm",
    about = "Package manager for AI tooling — skills, agents, MCP servers, prompts, configs",
    version
)]
struct Cli {
    /// Registry URL (overrides `QPM_REGISTRY` env var and config file)
    #[arg(long, global = true)]
    registry: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage the global package cache
    Cache {
        #[command(subcommand)]
        action: CacheSubcommand,
    },

    /// Manage CLI configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Create a new qpm.json manifest interactively
    Init {
        /// Overwrite existing qpm.json
        #[arg(long)]
        force: bool,
    },

    /// Authenticate with the registry
    Login,

    /// Remove stored credentials
    Logout,

    /// Show the currently authenticated user
    Whoami,

    /// Manage Ed25519 signing keys
    Key {
        #[command(subcommand)]
        action: KeySubcommand,
    },

    /// Manage long-lived API tokens for CI/CD and automation
    Token {
        #[command(subcommand)]
        action: TokenSubcommand,
    },

    /// Add a package to dependencies and install it
    Add {
        /// Package name, optionally with version: name[@version]
        package: String,

        /// Add to devDependencies instead
        #[arg(long, short = 'D')]
        save_dev: bool,

        /// Add to peerDependencies instead
        #[arg(long, short = 'P')]
        save_peer: bool,

        /// Generate config for a specific tool only: cursor, claude-code, or all (default)
        #[arg(long, value_name = "TOOL", default_value = "all")]
        setup: SetupTargetArg,

        /// Skip post-install tool setup entirely
        #[arg(long)]
        no_setup: bool,
    },

    /// Manage distribution tags (e.g., latest, beta, canary)
    DistTag {
        #[command(subcommand)]
        action: DistTagSubcommand,
    },

    /// Mark a package or version as deprecated
    Deprecate {
        /// Package name, optionally with version: name[@version]
        target: String,
        /// Deprecation message shown to users on install
        message: Option<String>,
        /// Remove the deprecation notice
        #[arg(long)]
        undo: bool,
    },

    /// Create a tarball from the current package
    Pack,

    /// Pack and upload the package to the registry
    Publish,

    /// Remove a package from dependencies and delete it
    Remove {
        /// Package name
        package: String,

        /// Remove from devDependencies instead
        #[arg(long, short = 'D')]
        save_dev: bool,

        /// Remove from peerDependencies instead
        #[arg(long, short = 'P')]
        save_peer: bool,
    },

    /// Create symlinks for local development
    Link {
        /// Package name or path to link. Omit to register current package globally.
        target: Option<String>,
    },

    /// Remove development symlinks
    Unlink {
        /// Package name. Omit to unregister current package globally.
        package: Option<String>,
    },

    /// Download and extract a package (or all deps from qpm.json)
    Install {
        /// Package name[@version]. Omit to install all deps from qpm.json.
        package: Option<String>,

        /// Generate config for a specific tool only: cursor, claude-code, or all (default)
        #[arg(long, value_name = "TOOL", default_value = "all")]
        setup: SetupTargetArg,

        /// Skip post-install tool setup entirely
        #[arg(long)]
        no_setup: bool,

        /// Fail if lockfile would change (CI mode)
        #[arg(long)]
        frozen_lockfile: bool,
    },

    /// Search the registry for packages
    Search {
        /// Search query
        query: String,

        /// Maximum number of results
        #[arg(long, default_value = "20")]
        limit: u32,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Update packages to latest versions within their semver ranges
    Update {
        /// Package name. Omit to update all dependencies.
        package: Option<String>,

        /// Ignore semver ranges; update to the absolute latest version
        #[arg(long)]
        latest: bool,

        /// Show what would change without modifying anything
        #[arg(long)]
        dry_run: bool,
    },

    /// Show package metadata
    Info {
        /// Package name
        package: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Generate shell completion scripts
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Verify signatures and integrity of installed packages
    Verify {
        /// Package name. Omit to verify all installed packages.
        package: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Exit with code 8 if any package is unverified
        #[arg(long)]
        strict: bool,
    },
}

#[derive(Subcommand)]
enum CacheSubcommand {
    /// Remove all entries from the global cache
    Clean,
    /// List all cached packages and their sizes
    List,
    /// Verify integrity of all cached entries
    Verify,
}

#[derive(Subcommand)]
enum KeySubcommand {
    /// Generate a new Ed25519 signing key pair
    Generate {
        /// Human-readable name for the key
        #[arg(long)]
        name: String,
    },
    /// List signing keys (registry keys by default, or local with --local)
    List {
        /// Show locally stored keys instead of registry keys
        #[arg(long)]
        local: bool,
    },
    /// Register a local public key with the registry
    Register {
        /// Name for the key on the registry (defaults to local key name)
        #[arg(long)]
        name: Option<String>,
        /// Key ID (fingerprint) to register; prompts if multiple local keys
        #[arg(long)]
        key_id: Option<String>,
    },
    /// Revoke a signing key on the registry
    Revoke {
        /// Key ID (fingerprint) to revoke
        key_id: String,
        /// Reason for revocation
        #[arg(long, value_parser = ["key-compromise", "superseded", "unspecified"])]
        reason: String,
        /// Optional human-readable revocation message
        #[arg(long)]
        message: Option<String>,
    },
    /// Rotate a signing key (replace with a new one, attested by the old key)
    Rotate {
        /// Key ID (fingerprint) of the old key to replace
        old_key_id: String,
        /// Name for the new key (defaults to old key's name)
        #[arg(long)]
        name: Option<String>,
    },
}

#[derive(Subcommand)]
enum TokenSubcommand {
    /// Create a new API token
    Create {
        /// Human-readable name for the token
        #[arg(long)]
        name: String,
        /// Comma-separated scopes: read, publish, admin, ci
        #[arg(long, value_delimiter = ',')]
        scopes: Vec<String>,
        /// Token lifetime: 30d, 90d, or 365d
        #[arg(long, value_parser = ["30d", "90d", "365d"], default_value = "90d")]
        expires_in: String,
        /// Restrict token to specific packages (glob pattern)
        #[arg(long)]
        package_scope: Option<String>,
    },
    /// List active API tokens
    List {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Revoke an API token
    Revoke {
        /// Token ID (UUID)
        id: String,
    },
}

#[derive(Subcommand)]
enum DistTagSubcommand {
    /// Assign a tag to a specific version
    Add {
        /// Package with version: name@version
        package_at_version: String,
        /// Tag name (e.g., latest, beta, canary)
        tag: String,
    },
    /// Remove a tag from a package
    Rm {
        /// Package name
        package: String,
        /// Tag name to remove
        tag: String,
    },
    /// List all tags for a package
    Ls {
        /// Package name
        package: String,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Set a config value (e.g. `qpm config set services.auth http://localhost:8787`)
    Set {
        /// Config key (registry, services.auth, services.package, services.mfa, services.search)
        key: String,
        /// Value to set
        value: String,
    },
    /// Get a config value
    Get {
        /// Config key
        key: String,
    },
    /// List all config values
    List,
    /// Delete a config value
    Delete {
        /// Config key
        key: String,
    },
}

#[derive(Clone, clap::ValueEnum)]
enum SetupTargetArg {
    All,
    Cursor,
    ClaudeCode,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Handle completions synchronously — no async runtime needed
    if let Commands::Completions { shell } = cli.command {
        let mut cmd = Cli::command();
        clap_complete::generate(shell, &mut cmd, "qpm", &mut std::io::stdout());
        return ExitCode::SUCCESS;
    }

    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Error: Failed to create async runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    match rt.block_on(run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(AppError::VerifyFailed(msg)) => {
            eprintln!("{msg}");
            ExitCode::from(8)
        }
        Err(e) => {
            let report = miette::Report::new(e);
            eprintln!("{report:?}");
            ExitCode::FAILURE
        }
    }
}

#[allow(clippy::too_many_lines)]
async fn run(cli: Cli) -> Result<(), AppError> {
    let registry = cli.registry.as_deref();

    match cli.command {
        Commands::Cache { action } => {
            let cache_action = match action {
                CacheSubcommand::Clean => commands::cache::CacheAction::Clean,
                CacheSubcommand::List => commands::cache::CacheAction::List,
                CacheSubcommand::Verify => commands::cache::CacheAction::Verify,
            };
            commands::cache::run(cache_action)
        }
        Commands::Config { action } => match action {
            ConfigAction::Set { ref key, ref value } => {
                commands::config::run(commands::config::ConfigAction::Set { key, value })
            }
            ConfigAction::Get { ref key } => {
                commands::config::run(commands::config::ConfigAction::Get { key })
            }
            ConfigAction::List => {
                commands::config::run(commands::config::ConfigAction::List)
            }
            ConfigAction::Delete { ref key } => {
                commands::config::run(commands::config::ConfigAction::Delete { key })
            }
        },
        Commands::Init { force } => {
            commands::init::run(commands::init::InitOptions { force })
        }
        Commands::Login => commands::login::run(registry).await,
        Commands::Logout => commands::logout::run(),
        Commands::Whoami => commands::whoami::run(registry).await,
        Commands::Key { action } => {
            let key_action = match &action {
                KeySubcommand::Generate { ref name } => {
                    commands::key::KeyAction::Generate { name }
                }
                KeySubcommand::List { local } => commands::key::KeyAction::List { local: *local },
                KeySubcommand::Register { ref name, ref key_id } => {
                    commands::key::KeyAction::Register {
                        name: name.as_deref(),
                        key_id: key_id.as_deref(),
                    }
                }
                KeySubcommand::Revoke {
                    ref key_id,
                    ref reason,
                    ref message,
                } => commands::key::KeyAction::Revoke {
                    key_id,
                    reason,
                    message: message.as_deref(),
                },
                KeySubcommand::Rotate {
                    ref old_key_id,
                    ref name,
                } => commands::key::KeyAction::Rotate {
                    old_key_id,
                    name: name.as_deref(),
                },
            };
            commands::key::run(key_action, registry).await
        }
        Commands::Token { action } => {
            let token_action = match &action {
                TokenSubcommand::Create {
                    ref name,
                    ref scopes,
                    ref expires_in,
                    ref package_scope,
                } => commands::token::TokenAction::Create {
                    name,
                    scopes,
                    expires_in,
                    package_scope: package_scope.as_deref(),
                },
                TokenSubcommand::List { json } => {
                    commands::token::TokenAction::List { json: *json }
                }
                TokenSubcommand::Revoke { ref id } => {
                    commands::token::TokenAction::Revoke { id }
                }
            };
            commands::token::run(token_action, registry).await
        }
        Commands::Add {
            ref package,
            save_dev,
            save_peer,
            ref setup,
            no_setup,
        } => {
            let category = if save_dev {
                util::package::DepCategory::DevDependencies
            } else if save_peer {
                util::package::DepCategory::PeerDependencies
            } else {
                util::package::DepCategory::Dependencies
            };
            let setup_target = if no_setup {
                None
            } else {
                Some(match setup {
                    SetupTargetArg::All => setup::SetupTarget::All,
                    SetupTargetArg::Cursor => {
                        setup::SetupTarget::Only(setup::Tool::Cursor)
                    }
                    SetupTargetArg::ClaudeCode => {
                        setup::SetupTarget::Only(setup::Tool::ClaudeCode)
                    }
                })
            };
            commands::add::run(commands::add::AddOptions {
                package,
                registry,
                category,
                setup_target,
            })
            .await
        }
        Commands::DistTag { action } => {
            let dist_tag_action = match &action {
                DistTagSubcommand::Add {
                    ref package_at_version,
                    ref tag,
                } => commands::dist_tag::DistTagAction::Add {
                    package_at_version,
                    tag,
                },
                DistTagSubcommand::Rm {
                    ref package,
                    ref tag,
                } => commands::dist_tag::DistTagAction::Rm { package, tag },
                DistTagSubcommand::Ls { ref package } => {
                    commands::dist_tag::DistTagAction::Ls { package }
                }
            };
            commands::dist_tag::run(dist_tag_action, registry).await
        }
        Commands::Deprecate {
            ref target,
            ref message,
            undo,
        } => {
            if !undo && message.is_none() {
                return Err(AppError::Other(
                    "A deprecation message is required. Use --undo to remove deprecation."
                        .to_string(),
                ));
            }
            let msg = if undo { None } else { message.as_deref() };
            commands::deprecate::run(commands::deprecate::DeprecateOptions {
                target,
                message: msg,
                registry,
            })
            .await
        }
        Commands::Link { ref target } => {
            let action = match target {
                Some(t) => commands::link::LinkAction::LinkTarget { target: t },
                None => commands::link::LinkAction::Register,
            };
            commands::link::run_link(&action)
        }
        Commands::Unlink { ref package } => {
            let action = match package {
                Some(p) => commands::link::UnlinkAction::UnlinkPackage { package: p },
                None => commands::link::UnlinkAction::Unregister,
            };
            commands::link::run_unlink(&action)
        }
        Commands::Remove {
            ref package,
            save_dev,
            save_peer,
        } => {
            let category = if save_dev {
                util::package::DepCategory::DevDependencies
            } else if save_peer {
                util::package::DepCategory::PeerDependencies
            } else {
                util::package::DepCategory::Dependencies
            };
            commands::remove::run(&commands::remove::RemoveOptions {
                package,
                category,
            })
        }
        Commands::Pack => commands::pack::run(),
        Commands::Publish => commands::publish::run(registry).await,
        Commands::Install {
            ref package,
            ref setup,
            no_setup,
            frozen_lockfile,
        } => {
            let setup_target = if no_setup {
                None
            } else {
                Some(match setup {
                    SetupTargetArg::All => setup::SetupTarget::All,
                    SetupTargetArg::Cursor => {
                        setup::SetupTarget::Only(setup::Tool::Cursor)
                    }
                    SetupTargetArg::ClaudeCode => {
                        setup::SetupTarget::Only(setup::Tool::ClaudeCode)
                    }
                })
            };
            commands::install::run(commands::install::InstallOptions {
                package: package.as_deref(),
                registry,
                setup_target,
                frozen_lockfile,
            })
            .await
        }
        Commands::Search {
            ref query,
            limit,
            json,
        } => {
            commands::search::run(commands::search::SearchOptions {
                query,
                limit,
                json,
                registry,
            })
            .await
        }
        Commands::Update {
            ref package,
            latest,
            dry_run,
        } => {
            commands::update::run(commands::update::UpdateOptions {
                package: package.as_deref(),
                registry,
                latest,
                dry_run,
            })
            .await
        }
        Commands::Info {
            ref package,
            json,
        } => {
            commands::info::run(commands::info::InfoOptions {
                package,
                json,
                registry,
            })
            .await
        }
        Commands::Completions { .. } => unreachable!(),
        Commands::Verify {
            ref package,
            json,
            strict,
        } => {
            commands::verify::run(commands::verify::VerifyOptions {
                package: package.as_deref(),
                json,
                strict,
                registry,
            })
            .await
        }
    }
}
