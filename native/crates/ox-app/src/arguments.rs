// SPDX-License-Identifier: AGPL-3.0-only
//! Preliminary command-line handling before `GApplication` parses the arguments.

/// Options and locations, respecting the standard `--` option terminator.
pub(crate) struct Arguments<'a> {
    options: Vec<&'a String>,
    pub(crate) locations: Vec<&'a String>,
}

impl<'a> Arguments<'a> {
    /// Splits the whole command line, excluding the program and first `--`.
    pub(crate) fn parse(arguments: &'a [String]) -> Self {
        let mut parsed = Self {
            options: Vec::new(),
            locations: Vec::new(),
        };
        let mut positional = false;
        for argument in arguments.iter().skip(1) {
            if !positional && argument == "--" {
                positional = true;
            } else if !positional && argument.starts_with("--") {
                parsed.options.push(argument);
            } else {
                parsed.locations.push(argument);
            }
        }
        parsed
    }

    /// Whether an option appears before the terminator.
    pub(crate) fn has(&self, option: &str) -> bool {
        self.options.iter().any(|argument| argument.as_str() == option)
    }
}

/// Removes a launcher option while preserving positional arguments verbatim.
pub(crate) fn without_option(arguments: Vec<String>, option: &str) -> Vec<String> {
    let end = arguments
        .iter()
        .skip(1)
        .position(|argument| argument == "--")
        .map(|index| index + 1);
    arguments
        .into_iter()
        .enumerate()
        .filter_map(|(index, argument)| {
            let is_option = index > 0 && end.is_none_or(|end| index < end);
            (!is_option || argument != option).then_some(argument)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_like_filenames_and_a_second_separator_remain_positional() {
        let arguments = [
            "openxplorer",
            "--select",
            "--",
            "--quit",
            "--restart",
            "--version",
            "--",
        ]
        .map(str::to_owned);
        let parsed = Arguments::parse(&arguments);

        assert!(parsed.has("--select"));
        for name in ["--quit", "--restart", "--version", "--"] {
            assert!(!parsed.has(name));
        }
        assert_eq!(parsed.locations, arguments[3..].iter().collect::<Vec<_>>());
    }
}
