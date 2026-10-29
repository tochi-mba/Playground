//! `lumen`: projection mapping for Windows (and anywhere wgpu runs).
//!
//! With no arguments the editor opens. Subcommands cover the headless workflows that make the
//! app scriptable and testable without a window: `render`, `validate`, `migrate`, `new`.

#![forbid(unsafe_code)]

mod autosave;
mod cli;
mod editor;
mod settings;
mod theme;
mod watchdog;

use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

/// Projection mapping editor and show player.
#[derive(Debug, Parser)]
#[command(name = "lumen", version, about, long_about = None)]
struct Args {
    /// Project file to open in the editor.
    project: Option<PathBuf>,

    /// UDP port for OSC control (0 disables).
    #[arg(long, default_value_t = 9000)]
    osc_port: u16,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Open a project straight into locked show mode with every output window up.
    Show {
        /// Project file.
        project: PathBuf,
        /// UDP port for OSC control (0 disables).
        #[arg(long, default_value_t = 9000)]
        osc_port: u16,
        /// Relaunch the show if the render loop stalls for more than this many seconds.
        #[arg(long)]
        restart_on_stall: Option<f64>,
        /// Serve the phone remote on this port (0 disables; omit to use the saved setting).
        #[arg(long)]
        remote_port: Option<u16>,
    },
    /// Render outputs to PNG files without opening a window.
    Render(cli::RenderArgs),
    /// Check a project file and report problems. Exit code 1 when there are errors.
    Validate {
        /// Project file.
        project: PathBuf,
    },
    /// Rewrite a project file in the current schema.
    Migrate {
        /// Project file.
        project: PathBuf,
        /// Write here instead of in place.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Write the two-projector demo project.
    New {
        /// Destination `.lumen.toml`.
        path: PathBuf,
        /// Overwrite an existing file.
        #[arg(long)]
        force: bool,
    },
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let args = Args::parse();
    let result = match args.command {
        None => editor::run(&editor::Launch {
            project: args.project,
            osc_port: args.osc_port,
            show: false,
            restart_on_stall: None,
            remote_port: None,
        }),
        Some(Command::Show {
            project,
            osc_port,
            restart_on_stall,
            remote_port,
        }) => editor::run(&editor::Launch {
            project: Some(project),
            osc_port,
            show: true,
            restart_on_stall,
            remote_port,
        }),
        Some(Command::Render(render)) => cli::render(&render),
        Some(Command::Validate { project }) => cli::validate(&project),
        Some(Command::Migrate { project, out }) => cli::migrate(&project, out.as_deref()),
        Some(Command::New { path, force }) => cli::new_project(&path, force),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            #[allow(clippy::print_stderr)]
            {
                eprintln!("error: {e:#}");
            }
            ExitCode::from(2)
        }
    }
}
