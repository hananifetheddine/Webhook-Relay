#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeliveryStatus {
    Pending,
    Success,
    Failed,
    Exhausted,
}

impl DeliveryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Exhausted => "exhausted",
        }
    }
}

pub fn endpoint_accepts(event_types: &[String], event_type: &str) -> bool {
    event_types
        .iter()
        .any(|candidate| candidate == "*" || candidate == event_type)
}

#[cfg(test)]
mod tests {
    use super::endpoint_accepts;

    #[test]
    fn matches_exact_type_or_wildcard() {
        let types = vec!["invoice.paid".to_string(), "user.created".to_string()];
        assert!(endpoint_accepts(&types, "invoice.paid"));
        assert!(!endpoint_accepts(&types, "invoice.failed"));

        let wildcard = vec!["*".to_string()];
        assert!(endpoint_accepts(&wildcard, "anything"));
    }
}
