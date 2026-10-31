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

/// How many standing CI slot sets the nominated team holds. A CI run leases one member of
/// the pool for its whole session, so this many runs can execute the live lanes at once,
/// each over documents no other run reads, writes or counts; twice the eight concurrent runs
/// the lanes are sized for, at 4 documents and 1 project per member, 64 of the team's 1000
/// documents.
pub const POOL_SIZE: u8 = 16;

/// CI identities are stable; local identities belong to one registered invocation.
///
/// A CI identity is one member of the standing pool (`pool` is `Some`), or the single
/// pre-pool standing set (`pool` is `None`), which is still recognized so nothing retires
/// it, and which no lane uses any more.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scope<'a> {
    team: &'a str,
    stamp: Option<&'a str>,
    pool: Option<u8>,
}
impl<'a> Scope<'a> {
    /// Validate the canonical team key of the pre-pool standing CI set.
    pub fn ci(team: &'a str) -> Result<Self, String> {
        if !team_valid(team) {
            return Err("invalid retained Linear team identity".into());
        }
        Ok(Self {
            team,
            stamp: None,
            pool: None,
        })
    }

    /// Validate the team key and a member `index` of the standing pool, `1..=POOL_SIZE`.
    pub fn pooled(team: &'a str, index: u8) -> Result<Self, String> {
        if !(1..=POOL_SIZE).contains(&index) {
            return Err(format!(
                "invalid retained Linear pool member {index}; the pool is 1..={POOL_SIZE}"
            ));
        }
        Ok(Self {
            pool: Some(index),
            ..Self::ci(team)?
        })
    }

    /// The pool member this standing set is, absent for a local set and the pre-pool one.
    #[must_use]
    pub fn pool_index(self) -> Option<u8> {
        self.pool
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
    let suffix = match (scope.local_stamp(), scope.pool_index()) {
        (None, None) => format!("ci {} {}", scope.team(), slot.name()),
        (None, Some(index)) => format!("ci {} {} pool-{index}", scope.team(), slot.name()),
        (Some(stamp), _) => format!("local {} {} {stamp}", scope.team(), slot.name()),
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
        ["ci", team, slot, pool] => (
            Scope::pooled(team, pool_index_read(pool)?).ok()?,
            Slot::read(slot)?,
        ),
        ["local", team, slot, stamp] => (Scope::local(team, stamp).ok()?, Slot::read(slot)?),
        _ => return None,
    };
    Some(Retained { scope, slot })
}

fn pool_index_read(token: &str) -> Option<u8> {
    let digits = token.strip_prefix("pool-")?;
    if digits.starts_with('0') || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

/// Which lane holds a pool member's lease: the plugin lane's three document slots and the
/// parent project, or the asset lane's one destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseLane {
    /// `onetaskgraph-linear`'s capability journey.
    Plugin,
    /// `onetaskgraph-linear-e2e`'s asset journey.
    Assets,
}
impl LeaseLane {
    fn name(self) -> &'static str {
        match self {
            Self::Plugin => "plugin",
            Self::Assets => "assets",
        }
    }
    fn read(value: &str) -> Option<Self> {
        match value {
            "plugin" => Some(Self::Plugin),
            "assets" => Some(Self::Assets),
            _ => None,
        }
    }
}

/// Who a lease names: a live holder's ownership stamp, or nobody once it was released.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeaseHolder<'a> {
    /// Held by the invocation carrying this canonical stamp.
    Held(&'a str),
    /// Released by its holder; the next generation may be claimed.
    Released,
}

/// One generation of the lease over one pool member, read off a lease issue's title.
///
/// A generation is claimed by creating the issue whose id [`lease_id`] derives from it,
/// which Linear refuses when that id already exists, so at most one invocation ever holds a
/// generation; the newest generation of a member is its current lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lease<'a> {
    /// The nominated team the pool belongs to.
    pub team: &'a str,
    /// The lane whose slots this member's lease covers.
    pub lane: LeaseLane,
    /// The pool member, `1..=POOL_SIZE`.
    pub index: u8,
    /// The generation, from 1, rising by one at every claim.
    pub generation: u32,
    /// Who holds it.
    pub holder: LeaseHolder<'a>,
}

/// The title of a lease issue; `team` and a held stamp are validated as a scope's are.
pub fn lease_title(lease: Lease<'_>) -> Result<String, String> {
    let scope = Scope::pooled(lease.team, lease.index)?;
    let holder = match lease.holder {
        LeaseHolder::Held(stamp) => {
            if !lease_stamp_valid(stamp) {
                return Err("invalid Linear lease holder stamp".into());
            }
            stamp
        }
        LeaseHolder::Released => "released",
    };
    if lease.generation == 0 {
        return Err("a Linear lease generation starts at 1".into());
    }
    Ok(format!(
        "{LIVE_ARTIFACT_TITLE_PREFIX}lease-v1 {} {} pool-{} gen-{} {holder}",
        scope.team(),
        lease.lane.name(),
        lease.index,
        lease.generation
    ))
}

/// The title prefix every lease of `team`'s `lane` begins with, for a listing filter.
#[must_use]
pub fn lease_title_prefix(team: &str, lane: LeaseLane) -> String {
    format!(
        "{LIVE_ARTIFACT_TITLE_PREFIX}lease-v1 {team} {} ",
        lane.name()
    )
}

/// Recognize only exact lease titles; anything else confers no lease and no authority.
#[must_use]
pub fn classify_lease(title: &str) -> Option<Lease<'_>> {
    let suffix = title
        .strip_prefix(LIVE_ARTIFACT_TITLE_PREFIX)?
        .strip_prefix("lease-v1 ")?;
    let parts: Vec<_> = suffix.split(' ').collect();
    let [team, lane, pool, generation, holder] = parts.as_slice() else {
        return None;
    };
    let index = pool_index_read(pool)?;
    Scope::pooled(team, index).ok()?;
    let digits = generation.strip_prefix("gen-")?;
    if digits.starts_with('0') || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let holder = match *holder {
        "released" => LeaseHolder::Released,
        stamp if lease_stamp_valid(stamp) => LeaseHolder::Held(stamp),
        _ => return None,
    };
    Some(Lease {
        team,
        lane: LeaseLane::read(lane)?,
        index,
        generation: digits.parse().ok()?,
        holder,
    })
}

/// A lease is held under the holder's machine stamp, `<host>-<process>-<micros>`: a local
/// set's canonical grammar, except that a host of `0` — a run no registry vouches for, as on
/// a runner with none — still names a holder, because what a lease needs of its holder is
/// only that no other claimant spells the same stamp.
fn lease_stamp_valid(stamp: &str) -> bool {
    stamp_valid(stamp)
        || stamp
            .strip_prefix("0-")
            .is_some_and(|rest| stamp_valid(&format!("1-{rest}")))
}

/// Whether `title` is any live-test identity that only its own lifecycle may remove: a
/// standing slot, a lease, or a malformed or future version of either, which confers no
/// authority to anyone.
#[must_use]
pub fn is_reserved_title(title: &str) -> bool {
    title
        .strip_prefix(LIVE_ARTIFACT_TITLE_PREFIX)
        .is_some_and(|suffix| suffix.starts_with("retained-") || suffix.starts_with("lease-"))
}
