//! The recovery passphrase that encrypts Operational Backups (ADR 0010 §7).
//!
//! The default is generated here: ten words drawn uniformly, from the
//! operating system's random source, from the 2048-word BIP-0039 English
//! list — 110 bits, the same scheme the age tools use. A person may instead
//! choose their own; [`check_strength`] applies the ADR's length rule,
//! refuses one built only from the most common passwords (with digits and
//! punctuation around them), and refuses the obviously repetitive.
//!
//! Confirmation — showing a generated passphrase once and having it typed
//! back exactly — is the setup screen's step (ADR 0010 §7); this module
//! judges only the value the person finally confirms.
//!
//! The wordlist is `assets/bip39-english.txt`, the BIP-0039 English list as
//! distributed with the `age` crate (MIT OR Apache-2.0).

use std::collections::HashSet;
use std::fmt;

/// The BIP-0039 English wordlist, one word per line.
pub const WORDLIST: &str = include_str!("../assets/bip39-english.txt");
/// Words in a generated passphrase.
pub const GENERATED_WORDS: usize = 10;

const MIN_CHARACTERS: usize = 20;
/// Windows Credential Manager holds at most 2,560 bytes; the bound is on the
/// value exactly as typed, since that is what is stored.
const MAX_BYTES: usize = 2048;
const MIN_WORDS: usize = 6;
const MAX_CHARACTERS: usize = 1024;
const MIN_DISTINCT_CHARACTERS: usize = 6;
const MIN_DISTINCT_WORDS: usize = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PassphraseProblem {
    /// Fewer than twenty characters and fewer than six words.
    TooShort,
    /// Long enough but built from too few distinct characters or words.
    TooRepetitive,
    /// Longer than any passphrase a person types, or than Windows can store.
    TooLong,
    /// Made only of very common passwords, digits and punctuation.
    TooCommon,
}

/// The most common passwords and keyboard runs, from published lists of
/// leaked passwords (facts, not a copied list). Matched as whole pieces after
/// digits, spaces and punctuation are removed.
const COMMON: &[&str] = &[
    "password",
    "passw0rd",
    "qwerty",
    "qwertyuiop",
    "asdfgh",
    "asdfghjkl",
    "zxcvbn",
    "zxcvbnm",
    "azerty",
    "abc",
    "abcdef",
    "iloveyou",
    "letmein",
    "admin",
    "administrator",
    "welcome",
    "monkey",
    "dragon",
    "master",
    "login",
    "princess",
    "sunshine",
    "football",
    "baseball",
    "shadow",
    "superman",
    "batman",
    "trustno",
    "hello",
    "freedom",
    "whatever",
    "qazwsx",
    "michael",
    "jennifer",
    "charlie",
    "daniel",
    "jordan",
    "hunter",
    "ranger",
    "buster",
    "soccer",
    "hockey",
    "killer",
    "george",
    "computer",
    "starwars",
    "secret",
    "pokemon",
    "mustang",
    "access",
    "flower",
    "cheese",
    "summer",
    "winter",
    "spring",
    "autumn",
    "love",
    "lovely",
    "angel",
    "babygirl",
    "tigger",
    "cookie",
    "chocolate",
    "matrix",
    "internet",
    "google",
    "facebook",
    "samsung",
    "apple",
    "orange",
    "banana",
    "pepper",
    "ginger",
    "maggie",
    "ashley",
    "nicole",
    "jessica",
    "thomas",
    "robert",
    "andrew",
    "joshua",
    "taylor",
    "michelle",
    "passpass",
    "pass",
    "test",
    "guest",
    "user",
    "root",
    "default",
    "changeme",
    "backup",
    "mypassword",
    "mysecret",
    "secure",
    "security",
    "private",
    "temp",
    "qwe",
    "asd",
    "zxc",
    "one",
    "two",
    "three",
];

#[derive(Debug)]
pub struct RandomSourceUnavailable;

impl fmt::Display for RandomSourceUnavailable {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("the operating system's random source is unavailable")
    }
}

impl std::error::Error for RandomSourceUnavailable {}

/// Ten words joined by hyphens, e.g. `ocean-table-...`.
pub fn generate_recovery_passphrase() -> Result<String, RandomSourceUnavailable> {
    let words: Vec<&str> = WORDLIST.lines().collect();
    debug_assert_eq!(words.len(), 2048);
    let mut chosen = Vec::with_capacity(GENERATED_WORDS);
    for _ in 0..GENERATED_WORDS {
        let mut bytes = [0u8; 2];
        getrandom::getrandom(&mut bytes).map_err(|_| RandomSourceUnavailable)?;
        // 2048 is a power of two, so eleven bits index it without bias.
        let index = usize::from(u16::from_le_bytes(bytes) & 0x07ff);
        chosen.push(words.get(index).copied().ok_or(RandomSourceUnavailable)?);
    }
    Ok(chosen.join("-"))
}

/// The ADR's rule for a passphrase a person chose: at least twenty
/// characters or six words, not built from too few distinct parts.
pub fn check_strength(candidate: &str) -> Result<(), PassphraseProblem> {
    // The bounds apply to the value exactly as typed: that is what is stored.
    if candidate.chars().count() > MAX_CHARACTERS || candidate.len() > MAX_BYTES {
        return Err(PassphraseProblem::TooLong);
    }
    let trimmed = candidate.trim();
    let characters = trimmed.chars().count();
    let words: Vec<&str> = trimmed
        .split(|character: char| character.is_whitespace() || character == '-')
        .filter(|word| !word.is_empty())
        .collect();
    if characters < MIN_CHARACTERS && words.len() < MIN_WORDS {
        return Err(PassphraseProblem::TooShort);
    }
    if only_common(trimmed) {
        return Err(PassphraseProblem::TooCommon);
    }
    let distinct_characters: HashSet<char> = trimmed
        .chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    let distinct_words: HashSet<String> = words.iter().map(|word| word.to_lowercase()).collect();
    if distinct_characters.len() < MIN_DISTINCT_CHARACTERS
        || (words.len() >= MIN_WORDS && distinct_words.len() < MIN_DISTINCT_WORDS)
    {
        return Err(PassphraseProblem::TooRepetitive);
    }
    Ok(())
}

/// Whether, once ASCII digits, spaces and punctuation are removed, nothing is
/// left but pieces from [`COMMON`] (or nothing at all).
fn only_common(candidate: &str) -> bool {
    let letters: String = candidate
        .chars()
        .filter(|character| {
            !(character.is_ascii_digit()
                || character.is_whitespace()
                || character.is_ascii_punctuation())
        })
        .flat_map(char::to_lowercase)
        .collect();
    if letters.is_empty() {
        return true;
    }
    // reachable[i]: the first i bytes split exactly into common pieces.
    let bytes = letters.as_bytes();
    let mut reachable = vec![false; bytes.len() + 1];
    reachable[0] = true;
    for start in 0..bytes.len() {
        if !reachable[start] {
            continue;
        }
        for piece in COMMON {
            let end = start + piece.len();
            if end <= bytes.len() && &bytes[start..end] == piece.as_bytes() {
                reachable[end] = true;
            }
        }
    }
    reachable[bytes.len()]
}
