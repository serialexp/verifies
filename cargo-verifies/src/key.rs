//! The requirement-key grammar records must follow (`FORMAT.md`). The
//! `verifies` macro enforces the same grammar at compile time; a reader
//! checks again so a record from any other producer is held to it too.

/// `REQ-<n>` or `REQ-<n>.<m>`, each number a positive `i32` written without
/// sign or leading zeros: exactly one spelling per key.
pub fn is_key(key: &str) -> bool {
    let Some(rest) = key.strip_prefix("REQ-") else {
        return false;
    };
    rest.splitn(2, '.').all(is_number)
}

fn is_number(digits: &str) -> bool {
    !digits.is_empty()
        && !digits.starts_with('0')
        && digits.bytes().all(|b| b.is_ascii_digit())
        && digits.parse::<i32>().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same table as the macro's: the two must agree.
    #[test]
    fn keys_have_exactly_one_spelling() {
        for good in ["REQ-1", "REQ-142.2", "REQ-2147483647.2147483647"] {
            assert!(is_key(good), "{good}");
        }
        for bad in [
            "",
            "REQ-",
            "REQ-0",
            "REQ-01",
            "REQ--1",
            "REQ-+1",
            "REQ-1.",
            "REQ-1.0",
            "REQ-1.01",
            "REQ-1.2.3",
            "REQ-1 .2",
            " REQ-1",
            "REQ-a",
            "req-1",
            "REQ-2147483648",
            "REQ-1,REQ-2",
        ] {
            assert!(!is_key(bad), "{bad:?}");
        }
    }
}
