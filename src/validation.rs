use crate::error::AppError;

const MAX_EVENT_TYPES: usize = 20;
const MAX_URL_LEN: usize = 2048;

pub fn normalize_email(email: &str) -> Result<String, AppError> {
    let email = email.trim().to_lowercase();
    let valid = (3..=255).contains(&email.len())
        && email.chars().filter(|c| *c == '@').count() == 1
        && !email.starts_with('@')
        && !email.ends_with('@')
        && !email.contains(char::is_whitespace);
    if !valid {
        return Err(AppError::bad_request("invalid email"));
    }
    Ok(email)
}

pub fn validate_password(password: &str) -> Result<(), AppError> {
    if !(8..=128).contains(&password.len()) {
        return Err(AppError::bad_request(
            "password must be between 8 and 128 characters",
        ));
    }
    Ok(())
}

pub fn validate_url(url: &str) -> Result<String, AppError> {
    let url = url.trim();
    if url.is_empty() || url.len() > MAX_URL_LEN {
        return Err(AppError::bad_request("invalid url"));
    }
    let parsed = reqwest::Url::parse(url).map_err(|_| AppError::bad_request("invalid url"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        _ => return Err(AppError::bad_request("url must use http or https")),
    }
    if parsed.host_str().is_none() {
        return Err(AppError::bad_request("url must include a host"));
    }
    Ok(url.to_string())
}

pub fn normalize_event_types(event_types: Vec<String>) -> Result<Vec<String>, AppError> {
    if event_types.is_empty() {
        return Err(AppError::bad_request("event_types must not be empty"));
    }
    if event_types.len() > MAX_EVENT_TYPES {
        return Err(AppError::bad_request(
            "event_types accepts at most 20 entries",
        ));
    }
    let mut normalized = Vec::with_capacity(event_types.len());
    for event_type in event_types {
        let event_type = event_type.trim().to_string();
        if !valid_event_type(&event_type) {
            return Err(AppError::bad_request(format!(
                "invalid event type: {event_type}"
            )));
        }
        if !normalized.contains(&event_type) {
            normalized.push(event_type);
        }
    }
    Ok(normalized)
}

pub fn validate_event_type(event_type: &str) -> Result<String, AppError> {
    let event_type = event_type.trim();
    if !valid_event_type(event_type) {
        return Err(AppError::bad_request("invalid event type"));
    }
    Ok(event_type.to_string())
}

fn valid_event_type(event_type: &str) -> bool {
    let len = event_type.chars().count();
    (1..=255).contains(&len)
        && event_type
            .chars()
            .all(|c| !c.is_whitespace() && !c.is_control())
}

#[cfg(test)]
mod tests {
    use super::{normalize_email, normalize_event_types, validate_password, validate_url};

    #[test]
    fn email_is_trimmed_and_lowercased() {
        assert_eq!(
            normalize_email("  Ada@Example.com ").unwrap(),
            "ada@example.com"
        );
        assert!(normalize_email("not-an-email").is_err());
        assert!(normalize_email("@missing.local").is_err());
    }

    #[test]
    fn password_length_is_bounded() {
        assert!(validate_password("short").is_err());
        assert!(validate_password("long-enough").is_ok());
    }

    #[test]
    fn url_must_be_http() {
        assert!(validate_url("https://example.com/hook").is_ok());
        assert!(validate_url("ftp://example.com/hook").is_err());
        assert!(validate_url("not a url").is_err());
    }

    #[test]
    fn event_types_are_deduplicated() {
        let types = normalize_event_types(vec![
            " invoice.paid ".into(),
            "invoice.paid".into(),
            "*".into(),
        ])
        .unwrap();
        assert_eq!(types, vec!["invoice.paid", "*"]);
        assert!(normalize_event_types(vec!["has space".into()]).is_err());
    }
}
