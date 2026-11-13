//! The speed comparison: prepare a user, import one that is not mirrored, and time one scenario
//! per request so the UI can fill in results as they arrive.

use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use lfm_store::{Account, JobStatus, MirroredUser};
use lfm_upstream::{CallOptions, Priority, User};
use serde::{Deserialize, Serialize};

use crate::demo::{Demo, Outcome, OwnImport, RacePage, ScenarioView, Side, facts, scenarios};
use crate::error::WebError;
use crate::routes::lastfm::{ImportView, import_view, lastfm_username};
use crate::session::Member;
use crate::state::{AppState, unix_now};
use crate::throttle::Throttle;

/// `{"username": "…"}`, in a body or the query.
#[derive(Debug, Deserialize)]
pub(crate) struct UserBody {
    username: String,
}

/// `{"username": "…", "scenario": "…"}`.
#[derive(Debug, Deserialize)]
pub(crate) struct RunBody {
    username: String,
    scenario: String,
}

/// `{"username": "…", "side": "lastfm" | "mirror", "page": 1, "tail": 30}`.
#[derive(Debug, Deserialize)]
pub(crate) struct RaceBody {
    username: String,
    side: Side,
    page: u32,
    /// How many of the page's oldest scrobbles to send back.
    tail: usize,
}

/// The most scrobbles one race page sends back.
const MAX_TAIL: usize = 50;

/// What the comparison can do for a user.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Prepared {
    /// The name as Last.fm spells it.
    username: String,
    /// `mirrored`, `importing` or `not_mirrored`.
    state: &'static str,
    /// Scrobbles on Last.fm, or mirrored here.
    playcount: u64,
    /// The largest history a demo import copies.
    max_scrobbles: u64,
    import: Option<ImportView>,
    /// For a mirrored user, the scenarios to run in order.
    scenarios: Vec<ScenarioView>,
}

/// Where a user stands on this instance.
enum Standing {
    Mirrored(Box<MirroredUser>),
    Importing(ImportView),
    /// Not mirrored, perhaps after a failed import.
    Absent(Option<ImportView>),
}

/// `POST /demo/prepare`.
pub(crate) async fn prepare(
    State(state): State<AppState>,
    Member(account): Member,
    Json(body): Json<UserBody>,
) -> Result<Json<Prepared>, WebError> {
    let demo = state.demo()?;
    limit(&demo.prepares, &account)?;
    let name = lastfm_username(&body.username)?;
    let prepared = match standing(&state, demo, &account, name).await? {
        Standing::Mirrored(user) => {
            let facts = facts(&state.store, &user).await?;
            Prepared {
                username: user.name.clone(),
                state: "mirrored",
                playcount: user.stats.playcount,
                max_scrobbles: demo.config.max_scrobbles,
                import: None,
                scenarios: scenarios(&facts).iter().map(ScenarioView::from).collect(),
            }
        }
        Standing::Importing(view) => Prepared {
            username: name.to_owned(),
            state: "importing",
            playcount: view.estimated_total,
            max_scrobbles: demo.config.max_scrobbles,
            import: Some(view),
            scenarios: Vec::new(),
        },
        Standing::Absent(failed) => {
            let user = lookup(&state, name).await?;
            Prepared {
                username: user.name,
                state: "not_mirrored",
                playcount: user.playcount,
                max_scrobbles: demo.config.max_scrobbles,
                import: failed,
                scenarios: Vec::new(),
            }
        }
    };
    Ok(Json(prepared))
}

/// `POST /demo/import`: copies the user's history up to now, once, without following them.
pub(crate) async fn import(
    State(state): State<AppState>,
    Member(account): Member,
    Json(body): Json<UserBody>,
) -> Result<Json<ImportView>, WebError> {
    let demo = state.demo()?;
    let name = lastfm_username(&body.username)?;
    match standing(&state, demo, &account, name).await? {
        Standing::Mirrored(user) => return Ok(Json(done(&user))),
        Standing::Importing(view) => return Ok(Json(view)),
        Standing::Absent(_) => {}
    }
    if demo.importing(account.id) {
        return Err(WebError::conflict(
            "demo_import_running",
            "Wait for your other import to finish.",
        ));
    }
    limit(&demo.import_starts, &account)?;
    let user = lookup(&state, name).await?;
    if user.playcount > demo.config.max_scrobbles {
        let message = format!(
            "This history has {} scrobbles. The demo imports up to {}.",
            user.playcount, demo.config.max_scrobbles
        );
        return Err(WebError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "history_too_large",
            message,
        ));
    }
    let task = state.sync.spawn_import(&user.name, unix_now());
    demo.started(account.id, &user.name, task);
    tracing::info!(account = account.id.0, user = %user.name, "demo import started");
    Ok(Json(ImportView {
        status: JobStatus::Queued.as_str(),
        imported: 0,
        estimated_total: user.playcount,
        error: None,
    }))
}

/// `GET /demo/import?username=`.
pub(crate) async fn progress(
    State(state): State<AppState>,
    Member(account): Member,
    Query(query): Query<UserBody>,
) -> Result<Json<ImportView>, WebError> {
    let demo = state.demo()?;
    let name = lastfm_username(&query.username)?;
    match standing(&state, demo, &account, name).await? {
        Standing::Mirrored(user) => Ok(Json(done(&user))),
        Standing::Importing(view) | Standing::Absent(Some(view)) => Ok(Json(view)),
        Standing::Absent(None) => Err(WebError::not_found()),
    }
}

/// `POST /demo/run`: times one scenario on Last.fm and on this mirror.
pub(crate) async fn run(
    State(state): State<AppState>,
    Member(account): Member,
    Json(body): Json<RunBody>,
) -> Result<Json<Outcome>, WebError> {
    let demo = state.demo()?;
    limit(&demo.runs, &account)?;
    let user = mirrored(&state, &body.username).await?;
    let facts = facts(&state.store, &user).await?;
    let scenario = scenarios(&facts)
        .into_iter()
        .find(|scenario| scenario.id == body.scenario)
        .ok_or_else(|| {
            WebError::new(
                StatusCode::NOT_FOUND,
                "unknown_scenario",
                "No such scenario.",
            )
        })?;
    demo.measure(&user.name, &scenario).await.map(Json)
}

/// `POST /demo/race`: one page of a mirrored user's history, from Last.fm or from this mirror.
pub(crate) async fn race(
    State(state): State<AppState>,
    Member(account): Member,
    Json(body): Json<RaceBody>,
) -> Result<Json<RacePage>, WebError> {
    let demo = state.demo()?;
    if body.side == Side::Lastfm {
        limit(&demo.race_pages, &account)?;
    }
    let user = mirrored(&state, &body.username).await?;
    let page = body.page.max(1);
    let tail = body.tail.min(MAX_TAIL);
    demo.race_page(&user.name, body.side, page, tail)
        .await
        .map(Json)
}

/// The user, if this instance mirrors them.
async fn mirrored(state: &AppState, username: &str) -> Result<MirroredUser, WebError> {
    let name = lastfm_username(username)?;
    state
        .store
        .user_by_name(name)
        .await?
        .filter(|user| user.sync_cursor > 0)
        .ok_or_else(|| WebError::conflict("not_mirrored", "Import this user before comparing."))
}

async fn standing(
    state: &AppState,
    demo: &Demo,
    account: &Account,
    name: &str,
) -> Result<Standing, WebError> {
    let user = state.store.user_by_name(name).await?;
    if let Some(user) = user.as_ref().filter(|user| user.sync_cursor > 0) {
        return Ok(Standing::Mirrored(Box::new(user.clone())));
    }
    let job = match &user {
        Some(user) => state.sync.progress(user.id).await?,
        None => None,
    };
    let view = match (job, demo.own_import(account.id, name)) {
        (Some(job), _) => Some(import_view(job, false)),
        (None, OwnImport::Running) => Some(waiting()),
        (None, OwnImport::Ended) => Some(ImportView {
            status: JobStatus::Failed.as_str(),
            error: Some("The import could not start. Try again.".to_owned()),
            ..waiting()
        }),
        (None, OwnImport::None) => None,
    };
    Ok(match view {
        Some(view) if view.status != JobStatus::Failed.as_str() => Standing::Importing(view),
        view => Standing::Absent(view),
    })
}

/// The Last.fm profile of `name`, or a clear error when Last.fm has no such user.
async fn lookup(state: &AppState, name: &str) -> Result<User, WebError> {
    let options = CallOptions::new(Priority::Interactive);
    state
        .upstream
        .user_info(name, &options)
        .await
        .map_err(|error| match error {
            lfm_upstream::Error::LastFm { code: 6, .. } => WebError::new(
                StatusCode::NOT_FOUND,
                "lastfm_user_not_found",
                "Last.fm has no user with that name.",
            ),
            other => WebError::lastfm(&other),
        })
}

fn waiting() -> ImportView {
    ImportView {
        status: JobStatus::Queued.as_str(),
        imported: 0,
        estimated_total: 0,
        error: None,
    }
}

fn done(user: &MirroredUser) -> ImportView {
    ImportView {
        status: JobStatus::Done.as_str(),
        imported: user.stats.playcount,
        estimated_total: user.stats.playcount,
        error: None,
    }
}

fn limit(throttle: &Throttle, account: &Account) -> Result<(), WebError> {
    if throttle.take(&account.id.0.to_string(), unix_now()) {
        Ok(())
    } else {
        Err(WebError::rate_limited())
    }
}
