mod model;
mod ops;
mod tui;

use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use model::{Agent, Inventory};

#[derive(Debug, Parser)]
#[command(
    name = "symskill",
    version,
    about = "Manage per-agent symlinks for SKILL.md skill directories"
)]
struct Cli {
    /// Skill source root(s). Repeat the flag or pass a comma-separated list.
    #[arg(
        short,
        long,
        value_name = "PATH",
        required = true,
        value_delimiter = ','
    )]
    source: Vec<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Launch the interactive terminal UI.
    Tui,
    /// List discovered skills and link status.
    List,
    /// Link a skill into one or more agent skill directories.
    Link {
        skill: String,
        #[arg(short, long, value_enum, default_values_t = AgentArg::defaults())]
        agent: Vec<AgentArg>,
    },
    /// Remove a managed symlink from one or more agent skill directories.
    Unlink {
        skill: String,
        #[arg(short, long, value_enum, default_values_t = AgentArg::defaults())]
        agent: Vec<AgentArg>,
    },
    /// Toggle a skill link for one or more agents.
    Toggle {
        skill: String,
        #[arg(short, long, value_enum, default_values_t = AgentArg::defaults())]
        agent: Vec<AgentArg>,
    },
    /// Replace wrong symlinks with links to the current source skill.
    Fix {
        skill: String,
        #[arg(short, long, value_enum, default_values_t = AgentArg::defaults())]
        agent: Vec<AgentArg>,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum AgentArg {
    Claude,
    Codex,
    Cursor,
}

impl AgentArg {
    fn defaults() -> Vec<Self> {
        vec![Self::Claude, Self::Codex]
    }
}

impl From<AgentArg> for Agent {
    fn from(value: AgentArg) -> Self {
        match value {
            AgentArg::Claude => Agent::Claude,
            AgentArg::Codex => Agent::Codex,
            AgentArg::Cursor => Agent::Cursor,
        }
    }
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let sources = cli.source;

    let command = cli.command.unwrap_or(Command::Tui);
    match command {
        Command::Tui => tui::run(sources),
        Command::List => {
            let inventory = Inventory::load(&sources);
            print_warnings(&inventory);
            print_inventory(&inventory);
            Ok(())
        }
        Command::Link { skill, agent } => apply_to_skill(&sources, &skill, agent, ops::link_skill),
        Command::Unlink { skill, agent } => {
            apply_to_skill(&sources, &skill, agent, ops::unlink_skill)
        }
        Command::Toggle { skill, agent } => {
            apply_to_skill(&sources, &skill, agent, ops::toggle_skill)
        }
        Command::Fix { skill, agent } => apply_to_skill(&sources, &skill, agent, ops::fix_skill),
    }
}

fn print_warnings(inventory: &Inventory) {
    for warning in &inventory.warnings {
        eprintln!("warning: {warning}");
    }
}

fn print_inventory(inventory: &Inventory) {
    println!(
        "{:<32} {:<8} {:<8} {:<8} path",
        "skill", "claude", "codex", "cursor"
    );
    for skill in &inventory.skills {
        let claude = inventory.status_label(skill, Agent::Claude);
        let codex = inventory.status_label(skill, Agent::Codex);
        let cursor = inventory.status_label(skill, Agent::Cursor);
        println!(
            "{:<32} {:<8} {:<8} {:<8} {}",
            skill.name,
            claude,
            codex,
            cursor,
            skill.path.display()
        );
    }
}

fn apply_to_skill<F>(
    sources: &[PathBuf],
    skill_name: &str,
    agents: Vec<AgentArg>,
    mut operation: F,
) -> Result<()>
where
    F: FnMut(&model::Skill, Agent) -> Result<ops::OperationResult>,
{
    let inventory = Inventory::load(sources);
    print_warnings(&inventory);

    let matches = inventory.find_all(skill_name);
    let skill = match matches.as_slice() {
        [] => bail!("skill not found: {skill_name}"),
        [skill] => *skill,
        matches => {
            let paths: String = matches
                .iter()
                .map(|skill| format!("\n  {}", skill.path.display()))
                .collect();
            bail!("skill name '{skill_name}' matches multiple skills, refusing to guess:{paths}");
        }
    };

    for agent_arg in agents {
        let agent = Agent::from(agent_arg);
        let result = operation(skill, agent)?;
        println!("{} {}: {}", agent.label(), skill.name, result.message);
    }

    Ok(())
}
