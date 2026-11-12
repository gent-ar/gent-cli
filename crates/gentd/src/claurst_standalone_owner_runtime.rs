use std::sync::{Arc, Mutex};

use crate::{
    claurst_acp_bridge::ClaurstAcpBridge, claurst_acp_transport::ClaurstAcpStdio,
    claurst_local_runtime::LocalProcessLaunch, claurst_local_runtime_owner::LocalRuntimeProcess,
    claurst_runtime_factory::LlamaContextSummarizer,
};

pub(crate) struct ClaurstStandaloneModel<L> {
    pub(super) llama: Arc<Mutex<L>>,
    pub(super) summarizer: Arc<LlamaContextSummarizer>,
    pub(super) port: u16,
    pub(super) llama_launch: LocalProcessLaunch,
}

impl<L> Clone for ClaurstStandaloneModel<L> {
    fn clone(&self) -> Self {
        Self {
            llama: Arc::clone(&self.llama),
            summarizer: Arc::clone(&self.summarizer),
            port: self.port,
            llama_launch: self.llama_launch.clone(),
        }
    }
}

pub(crate) struct ClaurstStandaloneRuntime<L, S> {
    pub(super) model: ClaurstStandaloneModel<L>,
    pub(super) bridge: Arc<ClaurstAcpBridge<S>>,
}

impl<L, S> ClaurstStandaloneRuntime<L, S> {
    #[must_use]
    pub(crate) fn bridge(&self) -> Arc<ClaurstAcpBridge<S>> {
        Arc::clone(&self.bridge)
    }

    #[must_use]
    pub(crate) fn summarizer(&self) -> Arc<LlamaContextSummarizer> {
        Arc::clone(&self.model.summarizer)
    }

    #[must_use]
    pub(crate) fn model(&self) -> ClaurstStandaloneModel<L> {
        self.model.clone()
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn port(&self) -> u16 {
        self.model.port
    }

    #[cfg(test)]
    pub(crate) fn llama(&self) -> std::sync::MutexGuard<'_, L> {
        self.model.llama.lock().unwrap()
    }
}

impl<L: LocalRuntimeProcess, S: ClaurstAcpStdio> ClaurstStandaloneRuntime<L, S> {
    pub(crate) fn exited(&mut self) -> Result<Option<String>, String> {
        if let Some(exit) = llama_lock(&self.model.llama)?.exited()? {
            return Ok(Some(exit));
        }
        self.bridge.exited()
    }

    pub(crate) fn shutdown(self) -> Result<(), String> {
        drop(self.bridge);
        match Arc::try_unwrap(self.model.llama) {
            Ok(llama) => llama
                .into_inner()
                .map_err(|_| "local llama.cpp lock is unavailable".to_owned())?
                .shutdown(),
            Err(_) => Ok(()),
        }
    }
}

fn llama_lock<L>(llama: &Mutex<L>) -> Result<std::sync::MutexGuard<'_, L>, String> {
    llama
        .lock()
        .map_err(|_| "local llama.cpp lock is unavailable".to_owned())
}
