use std::{env, process::ExitCode};

use flowtile_domain::RuntimeMode;
use flowtile_wm_core::CoreDaemonBootstrap;

fn main() -> ExitCode {
    match parse_runtime_mode(env::args().skip(1)) {
        Ok(runtime_mode) => {
            let bootstrap = CoreDaemonBootstrap::new(runtime_mode);
            println!("flowtile-core-daemon bootstrap");
            for line in bootstrap.summary_lines() {
                println!("{line}");
            }
            ExitCode::SUCCESS
        }
        Err(message) => {
            eprintln!("{message}");
            eprintln!("usage: flowtile-core-daemon [wm-only|extended-shell|safe-mode]");
            ExitCode::from(2)
        }
    }
}

fn parse_runtime_mode<I>(mut arguments: I) -> Result<RuntimeMode, String>
where
    I: Iterator<Item = String>,
{
    match (arguments.next(), arguments.next()) {
        (None, None) => Ok(RuntimeMode::WmOnly),
        (Some(value), None) => {
            RuntimeMode::parse(&value).ok_or_else(|| format!("unsupported runtime mode '{value}'"))
        }
        (_, Some(_)) => Err("expected at most one runtime mode argument".to_string()),
    }
}
