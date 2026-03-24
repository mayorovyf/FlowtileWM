mod cli;
mod control;
mod hotkeys;
mod ipc;
mod projection;
mod watch;

use std::{env, process::ExitCode};

use cli::{DaemonCommand, parse_command, print_usage};
use flowtile_wm_core::{CoreDaemonBootstrap, CoreDaemonRuntime};
use hotkeys::ensure_bind_control_mode_supported;

fn main() -> ExitCode {
    match parse_command(env::args().skip(1).collect()) {
        Ok(command) => run(command),
        Err(message) => {
            eprintln!("{message}");
            print_usage();
            ExitCode::from(2)
        }
    }
}

fn run(command: DaemonCommand) -> ExitCode {
    match command {
        DaemonCommand::Bootstrap { runtime_mode } => {
            let bootstrap = CoreDaemonBootstrap::new(runtime_mode);
            println!("flowtile-core-daemon bootstrap");
            for line in bootstrap.summary_lines() {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        DaemonCommand::RunOnce {
            runtime_mode,
            dry_run,
        } => {
            let mut runtime = CoreDaemonRuntime::new(runtime_mode);
            if let Err(error) = ensure_bind_control_mode_supported(runtime.bind_control_mode()) {
                eprintln!("bind control mode startup failed: {error}");
                return ExitCode::from(1);
            }
            match runtime.scan_and_sync(dry_run) {
                Ok(report) => {
                    println!("flowtile-core-daemon run-once");
                    for line in report.summary_lines() {
                        println!("{line}");
                    }
                    ExitCode::SUCCESS
                }
                Err(error) => {
                    eprintln!("{error:?}");
                    ExitCode::from(1)
                }
            }
        }
        DaemonCommand::Watch {
            runtime_mode,
            dry_run,
            interval_ms,
            iterations,
            poll_only,
        } => watch::run_watch(runtime_mode, dry_run, interval_ms, iterations, poll_only),
    }
}
