//! Public fakes and safe recorded-fixture helpers for Gent contract tests.
//!
//! This crate deliberately contains no production process implementation and no
//! provider credentials. Its fixture loader rejects unredacted secrets before a
//! transcript can become test evidence.

mod fake_bridge;
mod fake_process;
mod transcript;

pub use fake_bridge::{BridgeSubmission, FakeExternalProviderBridge};
pub use fake_process::{FakeProcess, FakeProcessSignal};
pub use transcript::{
    PublicDriverFixture, PublicDriverFrame, TranscriptError, load_public_driver_fixture,
    load_public_driver_fixtures,
};
