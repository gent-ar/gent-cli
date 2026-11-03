use super::{
    Admission, Bridge, Call, Decision, Error, MAX_MESSAGE, Ordering, PREFIX, Reply, Route, Value,
};
impl Bridge {
    fn bound_route<'a>(
        &'a self,
        authorization: &str,
        origin: Option<&str>,
        path: &'a str,
    ) -> Result<(&'a str, &'a Route), Error> {
        let expected = format!("Bearer {}", self.token);
        if authorization.len() != expected.len()
            || authorization
                .bytes()
                .zip(expected.bytes())
                .fold(0u8, |v, (a, b)| v | (a ^ b))
                != 0
        {
            return Err(Error::Authentication);
        }
        if origin.is_some() {
            return Err(Error::Authentication);
        }
        if self.revoked.load(Ordering::Acquire) || !(self.current)() {
            return Err(Error::Revoked);
        }
        let id = path.strip_prefix(PREFIX).ok_or(Error::Route)?;
        let route = self.routes.get(id).ok_or(Error::Route)?;
        if route.poisoned.load(Ordering::Acquire) {
            return Err(Error::Unprovable);
        }
        Ok((id, route))
    }

    fn authorize(
        &self,
        key: &str,
        scope: Option<&gent_ports::HostMcpScope>,
        route: &Route,
        operation: &str,
        message: &Value,
    ) -> Result<(), Error> {
        let tool = if operation == "tools/call" {
            Some(
                message["params"]["name"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 160)
                    .ok_or(Error::Protocol)?,
            )
        } else {
            None
        };
        match self.authorizer.decide(&Call {
            request_key: key,
            scope,
            server: &route.name,
            tool,
            params: &message["params"],
        }) {
            Decision::Allow => Ok(()),
            Decision::Deny => Err(Error::Denied),
            Decision::Ask => Err(Error::Approval),
            Decision::NotReady => Err(Error::NotReady),
        }
    }

    pub fn dispatch(
        &self,
        authorization: &str,
        origin: Option<&str>,
        path: &str,
        method: &str,
        body: &[u8],
    ) -> Result<Reply, Error> {
        let (id, route) = self.bound_route(authorization, origin, path)?;
        if method == "GET" {
            return Ok(Reply::MethodNotAllowed);
        }
        if method != "POST" || body.len() > MAX_MESSAGE {
            return Err(Error::Protocol);
        }
        let message: Value = serde_json::from_slice(body).map_err(|_| Error::Protocol)?;
        if message["jsonrpc"] != "2.0" || !message.is_object() {
            return Err(Error::Protocol);
        }
        let operation = message["method"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 160)
            .ok_or(Error::Protocol)?;
        let scope = self.journal.capture_scope()?;
        if operation == "notifications/initialized" && message.get("id").is_none() {
            let ticket = self
                .operation
                .as_ref()
                .map(|acquire| acquire())
                .transpose()?;
            route
                .peer
                .notify_bound(&message, self.guard(scope), ticket)?;
            return Ok(Reply::Accepted);
        }
        if operation == "notifications/cancelled" && message.get("id").is_none() {
            let cancelled = rpc_id(&message["params"]["requestId"])?;
            if !route
                .pending
                .lock()
                .map_err(|_| Error::Unprovable)?
                .contains(&cancelled)
            {
                return Err(Error::Protocol);
            }
            let ticket = self
                .operation
                .as_ref()
                .map(|acquire| acquire())
                .transpose()?;
            route
                .peer
                .notify_bound(&message, self.guard(scope), ticket)?;
            return Ok(Reply::Accepted);
        }
        let request_id = rpc_id(message.get("id").ok_or(Error::Protocol)?)?;
        if !matches!(
            operation,
            "initialize" | "ping" | "tools/list" | "tools/call"
        ) {
            return Ok(Reply::Json(
                serde_json::json!({"jsonrpc":"2.0","id":message["id"],"error":{"code":-32601,"message":"This host MCP transport supports tools only; the requested protocol feature is unavailable."}}),
            ));
        }
        let key = format!("{id}:{request_id}");
        let forwarded = super::super::mcp_subset::forwarded(&message)?;
        let metadata = matches!(operation, "initialize" | "ping" | "tools/list");
        if !metadata {
            self.authorize(&key, scope.as_ref(), route, operation, &message)?;
        }
        let canonical = serde_json::to_vec(&message).map_err(|_| Error::Protocol)?;
        let ticket = self
            .operation
            .as_ref()
            .map(|acquire| acquire())
            .transpose()?;
        match self
            .journal
            .reserve_bound(&key, &canonical, scope.as_ref())?
        {
            Admission::Cached(value) => return Ok(Reply::Json(value)),
            Admission::Unknown => return Err(Error::Unprovable),
            Admission::Fresh => (),
        }
        {
            let mut pending = route.pending.lock().map_err(|_| Error::Unprovable)?;
            if pending.len() >= 8 || pending.contains(&request_id) {
                self.journal.unknown(&key);
                return Err(Error::Busy);
            }
            pending.insert(request_id.clone());
        }
        if self.revoked.load(Ordering::Acquire)
            || !(self.current)()
            || !self.journal.current_scope(scope.as_ref())
        {
            self.journal.unknown(&key);
            if let Ok(mut pending) = route.pending.lock() {
                pending.remove(&request_id);
            }
            return Err(Error::Revoked);
        }
        let guard = self.guard(scope);
        let result = route
            .peer
            .request_bound(&forwarded, guard)
            .and_then(|reply| {
                if serde_json::to_vec(&reply)
                    .map_err(|_| Error::Protocol)?
                    .len()
                    > MAX_MESSAGE
                    || reply["jsonrpc"] != "2.0"
                    || reply.get("id") != message.get("id")
                    || (reply.get("result").is_some() == reply.get("error").is_some())
                {
                    return Err(Error::Protocol);
                }
                let reply = super::super::mcp_subset::response(operation, reply)?;
                self.journal.complete(&key, &reply)?;
                Ok(reply)
            });
        if let Ok(mut pending) = route.pending.lock() {
            pending.remove(&request_id);
        }
        if let Ok(reply) = result {
            Ok(Reply::Json(reply))
        } else {
            self.journal.unknown(&key);
            if let Some(ticket) = ticket {
                route.peer.retain_until_settled(ticket);
            }
            route.poisoned.store(true, Ordering::Release);
            route.peer.revoke();
            Err(Error::Unprovable)
        }
    }
}

fn rpc_id(value: &Value) -> Result<String, Error> {
    if value.as_i64().is_some()
        || value
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 128)
    {
        serde_json::to_string(value).map_err(|_| Error::Protocol)
    } else {
        Err(Error::Protocol)
    }
}
