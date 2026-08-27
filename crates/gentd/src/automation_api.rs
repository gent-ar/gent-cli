use gent_protocol::AutomationFrame;
use gent_runtime::{AutomationResult, AutomationService};
use gent_store::SqliteLedger;

pub(crate) fn exchange(
    service: &AutomationService<SqliteLedger>,
    frame: AutomationFrame,
) -> Result<AutomationFrame, String> {
    match frame {
        AutomationFrame::ListRequest {
            request_id,
            workspace_id,
        } => match service
            .list(&workspace_id)
            .map_err(|error| error.to_string())?
        {
            AutomationResult::Definitions(automations) => Ok(AutomationFrame::List {
                request_id,
                workspace_id,
                automations,
            }),
            AutomationResult::DeniedObserver => {
                Err("automations are unavailable while gentd is observer-disabled".into())
            }
            _ => Err("automation catalog returned an invalid result".into()),
        },
        AutomationFrame::CreateRequest {
            request_id,
            definition,
        } => match service
            .create(definition)
            .map_err(|error| error.to_string())?
        {
            AutomationResult::Definition(definition) => Ok(AutomationFrame::Created {
                request_id,
                definition,
            }),
            AutomationResult::DeniedObserver => {
                Err("automations are unavailable while gentd is observer-disabled".into())
            }
            _ => Err("automation create returned an invalid result".into()),
        },
        AutomationFrame::RunsRequest {
            request_id,
            automation_id,
            limit,
        } => match service
            .runs(&automation_id, limit)
            .map_err(|error| error.to_string())?
        {
            AutomationResult::Runs(runs) => Ok(AutomationFrame::Runs {
                request_id,
                automation_id,
                runs,
            }),
            AutomationResult::DeniedObserver => {
                Err("automations are unavailable while gentd is observer-disabled".into())
            }
            _ => Err("automation run list returned an invalid result".into()),
        },
        AutomationFrame::List { .. }
        | AutomationFrame::Created { .. }
        | AutomationFrame::RunAccepted { .. }
        | AutomationFrame::Runs { .. } => Err("automation response frames are server-only".into()),
        AutomationFrame::RunRequest { .. } => {
            Err("automation run requires the composed chat lifecycle".into())
        }
    }
}
