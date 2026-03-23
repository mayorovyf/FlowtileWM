use std::{env, process::ExitCode};

use flowtile_domain::RuntimeMode;
use flowtile_ipc::bootstrap as ipc_bootstrap;

fn main() -> ExitCode {
    let mut arguments = env::args().skip(1);
    match arguments.next().as_deref() {
        None | Some("help") | Some("--help") => {
            print_help();
            ExitCode::SUCCESS
        }
        Some("status") => {
            if arguments.next().is_some() {
                eprintln!("status does not accept extra arguments");
                return ExitCode::from(2);
            }

            let ipc = ipc_bootstrap();
            println!("flowtile-cli bootstrap status");
            println!("transport: {}", ipc.transport);
            println!("protocol version: {}", ipc.protocol_version);
            println!("known commands: {}", ipc.commands.join(", "));
            println!(
                "runtime modes planned: {}, {}, {}",
                RuntimeMode::WmOnly,
                RuntimeMode::ExtendedShell,
                RuntimeMode::SafeMode
            );
            println!("live daemon handshake: pending for a later version");
            ExitCode::SUCCESS
        }
        Some(command) => {
            eprintln!("unsupported command '{command}'");
            print_help();
            ExitCode::from(2)
        }
    }
}

fn print_help() {
    println!("flowtile-cli");
    println!("usage:");
    println!("  flowtile-cli help");
    println!("  flowtile-cli status");
}
