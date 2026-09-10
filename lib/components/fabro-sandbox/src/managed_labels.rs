use std::collections::{BTreeMap, HashMap};

use fabro_types::{RunId, SandboxProviderKind};

pub(crate) const MANAGED_LABEL: &str = "sh.fabro.managed";
pub(crate) const MANAGED_LABEL_VALUE: &str = "true";
pub(crate) const RUN_ID_LABEL: &str = "sh.fabro.run_id";

/// True when the provided label map carries the Fabro managed sentinel.
pub(crate) fn is_managed(labels: &BTreeMap<String, String>) -> bool {
    labels.get(MANAGED_LABEL).map(String::as_str) == Some(MANAGED_LABEL_VALUE)
}

/// Refuses a sandbox fabro did not create, or one created for another run.
///
/// Providers share a daemon or an organization with every other
/// application, so a persisted id is trusted only when the sandbox behind
/// it still carries fabro's labels.
pub(crate) fn verify_managed(
    kind: &SandboxProviderKind,
    sandbox_id: &str,
    labels: &BTreeMap<String, String>,
    run_id: Option<&RunId>,
) -> crate::Result<()> {
    if !is_managed(labels) {
        return Err(crate::Error::message(format!(
            "Refusing to operate on {kind} sandbox '{sandbox_id}' because it is missing label {MANAGED_LABEL}={MANAGED_LABEL_VALUE}"
        )));
    }
    if let Some(run_id) = run_id {
        let actual = labels.get(RUN_ID_LABEL).map(String::as_str);
        let expected = run_id.to_string();
        if actual != Some(expected.as_str()) {
            return Err(crate::Error::message(format!(
                "Refusing to operate on {kind} sandbox '{sandbox_id}' because label {RUN_ID_LABEL}={actual:?} does not match run {run_id}"
            )));
        }
    }
    Ok(())
}

pub(crate) fn for_run(run_id: Option<&RunId>) -> HashMap<String, String> {
    let mut labels = HashMap::new();
    insert_for_run(&mut labels, run_id);
    labels
}

pub(crate) fn merge_for_run(
    user_labels: Option<&HashMap<String, String>>,
    run_id: Option<&RunId>,
) -> HashMap<String, String> {
    let mut labels = user_labels.cloned().unwrap_or_default();
    insert_for_run(&mut labels, run_id);
    labels
}

fn insert_for_run(labels: &mut HashMap<String, String>, run_id: Option<&RunId>) {
    labels.insert(MANAGED_LABEL.to_string(), MANAGED_LABEL_VALUE.to_string());
    if let Some(run_id) = run_id {
        labels.insert(RUN_ID_LABEL.to_string(), run_id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use fabro_types::RunId;

    use super::*;

    fn conservative_daytona_key(key: &str) -> bool {
        key.chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_'))
    }

    #[test]
    fn managed_label_keys_match_docker_and_use_conservative_ascii() {
        assert_eq!(MANAGED_LABEL, "sh.fabro.managed");
        assert_eq!(RUN_ID_LABEL, "sh.fabro.run_id");
        assert!(conservative_daytona_key(MANAGED_LABEL));
        assert!(conservative_daytona_key(RUN_ID_LABEL));
    }

    #[test]
    fn managed_labels_include_run_id_when_present() {
        let run_id: RunId = "01HY0000000000000000000000".parse().unwrap();
        let labels = for_run(Some(&run_id));

        assert_eq!(labels.get(MANAGED_LABEL).map(String::as_str), Some("true"));
        assert_eq!(
            labels.get(RUN_ID_LABEL).map(String::as_str),
            Some("01HY0000000000000000000000")
        );
        assert!(is_managed(&labels.clone().into_iter().collect()));
    }

    #[test]
    fn managed_labels_override_reserved_user_labels() {
        let run_id: RunId = "01HY0000000000000000000000".parse().unwrap();
        let user_labels = HashMap::from([
            ("team".to_string(), "platform".to_string()),
            (MANAGED_LABEL.to_string(), "false".to_string()),
            (RUN_ID_LABEL.to_string(), "wrong".to_string()),
        ]);

        let labels = merge_for_run(Some(&user_labels), Some(&run_id));

        assert_eq!(labels.get("team").map(String::as_str), Some("platform"));
        assert_eq!(labels.get(MANAGED_LABEL).map(String::as_str), Some("true"));
        assert_eq!(
            labels.get(RUN_ID_LABEL).map(String::as_str),
            Some("01HY0000000000000000000000")
        );
    }

    #[test]
    fn verify_managed_requires_fabro_ownership_and_matching_run() {
        let run_id: RunId = "01HY0000000000000000000000".parse().unwrap();
        let kind = SandboxProviderKind::DOCKER;
        let mut labels = BTreeMap::new();
        assert!(verify_managed(&kind, "c1", &labels, None).is_err());
        labels.insert(MANAGED_LABEL.to_string(), "true".to_string());
        assert!(verify_managed(&kind, "c1", &labels, None).is_ok());
        assert!(verify_managed(&kind, "c1", &labels, Some(&run_id)).is_err());
        labels.insert(RUN_ID_LABEL.to_string(), run_id.to_string());
        assert!(verify_managed(&kind, "c1", &labels, Some(&run_id)).is_ok());
    }
}
