use ratatui::{
    style::{Color, Modifier, Style},
    text::Line,
};

pub(super) fn permission_lines(
    request: &gent_types::PermissionDecisionRequest,
) -> Vec<Line<'static>> {
    let mut lines = vec![Line::styled(
        format!(
            "Permission required · {} · {:?} · /approve once · /approve-tool · /approve-category · /deny",
            request.request.tool_name, request.request.category
        ),
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    )];
    if let Some(question) = question_summary(request.request.input.as_ref()) {
        lines.push(Line::styled(
            format!("Question · {question} · /answer {{...}}"),
            Style::default().fg(Color::Cyan),
        ));
    }
    lines
}

pub(super) fn install_hold_lines(hold: &crate::prompt_hold::InstallHold) -> Vec<Line<'static>> {
    crate::prompt_hold::install_lines(hold, "gent")
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let style = if index == 0 {
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Cyan)
            };
            Line::styled(line.trim_start_matches("! ").to_owned(), style)
        })
        .collect()
}

fn question_summary(input: Option<&serde_json::Value>) -> Option<String> {
    let questions = input?.get("questions")?.as_array()?;
    let values = questions
        .iter()
        .filter_map(|question| question.get("question").and_then(serde_json::Value::as_str))
        .take(2)
        .collect::<Vec<_>>();
    (!values.is_empty()).then(|| values.join(" · "))
}
