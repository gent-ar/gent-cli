use ratatui::{
    style::{Color, Style},
    text::Line,
};

const DIFF_ROWS: usize = 12;
const MAX_LINES: usize = 400;

pub(super) fn lines(old: &str, new: &str) -> Vec<(char, String)> {
    let old = old.lines().collect::<Vec<_>>();
    let new = new.lines().collect::<Vec<_>>();
    if old.len() > MAX_LINES || new.len() > MAX_LINES {
        return old
            .iter()
            .map(|line| ('-', (*line).to_owned()))
            .chain(new.iter().map(|line| ('+', (*line).to_owned())))
            .collect();
    }
    let mut table = vec![vec![0u16; new.len() + 1]; old.len() + 1];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            table[i][j] = if old[i] == new[j] {
                table[i + 1][j + 1] + 1
            } else {
                table[i + 1][j].max(table[i][j + 1])
            };
        }
    }
    let (mut i, mut j) = (0, 0);
    let mut changes = Vec::new();
    while i < old.len() || j < new.len() {
        if i < old.len() && j < new.len() && old[i] == new[j] {
            changes.push((' ', old[i].to_owned()));
            i += 1;
            j += 1;
        } else if i < old.len() && (j == new.len() || table[i + 1][j] >= table[i][j + 1]) {
            changes.push(('-', old[i].to_owned()));
            i += 1;
        } else {
            changes.push(('+', new[j].to_owned()));
            j += 1;
        }
    }
    trim_context(changes)
}

pub(super) fn patch(text: &str) -> Vec<(char, String)> {
    let changes = text
        .lines()
        .filter(|line| {
            !line.starts_with("@@") && !line.starts_with("+++") && !line.starts_with("---")
        })
        .filter_map(|line| {
            let mut chars = line.chars();
            match chars.next() {
                Some(sign @ ('+' | '-' | ' ')) => Some((sign, chars.as_str().to_owned())),
                _ => None,
            }
        })
        .collect();
    trim_context(changes)
}

pub(super) fn counts(changes: &[(char, String)]) -> (usize, usize) {
    (
        changes.iter().filter(|(sign, _)| *sign == '+').count(),
        changes.iter().filter(|(sign, _)| *sign == '-').count(),
    )
}

fn trim_context(changes: Vec<(char, String)>) -> Vec<(char, String)> {
    let first = changes.iter().position(|(sign, _)| *sign != ' ');
    let last = changes.iter().rposition(|(sign, _)| *sign != ' ');
    match (first, last) {
        (Some(first), Some(last)) => {
            changes[first.saturating_sub(1)..=(last + 1).min(changes.len() - 1)].to_vec()
        }
        _ => Vec::new(),
    }
}

pub(super) fn rendered(changes: &[(char, String)]) -> Vec<Line<'static>> {
    let mut lines = changes
        .iter()
        .take(DIFF_ROWS)
        .map(|(sign, text)| {
            let color = match sign {
                '+' => Color::Green,
                '-' => Color::Red,
                _ => Color::DarkGray,
            };
            Line::styled(format!("    {sign} {text}"), Style::default().fg(color))
        })
        .collect::<Vec<_>>();
    if changes.len() > DIFF_ROWS {
        lines.push(Line::styled(
            format!("    … {} more lines", changes.len() - DIFF_ROWS),
            Style::default().fg(Color::DarkGray),
        ));
    }
    lines
}
