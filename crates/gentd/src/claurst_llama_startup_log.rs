use std::{
    io::Read,
    num::NonZeroU16,
    process::Command,
    sync::{Arc, Mutex},
};

pub(super) fn configure_ready_notification(command: &mut Command, arguments: &[String]) {
    if arguments.windows(2).any(|pair| pair == ["--port", "0"]) {
        for (key, _) in std::env::vars_os().filter(|(key, _)| {
            key.to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("LLAMA_ARG_")
        }) {
            command.env_remove(key);
        }
        command.args([
            "--log-verbosity",
            "3",
            "--log-colors",
            "off",
            "--log-prefix",
        ]);
    }
}

#[derive(Clone, Default)]
pub(super) struct LlamaStartupLog(Arc<Mutex<Startup>>);

#[derive(Default)]
struct Startup {
    port: Option<NonZeroU16>,
    failure: Option<&'static str>,
}

impl LlamaStartupLog {
    pub(super) fn capture(&self, mut stderr: impl Read) {
        let mut chunk = [0_u8; 4096];
        let mut line = Vec::with_capacity(256);
        let mut overlong = false;
        while let Ok(count) = stderr.read(&mut chunk) {
            if count == 0 {
                break;
            }
            for byte in &chunk[..count] {
                if *byte == b'\n' {
                    if !overlong {
                        self.observe(&line);
                    }
                    line.clear();
                    overlong = false;
                } else if line.len() < 1024 {
                    line.push(*byte);
                } else {
                    overlong = true;
                }
            }
        }
        if !overlong {
            self.observe(&line);
        }
    }

    fn observe(&self, line: &[u8]) {
        let Ok(line) = std::str::from_utf8(line) else {
            return;
        };
        let line = line.trim_end();
        if let Some(port) = line
            .rsplit_once("srv  llama_server: listening on http://127.0.0.1:")
            .and_then(|(_, port)| port.parse::<NonZeroU16>().ok())
            && let Ok(mut startup) = self.0.lock()
        {
            startup.port = Some(port);
        }
        let detail = if line.contains("out of memory") {
            "GPU or system memory allocation failed"
        } else if line.contains("couldn't bind HTTP server socket") {
            "the loopback listener could not bind"
        } else if line.contains("failed to allocate") {
            "model memory allocation failed"
        } else if line.contains("exiting due to model loading error") {
            "the local model could not be loaded"
        } else {
            return;
        };
        if let Ok(mut startup) = self.0.lock()
            && startup.failure.is_none()
            && startup.port.is_none()
        {
            startup.failure = Some(detail);
        }
    }

    pub(super) fn port(&self) -> Option<u16> {
        self.0.lock().ok()?.port.map(NonZeroU16::get)
    }

    pub(super) fn failure(&self) -> Option<&'static str> {
        self.0.lock().ok()?.failure
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_bound_loopback_port_and_only_retains_fixed_diagnostics() {
        let log = LlamaStartupLog::default();
        let stderr = b"0.14.667.369 E ggml_cuda: allocating: CUDA out of memory: private value\r\n0.17.882.746 I srv  llama_server: listening on http://127.0.0.1:56216\r\n";
        log.capture(&stderr[..]);
        assert_eq!(
            log.failure(),
            Some("GPU or system memory allocation failed")
        );
        assert_eq!(log.port(), Some(56216));
    }
}
