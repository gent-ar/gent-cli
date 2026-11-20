const REASONING_CLOSE: &str = "</think>";

#[must_use]
pub fn answer(text: &str) -> &str {
    text.rsplit_once(REASONING_CLOSE)
        .map_or(text, |(_, answer)| answer.trim_start())
}

#[cfg(test)]
mod tests {
    use super::answer;

    #[test]
    fn only_what_follows_the_last_reasoning_close_tag_is_the_answer() {
        assert_eq!(answer("</think>\n\nThe file is fine."), "The file is fine.");
        assert_eq!(answer("<think>\nplan\n</think>\n\nDone."), "Done.");
        assert_eq!(answer("</think>"), "");
        assert_eq!(answer("  plain text"), "  plain text");
    }
}
