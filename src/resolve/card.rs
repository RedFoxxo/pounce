//! Card id → type, resource, project and process (port of `card.ts`).

use std::sync::Arc;

use serde_json::{Value, json};

use super::matching::{Reason, Resolved, Unresolved};
use crate::catalog::{CatalogResource, Request};
use crate::http::v1::ReadOptions;
use crate::tools::Ctx;

/// A project as a card names it.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectRef {
    pub id: i64,
    pub name: String,
}

impl ProjectRef {
    /// `{id, name}`.
    pub fn to_json(&self) -> Value {
        json!({ "id": self.id, "name": self.name })
    }
}

/// What pounce needs to know about any card before acting on it.
#[derive(Debug, Clone)]
pub struct CardInfo {
    pub id: i64,
    pub name: String,
    /// Entity type name, e.g. `UserStory`.
    pub entity_type: String,
    pub resource: Arc<CatalogResource>,
    pub project: Option<ProjectRef>,
    pub process_id: Option<i64>,
}

fn unresolved(reason: Reason, message: String) -> Box<Unresolved> {
    Box::new(Unresolved {
        message,
        reason,
        candidates: None,
        error: None,
    })
}

/// Resolves any card id to its type, resource path, project and process via `Generals/{id}`.
pub async fn card_info(ctx: &Ctx, id: i64) -> Resolved<CardInfo> {
    let options = ReadOptions {
        include: Some("[Id,Name,EntityType,Project[Id,Name,Process]]".into()),
        ..ReadOptions::default()
    };
    let data = match ctx.v1.get("Generals", id, &options).await {
        Ok(data) => data,
        Err(e) => {
            let missing = e.status == 404;
            return Err(Box::new(Unresolved {
                message: if missing {
                    format!("No card with id {id} (or no access to it).")
                } else {
                    format!("Could not look up card {id}")
                },
                reason: if missing {
                    Reason::NotFound
                } else {
                    Reason::Error
                },
                candidates: None,
                error: Some(e),
            }));
        }
    };
    let Some(type_name) = data["EntityType"]["Name"]
        .as_str()
        .filter(|n| !n.is_empty())
    else {
        return Err(unresolved(
            Reason::Error,
            format!("Card {id} has no entity type in the response."),
        ));
    };
    let catalog = ctx
        .catalog
        .get(&Request::Names(vec![type_name.to_string()]))
        .await;
    let Some(resource) = catalog.find(type_name).filter(|r| r.available) else {
        return Err(unresolved(
            Reason::Error,
            format!("Card {id} is a {type_name}, which the catalog does not describe."),
        ));
    };
    let project = &data["Project"];
    let has_project = project.is_object();
    Ok(CardInfo {
        id: data["Id"].as_i64().unwrap_or(id),
        name: data["Name"].as_str().unwrap_or_default().to_string(),
        entity_type: resource.name.clone(),
        resource: resource.clone(),
        project: has_project.then(|| ProjectRef {
            id: project["Id"].as_i64().unwrap_or_default(),
            name: project["Name"].as_str().unwrap_or_default().to_string(),
        }),
        process_id: project["Process"]
            .is_object()
            .then(|| project["Process"]["Id"].as_i64().unwrap_or_default()),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;
    use crate::catalog::{Provider, snapshot};
    use crate::config::Config;
    use crate::http::{BoxFuture, Request as HttpRequest, Response, Transport};
    use crate::tools::Clock;
    use crate::tools::context::{Options, context};

    /// Each request's path and query pairs.
    type Seen = Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>;

    /// Answers every request with one recorded response and keeps each request's path and query.
    #[derive(Clone)]
    struct Once {
        status: u16,
        text: &'static str,
        seen: Seen,
    }

    impl Transport for Once {
        fn send(
            &self,
            request: HttpRequest,
            _timeout: Duration,
        ) -> BoxFuture<'_, Result<Response, String>> {
            Box::pin(async move {
                let url = url::Url::parse(&request.url).unwrap();
                let query = url
                    .query_pairs()
                    .map(|(k, v)| (k.into(), v.into()))
                    .collect();
                self.seen
                    .lock()
                    .unwrap()
                    .push((url.path().to_string(), query));
                Ok(Response {
                    status: self.status,
                    location: None,
                    body: Ok(self.text.to_string()),
                })
            })
        }
    }

    fn ctx(transport: Once) -> Ctx {
        context(
            Config::new("https://tp.example", "test-token-0000"),
            Options {
                transport: Box::new(transport),
                log: Arc::new(|_| {}),
                clock: Clock::fixed(0),
                catalog: Some(Provider::fixed(snapshot())),
                settle_delays_ms: vec![0],
            },
        )
    }

    fn expect_request(transport: &Once, path: &str) {
        let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
        assert_eq!(
            *transport.seen.lock().unwrap(),
            vec![(
                path.to_string(),
                vec![
                    pair("format", "json"),
                    pair("include", "[Id,Name,EntityType,Project[Id,Name,Process]]"),
                    pair("access_token", "test-token-0000"),
                ]
            )]
        );
    }

    #[tokio::test]
    async fn resolves_type_resource_project_and_process_from_generals() {
        let transport = Once {
            status: 200,
            text: r#"{"ResourceType":"General","Id":36217,"Name":"Card 36217","EntityType":{"ResourceType":"EntityType","Id":4,"Name":"UserStory"},"Project":{"ResourceType":"Project","Id":26080,"Name":"SBP","Process":{"ResourceType":"Process","Id":13,"Name":"Scrum"}}}"#,
            seen: Arc::default(),
        };
        let info = card_info(&ctx(transport.clone()), 36217).await.unwrap();
        expect_request(&transport, "/api/v1/Generals/36217");
        assert_eq!(info.entity_type, "UserStory");
        assert_eq!(info.resource.path, "UserStories");
        assert_eq!(
            info.project,
            Some(ProjectRef {
                id: 26080,
                name: "SBP".into()
            })
        );
        assert_eq!(info.process_id, Some(13));
    }

    #[tokio::test]
    async fn reports_a_missing_card() {
        let transport = Once {
            status: 404,
            text: r#"{"Message":"General with Id 1 not found or access is forbidden."}"#,
            seen: Arc::default(),
        };
        let r = card_info(&ctx(transport.clone()), 1).await.unwrap_err();
        expect_request(&transport, "/api/v1/Generals/1");
        assert_eq!(r.message, "No card with id 1 (or no access to it).");
        assert_eq!(r.reason, Reason::NotFound);
    }
}
