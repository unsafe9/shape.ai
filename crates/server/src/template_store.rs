//! Template persistence (CC3.2): builtin + user [`TemplateContract`]s stored as
//! [`Record`]s through the shared storage adapter.
//!
//! A template is one Record: `kind="template"`, `id="template:{templateId}"`,
//! payload = the `TemplateContract` JSON. The builtin templates from
//! [`shape_scene_core::registry`] are seeded ONCE, guarded by a metadata Record
//! (`templates-seeded`), so reopening a store does not duplicate them and a user
//! edit to a builtin id is never clobbered by a re-seed.
//!
//! Deleting a template writes a TOMBSTONE Record
//! (`kind="template-tombstone"`, `id="template-tombstone:{id}"`) in addition to
//! removing the template Record. The tombstone is what keeps a deleted *builtin*
//! gone: the seed step skips any id that carries a tombstone, so a builtin the
//! user removed is not silently resurrected on the next seed (e.g. after a
//! restart that re-runs the seed guard against a fresh-looking metadata Record).
//!
//! These are plain functions over a `&mut SqliteAdapter`; the registry wraps
//! them with its shared-store mutex (see [`crate::registry`]).

use shape_scene_core::{registry, TemplateContract};
use shape_storage_core::{Record, StorageAdapter};

/// Record `kind` for a stored template.
pub const KIND_TEMPLATE: &str = "template";
/// Record `kind` for a template tombstone.
pub const KIND_TEMPLATE_TOMBSTONE: &str = "template-tombstone";
/// Record `kind` (and id) of the one-shot seed guard.
pub const KIND_TEMPLATES_SEEDED: &str = "templates-seeded";

/// The fixed Record id of the seed guard. No `template:` prefix, so it is never
/// mistaken for a template Record by a prefix scan.
const SEEDED_RECORD_ID: &str = "templates-seeded";

/// The Record id for a stored template: `"template:{templateId}"`.
pub fn template_record_id(template_id: &str) -> String {
    format!("template:{template_id}")
}

/// The Record id for a template tombstone: `"template-tombstone:{templateId}"`.
pub fn tombstone_record_id(template_id: &str) -> String {
    format!("template-tombstone:{template_id}")
}

/// Whether `template_id` has been tombstoned (deleted).
fn is_tombstoned<S: StorageAdapter>(store: &S, template_id: &str) -> bool {
    store.load(&tombstone_record_id(template_id)).is_ok()
}

/// Seed the builtin templates ONCE. Idempotent: guarded by the
/// `templates-seeded` Record, so a second call (or a reopen) is a no-op. Any
/// builtin id that carries a tombstone is skipped, so a deleted builtin stays
/// deleted across a re-seed.
///
/// Returns the number of builtins actually written on this call (0 once seeded).
pub fn seed_builtins<S: StorageAdapter>(store: &mut S) -> anyhow::Result<usize> {
    if store.load(SEEDED_RECORD_ID).is_ok() {
        return Ok(0);
    }

    let mut written = 0usize;
    for contract in registry() {
        if is_tombstoned(store, &contract.metadata.id) {
            continue;
        }
        write_template(store, &contract)?;
        written += 1;
    }

    // Stamp the guard last so a crash mid-seed re-runs the seed (skipping the
    // already-written + tombstoned ids) rather than leaving a half-seeded store
    // marked done.
    store.save(Record {
        id: SEEDED_RECORD_ID.to_string(),
        kind: KIND_TEMPLATES_SEEDED.to_string(),
        version: 1,
        payload: Vec::new(),
    })?;
    Ok(written)
}

/// Persist (insert or overwrite) one template Record from its contract.
fn write_template<S: StorageAdapter>(
    store: &mut S,
    contract: &TemplateContract,
) -> anyhow::Result<()> {
    store.save(Record {
        id: template_record_id(&contract.metadata.id),
        kind: KIND_TEMPLATE.to_string(),
        version: 1,
        payload: serde_json::to_vec(contract).expect("template contract serializes"),
    })?;
    Ok(())
}

/// Create (or overwrite) a user template from `contract`. Clears any tombstone
/// on the same id, so re-creating a previously-deleted template id revives it.
pub fn create_template<S: StorageAdapter>(
    store: &mut S,
    contract: &TemplateContract,
) -> anyhow::Result<()> {
    let id = contract.metadata.id.clone();
    store.delete(&tombstone_record_id(&id))?;
    write_template(store, contract)
}

/// List every stored template (builtins seeded earlier + user templates), in
/// id-sorted order (the storage layer lists ids deterministically). Tombstoned
/// ids are not listed because their template Record was removed on delete and
/// the seed step never rewrites them.
pub fn list_templates<S: StorageAdapter>(store: &S) -> Vec<TemplateContract> {
    let prefix = "template:";
    let mut out: Vec<TemplateContract> = store
        .list()
        .unwrap_or_default()
        .into_iter()
        .filter(|id| id.starts_with(prefix))
        .filter_map(|id| store.load(&id).ok())
        .filter(|r| r.kind == KIND_TEMPLATE)
        .filter_map(|r| serde_json::from_slice(&r.payload).ok())
        .collect();
    out.sort_by(|a: &TemplateContract, b: &TemplateContract| a.metadata.id.cmp(&b.metadata.id));
    out
}

/// Delete a template by id: remove its Record and write a tombstone so a
/// deleted builtin is not re-seeded. Returns whether a template Record existed.
pub fn delete_template<S: StorageAdapter>(store: &mut S, template_id: &str) -> anyhow::Result<bool> {
    let removed = store.delete(&template_record_id(template_id))?;
    store.save(Record {
        id: tombstone_record_id(template_id),
        kind: KIND_TEMPLATE_TOMBSTONE.to_string(),
        version: 1,
        payload: Vec::new(),
    })?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shape_scene_core::{
        RecipeLayout, TemplateCategory, TemplateContract, TemplateExports, TemplateMetadata,
        TemplateRecipe, TemplateTags,
    };
    use shape_storage_core::SqliteAdapter;

    fn store() -> SqliteAdapter {
        SqliteAdapter::open_in_memory().expect("in-memory sqlite")
    }

    fn user_template(id: &str) -> TemplateContract {
        TemplateContract {
            metadata: TemplateMetadata {
                id: id.to_string(),
                title: format!("User {id}"),
                description: String::new(),
                category: TemplateCategory::General,
                icon: None,
                template_kind: "user".to_string(),
            },
            recipe: TemplateRecipe {
                frames: vec![],
                shapes: vec![],
                edges: vec![],
            },
            layout: RecipeLayout {
                origin: None,
                default_shape_size: None,
            },
            exports: TemplateExports {
                allowed: vec![],
                default: None,
            },
            tags: TemplateTags { suggested: vec![] },
            prompt_hints: None,
        }
    }

    #[test]
    fn seed_is_idempotent() {
        let mut s = store();
        let builtin_count = registry().len();

        let first = seed_builtins(&mut s).unwrap();
        assert_eq!(first, builtin_count, "first seed writes every builtin");
        assert_eq!(list_templates(&s).len(), builtin_count);

        let second = seed_builtins(&mut s).unwrap();
        assert_eq!(second, 0, "second seed is a no-op");
        assert_eq!(
            list_templates(&s).len(),
            builtin_count,
            "no duplicates after re-seed"
        );
    }

    #[test]
    fn deleting_a_builtin_tombstones_it_and_blocks_reseed() {
        let mut s = store();
        seed_builtins(&mut s).unwrap();
        let builtin_count = registry().len();

        // Delete a real builtin id.
        let victim = registry()[0].metadata.id.clone();
        assert!(delete_template(&mut s, &victim).unwrap());
        assert_eq!(list_templates(&s).len(), builtin_count - 1);
        assert!(!list_templates(&s).iter().any(|t| t.metadata.id == victim));

        // A re-seed (simulating a restart that re-runs the guard) must NOT bring
        // the deleted builtin back. Force the guard off to prove the tombstone —
        // not just the guard — is what protects the deletion.
        s.delete(SEEDED_RECORD_ID).unwrap();
        let written = seed_builtins(&mut s).unwrap();
        assert_eq!(
            written,
            builtin_count - 1,
            "re-seed skips the tombstoned builtin"
        );
        assert!(!list_templates(&s).iter().any(|t| t.metadata.id == victim));
    }

    #[test]
    fn create_list_delete_user_template() {
        let mut s = store();
        seed_builtins(&mut s).unwrap();
        let builtin_count = registry().len();

        create_template(&mut s, &user_template("my-tpl")).unwrap();
        let listed = list_templates(&s);
        assert_eq!(listed.len(), builtin_count + 1);
        let mine = listed.iter().find(|t| t.metadata.id == "my-tpl").unwrap();
        assert_eq!(mine.metadata.title, "User my-tpl");

        assert!(delete_template(&mut s, "my-tpl").unwrap());
        assert_eq!(list_templates(&s).len(), builtin_count);
        assert!(!list_templates(&s).iter().any(|t| t.metadata.id == "my-tpl"));
    }

    #[test]
    fn recreating_a_deleted_user_template_revives_it() {
        let mut s = store();
        create_template(&mut s, &user_template("t")).unwrap();
        assert!(delete_template(&mut s, "t").unwrap());
        assert!(list_templates(&s).is_empty());
        // Re-create clears the tombstone so it lists again.
        create_template(&mut s, &user_template("t")).unwrap();
        assert_eq!(list_templates(&s).len(), 1);
    }
}
