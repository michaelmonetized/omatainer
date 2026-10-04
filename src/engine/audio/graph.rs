//! Exact graph links are saved with an audio profile and reviewed before activation.
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Routes {
    pub outputs: Vec<Link>,
    pub inputs: Vec<Link>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Link {
    pub channel: u16,
    pub endpoint: String,
}

impl Routes {
    /// Validate saved endpoints.
    /// Takes saved input and output maps; returns refusal for duplicates, self links or excessive values.
    pub fn validate(&self) -> Result<(), String> {
        for links in [&self.outputs, &self.inputs] {
            if links.len() > 64 {
                return Err("Use at most 64 graph links in each direction".into());
            }
            let mut channels = BTreeSet::new();
            let mut endpoints = BTreeSet::new();
            for link in links {
                if link.channel >= 64
                    || !channels.insert(link.channel)
                    || !endpoints.insert(&link.endpoint)
                    || link.endpoint.len() > 255
                    || link.endpoint.chars().any(char::is_control)
                    || link.endpoint.trim() != link.endpoint
                    || link.endpoint.split_once(':').is_none_or(|(client, port)| {
                        client.is_empty() || port.is_empty() || owned(client)
                    })
                {
                    return Err(
                        "Graph links require unique channels and exact external port names".into(),
                    );
                }
            }
        }
        Ok(())
    }
}

/// Identify this application's graph clients.
/// Takes a client name; returns whether its input or output belongs to Omatainer.
pub(super) fn owned(client: &str) -> bool {
    matches!(client, "Omatainer" | "OmatainerCapture")
}

/// Reject a path back into the application.
/// Takes actual and proposed directed client edges; returns refusal if either owned client can feed itself.
pub(super) fn no_feedback(edges: &[(String, String)]) -> Result<(), String> {
    if edges.len() > 16384 {
        return Err("Graph connection inventory exceeds the review limit".into());
    }
    let canonical = |name: &str| {
        if owned(name) {
            "Omatainer".to_owned()
        } else {
            name.to_owned()
        }
    };
    let mut graph = BTreeMap::<String, Vec<String>>::new();
    for (source, destination) in edges {
        graph
            .entry(canonical(source))
            .or_default()
            .push(canonical(destination));
    }
    let mut pending = graph.get("Omatainer").cloned().unwrap_or_default();
    let mut visited = BTreeSet::new();
    while let Some(client) = pending.pop() {
        if client == "Omatainer" {
            return Err(
                "Graph route could feed audio back into Omatainer; connection refused".into(),
            );
        }
        if visited.insert(client.clone()) {
            if let Some(next) = graph.get(&client) {
                pending.extend(next.iter().cloned());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_routes_reject_duplicate_and_self_endpoints_and_round_trip() {
        let mut routes = Routes {
            outputs: vec![Link {
                channel: 0,
                endpoint: "interface:playback_1".into(),
            }],
            inputs: Vec::new(),
        };
        routes.validate().unwrap();
        assert_eq!(
            serde_json::from_slice::<Routes>(&serde_json::to_vec(&routes).unwrap()).unwrap(),
            routes
        );
        routes.outputs.push(routes.outputs[0].clone());
        assert!(routes.validate().is_err());
        routes.outputs.pop();
        routes.outputs[0].endpoint = "OmatainerCapture:input_01".into();
        assert!(routes.validate().is_err());
    }
    #[test]
    fn feedback_check_follows_processors_and_merges_owned_capture() {
        let mut edges = vec![
            ("Omatainer".into(), "compressor".into()),
            ("compressor".into(), "speakers".into()),
        ];
        no_feedback(&edges).unwrap();
        edges.push(("speakers".into(), "OmatainerCapture".into()));
        assert!(no_feedback(&edges).is_err());
        edges.pop();
        edges.push(("independent".into(), "OmatainerCapture".into()));
        no_feedback(&edges).unwrap();
        edges.push(("compressor".into(), "independent".into()));
        assert!(no_feedback(&edges).is_err());
    }
}
