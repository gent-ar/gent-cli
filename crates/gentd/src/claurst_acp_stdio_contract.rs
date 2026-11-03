pub(crate) trait ClaurstAcpStdio {
    /// Host-minted roster already installed in verified private SDK settings.
    fn fixed_mcp_roster_sha256(&self) -> Option<&str> {
        None
    }
    fn sandbox_evidence(&self) -> Option<&gent_drivers::ProcessSandboxEvidence> {
        None
    }
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String>;
    fn try_read_frame(&mut self, maximum_bytes: usize) -> Result<Option<Vec<u8>>, String>;
    fn exited(&mut self) -> Result<Option<String>, String> {
        Ok(None)
    }
}
