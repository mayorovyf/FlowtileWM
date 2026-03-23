use std::{
    env,
    io::{self, BufRead},
    process::ExitCode,
    sync::mpsc,
    thread,
    time::Duration,
};

use flowtile_domain::RuntimeMode;
use flowtile_windows_adapter::{LiveObservationOptions, ObservationStreamError, WindowsAdapter};
use flowtile_wm_core::{CoreDaemonBootstrap, CoreDaemonRuntime, RuntimeCycleReport};

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
            match runtime.scan_and_sync(dry_run) {
                Ok(report) => {
                    println!("flowtile-core-daemon run-once");
                    print_report(&report);
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
        } => {
            let adapter = WindowsAdapter::new();
            let mut runtime = CoreDaemonRuntime::with_adapter(runtime_mode, adapter.clone());
            let mut observer = if poll_only {
                None
            } else {
                match adapter.spawn_observer(LiveObservationOptions {
                    fallback_scan_interval_ms: interval_ms.max(1_000),
                    ..LiveObservationOptions::default()
                }) {
                    Ok(stream) => Some(stream),
                    Err(error) => {
                        eprintln!(
                            "live observation failed to start: {error}; falling back to polling"
                        );
                        None
                    }
                }
            };
            let (command_sender, command_receiver) = mpsc::channel::<WatchCommand>();
            let _stdin_thread = thread::spawn(move || {
                let stdin = io::stdin();
                let mut locked = stdin.lock();
                let mut line = String::new();

                loop {
                    line.clear();
                    match locked.read_line(&mut line) {
                        Ok(0) => break,
                        Ok(_) => {
                            let command = line.trim().to_ascii_lowercase();
                            let watch_command = match command.as_str() {
                                "unwind" => Some(WatchCommand::Unwind),
                                "rescan" => Some(WatchCommand::Rescan),
                                "quit" | "exit" => Some(WatchCommand::Quit),
                                _ => None,
                            };
                            if let Some(watch_command) = watch_command
                                && command_sender.send(watch_command).is_err()
                            {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            });

            println!("flowtile-core-daemon watch");
            println!(
                "observation mode: {}",
                if observer.is_some() {
                    "live-hooks"
                } else {
                    "polling-fallback"
                }
            );
            println!("stdin commands: unwind, rescan, quit");

            let mut completed_iterations = 0_u64;
            if let Some(live_observer) = observer.as_mut() {
                match live_observer.recv_timeout(Duration::from_millis(interval_ms.max(5_000))) {
                    Ok(observation) => match runtime.apply_observation(observation, dry_run) {
                        Ok(Some(report)) => {
                            print_iteration(completed_iterations + 1, &report);
                            completed_iterations += 1;
                            if iterations.is_some_and(|limit| completed_iterations >= limit) {
                                return ExitCode::SUCCESS;
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            eprintln!("{error:?}");
                            return ExitCode::from(1);
                        }
                    },
                    Err(ObservationStreamError::Timeout) => {
                        eprintln!(
                            "live observation did not produce an initial snapshot in time; falling back to polling"
                        );
                        observer = None;
                    }
                    Err(error) => {
                        eprintln!(
                            "live observation failed during startup: {error}; falling back to polling"
                        );
                        observer = None;
                    }
                }
            }

            loop {
                if let Ok(command) = command_receiver.try_recv() {
                    match command {
                        WatchCommand::Unwind => {
                            runtime.request_emergency_unwind("stdin-command");
                            println!("management disabled by emergency unwind");
                            continue;
                        }
                        WatchCommand::Rescan => match runtime.scan_and_sync(dry_run) {
                            Ok(report) => {
                                println!("manual rescan");
                                print_iteration(completed_iterations + 1, &report);
                                completed_iterations += 1;
                                if iterations.is_some_and(|limit| completed_iterations >= limit) {
                                    return ExitCode::SUCCESS;
                                }
                            }
                            Err(error) => {
                                eprintln!("{error:?}");
                                return ExitCode::from(1);
                            }
                        },
                        WatchCommand::Quit => return ExitCode::SUCCESS,
                    }
                }

                let mut fallback_to_polling = false;
                let cycle_result = if let Some(live_observer) = observer.as_mut() {
                    match live_observer.recv_timeout(Duration::from_millis(interval_ms)) {
                        Ok(observation) => runtime.apply_observation(observation, dry_run),
                        Err(ObservationStreamError::Timeout) => {
                            runtime.scan_and_sync(dry_run).map(Some)
                        }
                        Err(error) => {
                            eprintln!(
                                "live observation became unavailable: {error}; switching to polling"
                            );
                            fallback_to_polling = true;
                            runtime.scan_and_sync(dry_run).map(Some)
                        }
                    }
                } else {
                    runtime.scan_and_sync(dry_run).map(Some)
                };

                if fallback_to_polling {
                    observer = None;
                }

                match cycle_result {
                    Ok(Some(report)) => {
                        print_iteration(completed_iterations + 1, &report);
                        completed_iterations += 1;
                    }
                    Ok(None) => continue,
                    Err(error) => {
                        eprintln!("{error:?}");
                        return ExitCode::from(1);
                    }
                }

                if iterations.is_some_and(|limit| completed_iterations >= limit) {
                    return ExitCode::SUCCESS;
                }

                if observer.is_none() {
                    thread::sleep(Duration::from_millis(interval_ms));
                }
            }
        }
    }
}

fn print_iteration(iteration: u64, report: &RuntimeCycleReport) {
    println!("iteration {iteration}");
    print_report(report);
}

fn print_report(report: &RuntimeCycleReport) {
    for line in report.summary_lines() {
        println!("{line}");
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum DaemonCommand {
    Bootstrap {
        runtime_mode: RuntimeMode,
    },
    RunOnce {
        runtime_mode: RuntimeMode,
        dry_run: bool,
    },
    Watch {
        runtime_mode: RuntimeMode,
        dry_run: bool,
        interval_ms: u64,
        iterations: Option<u64>,
        poll_only: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum WatchCommand {
    Unwind,
    Rescan,
    Quit,
}

fn parse_command(arguments: Vec<String>) -> Result<DaemonCommand, String> {
    if arguments.is_empty() {
        return Ok(DaemonCommand::Bootstrap {
            runtime_mode: RuntimeMode::WmOnly,
        });
    }

    let first = arguments[0].as_str();
    match first {
        "bootstrap" => Ok(DaemonCommand::Bootstrap {
            runtime_mode: parse_runtime_mode_flags(&arguments[1..])?,
        }),
        "run-once" => {
            let (runtime_mode, dry_run, _, _, _) = parse_runtime_flags(&arguments[1..])?;
            Ok(DaemonCommand::RunOnce {
                runtime_mode,
                dry_run,
            })
        }
        "watch" => {
            let (runtime_mode, dry_run, interval_ms, iterations, poll_only) =
                parse_runtime_flags(&arguments[1..])?;
            Ok(DaemonCommand::Watch {
                runtime_mode,
                dry_run,
                interval_ms,
                iterations,
                poll_only,
            })
        }
        value if RuntimeMode::parse(value).is_some() => Ok(DaemonCommand::Bootstrap {
            runtime_mode: RuntimeMode::parse(value)
                .ok_or_else(|| format!("unsupported runtime mode '{value}'"))?,
        }),
        _ => Err(format!("unsupported command '{}'", arguments[0])),
    }
}

fn parse_runtime_mode_flags(arguments: &[String]) -> Result<RuntimeMode, String> {
    let (runtime_mode, _, _, _, _) = parse_runtime_flags(arguments)?;
    Ok(runtime_mode)
}

fn parse_runtime_flags(
    arguments: &[String],
) -> Result<(RuntimeMode, bool, u64, Option<u64>, bool), String> {
    let mut runtime_mode = RuntimeMode::WmOnly;
    let mut dry_run = false;
    let mut interval_ms = 750_u64;
    let mut iterations = None;
    let mut poll_only = false;
    let mut index = 0_usize;

    while index < arguments.len() {
        match arguments[index].as_str() {
            "--dry-run" => {
                dry_run = true;
                index += 1;
            }
            "--poll-only" => {
                poll_only = true;
                index += 1;
            }
            "--interval-ms" => {
                let Some(value) = arguments.get(index + 1) else {
                    return Err("--interval-ms expects a value".to_string());
                };
                interval_ms = value
                    .parse::<u64>()
                    .map_err(|_| format!("invalid --interval-ms value '{value}'"))?;
                index += 2;
            }
            "--iterations" => {
                let Some(value) = arguments.get(index + 1) else {
                    return Err("--iterations expects a value".to_string());
                };
                iterations = Some(
                    value
                        .parse::<u64>()
                        .map_err(|_| format!("invalid --iterations value '{value}'"))?,
                );
                index += 2;
            }
            value => {
                runtime_mode = RuntimeMode::parse(value)
                    .ok_or_else(|| format!("unsupported runtime mode '{value}'"))?;
                index += 1;
            }
        }
    }

    Ok((runtime_mode, dry_run, interval_ms, iterations, poll_only))
}

fn print_usage() {
    println!("flowtile-core-daemon");
    println!("usage:");
    println!("  flowtile-core-daemon");
    println!("  flowtile-core-daemon bootstrap [wm-only|extended-shell|safe-mode]");
    println!("  flowtile-core-daemon run-once [--dry-run] [wm-only|extended-shell|safe-mode]");
    println!(
        "  flowtile-core-daemon watch [--dry-run] [--poll-only] [--interval-ms N] [--iterations N] [wm-only|extended-shell|safe-mode]"
    );
}
