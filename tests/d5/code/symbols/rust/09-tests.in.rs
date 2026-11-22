pub fn parse(text: &str) -> usize {
    text.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_text() {
        assert_eq!(parse("ab"), 2);
    }
}
