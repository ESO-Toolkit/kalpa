//! Keep owned Windows debug test launches out of the installed app's profile.
//! Release builds always use the production identifier and credential service.

use std::borrow::Cow;

pub fn identifier() -> Cow<'static, str> {
    match e2e_token() {
        Some(token) => Cow::Owned(format!("com.kalpa.desktop.e2e.{token}")),
        None => Cow::Borrowed("com.kalpa.desktop"),
    }
}

pub fn credential_service() -> Cow<'static, str> {
    match e2e_token() {
        Some(token) => Cow::Owned(format!("kalpa-e2e-{token}")),
        None => Cow::Borrowed("kalpa"),
    }
}

pub fn is_e2e() -> bool {
    e2e_token().is_some()
}

#[cfg(all(windows, debug_assertions))]
fn e2e_token() -> Option<&'static str> {
    // Resolve once, before logging or credentials are opened. Every consumer
    // must use the same namespace for the lifetime of the process.
    static TOKEN: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    TOKEN
        .get_or_init(|| {
            std::env::var_os("KALPA_E2E_TOKEN").map(|value| {
                let token = value
                    .into_string()
                    .expect("KALPA_E2E_TOKEN must be Unicode");
                validate_token(&token).expect("KALPA_E2E_TOKEN must be a safe profile name");
                token
            })
        })
        .as_deref()
}

#[cfg(not(all(windows, debug_assertions)))]
fn e2e_token() -> Option<&'static str> {
    None
}

#[cfg(all(windows, debug_assertions))]
fn validate_token(token: &str) -> Result<(), &'static str> {
    if token.is_empty()
        || token.len() > 128
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err("expected 1-128 ASCII letters, digits, hyphens or underscores");
    }
    Ok(())
}

#[cfg(all(test, windows, debug_assertions))]
mod tests {
    use super::validate_token;

    #[test]
    fn rejects_tokens_that_can_escape_or_alias_a_profile() {
        for token in ["", ".", "..", "a/b", "a\\b", "a:b", "a b", "a.", "é"] {
            assert!(validate_token(token).is_err(), "accepted {token:?}");
        }
        assert!(validate_token(&"a".repeat(129)).is_err());
        assert!(validate_token("123-abc_DEF").is_ok());
    }
}
