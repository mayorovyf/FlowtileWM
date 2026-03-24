use std::{
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::Sender,
    thread::{self, JoinHandle},
};

use flowtile_config_rules::HotkeyBinding;
use serde::{Deserialize, Serialize};

use crate::control::{ControlMessage, WatchCommand};

use super::HotkeyListenerError;

const HOTKEY_SCRIPT_NAME: &str = "observe-hotkeys.ps1";

pub(super) struct ScriptHotkeyRuntime {
    child: Child,
    stdout_thread: Option<JoinHandle<()>>,
    stderr_thread: Option<JoinHandle<()>>,
}

impl ScriptHotkeyRuntime {
    pub(super) fn shutdown(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();

        if let Some(stdout_thread) = self.stdout_thread.take() {
            let _ = stdout_thread.join();
        }
        if let Some(stderr_thread) = self.stderr_thread.take() {
            let _ = stderr_thread.join();
        }
    }
}

#[derive(Serialize)]
struct HotkeyRegistrationRequest {
    hotkeys: Vec<HotkeyRegistration>,
}

#[derive(Serialize)]
struct HotkeyRegistration {
    trigger: String,
    command: String,
}

#[derive(Deserialize)]
struct HotkeyScriptEvent {
    kind: String,
    #[serde(default)]
    trigger: Option<String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

pub(super) fn spawn_script(
    bindings: &[HotkeyBinding],
    command_sender: Sender<ControlMessage>,
) -> Result<Option<ScriptHotkeyRuntime>, HotkeyListenerError> {
    let registrations = bindings
        .iter()
        .filter(|binding| WatchCommand::from_hotkey_command(&binding.command).is_some())
        .map(|binding| HotkeyRegistration {
            trigger: binding.trigger.clone(),
            command: binding.command.clone(),
        })
        .collect::<Vec<_>>();

    if registrations.is_empty() {
        return Ok(None);
    }

    let script_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("scripts")
        .join(HOTKEY_SCRIPT_NAME);
    let payload = serde_json::to_vec(&HotkeyRegistrationRequest {
        hotkeys: registrations,
    })?;

    let mut command = Command::new("pwsh");
    command
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(&script_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(&payload)?;
    }

    let stdout = child
        .stdout
        .take()
        .ok_or(HotkeyListenerError::MissingStdout)?;
    let stderr = child
        .stderr
        .take()
        .ok_or(HotkeyListenerError::MissingStderr)?;

    let stdout_thread = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }

                    match serde_json::from_str::<HotkeyScriptEvent>(line) {
                        Ok(event) => match event.kind.as_str() {
                            "command" => {
                                let Some(command_name) = event.command.as_deref() else {
                                    eprintln!(
                                        "hotkey listener emitted command event without command"
                                    );
                                    continue;
                                };
                                let Some(command) = WatchCommand::from_hotkey_command(command_name)
                                else {
                                    eprintln!(
                                        "hotkey listener emitted unsupported command '{command_name}'"
                                    );
                                    continue;
                                };
                                if command_sender.send(ControlMessage::Watch(command)).is_err() {
                                    break;
                                }
                            }
                            "warning" => {
                                let trigger = event.trigger.unwrap_or_else(|| "?".to_string());
                                let command = event.command.unwrap_or_else(|| "?".to_string());
                                let message = event
                                    .message
                                    .unwrap_or_else(|| "unknown hotkey warning".to_string());
                                eprintln!("hotkey warning for {trigger} ({command}): {message}");
                            }
                            other => {
                                eprintln!(
                                    "hotkey listener emitted unsupported event kind '{other}'"
                                );
                            }
                        },
                        Err(error) => {
                            eprintln!("hotkey listener returned invalid json: {error}");
                            break;
                        }
                    }
                }
                Err(error) => {
                    eprintln!("failed to read hotkey listener stdout: {error}");
                    break;
                }
            }
        }
    });

    let stderr_thread = thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    let line = line.trim();
                    if line.is_empty() {
                        continue;
                    }
                    eprintln!("hotkey listener: {line}");
                }
                Err(error) => {
                    eprintln!("failed to read hotkey listener stderr: {error}");
                    break;
                }
            }
        }
    });

    Ok(Some(ScriptHotkeyRuntime {
        child,
        stdout_thread: Some(stdout_thread),
        stderr_thread: Some(stderr_thread),
    }))
}
