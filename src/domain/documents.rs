use serde_json::Value;
use typesense::models::{IndexAction, SearchParameters, SearchResult};

/// The search bar's inputs. Empty text fields are left out of the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentQuery {
    pub q: String,
    pub query_by: String,
    pub filter_by: String,
    pub sort_by: String,
    /// 1-based.
    pub page: u32,
    pub per_page: u32,
}

impl Default for DocumentQuery {
    fn default() -> Self {
        Self {
            q: "*".to_owned(),
            query_by: String::new(),
            filter_by: String::new(),
            sort_by: String::new(),
            page: 1,
            per_page: 25,
        }
    }
}

impl DocumentQuery {
    pub fn to_params(&self) -> SearchParameters<'_> {
        let mut params = SearchParameters::new();
        params.q = Some(trimmed(&self.q).unwrap_or_else(|| "*".into()));
        params.query_by = trimmed(&self.query_by);
        params.filter_by = trimmed(&self.filter_by);
        params.sort_by = trimmed(&self.sort_by);
        params.page = Some(i32::try_from(self.page).unwrap_or(i32::MAX));
        params.per_page = Some(i32::try_from(self.per_page).unwrap_or(i32::MAX));
        params
    }
}

fn trimmed(s: &str) -> Option<std::borrow::Cow<'_, str>> {
    let s = s.trim();
    (!s.is_empty()).then_some(s.into())
}

/// One page of results, reduced to what the table needs.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchPage {
    pub found: u64,
    pub out_of: u64,
    pub page: u32,
    pub search_time_ms: u64,
    pub documents: Vec<Value>,
}

impl SearchPage {
    pub fn from_result(result: SearchResult<Value>) -> Self {
        let count = |n: Option<i32>| u64::try_from(n.unwrap_or(0)).unwrap_or(0);
        Self {
            found: count(result.found),
            out_of: count(result.out_of),
            page: u32::try_from(result.page.unwrap_or(1)).unwrap_or(1),
            search_time_ms: count(result.search_time_ms),
            documents: result
                .hits
                .unwrap_or_default()
                .into_iter()
                .filter_map(|hit| hit.document)
                .collect(),
        }
    }

    /// Pages needed for `found` hits at `per_page`; at least one so an empty result still has a page.
    pub fn page_count(&self, per_page: u32) -> u32 {
        let per_page = u64::from(per_page.max(1));
        u32::try_from(self.found.div_ceil(per_page).max(1)).unwrap_or(u32::MAX)
    }
}

/// How an import treats documents that do or do not already exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumIter)]
pub enum ImportAction {
    Create,
    Upsert,
    Update,
    Emplace,
}

impl ImportAction {
    pub fn label(self) -> &'static str {
        match self {
            ImportAction::Create => "create",
            ImportAction::Upsert => "upsert",
            ImportAction::Update => "update",
            ImportAction::Emplace => "emplace",
        }
    }

    pub fn to_sdk(self) -> IndexAction {
        match self {
            ImportAction::Create => IndexAction::Create,
            ImportAction::Upsert => IndexAction::Upsert,
            ImportAction::Update => IndexAction::Update,
            ImportAction::Emplace => IndexAction::Emplace,
        }
    }
}

/// The outcome of one imported line, as Typesense reports it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct ImportLine {
    pub success: bool,
    #[serde(default)]
    pub error: Option<String>,
    /// The offending input line, echoed back by Typesense on failure.
    #[serde(default)]
    pub document: Option<String>,
}

/// Parses the JSONL body of an import response, one result per input line.
pub fn parse_import_report(body: &str) -> Result<Vec<ImportLine>, String> {
    body.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).map_err(|e| format!("Unexpected import response line: {e}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;
    use typesense::models::SearchResultHit;

    use super::*;

    fn hit(doc: Option<Value>) -> SearchResultHit<Value> {
        SearchResultHit {
            document: doc,
            ..SearchResultHit::new()
        }
    }

    #[test]
    fn default_query_lists_everything_from_page_one() {
        let q = DocumentQuery::default();
        assert_eq!((q.q.as_str(), q.page, q.per_page), ("*", 1, 25));
    }

    #[test]
    fn params_omit_blank_fields_and_trim_the_rest() {
        let q = DocumentQuery {
            q: "  shoes ".into(),
            query_by: "name,brand".into(),
            filter_by: "  ".into(),
            sort_by: String::new(),
            page: 3,
            per_page: 50,
        };
        let p = q.to_params();
        assert_eq!(p.q.as_deref(), Some("shoes"));
        assert_eq!(p.query_by.as_deref(), Some("name,brand"));
        assert_eq!(p.filter_by, None);
        assert_eq!(p.sort_by, None);
        assert_eq!((p.page, p.per_page), (Some(3), Some(50)));
    }

    #[test]
    fn a_blank_query_text_means_match_all() {
        let q = DocumentQuery {
            q: " ".into(),
            ..DocumentQuery::default()
        };
        assert_eq!(q.to_params().q.as_deref(), Some("*"));
    }

    #[test]
    fn a_result_is_reduced_to_counts_and_documents() {
        let result = SearchResult {
            found: Some(120),
            out_of: Some(500),
            page: Some(2),
            search_time_ms: Some(4),
            hits: Some(vec![
                hit(Some(serde_json::json!({"id": "1"}))),
                hit(None),
                hit(Some(serde_json::json!({"id": "2"}))),
            ]),
            ..SearchResult::new()
        };
        let page = SearchPage::from_result(result);
        assert_eq!((page.found, page.out_of, page.page), (120, 500, 2));
        assert_eq!(page.search_time_ms, 4);
        assert_eq!(page.documents.len(), 2);
    }

    #[test]
    fn an_empty_result_has_safe_defaults() {
        let page = SearchPage::from_result(SearchResult::new());
        assert_eq!((page.found, page.out_of, page.page), (0, 0, 1));
        assert!(page.documents.is_empty());
    }

    #[test]
    fn page_count_rounds_up_and_is_at_least_one() {
        let page = |found| SearchPage {
            found,
            ..SearchPage::default()
        };
        assert_eq!(page(0).page_count(25), 1);
        assert_eq!(page(25).page_count(25), 1);
        assert_eq!(page(26).page_count(25), 2);
        assert_eq!(page(100).page_count(0), 100);
    }

    #[test]
    fn import_actions_map_to_the_sdk_and_have_labels() {
        let labels: Vec<&str> = ImportAction::iter().map(ImportAction::label).collect();
        assert_eq!(labels, ["create", "upsert", "update", "emplace"]);
        assert_eq!(ImportAction::Upsert.to_sdk(), IndexAction::Upsert);
        assert_eq!(ImportAction::Emplace.to_sdk(), IndexAction::Emplace);
    }

    #[test]
    fn import_report_lines_are_parsed_in_order() {
        let body = "{\"success\":true}\n\n{\"success\":false,\"error\":\"Field `x` must be int\",\"document\":\"{\\\"x\\\":\\\"a\\\"}\"}\n";
        let lines = parse_import_report(body).unwrap();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].success && lines[0].error.is_none());
        assert_eq!(lines[1].error.as_deref(), Some("Field `x` must be int"));
        assert!(lines[1].document.as_deref().unwrap().contains("\"x\""));
    }

    #[test]
    fn a_malformed_report_line_is_an_error() {
        let err = parse_import_report("not json").unwrap_err();
        assert!(err.starts_with("Unexpected import response line"));
    }

    #[test]
    fn an_empty_report_has_no_lines() {
        assert!(parse_import_report("").unwrap().is_empty());
    }
}
