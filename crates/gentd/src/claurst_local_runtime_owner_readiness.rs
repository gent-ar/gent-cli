use std::{
    io::Write,
    net::{TcpStream, ToSocketAddrs},
    time::{Duration, Instant},
};

const DEFAULT_READINESS_TIMEOUT: Duration = Duration::from_secs(300);

pub(crate) trait LlamaServerReadiness {
    fn wait_ready<P: super::LocalRuntimeProcess>(
        &self,
        server_url: &str,
        process: &mut P,
    ) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct HttpLlamaServerReadiness {
    retry_delay: Duration,
    timeout: Duration,
}

impl HttpLlamaServerReadiness {
    #[must_use]
    pub(crate) const fn new(retry_delay: Duration) -> Self {
        Self {
            retry_delay,
            timeout: DEFAULT_READINESS_TIMEOUT,
        }
    }

    #[cfg(test)]
    const fn with_timeout(retry_delay: Duration, timeout: Duration) -> Self {
        Self {
            retry_delay,
            timeout,
        }
    }
}
impl Default for HttpLlamaServerReadiness {
    fn default() -> Self {
        Self::new(Duration::from_millis(200))
    }
}
impl LlamaServerReadiness for HttpLlamaServerReadiness {
    fn wait_ready<P: super::LocalRuntimeProcess>(
        &self,
        server_url: &str,
        process: &mut P,
    ) -> Result<(), String> {
        let address = server_url
            .strip_prefix("http://")
            .ok_or_else(|| "local llama.cpp server URL must use http".to_owned())?;
        if address.contains('/') {
            return Err("local llama.cpp server URL must not include a path".into());
        }
        let deadline = Instant::now() + self.timeout;
        loop {
            if health_check(address).is_ok() {
                return Ok(());
            }
            if let Some(error) = process.exited()? {
                return Err(error);
            }
            if Instant::now() >= deadline {
                return Err("local llama.cpp server readiness timed out".into());
            }
            std::thread::sleep(self.retry_delay);
        }
    }
}
fn health_check(address: &str) -> Result<(), String> {
    let socket = address
        .to_socket_addrs()
        .map_err(|error| error.to_string())?
        .next()
        .ok_or_else(|| "local llama.cpp server address is empty".to_owned())?;
    let mut stream = TcpStream::connect_timeout(&socket, Duration::from_millis(500))
        .map_err(|error| error.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_millis(500)))
        .map_err(|error| error.to_string())?;
    stream
        .write_all(b"GET /health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .map_err(|error| error.to_string())?;
    let mut response = [0_u8; 64];
    let count =
        std::io::Read::read(&mut stream, &mut response).map_err(|error| error.to_string())?;
    let response = std::str::from_utf8(&response[..count]).map_err(|error| error.to_string())?;
    response
        .starts_with("HTTP/1.1 2")
        .then_some(())
        .ok_or_else(|| "local llama.cpp health endpoint was not successful".into())
}

#[cfg(test)]
mod tests {
    use super::{HttpLlamaServerReadiness, LlamaServerReadiness};
    use crate::claurst_local_runtime_owner::LocalRuntimeProcess;
    use std::time::Duration;

    struct Running;

    impl LocalRuntimeProcess for Running {
        fn exited(&mut self) -> Result<Option<String>, String> {
            Ok(None)
        }

        fn shutdown(&mut self) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn readiness_has_a_bounded_wait() {
        let mut process = Running;
        let error = HttpLlamaServerReadiness::with_timeout(
            Duration::from_millis(1),
            Duration::from_millis(5),
        )
        .wait_ready("http://127.0.0.1:1", &mut process)
        .unwrap_err();
        assert_eq!(error, "local llama.cpp server readiness timed out");
    }
}
