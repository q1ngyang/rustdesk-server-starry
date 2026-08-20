use serde_derive::Serialize;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct MatchedRule {
    pub(crate) name: String,
    pub(crate) index: usize,
    pub(crate) direction: String,
    #[serde(skip_serializing)]
    pub(crate) relay_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct CandidateTrace {
    pub(crate) relay_id: String,
    pub(crate) configured_order: usize,
    pub(crate) priority: Option<usize>,
    pub(crate) eligible: bool,
    pub(crate) exclusion_reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct SelectionTrace {
    pub(crate) kind: String,
    pub(crate) relay_id: Option<String>,
    pub(crate) predicted_index: Option<usize>,
    pub(crate) non_binding: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct AllocationTrace {
    pub(crate) config_generation: u64,
    pub(crate) health_snapshot_id: String,
    pub(crate) matched_rule: Option<MatchedRule>,
    pub(crate) candidates: Vec<CandidateTrace>,
    pub(crate) selection: SelectionTrace,
    pub(crate) warnings: Vec<String>,
}

/// Explain a Relay choice from immutable inputs. This function performs no
/// I/O, reads no globals, and cannot advance the production rotation counter.
pub(crate) fn explain_relay_selection(
    configured_relays: &[String],
    eligible_relays: &[String],
    matched_rule: Option<MatchedRule>,
    rotation_snapshot: usize,
    config_generation: u64,
    health_snapshot_id: String,
    exclusion_reasons: &HashMap<String, String>,
) -> AllocationTrace {
    let eligible: HashSet<String> = eligible_relays
        .iter()
        .map(|relay| relay.to_ascii_lowercase())
        .collect();
    let candidates = configured_relays
        .iter()
        .enumerate()
        .map(|(configured_order, relay)| {
            let is_eligible = eligible.contains(&relay.to_ascii_lowercase());
            let priority = eligible_relays
                .iter()
                .position(|candidate| candidate.eq_ignore_ascii_case(relay))
                .map(|index| index + 1);
            CandidateTrace {
                relay_id: relay.clone(),
                configured_order,
                priority,
                eligible: is_eligible,
                exclusion_reason: (!is_eligible).then(|| {
                    exclusion_reasons
                        .get(&relay.to_ascii_lowercase())
                        .cloned()
                        .unwrap_or_else(|| "transport_or_health_ineligible".to_owned())
                }),
            }
        })
        .collect();

    let (kind, relay_id, predicted_index) = if let Some(rule) = matched_rule.as_ref() {
        if eligible.contains(&rule.relay_id.to_ascii_lowercase()) {
            ("geo_rule".to_owned(), Some(rule.relay_id.clone()), None)
        } else {
            ("no_eligible_relay".to_owned(), None, None)
        }
    } else if eligible_relays.is_empty() {
        ("no_eligible_relay".to_owned(), None, None)
    } else if eligible_relays.len() == 1 {
        (
            "single_candidate".to_owned(),
            Some(eligible_relays[0].clone()),
            Some(0),
        )
    } else {
        let predicted = rotation_snapshot % eligible_relays.len();
        (
            "rotation_prediction".to_owned(),
            Some(eligible_relays[predicted].clone()),
            Some(predicted),
        )
    };

    AllocationTrace {
        config_generation,
        health_snapshot_id,
        matched_rule,
        candidates,
        selection: SelectionTrace {
            kind,
            relay_id,
            predicted_index,
            non_binding: true,
        },
        warnings: vec![
            "Simulation is non-binding and did not register clients or establish an HBBR session"
                .to_owned(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_fallback_is_predicted_without_mutable_state() {
        let configured = vec!["relay-a".to_owned(), "relay-b".to_owned()];
        let eligible = configured.clone();
        let reasons = HashMap::new();
        let first = explain_relay_selection(
            &configured,
            &eligible,
            None,
            7,
            42,
            "health-9".to_owned(),
            &reasons,
        );
        let second = explain_relay_selection(
            &configured,
            &eligible,
            None,
            7,
            42,
            "health-9".to_owned(),
            &reasons,
        );
        assert_eq!(first, second);
        assert_eq!(first.selection.kind, "rotation_prediction");
        assert_eq!(first.selection.relay_id.as_deref(), Some("relay-b"));
        assert!(first.selection.non_binding);
    }

    #[test]
    fn trace_explains_eligible_and_filtered_candidates() {
        let configured = vec!["relay-a".to_owned(), "relay-b".to_owned()];
        let eligible = vec!["relay-b".to_owned()];
        let reasons = HashMap::from([("relay-a".to_owned(), "wss_unhealthy".to_owned())]);
        let trace = explain_relay_selection(
            &configured,
            &eligible,
            None,
            0,
            3,
            "health-4".to_owned(),
            &reasons,
        );
        assert!(!trace.candidates[0].eligible);
        assert_eq!(
            trace.candidates[0].exclusion_reason.as_deref(),
            Some("wss_unhealthy")
        );
        assert!(trace.candidates[1].eligible);
        assert_eq!(trace.selection.relay_id.as_deref(), Some("relay-b"));
    }
}
