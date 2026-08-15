//! Public fakes and safe recorded-fixture helpers for Gent contract tests.
//!
//! This crate deliberately contains no production process implementation and no
//! provider credentials. Its fixture loader rejects unredacted secrets before a
//! transcript can become test evidence.

mod evidence_manifest;
mod fake_bridge;
mod fake_legacy_event_tap;
mod fake_process;
mod transcript;
mod transcript_catalog;
mod transcript_fixture;
mod transcript_manifest;
mod transcript_provenance;

pub use evidence_manifest::validate_evidence_manifest;
pub use fake_bridge::{BridgeSubmission, FakeExternalProviderBridge};
pub use fake_legacy_event_tap::FakeLegacyEventTap;
pub use fake_process::{FakeProcess, FakeProcessSignal};
pub use transcript::{
    PublicDriverFixture, PublicDriverFrame, TranscriptError, load_public_driver_fixture,
    load_public_driver_fixtures,
};
pub use transcript_catalog::{PUBLIC_PROVIDERS, REQUIRED_SCENARIOS};
pub use transcript_manifest::validate_public_driver_manifest;
