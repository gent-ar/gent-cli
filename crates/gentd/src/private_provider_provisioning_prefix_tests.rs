use super::*;

#[derive(Clone)]
struct Unsupported;

impl DependencyInstaller for Unsupported {
    fn install(
        &self,
        _: &NpmGlobalPrefix,
        _: &ApprovedPackageInstall,
    ) -> Result<(), InstallerError> {
        Err(InstallerError::UnsupportedPrefix)
    }
}

#[test]
fn unsupported_prefix_is_pre_effect_while_real_installer_failures_remain_uncertain() {
    let (_temp, runtime) = runtime();
    let provisioner = PrivateProviderProvisioner::new(
        runtime,
        Unsupported,
        GoodPolicy,
        Some(NeverVerifier),
        TestAcceptedReceiptReader,
    );
    let error = provisioner.provision(&request()).unwrap_err();
    assert!(matches!(error, PrivateProvisionError::UnsupportedPrefix));
    assert!(error.is_definitely_pre_effect());
    assert!(
        !PrivateProvisionError::Installer("npm may already have changed files".into())
            .is_definitely_pre_effect()
    );
}
