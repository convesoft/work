use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde_json::json;

use super::{
    Claim, ClaimAcquired, ClaimAuthorization, ClaimCandidate, ClaimEnded, ClaimEnding,
    ClaimInspection, ClaimOutcome, ClaimReassigned,
};
use crate::core::coordination::{
    CoordinationGuard, EntitySource, ExecutionError, ExecutionResult, SessionIdentity, new_id,
    now_timestamp, valid_id, yaml_bytes,
};
use crate::core::items::{Completion, ManualState};

#[derive(Clone, Copy, Debug, Default)]
pub struct ClaimStore;

/// One operation's ownership projection; never retained across guard lifetimes.
pub(crate) struct OwnershipSnapshot {
    current: BTreeMap<String, Vec<Claim>>,
}
impl OwnershipSnapshot {
    pub(crate) fn material_claims(&self) -> impl Iterator<Item = &Claim> {
        self.current
            .values()
            .flatten()
            .filter(|claim| claim.workspace_id.is_some())
    }
    pub(crate) fn exclusion(&self) -> ExecutionResult<()> {
        for item_id in self.current.keys() {
            self.current(item_id)?;
        }
        Ok(())
    }
    pub(crate) fn current(&self, item_id: &str) -> ExecutionResult<Option<Claim>> {
        let Some(claims) = self.current.get(item_id) else {
            return Ok(None);
        };
        if claims.len() > 1 {
            return Err(conflict(item_id, &claims.iter().collect::<Vec<_>>()));
        }
        Ok(claims.first().cloned())
    }
}

struct Snapshot {
    records: BTreeMap<String, ClaimInspection>,
    sources: Vec<(PathBuf, EntitySource)>,
}
fn acquisition_path(id: &str) -> PathBuf {
    PathBuf::from(format!("claims/{id}.yaml"))
}
fn ending_path(id: &str) -> PathBuf {
    PathBuf::from(format!("claims/{id}.end.yaml"))
}
fn argument_id(id: &str) -> ExecutionResult<()> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(ExecutionError::new(
            "invalid_argument",
            "claim and item references require full lowercase IDs",
        ))
    }
}
fn actor_valid(actor: &str) -> ExecutionResult<()> {
    if actor.is_empty() {
        Err(ExecutionError::new(
            "invalid_argument",
            "actor must be nonempty",
        ))
    } else {
        Ok(())
    }
}
fn stale(id: &str) -> ExecutionError {
    let mut error = ExecutionError::new(
        "stale_claim",
        "claim/session pair is no longer the current owner",
    );
    error.details = json!({"claim_id":id});
    error
}
fn recovery_valid(actor: &str, reason: &str, stopped: bool) -> ExecutionResult<()> {
    actor_valid(actor)?;
    if reason.is_empty() || !stopped {
        return Err(ExecutionError::new(
            "invalid_argument",
            "recovery requires nonempty reason and executors_stopped:true",
        ));
    }
    Ok(())
}

impl Snapshot {
    fn load(guard: &CoordinationGuard) -> ExecutionResult<Self> {
        guard.verify()?;
        let mut records = BTreeMap::new();
        let mut endings = BTreeMap::new();
        let mut sources = Vec::new();
        for name in guard.names(Path::new("claims"))? {
            let (id, ended) = if let Some(id) = name.strip_suffix(".end.yaml") {
                (id, true)
            } else if let Some(id) = name.strip_suffix(".yaml") {
                (id, false)
            } else {
                return Err(ExecutionError::new(
                    "invalid_format",
                    "unknown file in claims directory",
                )
                .at(guard.root_path().join("claims").join(name)));
            };
            let path = PathBuf::from("claims").join(&name);
            if !valid_id(id) {
                return Err(
                    ExecutionError::new("invalid_format", "invalid claim filename")
                        .at(guard.root_path().join(&path)),
                );
            }
            let source = guard.read(&path)?;
            let metadata = &guard.metadata;
            if ended {
                let ending = ClaimEnding::parse_at(&source.raw, &guard.root_path().join(&path))?;
                if ending.claim_id != id {
                    return Err(ExecutionError::new(
                        "invalid_format",
                        "claim ending filename/ID mismatch",
                    )
                    .at(guard.root_path().join(&path)));
                }
                check_identity(
                    &ending.store_id,
                    &ending.recovery_generation,
                    metadata,
                    guard,
                    &path,
                )?;
                endings.insert(id.to_owned(), ending);
            } else {
                let claim = Claim::parse_at(&source.raw, &guard.root_path().join(&path))?;
                if claim.id != id {
                    return Err(ExecutionError::new(
                        "invalid_format",
                        "claim filename/ID mismatch",
                    )
                    .at(guard.root_path().join(&path)));
                }
                check_identity(
                    &claim.store_id,
                    &claim.recovery_generation,
                    metadata,
                    guard,
                    &path,
                )?;
                records.insert(
                    id.to_owned(),
                    ClaimInspection {
                        claim,
                        ending: None,
                        current: true,
                    },
                );
            }
            sources.push((path, source));
        }
        for (id, ending) in endings {
            let record = records.get_mut(&id).ok_or_else(|| {
                ExecutionError::new("invalid_format", "dangling claim ending")
                    .at(guard.root_path().join(ending_path(&id)))
            })?;
            if ending.item_id != record.claim.item_id
                || ending.store_id != record.claim.store_id
                || ending.recovery_generation != record.claim.recovery_generation
            {
                return Err(ExecutionError::new(
                    "invalid_format",
                    "ending does not match acquisition identity",
                )
                .at(guard.root_path().join(ending_path(&id))));
            }
            record.current = false;
            record.ending = Some(ending);
        }
        Ok(Self { records, sources })
    }
    fn inspect(&self, id: &str) -> ExecutionResult<&ClaimInspection> {
        self.records.get(id).ok_or_else(|| {
            let mut error = ExecutionError::new("not_found", "claim was not found");
            error.details = json!({"claim_id":id});
            error
        })
    }
    fn current(&self, item_id: &str) -> ExecutionResult<Option<&Claim>> {
        let claims: Vec<_> = self
            .records
            .values()
            .filter(|r| r.current && r.claim.item_id == item_id)
            .map(|r| &r.claim)
            .collect();
        if claims.len() > 1 {
            return Err(conflict(item_id, &claims));
        }
        Ok(claims.into_iter().next())
    }
    fn exclusion(&self) -> ExecutionResult<()> {
        for record in self.records.values().filter(|r| r.current) {
            self.current(&record.claim.item_id)?;
        }
        Ok(())
    }
    fn recheck(&self, guard: &CoordinationGuard) -> ExecutionResult<()> {
        for (path, source) in &self.sources {
            guard.recheck(path, source)?;
        }
        guard.verify()
    }
    fn owner(&self, id: &str, session: &SessionIdentity) -> ExecutionResult<&Claim> {
        let record = self.records.get(id).ok_or_else(|| stale(id))?;
        if !record.current || &record.claim.session != session {
            return Err(stale(id));
        }
        if self.current(&record.claim.item_id)?.map(|c| c.id.as_str()) != Some(id) {
            return Err(stale(id));
        }
        Ok(&record.claim)
    }
}
fn check_identity(
    store_id: &str,
    generation: &str,
    metadata: &crate::core::storage::StoreMetadata,
    guard: &CoordinationGuard,
    path: &Path,
) -> ExecutionResult<()> {
    if store_id != metadata.store_id || generation != metadata.recovery_generation {
        let mut error =
            ExecutionError::new("invalid_format", "claim store identity/generation mismatch")
                .at(guard.root_path().join(path));
        error.details = json!({"store_id":store_id,"recovery_generation":generation,
            "expected_store_id":metadata.store_id,"expected_generation":metadata.recovery_generation});
        return Err(error);
    }
    Ok(())
}
fn conflict(item: &str, claims: &[&Claim]) -> ExecutionError {
    let mut error = ExecutionError::new("claim_conflict", "item has current ownership");
    error.details = json!({"item_id":item,"claim_ids":claims.iter().map(|c| &c.id).collect::<Vec<_>>(),
        "claims":claims.iter().map(|c| c.to_json()).collect::<Vec<_>>()});
    error
}
fn candidate_valid(candidate: &ClaimCandidate) -> ExecutionResult<()> {
    argument_id(&candidate.header.id)?;
    for id in [
        &candidate.workspace_id,
        &candidate.run_id,
        &candidate.session_record_id,
    ]
    .into_iter()
    .flatten()
    {
        argument_id(id)?;
    }
    if candidate.evaluation.id != candidate.header.id {
        return Err(ExecutionError::new(
            "invalid_argument",
            "candidate graph identity mismatch",
        ));
    }
    if candidate.header.completion != Completion::Manual
        || candidate.header.state != Some(ManualState::Open)
        || !candidate.evaluation.executable
        || candidate.evaluation.effective_done
        || !candidate.evaluation.blockers.is_empty()
    {
        let mut error = ExecutionError::new(
            "not_ready",
            "claim requires an executable open manual item with satisfied prerequisites",
        );
        let blockers: Vec<_> = candidate
            .evaluation
            .blockers
            .iter()
            .map(|b| match b {
                crate::core::graph::Blocker::Completed => json!({"kind":"completed"}),
                crate::core::graph::Blocker::Aggregate => json!({"kind":"aggregate"}),
                crate::core::graph::Blocker::Prerequisite { id, inherited_from } => {
                    json!({"kind":"prerequisite","id":id,"inherited_from":inherited_from})
                }
                crate::core::graph::Blocker::Child(id) => json!({"kind":"child","id":id}),
            })
            .collect();
        error.details = json!({"item_id":candidate.header.id,"blockers":blockers});
        return Err(error);
    }
    Ok(())
}
fn new_claim(
    guard: &CoordinationGuard,
    candidate: &ClaimCandidate,
    actor: &str,
    session: &SessionIdentity,
) -> ExecutionResult<Claim> {
    Ok(Claim {
        store_id: guard.metadata.store_id.clone(),
        recovery_generation: guard.metadata.recovery_generation.clone(),
        id: new_id()?,
        item_id: candidate.header.id.clone(),
        actor: actor.to_owned(),
        session: session.clone(),
        acquired_at: now_timestamp(),
        workspace_id: candidate.workspace_id.clone(),
        run_id: candidate.run_id.clone(),
        session_record_id: candidate.session_record_id.clone(),
    })
}
fn end(
    guard: &CoordinationGuard,
    snapshot: &Snapshot,
    claim: &Claim,
    actor: &str,
    reason: &str,
    outcome: ClaimOutcome,
    recovery: bool,
) -> ExecutionResult<ClaimEnded> {
    end_checked(
        guard,
        snapshot,
        claim,
        actor,
        reason,
        outcome,
        recovery,
        || Ok(()),
    )
}
// The callback is the coordinator's material-source check, separate from the
// retained claim-source checks owned here.
#[allow(clippy::too_many_arguments)]
fn end_checked(
    guard: &CoordinationGuard,
    snapshot: &Snapshot,
    claim: &Claim,
    actor: &str,
    reason: &str,
    outcome: ClaimOutcome,
    recovery: bool,
    recheck: impl FnOnce() -> ExecutionResult<()>,
) -> ExecutionResult<ClaimEnded> {
    let ending = ClaimEnding {
        store_id: claim.store_id.clone(),
        recovery_generation: claim.recovery_generation.clone(),
        claim_id: claim.id.clone(),
        item_id: claim.item_id.clone(),
        ended_at: now_timestamp(),
        actor: actor.to_owned(),
        reason: reason.to_owned(),
        outcome,
        recovery,
    };
    snapshot.recheck(guard)?;
    recheck()?;
    guard
        .create(&ending_path(&claim.id), &yaml_bytes(&ending.to_json()))
        .map_err(|error| ending_error(guard, &ending, error))?;
    Ok(ClaimEnded {
        claim: claim.clone(),
        ending,
        changed: true,
    })
}

fn ending_error(
    guard: &CoordinationGuard,
    ending: &ClaimEnding,
    mut error: ExecutionError,
) -> ExecutionError {
    let path = crate::core::coordination::encode_path(
        &guard.root_path().join(ending_path(&ending.claim_id)),
    );
    let created = if error.details["publication"] == "published" {
        vec![json!({"id":ending.claim_id,"path":path})]
    } else {
        Vec::new()
    };
    let uncertain = if error.details["publication"] == "possible" {
        vec![path]
    } else {
        Vec::new()
    };
    error.details["partial"] =
        json!({"created":created,"updated":[],"deleted":[],"uncertain_paths":uncertain});
    error
}

impl ClaimStore {
    pub(crate) fn ownership_snapshot(
        guard: &CoordinationGuard,
    ) -> ExecutionResult<OwnershipSnapshot> {
        let mut current = BTreeMap::<String, Vec<Claim>>::new();
        for record in Snapshot::load(guard)?
            .records
            .into_values()
            .filter(|record| record.current)
        {
            current
                .entry(record.claim.item_id.clone())
                .or_default()
                .push(record.claim);
        }
        Ok(OwnershipSnapshot { current })
    }
    pub fn validate_candidate(candidate: &ClaimCandidate) -> ExecutionResult<()> {
        candidate_valid(candidate)
    }
    /// Read-only gate before coordinator workspace/binding setup.
    pub fn validate_acquire(
        guard: &CoordinationGuard,
        candidate: &ClaimCandidate,
        actor: &str,
        session: &SessionIdentity,
    ) -> ExecutionResult<()> {
        acquisition_snapshot(guard, candidate, actor, session).map(|_| ())
    }
    pub fn validate_reassign(
        guard: &CoordinationGuard,
        claim_id: &str,
        candidate: &ClaimCandidate,
        actor: &str,
        session: &SessionIdentity,
        reason: &str,
        executors_stopped: bool,
    ) -> ExecutionResult<()> {
        reassignment_snapshot(
            guard,
            claim_id,
            candidate,
            actor,
            session,
            reason,
            executors_stopped,
        )
        .map(|_| ())
    }
    pub fn list(
        guard: &CoordinationGuard,
        item_id: Option<&str>,
        current_only: bool,
    ) -> ExecutionResult<Vec<ClaimInspection>> {
        if let Some(id) = item_id {
            argument_id(id)?;
        }
        Ok(Snapshot::load(guard)?
            .records
            .into_values()
            .filter(|r| {
                item_id.is_none_or(|id| r.claim.item_id == id) && (!current_only || r.current)
            })
            .collect())
    }
    pub fn inspect(guard: &CoordinationGuard, claim_id: &str) -> ExecutionResult<ClaimInspection> {
        argument_id(claim_id)?;
        Ok(Snapshot::load(guard)?.inspect(claim_id)?.clone())
    }
    pub fn current(guard: &CoordinationGuard, item_id: &str) -> ExecutionResult<Option<Claim>> {
        argument_id(item_id)?;
        Ok(Snapshot::load(guard)?.current(item_id)?.cloned())
    }
    /// Validate every supplied pair, including stale pairs on unclaimed items.
    /// Only claimed source files being modified require a matching pair.
    pub fn authorize(
        guard: &CoordinationGuard,
        item_id: &str,
        authorizations: &[ClaimAuthorization],
    ) -> ExecutionResult<Option<Claim>> {
        argument_id(item_id)?;
        let snapshot = Snapshot::load(guard)?;
        snapshot.exclusion()?;
        let mut ids = BTreeSet::new();
        for pair in authorizations {
            argument_id(&pair.claim_id)?;
            pair.session.validate()?;
            if !ids.insert(&pair.claim_id) {
                return Err(ExecutionError::new(
                    "invalid_argument",
                    "duplicate claim authorization",
                ));
            }
            snapshot.owner(&pair.claim_id, &pair.session)?;
        }
        let current = snapshot.current(item_id)?;
        if let Some(claim) = current
            && !ids.contains(&claim.id)
        {
            return Err(conflict(item_id, &[claim]));
        }
        snapshot.recheck(guard)?;
        Ok(current.cloned())
    }
    pub fn acquire(
        guard: &CoordinationGuard,
        candidate: &ClaimCandidate,
        actor: &str,
        session: &SessionIdentity,
    ) -> ExecutionResult<ClaimAcquired> {
        Self::acquire_checked(guard, candidate, actor, session, || Ok(()))
    }
    pub fn acquire_checked(
        guard: &CoordinationGuard,
        candidate: &ClaimCandidate,
        actor: &str,
        session: &SessionIdentity,
        recheck: impl FnOnce() -> ExecutionResult<()>,
    ) -> ExecutionResult<ClaimAcquired> {
        let snapshot = acquisition_snapshot(guard, candidate, actor, session)?;
        let claim = new_claim(guard, candidate, actor, session)?;
        snapshot.recheck(guard)?;
        recheck()?;
        guard
            .create(&acquisition_path(&claim.id), &yaml_bytes(&claim.to_json()))
            .map_err(|error| acquisition_error(guard, &claim, error))?;
        Ok(ClaimAcquired {
            claim,
            item: candidate.header.clone(),
            changed: true,
        })
    }
    pub fn release(
        guard: &CoordinationGuard,
        claim_id: &str,
        session: &SessionIdentity,
        reason: &str,
    ) -> ExecutionResult<ClaimEnded> {
        Self::owner_end(guard, claim_id, session, reason, ClaimOutcome::Released)
    }
    /// Called only after the coordinator has durably closed the item.
    /// A still-current acquisition can be ended on an already-done close retry.
    pub fn completed(
        guard: &CoordinationGuard,
        claim_id: &str,
        session: &SessionIdentity,
        reason: &str,
    ) -> ExecutionResult<ClaimEnded> {
        Self::owner_end(guard, claim_id, session, reason, ClaimOutcome::Completed)
    }
    fn owner_end(
        guard: &CoordinationGuard,
        claim_id: &str,
        session: &SessionIdentity,
        reason: &str,
        outcome: ClaimOutcome,
    ) -> ExecutionResult<ClaimEnded> {
        argument_id(claim_id)?;
        session.validate()?;
        let snapshot = Snapshot::load(guard)?;
        snapshot.exclusion()?;
        let record = snapshot
            .records
            .get(claim_id)
            .ok_or_else(|| stale(claim_id))?;
        if &record.claim.session != session {
            return Err(stale(claim_id));
        }
        if let Some(ending) = &record.ending {
            if outcome == ClaimOutcome::Released
                && ending.outcome == ClaimOutcome::Released
                && !ending.recovery
            {
                return Ok(ClaimEnded {
                    claim: record.claim.clone(),
                    ending: ending.clone(),
                    changed: false,
                });
            }
            return Err(stale(claim_id));
        }
        let claim = snapshot.owner(claim_id, session)?;
        end(
            guard,
            &snapshot,
            claim,
            &claim.actor,
            reason,
            outcome,
            false,
        )
    }
    pub fn recover(
        guard: &CoordinationGuard,
        claim_id: &str,
        actor: &str,
        reason: &str,
        executors_stopped: bool,
    ) -> ExecutionResult<ClaimEnded> {
        argument_id(claim_id)?;
        recovery_valid(actor, reason, executors_stopped)?;
        let snapshot = Snapshot::load(guard)?;
        snapshot.exclusion()?;
        let record = snapshot
            .records
            .get(claim_id)
            .ok_or_else(|| stale(claim_id))?;
        if !record.current {
            return Err(stale(claim_id));
        }
        end(
            guard,
            &snapshot,
            &record.claim,
            actor,
            reason,
            ClaimOutcome::Released,
            true,
        )
    }
    pub fn reassign(
        guard: &CoordinationGuard,
        claim_id: &str,
        candidate: &ClaimCandidate,
        actor: &str,
        session: &SessionIdentity,
        reason: &str,
        executors_stopped: bool,
    ) -> ExecutionResult<ClaimReassigned> {
        Self::reassign_checked(
            guard,
            claim_id,
            candidate,
            actor,
            session,
            reason,
            executors_stopped,
            || Ok(()),
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn reassign_checked(
        guard: &CoordinationGuard,
        claim_id: &str,
        candidate: &ClaimCandidate,
        actor: &str,
        session: &SessionIdentity,
        reason: &str,
        executors_stopped: bool,
        mut recheck: impl FnMut() -> ExecutionResult<()>,
    ) -> ExecutionResult<ClaimReassigned> {
        let snapshot = reassignment_snapshot(
            guard,
            claim_id,
            candidate,
            actor,
            session,
            reason,
            executors_stopped,
        )?;
        let record = snapshot.inspect(claim_id)?;
        // Generate and validate the replacement before ending ownership.
        let claim = new_claim(guard, candidate, actor, session)?;
        if guard.optional(&acquisition_path(&claim.id))?.is_some()
            || guard.optional(&ending_path(&claim.id))?.is_some()
        {
            return Err(ExecutionError::new(
                "claim_conflict",
                "generated claim ID collision",
            ));
        }
        end_checked(
            guard,
            &snapshot,
            &record.claim,
            actor,
            reason,
            ClaimOutcome::Reassigned,
            true,
            &mut recheck,
        )
        .map_err(|error| reassignment_error(guard, claim_id, &claim.id, false, error))?;
        recheck().map_err(|error| reassignment_error(guard, claim_id, &claim.id, true, error))?;
        publish_replacement(guard, claim_id, &claim)?;
        Ok(ClaimReassigned {
            previous_claim_id: claim_id.to_owned(),
            claim,
            item: candidate.header.clone(),
            changed: true,
        })
    }
}

fn acquisition_snapshot(
    guard: &CoordinationGuard,
    candidate: &ClaimCandidate,
    actor: &str,
    session: &SessionIdentity,
) -> ExecutionResult<Snapshot> {
    actor_valid(actor)?;
    session.validate()?;
    candidate_valid(candidate)?;
    let snapshot = Snapshot::load(guard)?;
    snapshot.exclusion()?;
    if let Some(claim) = snapshot.current(&candidate.header.id)? {
        return Err(conflict(&candidate.header.id, &[claim]));
    }
    Ok(snapshot)
}
fn reassignment_snapshot(
    guard: &CoordinationGuard,
    claim_id: &str,
    candidate: &ClaimCandidate,
    actor: &str,
    session: &SessionIdentity,
    reason: &str,
    executors_stopped: bool,
) -> ExecutionResult<Snapshot> {
    argument_id(claim_id)?;
    recovery_valid(actor, reason, executors_stopped)?;
    session.validate()?;
    candidate_valid(candidate)?;
    let snapshot = Snapshot::load(guard)?;
    snapshot.exclusion()?;
    let record = snapshot
        .records
        .get(claim_id)
        .ok_or_else(|| stale(claim_id))?;
    if !record.current {
        return Err(stale(claim_id));
    }
    if record.claim.item_id != candidate.header.id {
        return Err(ExecutionError::new(
            "invalid_argument",
            "reassignment candidate must match the old item",
        ));
    }
    Ok(snapshot)
}
fn acquisition_error(
    guard: &CoordinationGuard,
    claim: &Claim,
    mut error: ExecutionError,
) -> ExecutionError {
    let path = crate::core::coordination::encode_path(
        &guard.root_path().join(acquisition_path(&claim.id)),
    );
    let created = if error.details["publication"] == "published" {
        vec![json!({"id":claim.id,"path":path})]
    } else {
        Vec::new()
    };
    let uncertain = if error.details["publication"] == "possible" {
        vec![path]
    } else {
        Vec::new()
    };
    error.details["partial"] =
        json!({"created":created,"updated":[],"deleted":[],"uncertain_paths":uncertain});
    error.details["claim_id"] = json!(claim.id);
    error
}

fn publish_replacement(
    guard: &CoordinationGuard,
    previous_id: &str,
    claim: &Claim,
) -> ExecutionResult<()> {
    guard
        .create(&acquisition_path(&claim.id), &yaml_bytes(&claim.to_json()))
        .map_err(|error| reassignment_error(guard, previous_id, &claim.id, true, error))
}
fn reassignment_error(
    guard: &CoordinationGuard,
    previous_id: &str,
    replacement_id: &str,
    ending_published: bool,
    mut error: ExecutionError,
) -> ExecutionError {
    use crate::core::coordination::encode_path;
    if !error.details.is_object() {
        error.details = json!({"cause_details":error.details});
    }
    if error.details.get("publication").is_none() {
        error.details["publication"] = json!("not_published");
    }
    let mut created = Vec::new();
    let mut uncertain_paths = Vec::new();
    let ending = guard.root_path().join(ending_path(previous_id));
    let acquisition = guard.root_path().join(acquisition_path(replacement_id));
    if ending_published {
        created.push(json!({"id":previous_id,"path":encode_path(&ending)}));
    }
    let (attempted_id, attempted_path) = if ending_published {
        (replacement_id, acquisition)
    } else {
        (previous_id, ending)
    };
    match error.details["publication"].as_str() {
        Some("published") => {
            created.push(json!({"id":attempted_id,"path":encode_path(&attempted_path)}))
        }
        Some("possible") => uncertain_paths.push(encode_path(&attempted_path)),
        _ => {}
    }
    error.details["partial"] =
        json!({"created":created,"updated":[],"deleted":[],"uncertain_paths":uncertain_paths});
    error.details["previous_claim_id"] = json!(previous_id);
    error.details["replacement_claim_id"] = json!(replacement_id);
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::project::discover;
    use crate::core::storage::Storage;
    use std::fs;
    use std::process::Command;

    struct Fixture {
        path: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("work-claim-source-{}", new_id().unwrap()));
            fs::create_dir(&path).unwrap();
            assert!(
                Command::new("git")
                    .arg("init")
                    .arg("--quiet")
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
            let project = discover(Some(&path)).unwrap();
            Storage::new(project).initialize().unwrap();
            Self { path }
        }
        fn guard(&self) -> CoordinationGuard {
            CoordinationGuard::acquire(&discover(Some(&self.path)).unwrap(), true).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.path).unwrap();
        }
    }
    fn claim(guard: &CoordinationGuard) -> Claim {
        Claim {
            store_id: guard.metadata.store_id.clone(),
            recovery_generation: guard.metadata.recovery_generation.clone(),
            id: new_id().unwrap(),
            item_id: new_id().unwrap(),
            actor: "worker".into(),
            session: SessionIdentity {
                namespace: "provider".into(),
                id: "session".into(),
            },
            acquired_at: now_timestamp(),
            workspace_id: None,
            run_id: None,
            session_record_id: None,
        }
    }
    #[test]
    fn changed_acquisition_source_refuses_ending_without_rewriting_any_claim() {
        let fixture = Fixture::new();
        let guard = fixture.guard();
        let claim = claim(&guard);
        let path = acquisition_path(&claim.id);
        let original = yaml_bytes(&claim.to_json());
        guard.create(&path, &original).unwrap();
        let snapshot = Snapshot::load(&guard).unwrap();
        // Replace with the same bytes: identity/fingerprint still changed.
        let replacement = guard.root_path().join("claims/replacement");
        fs::write(&replacement, &original).unwrap();
        fs::rename(replacement, guard.root_path().join(&path)).unwrap();
        let error = end(
            &guard,
            &snapshot,
            &claim,
            "worker",
            "",
            ClaimOutcome::Released,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code, "conflict");
        assert_eq!(error.path.unwrap(), guard.root_path().join(&path));
        assert!(!guard.root_path().join(ending_path(&claim.id)).exists());
        assert_eq!(fs::read(guard.root_path().join(&path)).unwrap(), original);
    }
    #[test]
    fn failed_replacement_publication_reports_persisted_ending_and_keeps_gap_inspectable() {
        let fixture = Fixture::new();
        let guard = fixture.guard();
        let claim = claim(&guard);
        guard
            .create(&acquisition_path(&claim.id), &yaml_bytes(&claim.to_json()))
            .unwrap();
        let snapshot = Snapshot::load(&guard).unwrap();
        end(
            &guard,
            &snapshot,
            &claim,
            "controller",
            "stopped",
            ClaimOutcome::Reassigned,
            true,
        )
        .unwrap();
        // Exercise the second publication's error path with an actual create-new
        // collision. Production validates generated-ID absence before ending.
        let error = publish_replacement(&guard, &claim.id, &claim).unwrap_err();
        assert_eq!(
            error.details["partial"]["created"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(error.details["partial"]["created"][0]["id"], claim.id);
        assert_eq!(error.details["publication"], "not_published");
        assert!(
            error.details["partial"]["uncertain_paths"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            ClaimStore::current(&guard, &claim.item_id)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            ClaimStore::inspect(&guard, &claim.id)
                .unwrap()
                .ending
                .unwrap()
                .outcome,
            ClaimOutcome::Reassigned
        );
    }
    #[test]
    fn partial_error_preserves_last_write_uncertainty_and_underlying_provenance() {
        let fixture = Fixture::new();
        let guard = fixture.guard();
        let previous = new_id().unwrap();
        let replacement = new_id().unwrap();
        let mut cause = ExecutionError::new("io", "parent sync interrupted");
        cause.details = json!({"publication":"possible","errno":5});
        let error = reassignment_error(&guard, &previous, &replacement, true, cause);
        assert_eq!(error.code, "io");
        assert_eq!(error.details["errno"], 5);
        assert_eq!(error.details["publication"], "possible");
        assert_eq!(
            error.details["partial"]["created"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            error.details["partial"]["uncertain_paths"],
            json!([crate::core::coordination::encode_path(
                &guard.root_path().join(acquisition_path(&replacement))
            )])
        );
    }

    #[test]
    fn ending_sync_failure_retains_uncertainty_for_release_and_reassignment() {
        for reassign in [false, true] {
            let fixture = Fixture::new();
            let guard = fixture.guard();
            let claim = claim(&guard);
            guard
                .create(&acquisition_path(&claim.id), &yaml_bytes(&claim.to_json()))
                .unwrap();
            let failure = crate::core::storage::files::fail_next("publication_sync");
            let error = if reassign {
                let candidate = ClaimCandidate {
                    header: crate::core::operations::default_header(
                        claim.item_id.clone(),
                        "item".into(),
                    ),
                    evaluation: crate::core::graph::Evaluation {
                        id: claim.item_id.clone(),
                        effective_done: false,
                        executable: true,
                        blockers: Vec::new(),
                    },
                    workspace_id: None,
                    run_id: None,
                    session_record_id: None,
                };
                ClaimStore::reassign(
                    &guard,
                    &claim.id,
                    &candidate,
                    "controller",
                    &claim.session,
                    "stopped",
                    true,
                )
                .unwrap_err()
            } else {
                ClaimStore::release(&guard, &claim.id, &claim.session, "").unwrap_err()
            };
            drop(failure);
            let ending = guard.root_path().join(ending_path(&claim.id));
            assert!(ending.exists());
            assert_eq!(error.code, "io");
            assert_eq!(error.details["errno"], 5);
            assert_eq!(error.details["publication"], "possible");
            assert_eq!(error.path.as_deref(), Some(ending.as_path()));
            assert_eq!(
                error.details["partial"]["uncertain_paths"],
                json!([crate::core::coordination::encode_path(&ending)])
            );
            assert!(
                error.details["partial"]["created"]
                    .as_array()
                    .unwrap()
                    .is_empty()
            );
            assert!(!ClaimStore::inspect(&guard, &claim.id).unwrap().current);
            assert_eq!(ClaimStore::list(&guard, None, false).unwrap().len(), 1);
        }
    }

    #[test]
    fn known_published_ending_is_reported_once_with_underlying_provenance() {
        let fixture = Fixture::new();
        let guard = fixture.guard();
        let claim = claim(&guard);
        let ending = ClaimEnding {
            store_id: claim.store_id.clone(),
            recovery_generation: claim.recovery_generation.clone(),
            claim_id: claim.id.clone(),
            item_id: claim.item_id.clone(),
            ended_at: now_timestamp(),
            actor: "worker".into(),
            reason: "".into(),
            outcome: ClaimOutcome::Released,
            recovery: false,
        };
        let path = guard.root_path().join(ending_path(&claim.id));
        let mut cause =
            ExecutionError::new("io", "verification failed after publication").at(&path);
        cause.details = json!({"publication":"published","errno":5});
        let error = ending_error(&guard, &ending, cause);
        assert_eq!(error.path.as_deref(), Some(path.as_path()));
        assert_eq!(error.details["errno"], 5);
        assert_eq!(error.details["publication"], "published");
        assert_eq!(
            error.details["partial"]["created"],
            json!([{"id":claim.id,"path":crate::core::coordination::encode_path(&path)}])
        );
        let replacement = new_id().unwrap();
        let error = reassignment_error(&guard, &claim.id, &replacement, false, error);
        assert_eq!(
            error.details["partial"]["created"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(error.details["partial"]["created"][0]["id"], claim.id);
    }
}
