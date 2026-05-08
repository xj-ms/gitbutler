//! The name of the file holding our state, useful for watching for changes.

use std::collections::HashMap;

use anyhow::Result;
use bstr::BString;
use but_db::{HunkAssignmentsHandle, HunkAssignmentsHandleMut};

use crate::HunkAssignment;

pub fn assignments(db: HunkAssignmentsHandle) -> Result<Vec<HunkAssignment>> {
    let (assignments, _) = assignments_with_previous_paths(db)?;
    Ok(assignments)
}

pub fn assignments_with_previous_paths(
    db: HunkAssignmentsHandle,
) -> Result<(Vec<HunkAssignment>, HashMap<BString, BString>)> {
    let rows = db.list_all()?;
    let mut previous_paths = HashMap::new();
    let mut assignments = Vec::with_capacity(rows.len());
    for row in rows {
        if let Some(prev) = row.previous_path_bytes.as_ref() {
            previous_paths.insert(
                BString::from(row.path_bytes.clone()),
                BString::from(prev.clone()),
            );
        }
        assignments.push(row.try_into()?);
    }
    Ok((assignments, previous_paths))
}

pub fn set_assignments(
    db: HunkAssignmentsHandleMut,
    assignments: Vec<HunkAssignment>,
    previous_paths: &HashMap<BString, BString>,
) -> Result<()> {
    let assignments: Vec<but_db::HunkAssignment> = assignments
        .into_iter()
        .map(|a| {
            let prev = previous_paths.get(&a.path_bytes).map(|p| p.to_vec());
            let mut db_assignment: but_db::HunkAssignment = a.try_into()?;
            db_assignment.previous_path_bytes = prev;
            Ok(db_assignment)
        })
        .collect::<Result<Vec<but_db::HunkAssignment>>>()?;
    db.set_all(assignments).map_err(Into::into)
}
