//! Shared policy for GPU contract tests across the workspace.

/// Report unsupported GPU hardware or unavailable native dependencies.
///
/// When `MOONFIELD_STRICT_GPU` is set, this panics instead of allowing the
/// caller to skip. Tolerant tests print the reason and return from the test.
pub fn skip_gpu(reason: impl std::fmt::Display) {
    skip_with_mode(std::env::var_os("MOONFIELD_STRICT_GPU").is_some(), reason);
}

fn skip_with_mode(strict: bool, reason: impl std::fmt::Display) {
    if strict {
        panic!("strict GPU mode (MOONFIELD_STRICT_GPU is set): {reason}");
    }
    eprintln!("skipping: {reason}");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "strict GPU mode")]
    fn test_strict_mode_turns_skips_into_failures() {
        skip_with_mode(true, "no Vulkan physical device available");
    }

    #[test]
    fn test_tolerant_mode_allows_skips() {
        skip_with_mode(false, "no Vulkan physical device available");
    }
}
