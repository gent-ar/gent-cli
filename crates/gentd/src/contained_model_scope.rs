use super::ModelError;
use gent_drivers::{ProviderProcess, interrupt::ProcessTreeSignal};
use gent_types::SandboxedLaunchRequest;
use std::{
    fs::File,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};
pub(crate) struct ModelResources {
    pub pins: Vec<File>,
    pub root: tempfile::TempDir,
    pub lifetime: Vec<Box<dyn Send>>,
}
struct Retained {
    _process: Option<Arc<dyn ProviderProcess>>,
    _resources: ModelResources,
}
fn retained() -> &'static Mutex<Vec<Retained>> {
    static VALUE: OnceLock<Mutex<Vec<Retained>>> = OnceLock::new();
    VALUE.get_or_init(Default::default)
}
pub(super) fn retain_launch_error(resources: ModelResources) {
    retained()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(Retained {
            _process: None,
            _resources: resources,
        });
}
pub(super) struct Scope {
    pub process: Arc<dyn ProviderProcess>,
    pub request: SandboxedLaunchRequest,
    pub run: String,
    pub model_digest: String,
    pub model_id: String,
    fence: Arc<dyn Fn() -> bool + Send + Sync>,
    resources: Mutex<Option<ModelResources>>,
    gate: Mutex<bool>,
    revoked: AtomicBool,
    retiring: AtomicBool,
    known: AtomicBool,
}
impl Scope {
    pub fn new(
        process: Arc<dyn ProviderProcess>,
        request: SandboxedLaunchRequest,
        run: String,
        model_digest: String,
        model_id: String,
        fence: Arc<dyn Fn() -> bool + Send + Sync>,
        resources: ModelResources,
    ) -> Arc<Self> {
        Arc::new(Self {
            process,
            request,
            run,
            model_digest,
            model_id,
            fence,
            resources: Mutex::new(Some(resources)),
            gate: Mutex::new(false),
            revoked: AtomicBool::new(false),
            retiring: AtomicBool::new(false),
            known: AtomicBool::new(false),
        })
    }
    pub fn current(&self) -> bool {
        !self.revoked.load(Ordering::Acquire)
            && (self.fence)()
            && matches!(self.process.try_exit_code(), Ok(None))
            && self
                .process
                .sandbox_evidence()
                .is_some_and(|p| p.matches(&self.request, &self.run))
    }
    pub fn write(&self, bytes: &[u8]) -> Result<(), ModelError> {
        {
            let mut writing = self
                .gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if self.revoked.load(Ordering::Acquire) || *writing || !self.current() {
                return Err(ModelError::Revoked);
            }
            *writing = true;
        }
        let result = self
            .process
            .write_frame(bytes)
            .map_err(|_| ModelError::Transport);
        *self
            .gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = false;
        result
    }
    pub fn retire(&self) -> Result<(), ModelError> {
        if self.retiring.swap(true, Ordering::AcqRel) {
            return if self.known.load(Ordering::Acquire) {
                Ok(())
            } else {
                Err(ModelError::Unknown)
            };
        }
        let writing = {
            let writing = self
                .gate
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.revoked.store(true, Ordering::Release);
            *writing
        };
        let signalled = self
            .process
            .signal_tree(if writing {
                ProcessTreeSignal::Kill
            } else {
                ProcessTreeSignal::Terminate
            })
            .is_ok();
        let known = signalled && matches!(self.process.try_exit_code(), Ok(Some(Some(0 | 125))));
        self.known.store(known, Ordering::Release);
        if known {
            Ok(())
        } else {
            Err(ModelError::Unknown)
        }
    }
    #[cfg(test)]
    pub fn root_path(&self) -> std::path::PathBuf {
        self.resources
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()
            .unwrap()
            .root
            .path()
            .into()
    }
    pub fn settled(&self) -> bool {
        self.known.load(Ordering::Acquire)
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        if self.retire().is_err() {
            if let Some(resources) = self
                .resources
                .get_mut()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                retained()
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(Retained {
                        _process: Some(self.process.clone()),
                        _resources: resources,
                    });
            }
        }
    }
}

#[cfg(test)]
#[path = "contained_model_scope_tests.rs"]
mod tests;
