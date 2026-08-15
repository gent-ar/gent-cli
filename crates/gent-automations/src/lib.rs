//! Automation boundary. Schedulers are hard-disabled until their own authority gate.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SchedulerMode {
    Disabled,
}
