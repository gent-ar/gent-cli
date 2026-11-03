//! Linux-only adapter: actual retained cgroup settlement precedes pipe joins.
//! This does not create a model permission grant or synthesize worker evidence.
use super::{mcp::Error, mcp_stdio::Spec};
use gent_linux_sandbox::host_stdio::{Config, HostStdioChild};
use process_wrap::std::ChildWrapper;
use std::{
    io,
    process::{ChildStderr, ChildStdin, ChildStdout, ExitStatus},
    sync::{Arc, Mutex},
};
pub(super) struct RetainedAuthority {
    _spec: Arc<Spec>,
    startup: Mutex<Option<super::mcp::Ticket>>,
}
impl RetainedAuthority {
    pub(super) fn new(spec: Arc<Spec>, startup: Option<super::mcp::Ticket>) -> Self {
        Self {
            _spec: spec,
            startup: Mutex::new(startup),
        }
    }
    pub(super) fn finish_startup(&self) {
        let ticket = self
            .startup
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(ticket);
    }
}
pub(super) fn validate(spec: &Spec, config: &Config) -> Result<(), Error> {
    if spec.startup.is_none() {
        return Err(Error::Unprovable);
    }
    let cwd = spec.cwd.canonicalize().map_err(|_| Error::Unprovable)?;
    let authority = spec
        .authority
        .path()
        .canonicalize()
        .map_err(|_| Error::Unprovable)?;
    if cwd != authority
        || config.cwd != cwd
        || !gent_linux_sandbox::host_stdio::private_owner_directory(&authority)
        || !["gent-mcp", "node-mcp"].contains(&config.program.provider.as_str())
        || !gent_linux_sandbox::host_stdio::validate_profile(&spec.program, &spec.args, &spec.env)
        || spec.program.to_str() != Some(config.program.canonical_path.as_str())
        || config.args != spec.args
        || config.env != spec.env
        || config.lifetime < spec.timeout
    {
        return Err(Error::Denied);
    }
    Ok(())
}
#[derive(Debug)]
struct NativeChild {
    child: HostStdioChild,
}
fn failure(_: &'static str) -> io::Error {
    io::Error::other("native MCP cgroup owner unavailable or settlement unknown")
}
pub(super) fn spawn(
    config: Config,
    authority: Arc<RetainedAuthority>,
) -> Result<Box<dyn ChildWrapper>, Error> {
    HostStdioChild::spawn_retained(config, authority)
        .map(|child| Box::new(NativeChild { child }) as Box<dyn ChildWrapper>)
        .map_err(|_| Error::Unprovable)
}
impl ChildWrapper for NativeChild {
    fn signal(&self, _: i32) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "native MCP signals require whole-cgroup termination",
        ))
    }
    fn wait_with_output(self: Box<Self>) -> io::Result<std::process::Output> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "native MCP output must use bounded framed actor",
        ))
    }
    fn inner(&self) -> &dyn ChildWrapper {
        self
    }
    fn inner_mut(&mut self) -> &mut dyn ChildWrapper {
        self
    }
    fn into_inner(self: Box<Self>) -> Box<dyn ChildWrapper> {
        self
    }
    fn stdin(&mut self) -> &mut Option<ChildStdin> {
        self.child.stdin()
    }
    fn stdout(&mut self) -> &mut Option<ChildStdout> {
        self.child.stdout()
    }
    fn stderr(&mut self) -> &mut Option<ChildStderr> {
        self.child.stderr()
    }
    fn id(&self) -> u32 {
        self.child.id()
    }
    fn start_kill(&mut self) -> io::Result<()> {
        self.child.terminate().map_err(failure)
    }
    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait().map_err(failure)
    }
    fn wait(&mut self) -> io::Result<ExitStatus> {
        self.child.wait().map_err(failure)
    }
}
