//! Ordered user collections. Pure, worker-side model: no filesystem or audio work.
//! The production catalog instantiates `CrateForest<TrackId>` and supplies its
//! known-track predicate. The generic key keeps media/version policy in Catalog.
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::Hash;

pub(crate) const MAX_CRATES: usize = 4_096;
pub(crate) const MAX_DEPTH: usize = 32;
pub(crate) const MAX_NAME_BYTES: usize = 256;
pub(crate) const MAX_MEMBERS: usize = 100_000;
pub(crate) const MAX_TOTAL_MEMBERS: usize = 250_000;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct CrateId(pub String);
impl CrateId {
    fn valid(&self) -> bool {
        self.0.len() == 32
            && self
                .0
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Node<K> {
    pub id: CrateId,
    pub name: String,
    pub children: Vec<CrateId>,
    pub members: Vec<K>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CrateForest<K> {
    revision: u64,
    roots: Vec<CrateId>,
    nodes: Vec<Node<K>>,
}
impl<K> Default for CrateForest<K> {
    fn default() -> Self {
        Self {
            revision: 0,
            roots: vec![],
            nodes: vec![],
        }
    }
}

/// `before: None` appends. Anchors refer to the unfiltered destination list.
#[derive(Clone, Debug)]
pub(crate) enum Edit<K> {
    Create {
        id: CrateId,
        name: String,
        parent: Option<CrateId>,
        before: Option<CrateId>,
    },
    Rename {
        id: CrateId,
        name: String,
    },
    MoveCrate {
        id: CrateId,
        parent: Option<CrateId>,
        before: Option<CrateId>,
    },
    DeleteSubtree {
        id: CrateId,
    },
    AddMembers {
        id: CrateId,
        members: Vec<K>,
        before: Option<K>,
    },
    RemoveMembers {
        id: CrateId,
        members: Vec<K>,
    },
    MoveMembers {
        source: CrateId,
        destination: CrateId,
        members: Vec<K>,
        before: Option<K>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Error {
    Conflict,
    Invalid(&'static str),
    Limit(&'static str),
    MissingCrate,
    UnknownTrack,
    MissingMember,
    MissingAnchor,
    ImportConflict,
    RevisionExhausted,
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Conflict => f.write_str("Crates changed; refresh before editing"),
            Self::Invalid(reason) => write!(f, "Invalid crates: {reason}"),
            Self::Limit(reason) => write!(f, "Crate limit exceeded: {reason}"),
            Self::MissingCrate => f.write_str("Crate no longer exists"),
            Self::UnknownTrack => f.write_str("Crate member is not a saved catalog track"),
            Self::MissingMember => f.write_str("Selected track is no longer in the source crate"),
            Self::MissingAnchor => f.write_str("Destination position no longer exists"),
            Self::ImportConflict => f.write_str(
                "Imported crate identity, order or placement conflicts; nothing changed",
            ),
            Self::RevisionExhausted => f.write_str("Crate revision exhausted; nothing changed"),
        }
    }
}
impl std::error::Error for Error {}

impl<K: Clone + Eq + Hash> CrateForest<K> {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn roots(&self) -> &[CrateId] {
        &self.roots
    }
    pub fn nodes(&self) -> &[Node<K>] {
        &self.nodes
    }
    pub fn node(&self, id: &CrateId) -> Option<&Node<K>> {
        self.nodes.iter().find(|node| &node.id == id)
    }

    pub fn validate(&self, known: impl Fn(&K) -> bool) -> Result<(), Error> {
        if self.nodes.len() > MAX_CRATES || self.roots.len() > MAX_CRATES {
            return Err(Error::Limit("4096 crates"));
        }
        let mut index = HashMap::with_capacity(self.nodes.len());
        let mut total = 0usize;
        let mut edges = self.roots.len();
        for node in &self.nodes {
            if !node.id.valid() || index.insert(&node.id, node).is_some() {
                return Err(Error::Invalid("duplicate or malformed crate ID"));
            }
            edges = edges
                .checked_add(node.children.len())
                .ok_or(Error::Limit("tree edges"))?;
            if edges > self.nodes.len() {
                return Err(Error::Invalid("too many parent edges"));
            }
            validate_name(&node.name)?;
            if node.children.len() > MAX_CRATES || node.members.len() > MAX_MEMBERS {
                return Err(Error::Limit("crate children or 100000 members"));
            }
            total = total
                .checked_add(node.members.len())
                .ok_or(Error::Limit("membership total"))?;
            if total > MAX_TOTAL_MEMBERS {
                return Err(Error::Limit("250000 memberships"));
            }
            let mut members = HashSet::with_capacity(node.members.len());
            for member in &node.members {
                if !known(member) {
                    return Err(Error::UnknownTrack);
                }
                if !members.insert(member) {
                    return Err(Error::Invalid("duplicate member"));
                }
            }
        }
        let siblings = |ids: &[CrateId]| -> Result<(), Error> {
            let mut names = HashSet::with_capacity(ids.len());
            for id in ids {
                let node = index.get(id).ok_or(Error::MissingCrate)?;
                if !names.insert(node.name.as_str()) {
                    return Err(Error::Invalid("duplicate sibling name"));
                }
            }
            Ok(())
        };
        siblings(&self.roots)?;
        for node in &self.nodes {
            siblings(&node.children)?;
        }
        let mut visited = HashSet::with_capacity(self.nodes.len());
        let mut stack: Vec<_> = self.roots.iter().map(|id| (id, 1usize)).collect();
        while let Some((id, depth)) = stack.pop() {
            if depth > MAX_DEPTH {
                return Err(Error::Limit("32 levels"));
            }
            if !visited.insert(id) {
                return Err(Error::Invalid("cycle or multiple parents"));
            }
            let node = index.get(id).ok_or(Error::MissingCrate)?;
            stack.extend(node.children.iter().map(|child| (child, depth + 1)));
        }
        if visited.len() != self.nodes.len() {
            return Err(Error::Invalid("unreachable crate or cycle"));
        }
        Ok(())
    }

    /// Returns true only for a real edit. On every error self is unchanged.
    pub fn apply(
        &mut self,
        expected: u64,
        edit: &Edit<K>,
        known: impl Fn(&K) -> bool,
    ) -> Result<bool, Error> {
        if expected != self.revision {
            return Err(Error::Conflict);
        }
        self.validate(&known)?;
        edit.validate_payload()?;
        let mut next = self.clone();
        next.apply_inner(edit)?;
        next.validate(known)?;
        self.commit(next)
    }

    /// Merge disjoint roots or an identical overlapping forest. Shared nodes
    /// cannot acquire new parents, children, members or relative root order.
    /// Track import validation/merging is the enclosing Catalog's transaction.
    pub fn merge_import(
        &mut self,
        expected: u64,
        other: &Self,
        known: impl Fn(&K) -> bool,
    ) -> Result<bool, Error> {
        if expected != self.revision {
            return Err(Error::Conflict);
        }
        self.validate(&known)?;
        other.validate(&known)?;
        let existing: HashMap<_, _> = self.nodes.iter().map(|node| (&node.id, node)).collect();
        let incoming: HashMap<_, _> = other.nodes.iter().map(|node| (&node.id, node)).collect();
        let parents = |forest: &Self| -> HashMap<CrateId, Option<CrateId>> {
            let mut result: HashMap<_, _> =
                forest.roots.iter().map(|id| (id.clone(), None)).collect();
            for node in &forest.nodes {
                result.extend(
                    node.children
                        .iter()
                        .map(|id| (id.clone(), Some(node.id.clone()))),
                );
            }
            result
        };
        let old_parents = parents(self);
        let new_parents = parents(other);
        for node in &other.nodes {
            if let Some(old) = existing.get(&node.id) {
                if **old != *node || old_parents.get(&node.id) != new_parents.get(&node.id) {
                    return Err(Error::ImportConflict);
                }
            }
        }
        let old_shared: Vec<_> = self
            .roots
            .iter()
            .filter(|id| incoming.contains_key(id))
            .collect();
        let new_shared: Vec<_> = other
            .roots
            .iter()
            .filter(|id| existing.contains_key(id))
            .collect();
        if old_shared != new_shared {
            return Err(Error::ImportConflict);
        }
        let added = other
            .nodes
            .iter()
            .filter(|node| !existing.contains_key(&node.id))
            .count();
        if self.nodes.len() + added > MAX_CRATES {
            return Err(Error::Limit("4096 crates"));
        }
        let mut next = self.clone();
        next.roots.extend(
            other
                .roots
                .iter()
                .filter(|id| !existing.contains_key(id))
                .cloned(),
        );
        next.nodes.extend(
            other
                .nodes
                .iter()
                .filter(|node| !existing.contains_key(&node.id))
                .cloned(),
        );
        next.validate(known)?;
        self.commit(next)
    }

    fn commit(&mut self, mut next: Self) -> Result<bool, Error> {
        if self.roots == next.roots && self.nodes == next.nodes {
            return Ok(false);
        }
        next.revision = self
            .revision
            .checked_add(1)
            .ok_or(Error::RevisionExhausted)?;
        *self = next;
        Ok(true)
    }
    fn node_mut(&mut self, id: &CrateId) -> Result<&mut Node<K>, Error> {
        self.nodes
            .iter_mut()
            .find(|node| &node.id == id)
            .ok_or(Error::MissingCrate)
    }
    fn children_mut(&mut self, parent: &Option<CrateId>) -> Result<&mut Vec<CrateId>, Error> {
        match parent {
            Some(id) => Ok(&mut self.node_mut(id)?.children),
            None => Ok(&mut self.roots),
        }
    }
    fn apply_inner(&mut self, edit: &Edit<K>) -> Result<(), Error> {
        match edit {
            Edit::Create {
                id,
                name,
                parent,
                before,
            } => {
                if self.node(id).is_some() {
                    return Err(Error::Invalid("duplicate crate ID"));
                }
                if self.nodes.len() == MAX_CRATES {
                    return Err(Error::Limit("4096 crates"));
                }
                insert_before(
                    self.children_mut(parent)?,
                    std::slice::from_ref(id),
                    before.as_ref(),
                )?;
                self.nodes.push(Node {
                    id: id.clone(),
                    name: name.clone(),
                    children: vec![],
                    members: vec![],
                });
            }
            Edit::Rename { id, name } => self.node_mut(id)?.name.clone_from(name),
            Edit::MoveCrate { id, parent, before } => {
                if self.node(id).is_none() {
                    return Err(Error::MissingCrate);
                }
                if before.as_ref() == Some(id) {
                    return Err(Error::Invalid("self anchor"));
                }
                // Full post-validation rejects descendant cycles and depth
                // overflow, before the candidate can replace current state.
                self.roots.retain(|child| child != id);
                for node in &mut self.nodes {
                    node.children.retain(|child| child != id);
                }
                insert_before(
                    self.children_mut(parent)?,
                    std::slice::from_ref(id),
                    before.as_ref(),
                )?;
            }
            Edit::DeleteSubtree { id } => {
                if self.node(id).is_none() {
                    return Err(Error::MissingCrate);
                }
                let index: HashMap<_, _> = self.nodes.iter().map(|node| (&node.id, node)).collect();
                let mut stack = vec![id];
                let mut deleted = HashSet::new();
                while let Some(next) = stack.pop() {
                    let node = index.get(next).ok_or(Error::MissingCrate)?;
                    deleted.insert(next.clone());
                    stack.extend(node.children.iter());
                }
                self.roots.retain(|child| !deleted.contains(child));
                self.nodes.retain(|node| !deleted.contains(&node.id));
                for node in &mut self.nodes {
                    node.children.retain(|child| !deleted.contains(child));
                }
            }
            Edit::AddMembers {
                id,
                members,
                before,
            } => {
                reject_self_anchor(members, before.as_ref())?;
                let destination = &mut self.node_mut(id)?.members;
                check_anchor(destination, before.as_ref())?;
                let existing: HashSet<_> = destination.iter().collect();
                let added: Vec<_> = members
                    .iter()
                    .filter(|member| !existing.contains(member))
                    .cloned()
                    .collect();
                if destination.len() + added.len() > MAX_MEMBERS {
                    return Err(Error::Limit("100000 members"));
                }
                insert_before(destination, &added, before.as_ref())?;
            }
            Edit::RemoveMembers { id, members } => {
                let source = &mut self.node_mut(id)?.members;
                require_members(source, members)?;
                let selected: HashSet<_> = members.iter().collect();
                source.retain(|member| !selected.contains(member));
            }
            Edit::MoveMembers {
                source,
                destination,
                members,
                before,
            } => {
                reject_self_anchor(members, before.as_ref())?;
                require_members(
                    &self.node(source).ok_or(Error::MissingCrate)?.members,
                    members,
                )?;
                check_anchor(
                    &self.node(destination).ok_or(Error::MissingCrate)?.members,
                    before.as_ref(),
                )?;
                let selected: HashSet<_> = members.iter().collect();
                self.node_mut(source)?
                    .members
                    .retain(|member| !selected.contains(member));
                let target = &mut self.node_mut(destination)?.members;
                target.retain(|member| !selected.contains(member));
                if target.len() + members.len() > MAX_MEMBERS {
                    return Err(Error::Limit("100000 members"));
                }
                insert_before(target, members, before.as_ref())?;
            }
        }
        Ok(())
    }
}
impl<K: Eq + Hash> Edit<K> {
    fn validate_payload(&self) -> Result<(), Error> {
        match self {
            Self::Create { id, name, .. } => {
                if !id.valid() {
                    return Err(Error::Invalid("malformed crate ID"));
                }
                validate_name(name)
            }
            Self::Rename { name, .. } => validate_name(name),
            Self::AddMembers { members, .. }
            | Self::RemoveMembers { members, .. }
            | Self::MoveMembers { members, .. } => {
                if members.len() > MAX_MEMBERS {
                    return Err(Error::Limit("100000 selected members"));
                }
                let unique: HashSet<_> = members.iter().collect();
                if unique.len() != members.len() {
                    return Err(Error::Invalid("duplicate selection"));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}
fn validate_name(name: &str) -> Result<(), Error> {
    if name.is_empty()
        || name.len() > MAX_NAME_BYTES
        || name.trim() != name
        || name.chars().any(char::is_control)
    {
        return Err(Error::Invalid(
            "name must be trimmed, nonempty, control-free and at most 256 UTF-8 bytes",
        ));
    }
    Ok(())
}
fn check_anchor<T: Eq>(items: &[T], before: Option<&T>) -> Result<(), Error> {
    if before.is_some_and(|id| !items.contains(id)) {
        Err(Error::MissingAnchor)
    } else {
        Ok(())
    }
}
fn reject_self_anchor<T: Eq>(selected: &[T], before: Option<&T>) -> Result<(), Error> {
    if before.is_some_and(|id| selected.contains(id)) {
        Err(Error::Invalid("self anchor"))
    } else {
        Ok(())
    }
}
fn insert_before<T: Clone + Eq>(
    items: &mut Vec<T>,
    selected: &[T],
    before: Option<&T>,
) -> Result<(), Error> {
    let index = match before {
        Some(id) => items
            .iter()
            .position(|item| item == id)
            .ok_or(Error::MissingAnchor)?,
        None => items.len(),
    };
    items.splice(index..index, selected.iter().cloned());
    Ok(())
}
fn require_members<T: Eq + Hash>(source: &[T], selected: &[T]) -> Result<(), Error> {
    let existing: HashSet<_> = source.iter().collect();
    if selected.iter().any(|member| !existing.contains(member)) {
        Err(Error::MissingMember)
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "crates/tests.rs"]
mod tests;
