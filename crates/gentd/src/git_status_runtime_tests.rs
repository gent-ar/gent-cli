use gent_ports::{GitExecutor, GitExecutorError, GitStatusOperation, GitStatusSummary};

use super::git_status_authority::{
    GitStatusApproval, GitStatusAuthorityError, GitStatusAuthorityRequest,
    ValidatedGitStatusAuthority, validate_git_status_authority,
};
use super::{GitStatusRuntimeError, PreparedGitStatusRuntime};

#[derive(Debug)]
struct NeverGit;

impl GitExecutor for NeverGit {
    fn status(&self, _: &GitStatusOperation) -> Result<GitStatusSummary, GitExecutorError> {
        panic!("construction must not invoke Git")
    }
}

#[test]
fn observer_authority_cannot_retain_an_injected_git_executor() {
    let result =
        PreparedGitStatusRuntime::new(&ValidatedGitStatusAuthority::Observer, (), NeverGit);
    assert_eq!(
        result.unwrap_err(),
        GitStatusRuntimeError::StatusAuthorityUnavailable
    );
}

#[test]
fn status_authority_requires_evidence_and_is_explicitly_read_only() {
    let missing =
        validate_git_status_authority(GitStatusAuthorityRequest::Approved(GitStatusApproval {
            evidence_reference: " ".into(),
        }));
    assert_eq!(
        missing,
        Err(GitStatusAuthorityError::MissingEvidenceReference)
    );
    let authority =
        validate_git_status_authority(GitStatusAuthorityRequest::Approved(GitStatusApproval {
            evidence_reference: "git-status-review-2026-08-16".into(),
        }))
        .unwrap();
    let runtime = PreparedGitStatusRuntime::new(&authority, (), NeverGit).unwrap();
    let _service = runtime.service();
}
