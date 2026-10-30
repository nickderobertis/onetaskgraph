//! Retained live-test identities. Recognition establishes ownership syntax only; callers
//! must independently verify the nominated team before mutating a discovered item.
use crate::LIVE_ARTIFACT_TITLE_PREFIX;

/// Four document slots and the project that holds filed documents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// Project-filed document probe.
    Filed,
    /// Team-owned document without a project.
    Loose,
    /// Metadata and follow-up update probe.
    FollowUp,
    /// Binary copy and recopy destination.
    Assets,
    /// Retained project, separate from the four document slots.
    Parent,
}
impl Slot {
    fn name(self) -> &'static str {
        match self {
            Self::Filed => "filed",
            Self::Loose => "loose",
            Self::FollowUp => "follow-up",
            Self::Assets => "assets",
            Self::Parent => "parent",
        }
    }
    fn read(value: &str) -> Option<Self> {
        match value {
            "filed" => Some(Self::Filed),
            "loose" => Some(Self::Loose),
            "follow-up" => Some(Self::FollowUp),
            "assets" => Some(Self::Assets),
            "parent" => Some(Self::Parent),
            _ => None,
        }
    }
}

/// CI identities are stable; local identities belong to one registered invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope<'a> {
    team: &'a str,
    stamp: Option<&'a str>,
}
impl<'a> Scope<'a> {
    /// Validate the canonical team key of the standing CI set.
    pub fn ci(team: &'a str) -> Result<Self, String> {
        if !team_valid(team) {
            return Err("invalid retained Linear team identity".into());
        }
        Ok(Self { team, stamp: None })
    }

    /// Validate the team key and a canonical, positively vouched local ownership stamp.
    pub fn local(team: &'a str, stamp: &'a str) -> Result<Self, String> {
        let scope = Self::ci(team)?;
        if !stamp_valid(stamp) {
            return Err("invalid retained Linear local ownership stamp".into());
        }
        Ok(Self {
            stamp: Some(stamp),
            ..scope
        })
    }

    /// The validated canonical nominated team key.
    #[must_use]
    pub fn team(self) -> &'a str {
        self.team
    }

    /// The validated invocation stamp, absent for the standing CI set.
    #[must_use]
    pub fn local_stamp(self) -> Option<&'a str> {
        self.stamp
    }

    /// Whether this is the standing CI set.
    #[must_use]
    pub fn is_ci(self) -> bool {
        self.stamp.is_none()
    }
}

/// Exact recognized identity; this alone does not verify remote team membership.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Retained<'a> {
    /// Who owns this standing set.
    pub scope: Scope<'a>,
    /// Which probe or parent this object represents.
    pub slot: Slot,
}

fn team_valid(team: &str) -> bool {
    !team.is_empty()
        && team
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_' || c == b'-')
}
fn stamp_valid(stamp: &str) -> bool {
    let parts: Vec<_> = stamp.split('-').collect();
    parts.len() == 3
        && parts.iter().all(|part| {
            !part.starts_with('0')
                && part.bytes().all(|c| c.is_ascii_digit())
                && part.parse::<u64>().is_ok_and(|value| value > 0)
        })
        && parts[..2].iter().all(|part| part.parse::<u32>().is_ok())
}

/// Format an identity whose team and stamp were validated at scope construction.
pub fn retained_title(scope: Scope<'_>, slot: Slot) -> Result<String, String> {
    let suffix = match scope.local_stamp() {
        None => format!("ci {} {}", scope.team(), slot.name()),
        Some(stamp) => format!("local {} {} {stamp}", scope.team(), slot.name()),
    };
    Ok(format!("{LIVE_ARTIFACT_TITLE_PREFIX}retained-v1 {suffix}"))
}

/// Recognize only exact identities. Unknown versions, extra tokens and malformed markers
/// confer no authority, including authority to clean up an item.
#[must_use]
pub fn classify_retained(title: &str) -> Option<Retained<'_>> {
    let suffix = title
        .strip_prefix(LIVE_ARTIFACT_TITLE_PREFIX)?
        .strip_prefix("retained-v1 ")?;
    let parts: Vec<_> = suffix.split(' ').collect();
    let (scope, slot) = match parts.as_slice() {
        ["ci", team, slot] => (Scope::ci(team).ok()?, Slot::read(slot)?),
        ["local", team, slot, stamp] => (Scope::local(team, stamp).ok()?, Slot::read(slot)?),
        _ => return None,
    };
    Some(Retained { scope, slot })
}
