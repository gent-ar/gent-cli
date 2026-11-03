//! MCP HTTP adapter on the already authenticated provider gateway.
use super::{
    mcp::{Bridge, Error, Reply},
    wire,
};
use std::io::Write;

pub(super) trait McpEndpoint: Send + Sync {
    fn respond(&self, request: wire::Request, stream: &mut dyn Write) -> Result<(), Error>;
    fn revoke(&self);
    fn settled(&self) -> bool;
}

#[cfg(test)]
#[path = "provider_mcp_http_tests.rs"]
mod tests;
impl McpEndpoint for Bridge {
    fn respond(&self, request: wire::Request, stream: &mut dyn Write) -> Result<(), Error> {
        let auth = request
            .headers
            .get("authorization")
            .ok_or(Error::Authentication)?;
        // The shared gateway parser has already checked Host, auth, unique
        // headers, framing and content type. Origin is still checked by MCP.
        let result = self.dispatch(
            auth,
            request.headers.get("origin").map(String::as_str),
            &request.path,
            &request.method,
            &request.body,
        );
        let (status, body) = match result {
            Ok(Reply::Accepted) => (202, Vec::new()),
            Ok(Reply::MethodNotAllowed) => (405, Vec::new()),
            Ok(Reply::Json(value)) => (
                200,
                serde_json::to_vec(&value).map_err(|_| Error::Protocol)?,
            ),
            Err(Error::Authentication) => (401, Vec::new()),
            Err(Error::Route) => (404, Vec::new()),
            Err(Error::Denied | Error::Approval) => (403, Vec::new()),
            Err(Error::Protocol) => (400, Vec::new()),
            Err(Error::Busy) => (429, Vec::new()),
            Err(_) => (503, Vec::new()),
        };
        // HTTP GET SSE is optional. Returning405 instead of holding seven idle
        // sockets avoids starving the bounded provider channels. Server-initiated
        // RPC needs a separate explicit capability before it can be advertised.
        let reason = match status {
            200 => "OK",
            202 => "Accepted",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            429 => "Too Many Requests",
            _ => "Service Unavailable",
        };
        write!(stream,"HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",body.len()).map_err(|_|Error::Unprovable)?;
        stream.write_all(&body).map_err(|_| Error::Unprovable)
    }
    fn revoke(&self) {
        Bridge::revoke(self)
    }
    fn settled(&self) -> bool {
        Bridge::settled(self)
    }
}
