//! Whether two Vault-relative names are the same file name to Windows (DG3
//! Vault-root amendment §4: "compared the way Windows compares names";
//! product owner 2026-09-23, option A).
//!
//! Windows file names are compared ordinally and case-insensitively, one
//! UTF-16 code unit at a time through the system's own uppercase table, with
//! no Unicode normalization. Rust's case tables are a different Unicode
//! version and map some characters to several, so an imitation would drift
//! from what NTFS decides; this asks Windows instead.
//!
//! This module is the ONE place PMC calls into Windows directly with
//! `unsafe` — a reviewed exception to the workspace's `unsafe_code` rule
//! that the product owner approved on 2026-09-23 for exactly this call.
//! Anything else needs the product owner again.

/// True when Windows would treat `a` and `b` as the same name.
#[must_use]
pub fn same_windows_name(a: &str, b: &str) -> bool {
    imp::same(a, b)
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::Globalization::{CompareStringOrdinal, CSTR_EQUAL};

    pub(super) fn same(a: &str, b: &str) -> bool {
        let a: Vec<u16> = a.encode_utf16().collect();
        let b: Vec<u16> = b.encode_utf16().collect();
        // Lengths are passed explicitly, so no terminator is needed; a name
        // too long for an `i32` count is not a name Windows can hold.
        let (Ok(a_len), Ok(b_len)) = (i32::try_from(a.len()), i32::try_from(b.len())) else {
            return false;
        };
        // SAFETY: both pointers come from live `Vec<u16>`s that outlive the
        // call, each with exactly the length passed beside it; the function
        // only reads them. A zero result (failure) is not `CSTR_EQUAL`, so a
        // failure reads as "not the same", never as a match.
        #[allow(unsafe_code)]
        let result = unsafe { CompareStringOrdinal(a.as_ptr(), a_len, b.as_ptr(), b_len, 1) };
        result == CSTR_EQUAL
    }
}

#[cfg(not(windows))]
mod imp {
    /// Not a shipping target: PMC runs on Windows. Builds elsewhere compare
    /// exactly, which never merges two names Windows would keep apart.
    pub(super) fn same(a: &str, b: &str) -> bool {
        a == b
    }
}

// Windows only: the tests pin what Windows itself decides.
#[cfg(test)]
#[cfg(windows)]
mod tests {
    use super::same_windows_name;

    #[test]
    fn names_that_differ_only_in_case_are_one_name() {
        assert!(same_windows_name(
            "evidence/Report.md",
            "EVIDENCE/report.MD"
        ));
        // Non-ASCII letters fold too: Greek, Cyrillic, Latin-1, full-width.
        assert!(same_windows_name("Σύνοψη.md", "ΣΎΝΟΨΗ.md"));
        assert!(same_windows_name("отчёт.md", "ОТЧЁТ.md"));
        assert!(same_windows_name("résumé.md", "RÉSUMÉ.md"));
        assert!(same_windows_name("ａｂｃ.md", "ＡＢＣ.md"));
    }

    #[test]
    fn names_windows_keeps_apart_stay_apart() {
        assert!(!same_windows_name("a.md", "b.md"));
        assert!(!same_windows_name("季報.md", "季报.md"));
        // No normalization: a precomposed é and e + combining accent are
        // two different names to NTFS.
        assert!(!same_windows_name("caf\u{e9}.md", "cafe\u{301}.md"));
        // One-to-many mappings are not applied: ß is not SS.
        assert!(!same_windows_name("straße.md", "STRASSE.md"));
        assert!(!same_windows_name("a.md", "a.md "));
    }

    #[test]
    fn chinese_names_compare_as_themselves() {
        assert!(same_windows_name(
            "證據/第三季報告.md",
            "證據/第三季報告.md"
        ));
        assert!(!same_windows_name(
            "證據/第三季報告.md",
            "證據/第四季報告.md"
        ));
    }
}
