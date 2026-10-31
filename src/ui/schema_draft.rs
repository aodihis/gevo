use std::collections::BTreeSet;

use typesense::models::{CollectionResponse, CollectionSchema, CollectionUpdateSchema, Field};

/// Types with form controls; anything else (object, float[] embeddings, ...) is read-only.
pub const BASIC_TYPES: [&str; 8] = [
    "string", "string[]", "int32", "int64", "float", "bool", "geopoint", "auto",
];

pub fn is_basic(field_type: &str) -> bool {
    BASIC_TYPES.contains(&field_type)
}

/// Typesense sorts numeric and bool fields unless told otherwise, and never strings.
fn sorts_by_default(field_type: &str) -> bool {
    matches!(field_type, "int32" | "int64" | "float" | "bool")
}

pub fn facet(f: &Field) -> bool {
    f.facet.unwrap_or(false)
}

pub fn index(f: &Field) -> bool {
    f.index.unwrap_or(true)
}

pub fn optional(f: &Field) -> bool {
    f.optional.unwrap_or(false)
}

pub fn sort(f: &Field) -> bool {
    f.sort.unwrap_or_else(|| sorts_by_default(&f.r#type))
}

/// A field for the "add" rows: indexed, string-typed, nothing else set.
pub fn blank_field() -> Field {
    let mut f = Field::new(String::new(), "string".to_owned());
    f.index = Some(true);
    f
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Alter(String),
    Create,
}

/// Edits to one collection's schema, collected until the user applies them as one request.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaDraft {
    target: Target,
    /// Collection name; only editable when creating.
    pub name: String,
    /// Create only: must name a numeric field.
    pub default_sorting_field: String,
    existing: Vec<Field>,
    dropped: BTreeSet<String>,
    pub added: Vec<Field>,
    documents: i64,
}

impl SchemaDraft {
    /// Alter an existing collection: its fields are locked, only add and drop are possible.
    pub fn alter(c: &CollectionResponse) -> Self {
        Self {
            target: Target::Alter(c.name.clone()),
            name: c.name.clone(),
            default_sorting_field: String::new(),
            existing: c.fields.clone(),
            dropped: BTreeSet::new(),
            added: Vec::new(),
            documents: c.num_documents,
        }
    }

    pub fn create() -> Self {
        Self {
            target: Target::Create,
            name: String::new(),
            default_sorting_field: String::new(),
            existing: Vec::new(),
            dropped: BTreeSet::new(),
            added: Vec::new(),
            documents: 0,
        }
    }

    pub fn is_create(&self) -> bool {
        self.target == Target::Create
    }

    /// The collection being altered; `None` when creating.
    pub fn alter_target(&self) -> Option<&str> {
        match &self.target {
            Target::Alter(name) => Some(name),
            Target::Create => None,
        }
    }

    pub fn existing(&self) -> &[Field] {
        &self.existing
    }

    pub fn documents(&self) -> i64 {
        self.documents
    }

    pub fn is_dropped(&self, name: &str) -> bool {
        self.dropped.contains(name)
    }

    /// Marks an existing field for removal, or takes the mark back. Unknown names are ignored.
    pub fn toggle_drop(&mut self, name: &str) {
        if !self.existing.iter().any(|f| f.name == name) {
            return;
        }
        if !self.dropped.remove(name) {
            self.dropped.insert(name.to_owned());
        }
    }

    pub fn add_blank(&mut self) {
        self.added.push(blank_field());
    }

    pub fn remove_added(&mut self, index: usize) {
        if index < self.added.len() {
            self.added.remove(index);
        }
    }

    pub fn change_count(&self) -> usize {
        self.dropped.len() + self.added.len()
    }

    /// One line per pending change, drops first because they are sent first.
    pub fn pending_lines(&self) -> Vec<String> {
        let drops = self.dropped.iter().map(|n| format!("− {n}"));
        let adds = self.added.iter().map(|f| {
            let mut line = format!("+ {} {}", f.name, f.r#type);
            for (on, label) in [
                (facet(f), "facet"),
                (sort(f) && !sorts_by_default(&f.r#type), "sort"),
                (optional(f), "optional"),
                (!index(f), "no index"),
            ] {
                if on {
                    line.push_str(" · ");
                    line.push_str(label);
                }
            }
            line
        });
        drops.chain(adds).collect()
    }

    /// Everything wrong with the draft, in display order; empty when it can be sent.
    pub fn problems(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.is_create() {
            if self.name.trim().is_empty() {
                out.push("Collection name is required.".to_owned());
            }
            if self.added.is_empty() {
                out.push("Add at least one field.".to_owned());
            }
        } else if self.change_count() == 0 {
            out.push("No changes to apply.".to_owned());
        }
        for (i, f) in self.added.iter().enumerate() {
            let name = f.name.trim();
            if name.is_empty() {
                out.push("Every new field needs a name.".to_owned());
            } else if self.added[..i].iter().any(|e| e.name.trim() == name) {
                out.push(format!("Field {name} is added twice."));
            } else if self
                .existing
                .iter()
                .any(|e| e.name == name && !self.dropped.contains(name))
            {
                out.push(format!(
                    "Field {name} already exists. Drop it first to change it."
                ));
            }
        }
        out
    }

    /// The `PATCH` body: drops first, then adds, so re-adding a dropped name replaces it.
    pub fn to_update(&self) -> CollectionUpdateSchema {
        let drops = self.dropped.iter().filter_map(|name| {
            let existing = self.existing.iter().find(|f| &f.name == name)?;
            let mut drop = Field::new(name.clone(), existing.r#type.clone());
            drop.drop = Some(true);
            Some(drop)
        });
        let adds = self.added.iter().map(|f| Field {
            name: f.name.trim().to_owned(),
            ..f.clone()
        });
        CollectionUpdateSchema::new(drops.chain(adds).collect())
    }

    /// The `POST /collections` body.
    pub fn to_create(&self) -> CollectionSchema<'static> {
        let fields = self.added.iter().map(|f| Field {
            name: f.name.trim().to_owned(),
            ..f.clone()
        });
        let mut schema =
            CollectionSchema::new(self.name.trim().to_owned().into(), fields.collect());
        let sort = self.default_sorting_field.trim();
        if !sort.is_empty() {
            schema.default_sorting_field = Some(sort.to_owned().into());
        }
        schema
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, ty: &str) -> Field {
        Field::new(name.to_owned(), ty.to_owned())
    }

    fn products() -> CollectionResponse {
        CollectionResponse::new(
            "products".into(),
            vec![
                field("name", "string"),
                field("price", "float"),
                field("legacy_sku", "string"),
            ],
            48_210,
            1,
        )
    }

    fn named(name: &str, ty: &str) -> Field {
        Field {
            name: name.to_owned(),
            r#type: ty.to_owned(),
            ..blank_field()
        }
    }

    #[test]
    fn flag_defaults_follow_typesense() {
        let s = field("a", "string");
        assert!(!facet(&s) && index(&s) && !optional(&s) && !sort(&s));
        assert!(sort(&field("n", "int32")));
        assert!(sort(&field("b", "bool")));
        assert!(!sort(&field("g", "geopoint")));
    }

    #[test]
    fn explicit_flags_override_defaults() {
        let mut f = field("n", "int32");
        f.sort = Some(false);
        f.index = Some(false);
        assert!(!sort(&f));
        assert!(!index(&f));
    }

    #[test]
    fn basic_types_are_the_form_types() {
        assert!(is_basic("string[]"));
        assert!(!is_basic("object"));
        assert!(!is_basic("float[]"));
    }

    #[test]
    fn alter_starts_locked_with_no_changes() {
        let d = SchemaDraft::alter(&products());
        assert!(!d.is_create());
        assert_eq!(d.existing().len(), 3);
        assert_eq!(d.documents(), 48_210);
        assert_eq!(d.change_count(), 0);
        assert!(d.pending_lines().is_empty());
    }

    #[test]
    fn alter_target_names_the_collection_only_when_altering() {
        assert_eq!(
            SchemaDraft::alter(&products()).alter_target(),
            Some("products")
        );
        assert_eq!(SchemaDraft::create().alter_target(), None);
    }

    #[test]
    fn toggle_drop_marks_and_unmarks_an_existing_field() {
        let mut d = SchemaDraft::alter(&products());
        d.toggle_drop("legacy_sku");
        assert!(d.is_dropped("legacy_sku"));
        assert_eq!(d.change_count(), 1);
        d.toggle_drop("legacy_sku");
        assert!(!d.is_dropped("legacy_sku"));
        assert_eq!(d.change_count(), 0);
    }

    #[test]
    fn toggle_drop_ignores_unknown_fields() {
        let mut d = SchemaDraft::alter(&products());
        d.toggle_drop("ghost");
        assert_eq!(d.change_count(), 0);
    }

    #[test]
    fn add_and_remove_rows() {
        let mut d = SchemaDraft::alter(&products());
        d.add_blank();
        d.add_blank();
        assert_eq!(d.added.len(), 2);
        d.remove_added(0);
        assert_eq!(d.added.len(), 1);
        d.remove_added(9);
        assert_eq!(d.added.len(), 1);
    }

    #[test]
    fn pending_lines_list_drops_then_adds_with_notable_flags() {
        let mut d = SchemaDraft::alter(&products());
        d.toggle_drop("legacy_sku");
        let mut tier = named("brand_tier", "string");
        tier.facet = Some(true);
        let mut released = named("released_at", "int64");
        released.optional = Some(true);
        d.added = vec![tier, released];
        assert_eq!(
            d.pending_lines(),
            [
                "− legacy_sku",
                "+ brand_tier string · facet",
                "+ released_at int64 · optional",
            ]
        );
    }

    #[test]
    fn a_string_field_sorted_explicitly_shows_sort() {
        let mut d = SchemaDraft::alter(&products());
        let mut f = named("title", "string");
        f.sort = Some(true);
        d.added = vec![f];
        assert_eq!(d.pending_lines(), ["+ title string · sort"]);
    }

    #[test]
    fn an_unindexed_field_says_so() {
        let mut d = SchemaDraft::alter(&products());
        let mut f = named("note", "string");
        f.index = Some(false);
        d.added = vec![f];
        assert_eq!(d.pending_lines(), ["+ note string · no index"]);
    }

    #[test]
    fn alter_without_changes_has_a_problem() {
        assert_eq!(
            SchemaDraft::alter(&products()).problems(),
            ["No changes to apply."]
        );
    }

    #[test]
    fn unnamed_and_duplicate_new_fields_are_problems() {
        let mut d = SchemaDraft::alter(&products());
        d.added = vec![
            named(" ", "string"),
            named("x", "int32"),
            named("x", "int32"),
        ];
        assert_eq!(
            d.problems(),
            ["Every new field needs a name.", "Field x is added twice."]
        );
    }

    #[test]
    fn a_new_field_may_not_shadow_an_existing_one_unless_it_is_dropped() {
        let mut d = SchemaDraft::alter(&products());
        d.added = vec![named("price", "float")];
        assert_eq!(
            d.problems(),
            ["Field price already exists. Drop it first to change it."]
        );
        d.toggle_drop("price");
        assert!(d.problems().is_empty());
    }

    #[test]
    fn create_needs_a_name_and_a_field() {
        let mut d = SchemaDraft::create();
        assert!(d.is_create());
        assert_eq!(
            d.problems(),
            ["Collection name is required.", "Add at least one field."]
        );
        d.name = "books".into();
        d.added = vec![named("title", "string")];
        assert!(d.problems().is_empty());
    }

    #[test]
    fn update_sends_drops_before_adds_so_a_changed_field_is_replaced() {
        let mut d = SchemaDraft::alter(&products());
        d.toggle_drop("price");
        d.added = vec![named(" price ", "int32")];
        let update = d.to_update();
        assert_eq!(update.fields.len(), 2);
        assert_eq!(update.fields[0].name, "price");
        assert_eq!(update.fields[0].drop, Some(true));
        assert_eq!(update.fields[0].r#type, "float");
        assert_eq!(update.fields[1].name, "price");
        assert_eq!(update.fields[1].r#type, "int32");
        assert_eq!(update.fields[1].drop, None);
    }

    #[test]
    fn create_body_carries_name_fields_and_default_sort() {
        let mut d = SchemaDraft::create();
        d.name = " books ".into();
        d.default_sorting_field = " year ".into();
        d.added = vec![named("year", "int32")];
        let schema = d.to_create();
        assert_eq!(schema.name, "books");
        assert_eq!(schema.default_sorting_field.as_deref(), Some("year"));
        assert_eq!(schema.fields.len(), 1);
    }

    #[test]
    fn a_blank_default_sort_is_omitted() {
        let mut d = SchemaDraft::create();
        d.name = "books".into();
        d.added = vec![named("title", "string")];
        assert_eq!(d.to_create().default_sorting_field, None);
    }
}
