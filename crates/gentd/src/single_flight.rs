use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, PoisonError},
};

#[derive(Debug)]
pub(crate) struct SingleFlight<T> {
    flights: Mutex<HashMap<String, Arc<OnceLock<T>>>>,
}

impl<T: Clone> SingleFlight<T> {
    pub(crate) fn new() -> Self {
        Self {
            flights: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn run(&self, key: &str, work: impl FnOnce() -> T) -> T {
        let flight = Arc::clone(self.lock().entry(key.to_owned()).or_default());
        let value = flight.get_or_init(work).clone();
        let mut flights = self.lock();
        if flights
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, &flight))
        {
            flights.remove(key);
        }
        value
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<OnceLock<T>>>> {
        self.flights.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
#[path = "single_flight_tests.rs"]
mod tests;
