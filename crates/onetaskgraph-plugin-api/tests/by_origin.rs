//! The by-origin listings' default bodies, met the way a plugin that implements neither meets
//! them: through `dyn`, over a listing that pages, so the default's walk and its own cursor are
//! what is proven.

use std::sync::atomic::{AtomicUsize, Ordering};

use onetaskgraph_plugin_api::{
    Capabilities, Cursor, DependencyEdge, DependencySupport, Direction, Health, Label, NativeId,
    Page, PageRequest, Project, ProjectQuery, SourceError, Status, StatusCategory, Support, Task,
    TaskQuery, TaskSource,
};
use serde_json::json;

/// A source holding `projects`, listed two a page whatever the caller asks for, which records
/// no documents at all.
struct Listing {
    projects: Vec<Project>,
    pages: AtomicUsize,
}

fn project(id: &str, origin: Option<serde_json::Value>) -> Project {
    Project {
        version: None,
        id: NativeId::from(id),
        title: format!("Project {id}"),
        content: None,
        status: Status {
            category: StatusCategory::Todo,
            name: "Todo".to_owned(),
        },
        labels: Vec::new(),
        url: None,
        location: None,
        created_at: None,
        updated_at: None,
        metadata: origin
            .map(|origin| [(TaskQuery::ORIGIN_KEY.to_owned(), origin)].into())
            .unwrap_or_default(),
        repositories: Vec::new(),
        classification: Default::default(),
    }
}

#[async_trait::async_trait]
impl TaskSource for Listing {
    fn kind(&self) -> &'static str {
        "listing"
    }
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            projects: Support::Native,
            documents: Support::Unsupported,
            comments: Support::Unsupported,
            assets: Support::Unsupported,
            priority: Support::Unsupported,
            filter_by_priority: Support::Unsupported,
            filter_by_comment_activity: Support::Unsupported,
            filter_by_metadata: Support::Unsupported,
            filter_by_origin: Support::Unsupported,
            reconcile_status: Support::Unsupported,
            complete_by_issue: Support::Unsupported,
            orphan_tasks: Support::Native,
            filter_by_label: Support::Unsupported,
            filter_by_status: Support::Unsupported,
            search_title: Support::Unsupported,
            search_content: Support::Unsupported,
            task_dependencies: DependencySupport::BothDirections,
            project_dependencies: DependencySupport::BothDirections,
            max_page_size: 2,
        }
    }
    async fn health(&self) -> Result<Health, SourceError> {
        Ok(Health {
            reachable: true,
            detail: None,
        })
    }
    async fn get_task(&self, _id: &NativeId) -> Result<Option<Task>, SourceError> {
        Ok(None)
    }
    async fn get_project(&self, _id: &NativeId) -> Result<Option<Project>, SourceError> {
        Ok(None)
    }
    async fn query_tasks(
        &self,
        _query: &TaskQuery,
        _page: &PageRequest,
    ) -> Result<Page<Task>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn query_projects(
        &self,
        _query: &ProjectQuery,
        page: &PageRequest,
    ) -> Result<Page<Project>, SourceError> {
        self.pages.fetch_add(1, Ordering::SeqCst);
        let start: usize = page
            .cursor
            .as_ref()
            .map_or(0, |cursor| cursor.0.parse().expect("a cursor this source gave"));
        let end = (start + 2).min(self.projects.len());
        Ok(Page {
            items: self.projects[start..end].to_vec(),
            next: (end < self.projects.len()).then(|| Cursor(end.to_string())),
        })
    }
    async fn labels(&self, _page: &PageRequest) -> Result<Page<Label>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn task_dependencies(
        &self,
        _id: &NativeId,
        _direction: Direction,
        _page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
    async fn project_dependencies(
        &self,
        _id: &NativeId,
        _direction: Direction,
        _page: &PageRequest,
    ) -> Result<Page<DependencyEdge>, SourceError> {
        Ok(Page::last(Vec::new()))
    }
}

fn ids(page: &Page<Project>) -> Vec<&str> {
    page.items.iter().map(|project| project.id.0.as_str()).collect()
}

#[tokio::test]
async fn the_default_lists_exactly_the_projects_recording_that_origin_across_its_own_pages() {
    let source: Box<dyn TaskSource> = Box::new(Listing {
        projects: vec![
            project("A", Some(json!("plans:P-1"))),
            project("B", None),
            project("C", Some(json!("plans:P-10"))),
            project("D", Some(json!("plans:P-1"))),
            project("E", Some(json!(["plans:P-1"]))),
            project("F", Some(json!("plans:P-1"))),
        ],
        pages: AtomicUsize::new(0),
    });

    // A page smaller than the matches walks them in the listing's order, by a cursor of the
    // default's own, and every page reads the listing to its end.
    let first = source
        .query_projects_by_origin(
            "plans:P-1",
            &PageRequest {
                cursor: None,
                limit: 2,
            },
        )
        .await
        .expect("the default answers");
    assert_eq!(ids(&first), ["A", "D"]);
    let second = source
        .query_projects_by_origin(
            "plans:P-1",
            &PageRequest {
                cursor: first.next.clone(),
                limit: 2,
            },
        )
        .await
        .expect("the default answers its own cursor");
    assert_eq!(
        ids(&second),
        ["F"],
        "a prefix of the origin, another origin and a value that is no string never match"
    );
    assert_eq!(second.next, None);

    // An origin nothing records is an empty answer, never a failure.
    let none = source
        .query_projects_by_origin(
            "plans:P-2",
            &PageRequest {
                cursor: None,
                limit: 10,
            },
        )
        .await
        .expect("the default answers");
    assert!(none.items.is_empty() && none.next.is_none());

    // A cursor it did not hand out is refused rather than read as a position.
    let refused = source
        .query_projects_by_origin(
            "plans:P-1",
            &PageRequest {
                cursor: Some(Cursor("4".to_owned())),
                limit: 2,
            },
        )
        .await
        .expect_err("a foreign cursor");
    assert!(matches!(refused, SourceError::Refused { .. }), "{refused:?}");
}

#[tokio::test]
async fn the_document_default_refuses_on_a_source_with_no_documents_as_its_listing_does() {
    let source: Box<dyn TaskSource> = Box::new(Listing {
        projects: Vec::new(),
        pages: AtomicUsize::new(0),
    });
    let refused = source
        .query_documents_by_origin(
            "plans:D-1",
            &PageRequest {
                cursor: None,
                limit: 2,
            },
        )
        .await
        .expect_err("a source with no documents");
    assert_eq!(refused, onetaskgraph_plugin_api::documentless("listing"));
}
