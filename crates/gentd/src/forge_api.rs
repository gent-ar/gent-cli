use gent_protocol::ForgeConnectorFrame;
use gent_runtime::Coordinator;
use gent_store::SqliteLedger;

pub(crate) fn exchange(
    coordinator: &Coordinator<SqliteLedger>,
    frame: ForgeConnectorFrame,
) -> Result<ForgeConnectorFrame, String> {
    match frame {
        ForgeConnectorFrame::ListRequest {
            request_id,
            workspace_id,
        } => Ok(ForgeConnectorFrame::List {
            request_id,
            connectors: coordinator
                .forge_connectors(&workspace_id)
                .map_err(|error| error.to_string())?,
            workspace_id,
        }),
        ForgeConnectorFrame::GetRequest {
            request_id,
            workspace_id,
            connector_id,
        } => Ok(ForgeConnectorFrame::Get {
            request_id,
            workspace_id: workspace_id.clone(),
            connector: coordinator
                .forge_connector(&connector_id)
                .map_err(|error| error.to_string())?
                .filter(|connector| connector.workspace_id == workspace_id),
        }),
        ForgeConnectorFrame::CreateRequest {
            request_id,
            connector,
        } => {
            coordinator
                .create_forge_connector(&connector)
                .map_err(|error| error.to_string())?;
            Ok(ForgeConnectorFrame::Created {
                request_id,
                connector,
            })
        }
        ForgeConnectorFrame::SetEnabledRequest {
            request_id,
            workspace_id,
            connector_id,
            enabled,
        } => Ok(ForgeConnectorFrame::SetEnabled {
            request_id,
            connector: coordinator
                .set_forge_connector_enabled(&workspace_id, &connector_id, enabled)
                .map_err(|error| error.to_string())?,
        }),
        ForgeConnectorFrame::InvokeRequest {
            request_id,
            workspace_id,
            connector_id,
            tool_name,
        } => {
            let invocation = coordinator
                .forge_invocation(&workspace_id, &connector_id, tool_name.as_deref())
                .map_err(|error| error.to_string())?;
            Ok(ForgeConnectorFrame::InvocationHandoff {
                request_id,
                workspace_id: invocation.workspace_id,
                connector_id: invocation.connector_id,
                tool_source_id: invocation.tool_source_id,
                tool_name: invocation.tool_name,
            })
        }
        ForgeConnectorFrame::List { .. } => Err("Forge response frames are server-only".into()),
        ForgeConnectorFrame::Get { .. }
        | ForgeConnectorFrame::Created { .. }
        | ForgeConnectorFrame::SetEnabled { .. }
        | ForgeConnectorFrame::InvocationHandoff { .. } => {
            Err("Forge response frames are server-only".into())
        }
    }
}
