//! The per-run lease over one member of the standing Linear slot pool.
//!
//! **Why a lease and not a lock.** The team's document limit counts trashed documents, so a
//! run cannot create and delete its own documents on every pass; it reuses a standing set.
//! A standing set that two runs reuse at once is shared state, and serialising every run
//! behind one queue was what made a pull request wait hours. So the team holds
//! [`POOL_SIZE`] standing sets, and each run claims one of them — its own, for as long as it
//! runs — by leasing it. No run waits on another while a member is free, and eight runs at
//! once leave half the pool unclaimed.
//!
//! **What makes a claim exclusive.** Generation `g` of member `k` is the issue whose id
//! [`lease_id`] derives from `(team, lane, k, g)`. Linear refuses to create an issue under
//! an id it already holds, trashed or not, so of every invocation that tries to create that
//! id exactly one succeeds. A claimant reads which generation is newest, and creates the
//! next; whether it won is read back off the issue the id names, never inferred from the
//! create's answer, so a create whose acknowledgement was lost still decides correctly.
//!
//! **Who may claim the next generation.** Only when the newest one is released, or was
//! claimed longer ago than [`STALE_AFTER`]: no GitHub-hosted job runs longer than six hours,
//! so a lease older than that belongs to a job that has ended — positive evidence its run is
//! over, read without a GitHub credential, which the Linear lane does not hold. A superseded
//! generation is deleted by the claimant that superseded it; its id is never asked for
//! again, because generations only rise.
use super::send;
use onetaskgraph_linear::live_artifact::{
    HolderStamp, Lease, LeaseHolder, LeaseLane, POOL_SIZE, Scope, classify_lease, lease_title,
    lease_title_prefix,
};

fn generation(value: u32) -> Result<std::num::NonZeroU32, String> {
    std::num::NonZeroU32::new(value).ok_or_else(|| "a Linear lease generation starts at 1".into())
}
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::time::Duration;

/// A lease claimed longer ago than this belongs to a job GitHub has already ended.
// llmlint: ignore[contracts_have_one_source_or_a_drift_gate] GitHub documents a hosted job's six-hour maximum in prose alone ("Each job in a workflow can run for up to 6 hours of execution time"), and nothing machine-readable states it; this adds half an hour of margin. It is load-bearing: were GitHub to raise that maximum, a job running past this bound could have its member reclaimed by another run while it still worked on it, so this constant has to move with GitHub's maximum.
pub const STALE_AFTER: Duration = Duration::from_secs(6 * 60 * 60 + 30 * 60);

/// How many times the whole pool is tried before a claim is refused. A member another
/// claimant won between this one's read and its create is simply passed over, so a second
/// round exists only for the read lagging a release.
const ROUNDS: u32 = 3;

const LEASES: &str = "query($after:String,$filter:IssueFilter!){issues(first:100,after:$after,filter:$filter){nodes{id title createdAt} pageInfo{hasNextPage endCursor}}}";
const LEASE: &str = "query($id:String!){issue(id:$id){id title createdAt}}";
const LEASE_CREATE: &str =
    "mutation($input:IssueCreateInput!){issueCreate(input:$input){success issue{id}}}";
const LEASE_UPDATE: &str = "mutation($id:String!,$input:IssueUpdateInput!){issueUpdate(id:$id,input:$input){success issue{id}}}";
const LEASE_DELETE: &str = "mutation($id:String!){issueDelete(id:$id){success}}";

/// The id generation `generation` of `team`'s `lane` lease over member `index` is created
/// under: deterministic, so every claimant of that generation asks for the same one, and
/// shaped as the version-4 UUID Linear's `IssueCreateInput.id` documents.
#[must_use]
pub fn lease_id(team: &str, lane: LeaseLane, index: u8, generation: u32) -> String {
    use sha2::{Digest, Sha256};
    let lane = match lane {
        LeaseLane::Plugin => "plugin",
        LeaseLane::Assets => "assets",
    };
    let digest = Sha256::digest(format!(
        "onetaskgraph-live-lease-v1\0{team}\0{lane}\0{index}\0{generation}"
    ));
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// One member of the pool this invocation holds, until [`Held::release`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Held {
    pub team: String,
    pub lane: LeaseLane,
    pub index: u8,
    pub generation: u32,
    pub id: String,
    /// The title the lease issue carries while this invocation holds it.
    pub title: String,
}
impl Held {
    /// The standing scope the leased member names.
    pub fn scope(&self) -> Scope<'_> {
        Scope::pooled(&self.team, self.index).expect("a held lease names a valid pool member")
    }

    /// Releases the lease, so the next claimant may take this member at once. Run on every
    /// way out of a lane, its failures included; what a lost release costs is that member
    /// for [`STALE_AFTER`], never another run's isolation.
    pub async fn release(&self, endpoint: &str, key: &str) -> Result<(), String> {
        let title = lease_title(Lease::new(
            self.scope(),
            self.lane,
            generation(self.generation)?,
            LeaseHolder::Released,
        )?);
        let data = send(
            endpoint,
            key,
            LEASE_UPDATE,
            json!({"id":self.id,"input":{"title":title}}),
        )
        .await
        .map_err(|error| format!("pool member {} lease was not released: {error}", self.index))?;
        if data["issueUpdate"]["success"] != true {
            return Err(format!(
                "Linear did not confirm releasing pool member {}'s lease",
                self.index
            ));
        }
        Ok(())
    }
}

/// The newest generation of each member's lease, and every older one still present.
struct Leases {
    newest: BTreeMap<u8, (u32, String, LeaseState)>,
    superseded: Vec<(u8, String)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LeaseState {
    Free,
    Held,
}

fn created_micros(node: &Value) -> Result<u64, String> {
    let created = node["createdAt"]
        .as_str()
        .ok_or("a lease issue answered no createdAt")?;
    let at = chrono::DateTime::parse_from_rfc3339(created)
        .map_err(|_| format!("a lease issue answered an unreadable createdAt {created:?}"))?;
    u64::try_from(at.timestamp_micros()).map_err(|_| "a lease issue predates the epoch".into())
}

fn state_of(lease: &Lease<'_>, created: u64, now: u64) -> LeaseState {
    match lease.holder() {
        LeaseHolder::Released => LeaseState::Free,
        LeaseHolder::Held(_) if now.saturating_sub(created) > STALE_AFTER.as_micros() as u64 => {
            LeaseState::Free
        }
        LeaseHolder::Held(_) => LeaseState::Held,
    }
}

async fn read_leases(
    endpoint: &str,
    key: &str,
    team: &str,
    lane: LeaseLane,
    now: u64,
) -> Result<Leases, String> {
    let prefix = lease_title_prefix(team, lane);
    let filter = json!({"title":{"startsWith":prefix},"team":{"key":{"eq":team}}});
    let mut after = Value::Null;
    let mut newest: BTreeMap<u8, (u32, String, LeaseState)> = BTreeMap::new();
    let mut superseded = Vec::new();
    for _ in 0..20 {
        let data = send(
            endpoint,
            key,
            LEASES,
            json!({"after":after,"filter":filter}),
        )
        .await?;
        let page = &data["issues"];
        for node in page["nodes"]
            .as_array()
            .ok_or("the lease listing answered no nodes")?
        {
            // A row that does not say what it is could be the newest held lease: fail closed.
            let title = node["title"]
                .as_str()
                .ok_or("a lease issue answered no title")?;
            let id = node["id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or("a lease issue answered no id")?;
            // A title under the prefix this lane does not recognize confers nothing, and is
            // not this lane's to remove either.
            let Some(lease) = classify_lease(title).filter(|lease| lease.lane() == lane) else {
                continue;
            };
            if !lease.team().eq_ignore_ascii_case(team)
                || id != lease_id(team, lane, lease.index(), lease.generation())
            {
                continue;
            }
            let state = state_of(&lease, created_micros(node)?, now);
            match newest.get(&lease.index()) {
                Some((generation, _, _)) if *generation >= lease.generation() => {
                    superseded.push((lease.index(), id.to_owned()));
                }
                _ => {
                    if let Some((_, older, _)) =
                        newest.insert(lease.index(), (lease.generation(), id.to_owned(), state))
                    {
                        superseded.push((lease.index(), older));
                    }
                }
            }
        }
        match page
            .pointer("/pageInfo/hasNextPage")
            .and_then(Value::as_bool)
        {
            Some(false) => return Ok(Leases { newest, superseded }),
            Some(true) => {
                let next = page
                    .pointer("/pageInfo/endCursor")
                    .and_then(Value::as_str)
                    .ok_or("the lease listing answered no cursor")?;
                if after.as_str() == Some(next) {
                    return Err("the lease listing cursor did not advance".into());
                }
                after = json!(next);
            }
            None => return Err("the lease listing answered no pagination".into()),
        }
    }
    Err("the lease listing did not terminate".into())
}

/// Whether the issue `id` names now carries `title`: the one reading that decides a claim.
async fn reads_as(endpoint: &str, key: &str, id: &str, title: &str) -> bool {
    send(endpoint, key, LEASE, json!({"id":id}))
        .await
        .is_ok_and(|data| {
            data.pointer("/issue/id").and_then(Value::as_str) == Some(id)
                && data.pointer("/issue/title").and_then(Value::as_str) == Some(title)
        })
}

/// Claims one free member of `lane`'s pool for `holder`, a canonical machine stamp.
///
/// Members are tried from one `holder` derives, so concurrent claimants mostly start at
/// different members rather than contending for the first. A member whose newest lease is
/// held, or whose next generation another claimant created first, is passed over; every
/// member held is a refusal naming the pool, never a wait on another run.
pub async fn claim(
    endpoint: &str,
    key: &str,
    team: &str,
    team_id: &str,
    lane: LeaseLane,
    holder: &str,
    now: impl Fn() -> u64,
) -> Result<Held, String> {
    use sha2::{Digest, Sha256};
    let start = Sha256::digest(holder.as_bytes())[0] % POOL_SIZE;
    let order: Vec<u8> = (0..POOL_SIZE)
        .map(|offset| (start + offset) % POOL_SIZE + 1)
        .collect();
    for round in 0..ROUNDS {
        if round > 0 {
            tokio::time::sleep(Duration::from_secs(5)).await;
        }
        if let Some(held) =
            claim_among(endpoint, key, team, team_id, lane, holder, &now, &order).await?
        {
            return Ok(held);
        }
    }
    Err(format!(
        "every one of the {POOL_SIZE} standing Linear slot sets for this lane is leased by a \
         run still going; no document was touched. A lease frees when its run releases it, or \
         {} hours after it was claimed",
        STALE_AFTER.as_secs_f64() / 3600.0
    ))
}

/// Asks Linear to create `held`'s issue a second time, under the same id, and requires the
/// refusal that every claim's exclusivity rests on: a Linear that took the second create would
/// let two claimants hold one member, so the lane fails here, naming that, rather than run.
/// The issue is read back afterwards to show it still names this holder.
pub async fn verify_exclusive(
    endpoint: &str,
    key: &str,
    team_id: &str,
    held: &Held,
) -> Result<(), String> {
    let held_title = lease_title(Lease::new(
        held.scope(),
        held.lane,
        generation(held.generation)?,
        LeaseHolder::Released,
    )?);
    let decoy = format!("{held_title} duplicate-probe");
    // Only Linear's own answer refusing the create is proof: a GraphQL error, or a create it
    // says did not succeed. A request that never got an answer proves nothing either way.
    let answered: Value = reqwest::Client::new()
        .post(endpoint)
        .header("Authorization", key)
        .json(&json!({"query":LEASE_CREATE,"variables":{"input":{"id":held.id,"title":decoy,"teamId":team_id}}}))
        .send()
        .await
        .map_err(|error| format!("the lease exclusivity probe got no answer: {error}"))?
        .json()
        .await
        .map_err(|error| format!("the lease exclusivity probe got no readable answer: {error}"))?;
    let refused = answered
        .get("errors")
        .and_then(Value::as_array)
        .is_some_and(|errors| !errors.is_empty())
        || answered.pointer("/data/issueCreate/success") == Some(&json!(false));
    let still = send(endpoint, key, LEASE, json!({"id":held.id})).await?;
    let title = still.pointer("/issue/title").and_then(Value::as_str);
    if !refused || title == Some(decoy.as_str()) {
        return Err(format!(
            "Linear did not refuse a second create under pool member {}'s lease id {}, so a lease is \
             not exclusive and no run may lease a member; the pool's leases rest on that refusal",
            held.index, held.id
        ));
    }
    // And the lease is still this invocation's, exactly as it was claimed.
    if still.pointer("/issue/id").and_then(Value::as_str) != Some(held.id.as_str())
        || title != Some(held.title.as_str())
    {
        return Err(format!(
            "pool member {}'s lease {} no longer reads as this run's ({title:?}); nothing is done \
             over a member whose lease cannot be read back",
            held.index, held.id
        ));
    }
    Ok(())
}

/// Claims member `index` alone, once: `None` when a run still going holds it.
#[allow(
    clippy::too_many_arguments,
    reason = "claim's lane identities plus the one member asked for"
)]
pub async fn claim_member(
    endpoint: &str,
    key: &str,
    team: &str,
    team_id: &str,
    lane: LeaseLane,
    holder: &str,
    now: impl Fn() -> u64,
    index: u8,
) -> Result<Option<Held>, String> {
    claim_among(endpoint, key, team, team_id, lane, holder, &now, &[index]).await
}

#[allow(
    clippy::too_many_arguments,
    reason = "claim's lane identities plus the members to try, each forwarded unchanged"
)]
async fn claim_among(
    endpoint: &str,
    key: &str,
    team: &str,
    team_id: &str,
    lane: LeaseLane,
    holder: &str,
    now: &impl Fn() -> u64,
    order: &[u8],
) -> Result<Option<Held>, String> {
    {
        let leases = read_leases(endpoint, key, team, lane, now()).await?;
        for &index in order {
            let generation = match leases.newest.get(&index) {
                Some((_, _, LeaseState::Held)) => continue,
                Some((generation, _, LeaseState::Free)) => generation
                    .checked_add(1)
                    .ok_or_else(|| format!("pool member {index}'s lease generations ran out"))?,
                None => 1,
            };
            let lease = Lease::new(
                Scope::pooled(team, index)?,
                lane,
                self::generation(generation)?,
                LeaseHolder::Held(HolderStamp::new(holder)?),
            )?;
            let title = lease_title(lease);
            let id = lease_id(team, lane, index, generation);
            // Refused when the id is already held: that is another claimant's win, and the
            // read below says so. A lost acknowledgement is decided by the same read.
            let _ = send(
                endpoint,
                key,
                LEASE_CREATE,
                json!({"input":{"id":id,"title":title,"teamId":team_id}}),
            )
            .await;
            if !reads_as(endpoint, key, &id, &title).await {
                continue;
            }
            // The generation this one superseded, and any older left behind, are dead: no
            // claimant ever asks for their ids again. Deleting them keeps the listing short.
            let mut stale: Vec<String> = leases
                .superseded
                .iter()
                .filter(|(member, _)| *member == index)
                .map(|(_, id)| id.clone())
                .collect();
            if let Some((_, older, _)) = leases.newest.get(&index) {
                stale.push(older.clone());
            }
            for older in stale {
                if let Err(error) = send(endpoint, key, LEASE_DELETE, json!({"id":older})).await {
                    eprintln!("a superseded lease of pool member {index} was left: {error}");
                }
            }
            return Ok(Some(Held {
                team: team.to_owned(),
                lane,
                index,
                generation,
                id,
                title,
            }));
        }
    }
    Ok(None)
}

/// Runs `body` holding `lease` — `None` for a local run, which leases nothing — and releases
/// the lease on every way out, a panic included, before the outcome or the panic leaves.
/// A release that fails is printed, never raised over the body's own outcome.
#[allow(
    dead_code,
    reason = "this module is compiled into both lanes' test binaries, and the asset lane blocks on its runtime by hand instead"
)]
pub async fn holding<F: std::future::Future>(
    lease: Option<&Held>,
    endpoint: &str,
    key: &str,
    body: F,
) -> F::Output {
    let outcome = super::unwinding(body).await;
    if let Some(lease) = lease
        && let Err(error) = lease.release(endpoint, key).await
    {
        eprintln!("{error}");
    }
    outcome.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

/// [`holding`] for a lane that drives its runtime by hand.
#[allow(
    dead_code,
    reason = "this module is compiled into both lanes' test binaries, and only the asset lane blocks on its runtime by hand"
)]
pub fn holding_blocking<T>(
    runtime: &tokio::runtime::Runtime,
    lease: Option<&Held>,
    endpoint: &str,
    key: &str,
    body: impl FnOnce() -> T,
) -> T {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body));
    if let Some(lease) = lease
        && let Err(error) = runtime.block_on(lease.release(endpoint, key))
    {
        eprintln!("{error}");
    }
    outcome.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}
