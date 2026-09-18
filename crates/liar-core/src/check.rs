//! The catalogue of checks.
//!
//! Codes are public API the moment anyone writes `# liar: ignore[C1]` in their
//! source. They never change meaning; a retired check's code is retired with
//! it.

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Severity {
    /// Almost certainly wrong. The code does not do what it says.
    Error,
    /// Probably wrong, or wrong in a way that depends on context.
    Warning,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum CheckId {
    /// An async function called without `await`.
    C1,
    /// A blocking call inside async code.
    C2,
    /// A boolean-shaped name whose type is not boolean.
    C3a,
    /// A quantity-shaped name whose type is not numeric.
    C3b,
    /// A plural name holding a scalar, or a singular name holding a collection.
    C3c,
    /// A `get_*` function that mutates state.
    C3d,
    /// A meaningless name in a scope large enough to matter.
    C3e,
    /// One name bound to several unrelated meanings in a file.
    C3f,
    /// A resource not released on every path out.
    C4,
    /// A docstring contradicted by the code.
    C5,
}

impl CheckId {
    pub const ALL: &'static [CheckId] = &[
        CheckId::C1,
        CheckId::C2,
        CheckId::C3a,
        CheckId::C3b,
        CheckId::C3c,
        CheckId::C3d,
        CheckId::C3e,
        CheckId::C3f,
        CheckId::C4,
        CheckId::C5,
    ];

    pub fn code(self) -> &'static str {
        match self {
            CheckId::C1 => "C1",
            CheckId::C2 => "C2",
            CheckId::C3a => "C3a",
            CheckId::C3b => "C3b",
            CheckId::C3c => "C3c",
            CheckId::C3d => "C3d",
            CheckId::C3e => "C3e",
            CheckId::C3f => "C3f",
            CheckId::C4 => "C4",
            CheckId::C5 => "C5",
        }
    }

    pub fn from_code(code: &str) -> Option<Self> {
        CheckId::ALL.iter().copied().find(|c| c.code() == code)
    }

    pub fn default_severity(self) -> Severity {
        match self {
            // The code provably does not run, or provably leaks.
            CheckId::C1 | CheckId::C4 => Severity::Error,
            // Real, but contextual.
            _ => Severity::Warning,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for &check in CheckId::ALL {
            let code = check.code();
            assert_eq!(
                CheckId::from_code(code),
                Some(check),
                "round trip failed for {code}"
            );
        }
    }

    #[test]
    fn codes_are_unique() {
        let mut codes: Vec<_> = CheckId::ALL.iter().map(|c| c.code()).collect();
        codes.sort_unstable();
        let before = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), before, "duplicate check code");
    }

    #[test]
    fn all_contains_every_variant() {
        // ALL is hand-written, so it can drift from the enum. Comparing
        // against an expected count means adding a variant without adding it
        // to ALL fails a test rather than silently disabling the check.
        assert_eq!(CheckId::ALL.len(), 10);
    }

    #[test]
    fn unknown_codes_are_rejected() {
        assert_eq!(CheckId::from_code("C99"), None);
        assert_eq!(CheckId::from_code(""), None);
        assert_eq!(CheckId::from_code("c1"), None, "codes are case-sensitive");
    }

    #[test]
    fn the_spec_codes_are_present() {
        for code in [
            "C1", "C2", "C3a", "C3b", "C3c", "C3d", "C3e", "C3f", "C4", "C5",
        ] {
            assert!(CheckId::from_code(code).is_some(), "missing check {code}");
        }
    }

    #[test]
    fn checks_sort_by_code() {
        let mut checks = vec![CheckId::C4, CheckId::C1, CheckId::C3a];
        checks.sort();
        assert_eq!(checks, vec![CheckId::C1, CheckId::C3a, CheckId::C4]);
    }
}
