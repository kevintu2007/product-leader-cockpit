//! The recovery passphrase of ADR 0010 §7: generated as ten words from the
//! BIP-0039 English list, or chosen by the person under a length rule.

use std::collections::HashSet;

use pmc_platform::recovery_passphrase::{
    check_strength, generate_recovery_passphrase, PassphraseProblem, GENERATED_WORDS, WORDLIST,
};

#[test]
fn the_wordlist_is_the_full_bip39_english_list() {
    let words: Vec<&str> = WORDLIST.lines().collect();
    assert_eq!(words.len(), 2048);
    assert_eq!(words.first(), Some(&"abandon"));
    assert_eq!(words.last(), Some(&"zoo"));
    let distinct: HashSet<&&str> = words.iter().collect();
    assert_eq!(distinct.len(), 2048);
}

#[test]
fn a_generated_passphrase_is_ten_listed_words() {
    let generated =
        generate_recovery_passphrase().unwrap_or_else(|error| panic!("generate failed: {error}"));
    let words: Vec<&str> = generated.split('-').collect();
    assert_eq!(words.len(), GENERATED_WORDS);
    assert_eq!(GENERATED_WORDS, 10);
    let list: HashSet<&str> = WORDLIST.lines().collect();
    assert!(words.iter().all(|word| list.contains(word)), "{generated}");
    assert!(check_strength(&generated).is_ok());
}

#[test]
fn generated_passphrases_differ() {
    let seen: HashSet<String> = (0..20)
        .map(|_| {
            generate_recovery_passphrase()
                .unwrap_or_else(|error| panic!("generate failed: {error}"))
        })
        .collect();
    assert_eq!(seen.len(), 20);
}

#[test]
fn a_long_or_many_word_passphrase_is_accepted() {
    assert!(check_strength("my long garden gate is painted green").is_ok());
    assert!(check_strength("Tr0ub4dor&3-and-then-some-more").is_ok());
    assert!(check_strength("ocean table lamp river cloud stone").is_ok());
}

#[test]
fn a_short_passphrase_is_refused() {
    assert_eq!(
        check_strength("short pass"),
        Err(PassphraseProblem::TooShort)
    );
    assert_eq!(check_strength(""), Err(PassphraseProblem::TooShort));
    assert_eq!(
        check_strength("   padded out with spaces   "),
        Ok(()),
        "inner length counts, not only the trimmed ends"
    );
}

#[test]
fn a_repetitive_passphrase_is_refused() {
    assert_eq!(
        check_strength("aaaaaaaaaaaaaaaaaaaaaaaa"),
        Err(PassphraseProblem::TooRepetitive)
    );
    assert_eq!(
        check_strength("word word word word word word"),
        Err(PassphraseProblem::TooRepetitive)
    );
    assert_eq!(
        check_strength("abababababababababababab"),
        Err(PassphraseProblem::TooRepetitive)
    );
}

#[test]
fn an_absurdly_long_passphrase_is_refused() {
    assert_eq!(
        check_strength(&"abcdefghij".repeat(200)),
        Err(PassphraseProblem::TooLong)
    );
}

#[test]
fn a_passphrase_made_only_of_common_passwords_is_refused() {
    for common in [
        "passwordpasswordpassword",
        "Password123 Password123!",
        "qwertyuiop-letmein-admin",
        "iloveyou iloveyou 123456789",
        "123456789012345678901234",
    ] {
        assert!(
            matches!(
                check_strength(common),
                Err(PassphraseProblem::TooCommon | PassphraseProblem::TooRepetitive)
            ),
            "{common}"
        );
    }
    assert_eq!(
        check_strength("password harbor violin crest ember plain"),
        Ok(()),
        "a common word among uncommon ones is fine"
    );
}

#[test]
fn the_limits_apply_to_the_passphrase_exactly_as_typed() {
    let padded = format!("{}{}", " ".repeat(10), "abcdefghij".repeat(102));
    assert_eq!(check_strength(&padded), Err(PassphraseProblem::TooLong));
    // Within 1,024 characters but beyond what Credential Manager can hold.
    let wide = "龍鳳麒麟".repeat(200);
    assert_eq!(check_strength(&wide), Err(PassphraseProblem::TooLong));
}
