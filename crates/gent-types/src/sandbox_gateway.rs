use std::net::{Ipv4Addr, SocketAddrV4};

use crate::SandboxLaunchContractError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxGatewayApi {
    Claude,
    Codex,
    Claurst,
}

#[derive(Clone, Eq, PartialEq)]
pub struct SandboxProviderGateway {
    api: SandboxGatewayApi,
    endpoint: SocketAddrV4,
    scoped_token: String,
    policy_digest_sha256: String,
    peer_nonce: String,
}

impl std::fmt::Debug for SandboxProviderGateway {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SandboxProviderGateway")
            .field("api", &self.api)
            .field("endpoint", &self.endpoint)
            .field("policy_digest_sha256", &self.policy_digest_sha256)
            .finish_non_exhaustive()
    }
}

impl SandboxProviderGateway {
    pub fn new(
        api: SandboxGatewayApi,
        endpoint: SocketAddrV4,
        scoped_token: String,
        policy_digest_sha256: String,
    ) -> Result<Self, SandboxLaunchContractError> {
        let valid_digest = |value: &str| {
            value.len() == 64
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        };
        if endpoint.ip() != &Ipv4Addr::LOCALHOST
            || endpoint.port() == 0
            || !valid_digest(&scoped_token)
            || !valid_digest(&policy_digest_sha256)
        {
            return Err(SandboxLaunchContractError::InvalidEgressPolicy);
        }
        Ok(Self {
            api,
            endpoint,
            scoped_token,
            policy_digest_sha256,
            peer_nonce: uuid::Uuid::new_v4().simple().to_string(),
        })
    }

    pub const fn api(&self) -> SandboxGatewayApi {
        self.api
    }
    pub const fn endpoint(&self) -> SocketAddrV4 {
        self.endpoint
    }
    pub fn scoped_token(&self) -> &str {
        &self.scoped_token
    }
    pub fn policy_digest_sha256(&self) -> &str {
        &self.policy_digest_sha256
    }
    pub fn peer_nonce(&self) -> &str {
        &self.peer_nonce
    }
    pub fn origin(&self) -> String {
        format!("http://{}", self.endpoint)
    }
}
