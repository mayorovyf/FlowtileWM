use std::{
    mem::zeroed,
    sync::mpsc::{self, Sender},
    thread::{self, JoinHandle},
    time::Duration,
};

use flowtile_config_rules::HotkeyBinding;
use windows_sys::Win32::{
    System::Threading::GetCurrentThreadId,
    UI::{
        Input::KeyboardAndMouse::{MOD_WIN, RegisterHotKey, UnregisterHotKey},
        WindowsAndMessaging::{GetMessageW, MSG, PostThreadMessageW, WM_HOTKEY, WM_QUIT},
    },
};

use crate::control::{ControlMessage, WatchCommand};

use super::{
    HotkeyListenerError,
    low_level::{ensure_message_queue, install_low_level_hook, shutdown_low_level_hook},
    trigger::parse_trigger,
};

pub(super) struct NativeHotkeyRuntime {
    thread_id: u32,
    worker: Option<JoinHandle<()>>,
}

impl NativeHotkeyRuntime {
    pub(super) fn shutdown(&mut self) {
        let _ = unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, 0, 0) };
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Clone)]
pub(super) struct NativeHotkeyRegistration {
    pub(super) trigger: String,
    pub(super) command: WatchCommand,
    pub(super) register_modifiers: u32,
    pub(super) required_modifiers: u32,
    pub(super) key: u32,
}

struct HotkeyStartup {
    thread_id: u32,
    active_registration_count: usize,
}

pub(super) fn spawn_native(
    bindings: &[HotkeyBinding],
    command_sender: Sender<ControlMessage>,
) -> Result<Option<NativeHotkeyRuntime>, HotkeyListenerError> {
    let registrations = bindings
        .iter()
        .filter_map(|binding| {
            let command = WatchCommand::from_hotkey_command(&binding.command)?;
            match parse_trigger(&binding.trigger) {
                Ok(parsed) => Some(NativeHotkeyRegistration {
                    trigger: binding.trigger.clone(),
                    command,
                    register_modifiers: parsed.register_modifiers,
                    required_modifiers: parsed.required_modifiers,
                    key: parsed.key,
                }),
                Err(message) => {
                    eprintln!(
                        "hotkey warning for {} ({}): {}",
                        binding.trigger, binding.command, message
                    );
                    None
                }
            }
        })
        .collect::<Vec<_>>();

    if registrations.is_empty() {
        return Ok(None);
    }

    let (startup_sender, startup_receiver) = mpsc::channel::<Result<HotkeyStartup, String>>();
    let worker = thread::spawn(move || {
        run_hotkey_thread(registrations, command_sender, startup_sender);
    });

    let startup = startup_receiver
        .recv_timeout(Duration::from_secs(5))
        .map_err(|error| {
            HotkeyListenerError::Startup(format!("hotkey listener startup timed out: {error}"))
        })?
        .map_err(HotkeyListenerError::Startup)?;

    if startup.active_registration_count == 0 {
        let _ = worker.join();
        return Ok(None);
    }

    Ok(Some(NativeHotkeyRuntime {
        thread_id: startup.thread_id,
        worker: Some(worker),
    }))
}

fn run_hotkey_thread(
    registrations: Vec<NativeHotkeyRegistration>,
    command_sender: Sender<ControlMessage>,
    startup_sender: mpsc::Sender<Result<HotkeyStartup, String>>,
) {
    ensure_message_queue();
    let thread_id = unsafe { GetCurrentThreadId() };

    let mut registered_ids = Vec::new();
    let mut registration_by_id = Vec::new();
    let mut fallback_registrations = Vec::new();

    for (index, registration) in registrations.into_iter().enumerate() {
        let hotkey_id = i32::try_from(index + 1).unwrap_or(i32::MAX);
        if should_force_low_level_hook(&registration) {
            eprintln!(
                "hotkey info for {} ({}): using low-level win-prefix capture path",
                registration.trigger,
                registration.command.as_hotkey_command_name(),
            );
            fallback_registrations.push(registration);
            continue;
        }

        let registered = unsafe {
            RegisterHotKey(
                std::ptr::null_mut(),
                hotkey_id,
                registration.register_modifiers,
                registration.key,
            ) != 0
        };
        if !registered {
            eprintln!(
                "hotkey warning for {} ({}): {}; using low-level hook fallback",
                registration.trigger,
                registration.command.as_hotkey_command_name(),
                last_error_message("RegisterHotKey")
            );
            fallback_registrations.push(registration);
            continue;
        }

        registered_ids.push(hotkey_id);
        registration_by_id.push((hotkey_id, registration.command));
    }

    let fallback_count = fallback_registrations.len();
    let mut low_level_hook = None;
    let mut active_low_level_count = 0usize;
    if fallback_count > 0 {
        match install_low_level_hook(thread_id, fallback_registrations, command_sender.clone()) {
            Ok(hook) => {
                low_level_hook = Some(hook);
                active_low_level_count = fallback_count;
            }
            Err(message) => {
                eprintln!("hotkey warning: low-level hook startup failed: {message}");
            }
        }
    }

    if registered_ids.is_empty() && active_low_level_count == 0 {
        let _ = startup_sender.send(Err("no hotkeys could be activated".to_string()));
        return;
    }

    let _ = startup_sender.send(Ok(HotkeyStartup {
        thread_id,
        active_registration_count: registered_ids.len() + active_low_level_count,
    }));

    let mut message: MSG = unsafe { zeroed() };
    loop {
        let status = unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) };
        if status <= 0 {
            break;
        }
        if message.message != WM_HOTKEY {
            continue;
        }

        let hotkey_id = message.wParam as i32;
        let command = registration_by_id
            .iter()
            .find_map(|(candidate_id, command)| (*candidate_id == hotkey_id).then_some(*command));
        let Some(command) = command else {
            continue;
        };

        if command_sender.send(ControlMessage::Watch(command)).is_err() {
            break;
        }
    }

    if let Some(hook) = low_level_hook {
        shutdown_low_level_hook(thread_id, hook);
    }

    for hotkey_id in registered_ids {
        let _ = unsafe { UnregisterHotKey(std::ptr::null_mut(), hotkey_id) };
    }
}

pub(super) fn last_error_message(api: &str) -> String {
    let code = unsafe { windows_sys::Win32::Foundation::GetLastError() };
    match code {
        1409 => format!(
            "{api} failed with Win32 error 1409 (hotkey is already registered by another application or another Flowtile daemon instance)"
        ),
        _ => format!("{api} failed with Win32 error {code}"),
    }
}

fn should_force_low_level_hook(registration: &NativeHotkeyRegistration) -> bool {
    registration.required_modifiers == MOD_WIN
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::MOD_WIN;

    use crate::control::WatchCommand;

    use super::{NativeHotkeyRegistration, should_force_low_level_hook};

    #[test]
    fn pure_win_bindings_always_use_low_level_runtime() {
        let registration = NativeHotkeyRegistration {
            trigger: "Win+H".to_string(),
            command: WatchCommand::FocusPrev,
            register_modifiers: MOD_WIN,
            required_modifiers: MOD_WIN,
            key: u32::from(b'H'),
        };

        assert!(should_force_low_level_hook(&registration));
    }
}
