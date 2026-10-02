//! Immutable, repository-wide ownership facts. The coordinator supplies the lock
//! and graph-validated candidates; this module never acquires a storage lock.
//!
//! @mara implements DES-CLAIM-API
//! @mara implements REQ-CLAIM-EXCLUSION
//! @mara implements REQ-CLAIM-RECOVERY

// Preserve the common error's structured publication/source provenance.
#![allow(clippy::result_large_err)]

mod format;
mod store;

use serde_json::{Value, json};
use std::path::Path;

use super::coordination::{ExecutionResult, SessionIdentity};
use super::graph::Evaluation;
use super::items::ItemHeader;

pub(crate) use format::valid_timestamp;
pub use store::ClaimStore;
pub(crate) use store::OwnershipSnapshot;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Claim {
    pub store_id: String,
    pub recovery_generation: String,
    pub id: String,
    pub item_id: String,
    pub actor: String,
    pub session: SessionIdentity,
    pub acquired_at: String,
    pub workspace_id: Option<String>,
    pub run_id: Option<String>,
    pub session_record_id: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimOutcome {
    Released,
    Completed,
    Reassigned,
}
impl ClaimOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Released => "released",
            Self::Completed => "completed",
            Self::Reassigned => "reassigned",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimEnding {
    pub store_id: String,
    pub recovery_generation: String,
    pub claim_id: String,
    pub item_id: String,
    pub ended_at: String,
    pub actor: String,
    pub reason: String,
    pub outcome: ClaimOutcome,
    pub recovery: bool,
}

/// Trusted snapshot constructed by the coordinator from its resolved graph.
/// Optional named-session context must be resolved and matched there first.
#[derive(Clone, Debug)]
pub struct ClaimCandidate {
    pub header: ItemHeader,
    pub evaluation: Evaluation,
    pub workspace_id: Option<String>,
    pub run_id: Option<String>,
    pub session_record_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimAuthorization {
    pub claim_id: String,
    pub session: SessionIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimInspection {
    pub claim: Claim,
    pub ending: Option<ClaimEnding>,
    pub current: bool,
}
#[derive(Clone, Debug)]
pub struct ClaimAcquired {
    pub claim: Claim,
    pub item: ItemHeader,
    pub changed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClaimEnded {
    pub claim: Claim,
    pub ending: ClaimEnding,
    pub changed: bool,
}
#[derive(Clone, Debug)]
pub struct ClaimReassigned {
    pub previous_claim_id: String,
    pub claim: Claim,
    pub item: ItemHeader,
    pub changed: bool,
}

impl Claim {
    pub fn parse(raw: &[u8]) -> ExecutionResult<Self> {
        format::claim(raw, Path::new("entity"))
    }
    pub(crate) fn parse_at(raw: &[u8], path: &Path) -> ExecutionResult<Self> {
        format::claim(raw, path).map_err(|error| error.at(path))
    }
    /// Acquisition envelope: optional keys are omitted rather than null.
    pub fn to_json(&self) -> Value {
        let mut value = json!({"format_version":1,"store_id":self.store_id,
            "recovery_generation":self.recovery_generation,"id":self.id,
            "item_id":self.item_id,"actor":self.actor,"session":self.session.to_json(),
            "acquired_at":self.acquired_at});
        for (key, field) in [
            ("workspace_id", &self.workspace_id),
            ("run_id", &self.run_id),
            ("session_record_id", &self.session_record_id),
        ] {
            if let Some(field) = field {
                value[key] = json!(field);
            }
        }
        value
    }
}
impl ClaimEnding {
    pub fn parse(raw: &[u8]) -> ExecutionResult<Self> {
        format::ending(raw, Path::new("entity"))
    }
    pub(crate) fn parse_at(raw: &[u8], path: &Path) -> ExecutionResult<Self> {
        format::ending(raw, path).map_err(|error| error.at(path))
    }
    pub fn to_json(&self) -> Value {
        let mut value = json!({"format_version":1,"store_id":self.store_id,
            "recovery_generation":self.recovery_generation,"claim_id":self.claim_id,
            "item_id":self.item_id,"ended_at":self.ended_at,"actor":self.actor,
            "reason":self.reason,"outcome":self.outcome.as_str()});
        if self.recovery {
            value["recovery"] = json!(true);
        }
        value
    }
}
impl ClaimInspection {
    pub fn to_json(&self) -> Value {
        json!({"claim":self.claim.to_json(),
            "ending":self.ending.as_ref().map(ClaimEnding::to_json),"current":self.current})
    }
}
impl ClaimEnded {
    pub fn to_json(&self) -> Value {
        json!({"claim":self.claim.to_json(),"ending":self.ending.to_json(),"changed":self.changed})
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;
    use crate::core::coordination::encode_path;

    #[test]
    fn source_path_parsing_changes_only_error_path_context() {
        let raw = b"format_version: 1\ninvalid: [unterminated\n";
        let path = Path::new("/store/claims/actual.yaml");
        for ending in [false, true] {
            let mut original = if ending {
                ClaimEnding::parse(raw).map(|_| ())
            } else {
                Claim::parse(raw).map(|_| ())
            }
            .unwrap_err();
            let contextual = if ending {
                ClaimEnding::parse_at(raw, path).map(|_| ())
            } else {
                Claim::parse_at(raw, path).map(|_| ())
            }
            .unwrap_err();
            assert_eq!(contextual.code, original.code);
            assert_eq!(contextual.message, original.message);
            assert_eq!(contextual.path.as_deref(), Some(path));
            for diagnostic in original.details["diagnostics"].as_array_mut().unwrap() {
                diagnostic["path"] = json!(encode_path(path));
            }
            assert_eq!(contextual.details, original.details);
        }
    }
}
