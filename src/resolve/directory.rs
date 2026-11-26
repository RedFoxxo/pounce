//! Reference data used to resolve names to ids (users, roles, teams, projects,
//! states, custom fields), loaded fully paged on first use and cached briefly.

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use regex::Regex;
use serde_json::{Map, Value, json};
use tokio::sync::OnceCell;

use super::matching::{MatchSpec, Resolved, Unresolved, r#match, match_active};
use crate::http::v1::{ListOptions, ReadOptions, V1, v1_string};
use crate::http::{Query, Res, err};
use crate::tools::Clock;

/// `{Id, Name}` as v1 nests it.
#[derive(Debug, Clone, PartialEq)]
pub struct V1Ref {
    pub id: i64,
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TpUser {
    pub id: i64,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub login: Option<String>,
    pub email: Option<String>,
    pub is_active: Option<bool>,
    pub delete_date: Option<String>,
    pub kind: Option<String>,
    pub role: Option<V1Ref>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TpRole {
    pub id: i64,
    pub name: String,
    pub has_effort: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TpTeam {
    pub id: i64,
    pub name: String,
    pub is_active: Option<bool>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TpProject {
    pub id: i64,
    pub name: String,
    pub abbreviation: Option<String>,
    pub is_active: Option<bool>,
    pub process: Option<V1Ref>,
}

/// A state's workflow; `parent_workflow` is set for team sub-workflows.
#[derive(Debug, Clone, PartialEq)]
pub struct TpWorkflow {
    pub id: i64,
    pub name: Option<String>,
    pub parent_workflow: Option<V1Ref>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TpState {
    pub id: i64,
    pub name: String,
    pub is_initial: Option<bool>,
    pub is_final: Option<bool>,
    pub is_planned: Option<bool>,
    pub numeric_priority: Option<f64>,
    pub workflow: Option<TpWorkflow>,
    pub parent_entity_state: Option<V1Ref>,
    pub role: Option<V1Ref>,
    pub entity_type: Option<V1Ref>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TpCustomField {
    pub id: i64,
    pub name: String,
    pub field_type: Option<String>,
    /// Newline-separated options for DropDown / MultipleSelectionList.
    pub value: Option<String>,
    pub required: Option<bool>,
    /// System fields (e.g. metrics) can only be set by Targetprocess itself.
    pub is_system: Option<bool>,
    /// `Config` present: its `CalculationModel` (`Some(None)` when that is null or missing).
    pub calculation_model: Option<Option<String>>,
    pub entity_type: Option<V1Ref>,
    pub process: Option<V1Ref>,
}

fn id_of(v: &Value) -> i64 {
    v.get("Id").and_then(Value::as_i64).unwrap_or(0)
}

fn str_of(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn bool_of(v: &Value, key: &str) -> Option<bool> {
    v.get(key).and_then(Value::as_bool)
}

fn ref_of(v: &Value, key: &str) -> Option<V1Ref> {
    let r = v.get(key).filter(|r| r.is_object())?;
    Some(V1Ref {
        id: id_of(r),
        name: str_of(r, "Name"),
    })
}

impl TpUser {
    pub fn from_value(v: &Value) -> Self {
        TpUser {
            id: id_of(v),
            first_name: str_of(v, "FirstName"),
            last_name: str_of(v, "LastName"),
            login: str_of(v, "Login"),
            email: str_of(v, "Email"),
            is_active: bool_of(v, "IsActive"),
            delete_date: str_of(v, "DeleteDate"),
            kind: str_of(v, "Kind"),
            role: ref_of(v, "Role"),
        }
    }
}

impl TpRole {
    pub fn from_value(v: &Value) -> Self {
        TpRole {
            id: id_of(v),
            name: str_of(v, "Name").unwrap_or_default(),
            has_effort: bool_of(v, "HasEffort"),
        }
    }
}

impl TpTeam {
    pub fn from_value(v: &Value) -> Self {
        TpTeam {
            id: id_of(v),
            name: str_of(v, "Name").unwrap_or_default(),
            is_active: bool_of(v, "IsActive"),
        }
    }
}

impl TpProject {
    pub fn from_value(v: &Value) -> Self {
        TpProject {
            id: id_of(v),
            name: str_of(v, "Name").unwrap_or_default(),
            abbreviation: str_of(v, "Abbreviation"),
            is_active: bool_of(v, "IsActive"),
            process: ref_of(v, "Process"),
        }
    }
}

impl TpState {
    pub fn from_value(v: &Value) -> Self {
        let workflow = v
            .get("Workflow")
            .filter(|w| w.is_object())
            .map(|w| TpWorkflow {
                id: id_of(w),
                name: str_of(w, "Name"),
                parent_workflow: ref_of(w, "ParentWorkflow"),
            });
        TpState {
            id: id_of(v),
            name: str_of(v, "Name").unwrap_or_default(),
            is_initial: bool_of(v, "IsInitial"),
            is_final: bool_of(v, "IsFinal"),
            is_planned: bool_of(v, "IsPlanned"),
            numeric_priority: v.get("NumericPriority").and_then(Value::as_f64),
            workflow,
            parent_entity_state: ref_of(v, "ParentEntityState"),
            role: ref_of(v, "Role"),
            entity_type: ref_of(v, "EntityType"),
        }
    }
}

impl TpCustomField {
    pub fn from_value(v: &Value) -> Self {
        TpCustomField {
            id: id_of(v),
            name: str_of(v, "Name").unwrap_or_default(),
            field_type: str_of(v, "FieldType"),
            value: str_of(v, "Value"),
            required: bool_of(v, "Required"),
            is_system: bool_of(v, "IsSystem"),
            calculation_model: v
                .get("Config")
                .filter(|c| c.is_object())
                .map(|c| str_of(c, "CalculationModel")),
            entity_type: ref_of(v, "EntityType"),
            process: ref_of(v, "Process"),
        }
    }
}

fn truthy(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|s| !s.is_empty())
}

pub fn full_name(u: &TpUser) -> String {
    let parts: Vec<&str> = [truthy(&u.first_name), truthy(&u.last_name)]
        .into_iter()
        .flatten()
        .collect();
    if !parts.is_empty() {
        return parts.join(" ");
    }
    match truthy(&u.login) {
        Some(login) => login.to_string(),
        None => format!("User {}", u.id),
    }
}

pub fn is_active_user(u: &TpUser) -> bool {
    u.is_active != Some(false) && truthy(&u.delete_date).is_none()
}

static OPTION_FIELD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(DropDown|MultipleSelectionList)$").unwrap());

/// Allowed values of a dropdown or multi-select custom field; `None` for free-form fields.
pub fn custom_field_options(field: &TpCustomField) -> Option<Vec<String>> {
    let kind = truthy(&field.field_type)?;
    if !OPTION_FIELD.is_match(kind) {
        return None;
    }
    Some(
        field
            .value
            .as_deref()
            .unwrap_or("")
            .split('\n')
            .map(|o| o.strip_suffix('\r').unwrap_or(o).trim())
            .filter(|o| !o.is_empty())
            .map(str::to_string)
            .collect(),
    )
}

pub fn is_team_workflow_state(state: &TpState) -> bool {
    state
        .workflow
        .as_ref()
        .is_some_and(|w| w.parent_workflow.is_some())
        || state.parent_entity_state.is_some()
}

const TTL_MS: i64 = 5 * 60 * 1000;
const DIRECTORY_LIMIT: usize = 20_000;

type Loaded = Res<Arc<dyn Any + Send + Sync>>;

struct Entry {
    at: i64,
    /// Shared by every caller while the load runs, so they make one request.
    value: Arc<OnceCell<Loaded>>,
}

/// Reference data used to resolve names to ids, loaded fully paged on first use
/// and cached briefly. Failed loads are not cached.
pub struct Directory {
    v1: V1,
    clock: Clock,
    cache: Mutex<HashMap<String, Entry>>,
}

const USER_INCLUDE: &str =
    "[Id,FirstName,LastName,Login,Email,IsActive,DeleteDate,Kind,Role[Id,Name]]";
const CUSTOM_FIELD_INCLUDE: &str =
    "[Id,Name,FieldType,Value,Required,IsSystem,Config,EntityType[Id,Name],Process[Id,Name]]";

impl Directory {
    pub fn new(v1: V1, clock: Clock) -> Self {
        Directory {
            v1,
            clock,
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn entries(&self) -> std::sync::MutexGuard<'_, HashMap<String, Entry>> {
        self.cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    async fn cached<T, F>(&self, key: String, load: impl FnOnce() -> F) -> Res<Arc<T>>
    where
        T: Send + Sync + 'static,
        F: Future<Output = Res<T>>,
    {
        let cell = {
            let mut entries = self.entries();
            let now = (self.clock.now_ms)();
            match entries.get(&key) {
                Some(hit) if now - hit.at < TTL_MS => hit.value.clone(),
                _ => {
                    let cell = Arc::new(OnceCell::new());
                    entries.insert(
                        key.clone(),
                        Entry {
                            at: now,
                            value: cell.clone(),
                        },
                    );
                    cell
                }
            }
        };
        let loaded = cell
            .get_or_init(|| async {
                let r = load().await;
                if r.is_err() {
                    self.entries().remove(&key);
                }
                r.map(|v| Arc::new(v) as Arc<dyn Any + Send + Sync>)
            })
            .await;
        match loaded {
            Ok(any) => Ok(any
                .clone()
                .downcast::<T>()
                .unwrap_or_else(|_| panic!("directory cache key {key} holds another type"))),
            Err(e) => Err(e.clone()),
        }
    }

    async fn all<T: Send + Sync + 'static>(
        &self,
        key: &str,
        path: &str,
        include: &str,
        parse: fn(&Value) -> T,
    ) -> Res<Arc<Vec<T>>> {
        self.cached(key.to_string(), || async {
            let options = ListOptions {
                read: ReadOptions { include: Some(include.into()), ..Default::default() },
                limit: Some(DIRECTORY_LIMIT),
                ..Default::default()
            };
            let r = self.v1.list(path, &options).await?;
            if r.truncated {
                return Err(err(
                    200,
                    format!("More than {DIRECTORY_LIMIT} {path}; names cannot be resolved reliably. Pass ids instead."),
                    "",
                ));
            }
            Ok(r.items.iter().map(parse).collect())
        })
        .await
    }

    pub async fn users(&self) -> Res<Arc<Vec<TpUser>>> {
        self.all("users", "Users", USER_INCLUDE, TpUser::from_value)
            .await
    }

    pub async fn roles(&self) -> Res<Arc<Vec<TpRole>>> {
        self.all("roles", "Roles", "[Id,Name,HasEffort]", TpRole::from_value)
            .await
    }

    pub async fn teams(&self) -> Res<Arc<Vec<TpTeam>>> {
        self.all("teams", "Teams", "[Id,Name,IsActive]", TpTeam::from_value)
            .await
    }

    pub async fn projects(&self) -> Res<Arc<Vec<TpProject>>> {
        self.all(
            "projects",
            "Projects",
            "[Id,Name,Abbreviation,IsActive,Process[Id,Name]]",
            TpProject::from_value,
        )
        .await
    }

    /// Every state of an entity type in a process, across the project workflow and team sub-workflows.
    pub async fn states(&self, process_id: i64, entity_type: &str) -> Res<Arc<Vec<TpState>>> {
        self.cached(format!("states:{process_id}:{entity_type}"), || async {
            let options = ListOptions {
                read: ReadOptions {
                    include: Some("[Id,Name,IsInitial,IsFinal,IsPlanned,NumericPriority,Workflow[Id,Name,ParentWorkflow],ParentEntityState[Id,Name],Role[Id,Name],EntityType[Id,Name]]".into()),
                    ..Default::default()
                },
                where_: Some(format!("(Process.Id eq {process_id}) and (EntityType.Name eq {})", v1_string(entity_type))),
                order_by: Some("NumericPriority".into()),
                limit: Some(DIRECTORY_LIMIT),
                ..Default::default()
            };
            let r = self.v1.list("EntityStates", &options).await?;
            if r.truncated {
                return Err(err(200, format!("More than {DIRECTORY_LIMIT} {entity_type} states; cannot resolve reliably"), ""));
            }
            Ok(r.items.iter().map(TpState::from_value).collect::<Vec<_>>())
        })
        .await
    }

    pub async fn custom_fields(
        &self,
        process_id: i64,
        entity_type: &str,
    ) -> Res<Arc<Vec<TpCustomField>>> {
        self.cached(format!("cf:{process_id}:{entity_type}"), || async {
            let options = ListOptions {
                read: ReadOptions { include: Some(CUSTOM_FIELD_INCLUDE.into()), ..Default::default() },
                where_: Some(format!("(Process.Id eq {process_id}) and (EntityType.Name eq {})", v1_string(entity_type))),
                limit: Some(DIRECTORY_LIMIT),
                ..Default::default()
            };
            let r = self.v1.list("CustomFields", &options).await?;
            if r.truncated {
                return Err(err(
                    200,
                    format!("More than {DIRECTORY_LIMIT} {entity_type} custom fields; cannot resolve reliably"),
                    "",
                ));
            }
            Ok(r.items.iter().map(TpCustomField::from_value).collect::<Vec<_>>())
        })
        .await
    }

    /// Distinct state names of an entity type across every process (or of every type), for search filters.
    pub async fn state_names(&self, entity_type: Option<&str>) -> Res<Arc<Vec<String>>> {
        self.cached(
            format!("statenames:{}", entity_type.unwrap_or("*")),
            || async {
                let options = ListOptions {
                    read: ReadOptions {
                        include: Some("[Name]".into()),
                        ..Default::default()
                    },
                    where_: entity_type.map(|t| format!("(EntityType.Name eq {})", v1_string(t))),
                    limit: Some(DIRECTORY_LIMIT),
                    ..Default::default()
                };
                let r = self.v1.list("EntityStates", &options).await?;
                if r.truncated {
                    return Err(err(
                        200,
                        format!("More than {DIRECTORY_LIMIT} states; cannot check the state name"),
                        "",
                    ));
                }
                let mut names: Vec<String> = Vec::new();
                for name in r
                    .items
                    .iter()
                    .filter_map(|x| x.get("Name").and_then(Value::as_str))
                {
                    if !name.is_empty() && !names.iter().any(|n| n == name) {
                        names.push(name.to_string());
                    }
                }
                Ok(names)
            },
        )
        .await
    }

    /// Names of the practices enabled in a process (e.g. "Time Tracking").
    pub async fn practices(&self, process_id: i64) -> Res<Arc<Vec<String>>> {
        self.cached(format!("practices:{process_id}"), || async {
            let options = ReadOptions {
                include: Some("[Id,Practices[Id,Name]]".into()),
                inner_take: Some(1000),
                ..Default::default()
            };
            let r = self.v1.get("Processes", process_id, &options).await?;
            let items = r.pointer("/Practices/Items").and_then(Value::as_array);
            Ok(items
                .into_iter()
                .flatten()
                .filter_map(|p| p.get("Name").and_then(Value::as_str))
                .filter(|n| !n.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>())
        })
        .await
    }

    /// Custom fields of an entity type in every process, including process-less ones (Extendable Domain types).
    pub async fn entity_custom_fields(&self, entity_type: &str) -> Res<Arc<Vec<TpCustomField>>> {
        self.cached(format!("cf-any:{entity_type}"), || async {
            let options = ListOptions {
                read: ReadOptions {
                    include: Some(CUSTOM_FIELD_INCLUDE.into()),
                    ..Default::default()
                },
                where_: Some(format!("(EntityType.Name eq {})", v1_string(entity_type))),
                limit: Some(DIRECTORY_LIMIT),
                ..Default::default()
            };
            let r = self.v1.list("CustomFields", &options).await?;
            Ok(r.items
                .iter()
                .map(TpCustomField::from_value)
                .collect::<Vec<_>>())
        })
        .await
    }

    pub async fn logged_user(&self) -> Res<Arc<TpUser>> {
        self.cached("me".into(), || async {
            let query =
                Query::new().with("include", "[Id,FirstName,LastName,Login,Email,IsActive]");
            let r = self.v1.get_path("Users/LoggedUser", query).await?;
            Ok(TpUser::from_value(&r))
        })
        .await
    }

    // ---- resolvers ---------------------------------------------------------

    /// Resolves a person. Inactive people are refused unless `allow_inactive`
    /// (reads, and removing someone who has left).
    pub async fn user(&self, input: &str, allow_inactive: bool) -> Resolved<TpUser> {
        let users = self
            .users()
            .await
            .map_err(|e| Unresolved::failed("Could not load users", e))?;
        let keys = |u: &TpUser| {
            vec![
                Some(full_name(u)),
                Some(format!(
                    "{} {}",
                    u.last_name.as_deref().unwrap_or(""),
                    u.first_name.as_deref().unwrap_or("")
                )),
                u.login.clone(),
                u.email.clone(),
                u.first_name.clone(),
                u.last_name.clone(),
            ]
        };
        let describe = |u: &TpUser| {
            let mut out = Map::new();
            out.insert("login".into(), json!(u.login));
            if !is_active_user(u) {
                out.insert("inactive".into(), json!(true));
            }
            out
        };
        let spec = MatchSpec {
            kind: "user",
            id: &|u: &TpUser| u.id,
            name: &full_name,
            keys: &keys,
            describe: Some(&describe),
        };
        let items: Vec<&TpUser> = users.iter().collect();
        let active: &dyn Fn(&TpUser) -> bool = if allow_inactive {
            &|_| true
        } else {
            &is_active_user
        };
        match_active(input, &spec, &items, active).cloned()
    }

    pub async fn me(&self) -> Resolved<TpUser> {
        match self.logged_user().await {
            Ok(user) => Ok(TpUser::clone(&user)),
            Err(e) => Err(Unresolved::failed(
                "Could not identify the current user (Users/LoggedUser)",
                e,
            )),
        }
    }

    pub async fn role(&self, input: &str) -> Resolved<TpRole> {
        let roles = self
            .roles()
            .await
            .map_err(|e| Unresolved::failed("Could not load roles", e))?;
        let spec = MatchSpec {
            kind: "role",
            id: &|x: &TpRole| x.id,
            name: &|x: &TpRole| x.name.clone(),
            keys: &|x: &TpRole| vec![Some(x.name.clone())],
            describe: None,
        };
        let items: Vec<&TpRole> = roles.iter().collect();
        r#match(input, &spec, &items).cloned()
    }

    pub async fn team(&self, input: &str, allow_inactive: bool) -> Resolved<TpTeam> {
        let teams = self
            .teams()
            .await
            .map_err(|e| Unresolved::failed("Could not load teams", e))?;
        let spec = MatchSpec {
            kind: "team",
            id: &|x: &TpTeam| x.id,
            name: &|x: &TpTeam| x.name.clone(),
            keys: &|x: &TpTeam| vec![Some(x.name.clone())],
            describe: None,
        };
        let items: Vec<&TpTeam> = teams.iter().collect();
        let active: &dyn Fn(&TpTeam) -> bool = if allow_inactive {
            &|_| true
        } else {
            &|t: &TpTeam| t.is_active != Some(false)
        };
        match_active(input, &spec, &items, active).cloned()
    }

    pub async fn project(&self, input: &str) -> Resolved<TpProject> {
        let projects = self
            .projects()
            .await
            .map_err(|e| Unresolved::failed("Could not load projects", e))?;
        let describe = |x: &TpProject| {
            let mut out = Map::new();
            out.insert("abbreviation".into(), json!(x.abbreviation));
            if x.is_active == Some(false) {
                out.insert("inactive".into(), json!(true));
            }
            out
        };
        let spec = MatchSpec {
            kind: "project",
            id: &|x: &TpProject| x.id,
            name: &|x: &TpProject| x.name.clone(),
            keys: &|x: &TpProject| vec![Some(x.name.clone()), x.abbreviation.clone()],
            describe: Some(&describe),
        };
        let items: Vec<&TpProject> = projects.iter().collect();
        r#match(input, &spec, &items).cloned()
    }

    /// Resolves a state name against one workflow's states: the project workflow
    /// by default, or a team sub-workflow when `team_workflow` is true.
    pub async fn state(
        &self,
        process_id: i64,
        entity_type: &str,
        input: &str,
        team_workflow: bool,
    ) -> Resolved<TpState> {
        let states = self
            .states(process_id, entity_type)
            .await
            .map_err(|e| Unresolved::failed(format!("Could not load {entity_type} states"), e))?;
        let items: Vec<&TpState> = states
            .iter()
            .filter(|s| is_team_workflow_state(s) == team_workflow)
            .collect();
        let kind = format!("{entity_type} state");
        let describe = |x: &TpState| {
            let mut out = Map::new();
            if let Some(w) = &x.workflow {
                // `Workflow?.Name`: an absent name is left out, as JSON drops `undefined`.
                if let Some(name) = &w.name {
                    out.insert("workflow".into(), json!(name));
                }
            }
            out
        };
        let spec = MatchSpec {
            kind: &kind,
            id: &|x: &TpState| x.id,
            name: &|x: &TpState| x.name.clone(),
            keys: &|x: &TpState| vec![Some(x.name.clone())],
            describe: Some(&describe),
        };
        r#match(input, &spec, &items).cloned()
    }

    /// States a team's assignment on a card can take: the team's sub-workflow for
    /// this entity type in this project (via TeamProjects), or the project
    /// workflow when the team has none.
    pub async fn team_states(
        &self,
        process_id: i64,
        entity_type: &str,
        team_id: i64,
        project_id: i64,
    ) -> Res<Vec<TpState>> {
        let states = self.states(process_id, entity_type).await?;
        let links = self
            .cached(format!("teamproject:{team_id}:{project_id}"), || async {
                let options = ListOptions {
                    read: ReadOptions {
                        include: Some(
                            "[Id,Workflows[Id,Name,EntityType[Name],ParentWorkflow]]".into(),
                        ),
                        inner_take: Some(1000),
                        ..Default::default()
                    },
                    where_: Some(format!(
                        "(Team.Id eq {team_id}) and (Project.Id eq {project_id})"
                    )),
                    ..Default::default()
                };
                Ok(self.v1.list("TeamProjects", &options).await?.items)
            })
            .await?;
        let workflow = links
            .iter()
            .filter_map(|l| l.pointer("/Workflows/Items").and_then(Value::as_array))
            .flatten()
            .find(|w| {
                w.get("ParentWorkflow").is_some_and(Value::is_object)
                    && w.pointer("/EntityType/Name").and_then(Value::as_str) == Some(entity_type)
            });
        let list = match workflow {
            Some(w) => {
                let id = w.get("Id").and_then(Value::as_i64);
                states
                    .iter()
                    .filter(|s| s.workflow.as_ref().map(|w| w.id) == id)
                    .cloned()
                    .collect()
            }
            None => states
                .iter()
                .filter(|s| !is_team_workflow_state(s))
                .cloned()
                .collect(),
        };
        Ok(list)
    }

    pub async fn custom_field(
        &self,
        process_id: i64,
        entity_type: &str,
        input: &str,
    ) -> Resolved<TpCustomField> {
        let fields = self
            .custom_fields(process_id, entity_type)
            .await
            .map_err(|e| {
                Unresolved::failed(format!("Could not load {entity_type} custom fields"), e)
            })?;
        let kind = format!("{entity_type} custom field");
        let describe = |x: &TpCustomField| {
            let mut out = Map::new();
            if let Some(t) = &x.field_type {
                out.insert("type".into(), json!(t));
            }
            out
        };
        let spec = MatchSpec {
            kind: &kind,
            id: &|x: &TpCustomField| x.id,
            name: &|x: &TpCustomField| x.name.clone(),
            keys: &|x: &TpCustomField| vec![Some(x.name.clone())],
            describe: Some(&describe),
        };
        let items: Vec<&TpCustomField> = fields.iter().collect();
        r#match(input, &spec, &items).cloned()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, Ordering};
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::http::{BoxFuture, Http, Request, Response, Transport};
    use crate::resolve::matching::Reason;

    type QueryCheck = Box<dyn Fn(&HashMap<String, String>) -> bool + Send + Sync>;

    struct Route {
        path: String,
        query: Option<QueryCheck>,
        status: u16,
        body: Value,
    }

    /// Answers GETs from canned routes (first match wins) and records each request path.
    #[derive(Default)]
    struct Stub {
        routes: Vec<Route>,
        calls: Mutex<Vec<String>>,
    }

    impl Stub {
        fn get(mut self, path: &str, body: Value) -> Self {
            self.routes.push(Route {
                path: path.into(),
                query: None,
                status: 200,
                body,
            });
            self
        }

        fn get_if(
            mut self,
            path: &str,
            body: Value,
            check: impl Fn(&HashMap<String, String>) -> bool + Send + Sync + 'static,
        ) -> Self {
            self.routes.push(Route {
                path: path.into(),
                query: Some(Box::new(check)),
                status: 200,
                body,
            });
            self
        }

        fn fail(mut self, path: &str, status: u16, body: Value) -> Self {
            self.routes.push(Route {
                path: path.into(),
                query: None,
                status,
                body,
            });
            self
        }

        fn count(&self, path: &str) -> usize {
            self.calls
                .lock()
                .unwrap()
                .iter()
                .filter(|p| *p == path)
                .count()
        }
    }

    impl Transport for Arc<Stub> {
        fn send(
            &self,
            request: Request,
            _timeout: Duration,
        ) -> BoxFuture<'_, Result<Response, String>> {
            Box::pin(async move {
                let url = url::Url::parse(&request.url).map_err(|e| e.to_string())?;
                let query: HashMap<String, String> = url.query_pairs().into_owned().collect();
                self.calls.lock().unwrap().push(url.path().to_string());
                let route = self.routes.iter().find(|r| {
                    let path_ok = match r.path.strip_suffix("/*") {
                        Some(prefix) => url.path().starts_with(&format!("{prefix}/")),
                        None => r.path == url.path(),
                    };
                    path_ok && r.query.as_ref().is_none_or(|q| q(&query))
                });
                Ok(match route {
                    Some(r) => Response {
                        status: r.status,
                        location: None,
                        body: Ok(r.body.to_string()),
                    },
                    None => Response {
                        status: 404,
                        location: None,
                        body: Ok("{}".into()),
                    },
                })
            })
        }
    }

    fn directory_with(stub: Stub, clock: Clock) -> (Arc<Stub>, Directory) {
        let stub = Arc::new(stub);
        let http = Http::new(
            "https://tp.example".into(),
            "t".into(),
            Box::new(stub.clone()),
            Arc::new(|_: &str| {}),
        );
        (stub, Directory::new(V1::new(Arc::new(http)), clock))
    }

    fn directory(stub: Stub) -> (Arc<Stub>, Directory) {
        directory_with(stub, Clock::fixed(0))
    }

    fn users() -> Value {
        json!([
            { "ResourceType": "User", "Id": 1, "FirstName": "Administrator", "LastName": "Administrator", "Email": "admin@nonexistingemail.com", "Login": "admin", "DeleteDate": null, "IsActive": false, "Kind": "User" },
            { "ResourceType": "User", "Id": 2286, "FirstName": "Foxxo", "LastName": "Vulpes", "Email": "foxxo@example.com", "Login": "fvulpes", "DeleteDate": null, "IsActive": true, "Kind": "User" },
            { "ResourceType": "User", "Id": 16, "FirstName": "Giorgio", "LastName": "Verdi", "Email": "giorgio@example.com", "Login": "giorgio@example.com", "DeleteDate": null, "IsActive": true, "Kind": "User" },
            { "ResourceType": "User", "Id": 17, "FirstName": "Giorgia", "LastName": "Rossi", "Email": "giorgia@example.com", "Login": "grossi", "DeleteDate": null, "IsActive": true, "Kind": "User" },
            { "ResourceType": "User", "Id": 2429, "FirstName": "Rocco", "LastName": "Neri", "Email": "rocco@example.com", "Login": "rocco@example.com", "DeleteDate": null, "IsActive": false, "Kind": "User" },
        ])
    }

    fn roles() -> Value {
        json!([
            { "ResourceType": "Role", "Id": 1, "Name": "Backend Developer", "HasEffort": true },
            { "ResourceType": "Role", "Id": 12, "Name": "Designer", "HasEffort": true },
            { "ResourceType": "Role", "Id": 13, "Name": "Developer", "HasEffort": true },
            { "ResourceType": "Role", "Id": 11, "Name": "Frontend Developer", "HasEffort": true },
            { "ResourceType": "Role", "Id": 7, "Name": "Product Owner", "HasEffort": true },
            { "ResourceType": "Role", "Id": 10, "Name": "Tester", "HasEffort": true },
            { "ResourceType": "Role", "Id": 8, "Name": "Top Manager", "HasEffort": false },
        ])
    }

    fn teams() -> Value {
        json!([
            { "ResourceType": "Team", "Id": 447, "Name": "Core Team", "IsActive": true },
            { "ResourceType": "Team", "Id": 19523, "Name": "Pandolfo Team", "IsActive": true },
            { "ResourceType": "Team", "Id": 300, "Name": "Legacy Team", "IsActive": false },
        ])
    }

    fn projects() -> Value {
        json!([
            { "ResourceType": "Project", "Id": 179, "Name": "Activity Manager", "IsActive": false, "Abbreviation": "AMP", "Process": { "ResourceType": "Process", "Id": 5, "Name": "Scrum Emme 4" } },
            { "ResourceType": "Project", "Id": 26080, "Name": "SBP", "IsActive": true, "Abbreviation": "SBP", "Process": { "ResourceType": "Process", "Id": 13, "Name": "Mamami 2025" } },
        ])
    }

    fn state(id: i64, name: &str, priority: f64, extra: Value) -> Value {
        let mut s = json!({
            "ResourceType": "EntityState", "Id": id, "Name": name, "IsInitial": false, "IsFinal": false, "IsPlanned": false,
            "NumericPriority": priority,
            "Workflow": { "ResourceType": "Workflow", "Id": 202, "Name": "Project workflow", "ParentWorkflow": null },
            "ParentEntityState": null,
            "Role": { "ResourceType": "Role", "Id": 13, "Name": "Developer" },
            "EntityType": { "ResourceType": "EntityType", "Id": 4, "Name": "UserStory" },
        });
        if let (Some(s), Value::Object(extra)) = (s.as_object_mut(), extra) {
            s.extend(extra);
        }
        s
    }

    /// UserStory states of process 13, plus one team sub-workflow state for the flagging logic.
    fn story_states() -> Vec<Value> {
        let po = json!({ "Role": { "ResourceType": "Role", "Id": 7, "Name": "Product Owner" } });
        vec![
            state(
                681,
                "Open",
                1.0,
                json!({ "IsInitial": true, "Role": { "ResourceType": "Role", "Id": 7, "Name": "Product Owner" } }),
            ),
            state(682, "Refining", 3.25, po.clone()),
            state(
                684,
                "Designable",
                3.8125,
                json!({ "Role": { "ResourceType": "Role", "Id": 12, "Name": "Designer" } }),
            ),
            state(683, "Estimated", 3.90625, json!({})),
            state(685, "Ready", 4.0, json!({})),
            state(686, "In Progress", 5.0, json!({})),
            state(687, "Coded", 6.0, json!({})),
            state(688, "Testing", 7.0, json!({})),
            state(689, "Deliverable", 7.5, po),
            state(690, "Done", 8.0, json!({ "IsFinal": true, "Role": null })),
            state(
                900,
                "Code Review",
                5.5,
                json!({
                    "Workflow": { "ResourceType": "Workflow", "Id": 950, "Name": "Core Team flow", "ParentWorkflow": { "ResourceType": "Workflow", "Id": 202 } },
                    "ParentEntityState": { "ResourceType": "EntityState", "Id": 686, "Name": "In Progress" },
                }),
            ),
        ]
    }

    fn cf(id: i64, name: &str, field_type: &str, value: Option<&str>) -> Value {
        json!({
            "ResourceType": "CustomField", "Id": id, "Name": name, "Value": value, "FieldType": field_type, "Required": false,
            "EntityType": { "ResourceType": "EntityType", "Id": 4, "Name": "UserStory" },
            "Process": { "ResourceType": "Process", "Id": 13, "Name": "Mamami 2025" },
        })
    }

    fn story_custom_fields() -> Vec<Value> {
        vec![
            cf(140, "Specification", "URL", None),
            cf(
                150,
                "BackEnd",
                "DropDown",
                Some("To Do\r\nDoing\r\nReview\r\nDone"),
            ),
            cf(
                152,
                "FrontEnd",
                "DropDown",
                Some("To Do\r\nDoing\r\nReview\r\nDone"),
            ),
            cf(153, "Figma", "URL", None),
            cf(159, "Total Hours", "Number", None),
            cf(160, "Release Instructions", "RichText", None),
        ]
    }

    fn where_has(text: &'static str) -> impl Fn(&HashMap<String, String>) -> bool + Send + Sync {
        move |q| q.get("where").is_some_and(|w| w.contains(text))
    }

    /// The reference-data endpoints the resolvers read.
    fn with_reference_data(stub: Stub) -> Stub {
        stub.get("/api/v1/Users", json!({ "Items": users() }))
            .get(
                "/api/v1/Users/LoggedUser",
                json!({ "ResourceType": "User", "Id": 2286, "FirstName": "Foxxo", "LastName": "Vulpes", "Login": "fvulpes", "IsActive": true, "Kind": "User" }),
            )
            .get("/api/v1/Roles", json!({ "Items": roles() }))
            .get("/api/v1/Teams", json!({ "Items": teams() }))
            .get("/api/v1/Projects", json!({ "Items": projects() }))
            .get_if("/api/v1/EntityStates", json!({ "Items": story_states() }), where_has("EntityType.Name eq 'UserStory'"))
            .get_if("/api/v1/CustomFields", json!({ "Items": story_custom_fields() }), where_has("EntityType.Name eq 'UserStory'"))
            .get("/api/v1/CustomFields", json!({ "Items": [] }))
    }

    fn reference() -> (Arc<Stub>, Directory) {
        directory(with_reference_data(Stub::default()))
    }

    fn message<T: std::fmt::Debug>(r: Resolved<T>) -> String {
        r.expect_err("should not resolve").message
    }

    #[tokio::test]
    async fn resolves_people_by_full_name_login_email_and_id() {
        let (_, dir) = reference();
        for input in [
            "Foxxo Vulpes",
            "fvulpes",
            "FOXXO@example.com",
            "vulpes foxxo",
            "2286",
            &2286.to_string(),
        ] {
            assert_eq!(
                dir.user(input, false).await.map(|u| u.id),
                Ok(2286),
                "{input}"
            );
        }
    }

    #[tokio::test]
    async fn accepts_a_unique_partial_match() {
        let (_, dir) = reference();
        assert_eq!(dir.user("foxxo", false).await.map(|u| u.id), Ok(2286));
    }

    #[tokio::test]
    async fn lists_every_candidate_when_ambiguous() {
        let (_, dir) = reference();
        let r = dir.user("giorg", false).await.expect_err("ambiguous");
        assert_eq!(r.reason, Reason::Ambiguous);
        let mut ids: Vec<i64> = r.candidates.iter().flatten().map(|c| c.id).collect();
        ids.sort();
        assert_eq!(ids, [16, 17]);
        assert!(r.message.contains("Giorgio Verdi (16)"));
        assert!(r.message.contains("Giorgia Rossi (17)"));
    }

    #[tokio::test]
    async fn refuses_inactive_people_with_a_clear_message() {
        let (_, dir) = reference();
        assert_eq!(
            message(dir.user("Rocco Neri", false).await),
            "User Rocco Neri (2429) is inactive or deleted."
        );
    }

    #[tokio::test]
    async fn an_exact_inactive_name_is_refused_not_swapped_for_a_partial_active_match() {
        let stub = Stub::default().get(
            "/api/v1/Users",
            json!({ "Items": [
                { "Id": 1, "FirstName": "John", "LastName": "Smith", "Login": "jsmith", "IsActive": false },
                { "Id": 2, "FirstName": "John", "LastName": "Smithson", "Login": "jsmithson", "IsActive": true },
            ] }),
        );
        let (_, dir) = directory(stub);
        assert_eq!(
            message(dir.user("John Smith", false).await),
            "User John Smith (1) is inactive or deleted."
        );
        assert_eq!(dir.user("smithson", false).await.map(|u| u.id), Ok(2));
    }

    #[tokio::test]
    async fn suggests_close_names_when_nothing_matches() {
        let (_, dir) = reference();
        assert!(
            message(dir.user("Foxo Vulpes", false).await)
                .contains("Did you mean: Foxxo Vulpes (2286)")
        );
    }

    #[tokio::test]
    async fn pages_the_full_user_list() {
        let first: Vec<Value> = (0..1000)
            .map(|i| json!({ "Id": i + 100, "FirstName": "U", "LastName": i.to_string(), "IsActive": true }))
            .collect();
        let stub = Stub::default()
            .get_if("/api/v1/Users", json!({ "Next": "x", "Items": first }), |q| q.get("skip").map(String::as_str) == Some("0"))
            .get_if(
                "/api/v1/Users",
                json!({ "Items": [{ "Id": 5000, "FirstName": "Late", "LastName": "Page", "IsActive": true }] }),
                |q| q.get("skip").map(String::as_str) == Some("1000"),
            );
        let (_, dir) = directory(stub);
        assert_eq!(dir.user("Late Page", false).await.map(|u| u.id), Ok(5000));
    }

    #[tokio::test]
    async fn caches_reference_data() {
        let (stub, dir) = reference();
        dir.user("fvulpes", false).await.unwrap();
        dir.user("giorgio@example.com", false).await.unwrap();
        assert_eq!(stub.count("/api/v1/Users"), 1);
    }

    #[tokio::test]
    async fn concurrent_callers_share_one_load_and_entries_expire() {
        let now = Arc::new(AtomicI64::new(0));
        let clock = Clock {
            now_ms: Arc::new({
                let now = now.clone();
                move || now.load(Ordering::SeqCst)
            }),
            today: Arc::new(String::new),
        };
        let (stub, dir) = directory_with(with_reference_data(Stub::default()), clock);
        let (a, b) = tokio::join!(dir.role("Tester"), dir.role("Designer"));
        assert_eq!((a.map(|r| r.id), b.map(|r| r.id)), (Ok(10), Ok(12)));
        assert_eq!(stub.count("/api/v1/Roles"), 1);
        now.store(TTL_MS - 1, Ordering::SeqCst);
        dir.role("Tester").await.unwrap();
        assert_eq!(stub.count("/api/v1/Roles"), 1);
        now.store(TTL_MS, Ordering::SeqCst);
        dir.role("Tester").await.unwrap();
        assert_eq!(stub.count("/api/v1/Roles"), 2);
    }

    #[tokio::test]
    async fn prefers_the_exact_role_over_partial_ones() {
        let (_, dir) = reference();
        assert_eq!(dir.role("developer").await.map(|r| r.id), Ok(13));
    }

    #[tokio::test]
    async fn reports_ambiguous_partial_roles() {
        let (_, dir) = reference();
        let r = dir.role("end developer").await.expect_err("ambiguous");
        let names: Vec<String> = r.candidates.into_iter().flatten().map(|c| c.name).collect();
        assert_eq!(names, ["Backend Developer", "Frontend Developer"]);
    }

    #[tokio::test]
    async fn refuses_inactive_teams() {
        let (_, dir) = reference();
        assert!(dir.team("core", false).await.is_ok());
        assert!(message(dir.team("Legacy Team", false).await).contains("inactive"));
    }

    #[tokio::test]
    async fn resolves_projects_by_abbreviation() {
        let (_, dir) = reference();
        assert_eq!(dir.project("amp").await.map(|p| p.id), Ok(179));
    }

    #[tokio::test]
    async fn carries_the_http_failure_and_does_not_cache_it() {
        let stub = Stub::default().fail("/api/v1/Roles", 403, json!({ "Message": "Forbidden" }));
        let (stub, dir) = directory(stub);
        let r = dir.role("Developer").await.expect_err("forbidden");
        assert_eq!(r.reason, Reason::Error);
        assert_eq!(r.error.map(|e| e.status), Some(403));
        dir.role("Developer").await.expect_err("forbidden again");
        assert_eq!(stub.count("/api/v1/Roles"), 2);
    }

    #[tokio::test]
    async fn resolves_states_within_the_project_workflow_only_by_default() {
        let (_, dir) = reference();
        assert_eq!(
            dir.state(13, "UserStory", "coded", false)
                .await
                .map(|s| s.id),
            Ok(687)
        );
        assert!(
            dir.state(13, "UserStory", "Code Review", false)
                .await
                .is_err()
        );
        assert_eq!(
            dir.state(13, "UserStory", "Code Review", true)
                .await
                .map(|s| s.id),
            Ok(900)
        );
    }

    #[test]
    fn flags_team_sub_workflow_states() {
        let flagged: Vec<i64> = story_states()
            .iter()
            .map(TpState::from_value)
            .filter(is_team_workflow_state)
            .map(|s| s.id)
            .collect();
        assert_eq!(flagged, [900]);
    }

    #[tokio::test]
    async fn reads_dropdown_options() {
        let (_, dir) = reference();
        let f = dir.custom_field(13, "UserStory", "frontend").await.unwrap();
        assert_eq!(
            custom_field_options(&f).unwrap(),
            ["To Do", "Doing", "Review", "Done"]
        );
        assert_eq!(
            custom_field_options(&TpCustomField::from_value(&story_custom_fields()[0])),
            None
        );
    }
}
