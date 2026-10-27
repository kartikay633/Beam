fn version_line() -> String {
    format!(
        "beam-cli {} (beam-core {})",
        env!("CARGO_PKG_VERSION"),
        beam_core::VERSION
    )
}

fn main() {
    println!("{}", version_line());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_line_names_both_crates() {
        let line = version_line();
        assert!(line.starts_with("beam-cli "));
        assert!(line.contains("beam-core "));
    }
}
