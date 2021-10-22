use std::collections::HashSet;
use std::collections::VecDeque;

use crate::graph::Graph;
use crate::graph::Node;

pub fn protect_branches(
    graph: &mut Graph,
    repo: &dyn crate::git::Repo,
    protected_branches: &crate::git::Branches,
) {
    let root_id = graph.root_id();

    let protected_oids: HashSet<_> = protected_branches
        .iter()
        .flat_map(|(_, branches)| branches.iter().map(|b| b.pull_id.unwrap_or(b.id)))
        .collect();

    for protected_oid in protected_oids.into_iter().filter(|protected_oid| {
        repo.merge_base(root_id, *protected_oid)
            .map(|merge_base_oid| merge_base_oid == root_id)
            .unwrap_or(false)
    }) {
        for commit in repo.commits_from(protected_oid) {
            if let Some(node) = graph.get_mut(commit.id) {
                if node.action.is_protected() {
                    break;
                }
                node.action = crate::graph::Action::Protected;
            }
            if commit.id == root_id {
                break;
            }
        }
    }
}

pub fn protect_large_branches(graph: &mut Graph, max: usize) -> Vec<String> {
    let mut large_branches = Vec::new();

    let mut protected_queue = VecDeque::new();
    if graph.root().action.is_protected() {
        protected_queue.push_back(graph.root_id());
    }
    while let Some(current_id) = protected_queue.pop_front() {
        let current_children = graph
            .get(current_id)
            .expect("all children exist")
            .children
            .clone();

        for child_id in current_children {
            let child_action = graph.get(child_id).expect("all children exist").action;
            if child_action.is_protected() {
                protected_queue.push_back(child_id);
            } else {
                let protected =
                    protect_large_branches_recursive(graph, child_id, 0, max, &mut large_branches);
                if protected {
                    protected_queue.push_back(child_id);
                }
            }
        }
    }

    large_branches
}

fn protect_large_branches_recursive(
    graph: &mut Graph,
    node_id: git2::Oid,
    count: usize,
    max: usize,
    large_branches: &mut Vec<String>,
) -> bool {
    let mut needs_protection = false;

    if !graph
        .get(node_id)
        .expect("all children exist")
        .branches
        .is_empty()
    {
    } else if count <= max {
        let current_children = graph
            .get(node_id)
            .expect("all children exist")
            .children
            .clone();

        for child_id in current_children {
            needs_protection |=
                protect_large_branches_recursive(graph, child_id, count + 1, max, large_branches);
        }
        if needs_protection {
            let mut node = graph.get_mut(node_id).expect("all children exist");
            node.action = crate::graph::Action::Protected;
        }
    } else {
        mark_branch_protected(graph, node_id, large_branches);
        needs_protection = true;
    }

    needs_protection
}

fn mark_branch_protected(graph: &mut Graph, node_id: git2::Oid, branches: &mut Vec<String>) {
    let mut protected_queue = VecDeque::new();
    protected_queue.push_back(node_id);
    while let Some(current_id) = protected_queue.pop_front() {
        let mut current = graph.get_mut(current_id).expect("all children exist");
        current.action = crate::graph::Action::Protected;

        if current.branches.is_empty() {
            protected_queue.extend(&graph.get(current_id).expect("all children exist").children);
        } else {
            branches.extend(current.branches.iter().map(|b| b.name.clone()));
        }
    }
}

pub fn protect_old_branches(graph: &mut Graph, earlier_than: std::time::SystemTime) -> Vec<String> {
    let mut old_branches = Vec::new();

    let mut protected_queue = VecDeque::new();
    if graph.root().action.is_protected() {
        protected_queue.push_back(graph.root_id());
    }
    while let Some(current_id) = protected_queue.pop_front() {
        let current_children = graph
            .get(current_id)
            .expect("all children exist")
            .children
            .clone();

        for child_id in current_children {
            let child_action = graph.get(child_id).expect("all children exist").action;
            if child_action.is_protected() {
                protected_queue.push_back(child_id);
            } else {
                if is_branch_old(graph, child_id, earlier_than) {
                    mark_branch_protected(graph, child_id, &mut old_branches);
                }
            }
        }
    }

    old_branches
}

pub fn trim_old_branches(graph: &mut Graph, earlier_than: std::time::SystemTime) -> Vec<String> {
    let mut old_branches = Vec::new();

    let mut protected_queue = VecDeque::new();
    if graph.root().action.is_protected() {
        protected_queue.push_back(graph.root_id());
    }
    while let Some(current_id) = protected_queue.pop_front() {
        let current_children = graph
            .get(current_id)
            .expect("all children exist")
            .children
            .clone();

        for child_id in current_children {
            let child_action = graph.get(child_id).expect("all children exist").action;
            if child_action.is_protected() {
                protected_queue.push_back(child_id);
            } else {
                if is_branch_old(graph, child_id, earlier_than) {
                    let removed = graph
                        .remove_child(current_id, child_id)
                        .expect("all children exist");
                    old_branches.extend(
                        removed
                            .breadth_first_iter()
                            .flat_map(|n| n.branches.iter().map(|b| b.name.clone())),
                    );
                }
            }
        }
    }

    old_branches
}

fn is_branch_old(graph: &Graph, node_id: git2::Oid, earlier_than: std::time::SystemTime) -> bool {
    let current = graph.get(node_id).expect("all children exist");

    if earlier_than < current.commit.time {
        return false;
    }

    for child_id in current.children.iter().copied() {
        if !is_branch_old(graph, child_id, earlier_than) {
            return false;
        }
    }

    true
}

pub fn protect_foreign_branches(graph: &mut Graph, user: &str) -> Vec<String> {
    let mut foreign_branches = Vec::new();

    let mut protected_queue = VecDeque::new();
    if graph.root().action.is_protected() {
        protected_queue.push_back(graph.root_id());
    }
    while let Some(current_id) = protected_queue.pop_front() {
        let current_children = graph
            .get(current_id)
            .expect("all children exist")
            .children
            .clone();

        for child_id in current_children {
            let child_action = graph.get(child_id).expect("all children exist").action;
            if child_action.is_protected() {
                protected_queue.push_back(child_id);
            } else {
                if !is_personal_branch(graph, child_id, user) {
                    mark_branch_protected(graph, child_id, &mut foreign_branches);
                }
            }
        }
    }

    foreign_branches
}

fn is_personal_branch(graph: &Graph, node_id: git2::Oid, user: &str) -> bool {
    let current = graph.get(node_id).expect("all children exist");

    if current.commit.committer.as_deref() == Some(user)
        || current.commit.author.as_deref() == Some(user)
    {
        return true;
    }

    for child_id in current.children.iter().copied() {
        if is_personal_branch(graph, child_id, user) {
            return true;
        }
    }

    false
}

/// Pre-requisites:
/// - Running protect_branches
///
/// # Panics
///
/// - If `new_base_id` doesn't exist
pub fn rebase_branches(graph: &mut Graph, new_base_id: git2::Oid) {
    debug_assert!(graph.get(new_base_id).is_some());

    let mut protected_queue = VecDeque::new();
    if graph.root().action.is_protected() {
        protected_queue.push_back(graph.root_id());
    }
    while let Some(current_id) = protected_queue.pop_front() {
        let current_children = graph
            .get(current_id)
            .expect("all children exist")
            .children
            .clone();

        let mut rebaseable = Vec::new();
        for child_id in current_children {
            let child_action = graph.get(child_id).expect("all children exist").action;
            if child_action.is_protected() {
                protected_queue.push_back(child_id);
            } else {
                rebaseable.push(child_id);
            }
        }

        if !rebaseable.is_empty() {
            let current = graph.get_mut(current_id).expect("all children exist");
            for child_id in rebaseable.iter().copied() {
                current.children.remove(&child_id);
            }
            graph
                .get_mut(new_base_id)
                .expect("pre-asserted")
                .children
                .extend(rebaseable);
        }
    }
}

pub fn pushable(graph: &mut Graph) {
    let mut node_queue: VecDeque<(git2::Oid, Option<&str>)> = VecDeque::new();

    // No idea if a parent commit invalidates our results
    if graph.root().action.is_protected() {
        node_queue.push_back((graph.root_id(), None));
    }
    while let Some((current_id, mut cause)) = node_queue.pop_front() {
        let current = graph.get_mut(current_id).expect("all children exist");
        if !current.action.is_protected() {
            if current.branches.iter().all(|b| Some(b.id) == b.push_id) {
                cause = Some("already pushed");
            } else if current.commit.wip_summary().is_some() {
                cause = Some("contains WIP commit");
            }

            if !current.branches.is_empty() {
                let branch = &current.branches[0];
                if let Some(cause) = cause {
                    log::debug!("{} isn't pushable, {}", branch.name, cause);
                } else {
                    log::debug!("{} is pushable", branch.name);
                    current.pushable = true;
                }
                // Bail out, only the first branch of a stack is up for consideration
                continue;
            }
        }

        node_queue.extend(current.children.iter().copied().map(|id| (id, cause)));
    }
}

/// Quick pass for what is droppable
///
/// We get into this state when a branch is squashed.  The id would be different due to metadata
/// but the tree_id, associated with the repo, is the same if your branch is up-to-date.
///
/// The big risk is if a commit was reverted.  To protect against this, we only look at the final
/// state of the branch and then check if it looks like a revert.
///
/// To avoid walking too much of the tree, we are going to assume only the first branch in a stack
/// could have been squash-merged.
///
/// This assumes that the Node was rebased onto all of the new potentially squash-merged Nodes and
/// we extract the potential tree_id's from those protected commits.
pub fn drop_by_tree_id(graph: &mut Graph) {
    let mut node_queue: VecDeque<(git2::Oid, HashSet<git2::Oid>)> = VecDeque::new();
    if graph.root().action.is_protected() {
        node_queue.push_back((graph.root_id(), HashSet::new()));
    }
    while let Some((protected_id, mut protected_tree_ids)) = node_queue.pop_front() {
        protected_tree_ids.insert(
            graph
                .get(protected_id)
                .expect("all children exist")
                .commit
                .tree_id,
        );

        let protected_children = graph
            .get(protected_id)
            .expect("all children exist")
            .children
            .clone();
        match protected_children.len() {
            0 => (),
            1 => {
                let child_id = protected_children.into_iter().next().unwrap();
                let child_action = graph.get(child_id).expect("all children exist").action;
                if child_action.is_protected() {
                    node_queue.push_back((child_id, protected_tree_ids));
                } else {
                    drop_first_branch_by_tree_id(
                        graph,
                        child_id,
                        HashSet::new(),
                        protected_tree_ids,
                    );
                }
            }
            _ => {
                for child_id in protected_children {
                    let child_action = graph.get(child_id).expect("all children exist").action;
                    if child_action.is_protected() {
                        node_queue.push_back((child_id, protected_tree_ids.clone()));
                    } else {
                        drop_first_branch_by_tree_id(
                            graph,
                            child_id,
                            HashSet::new(),
                            protected_tree_ids.clone(),
                        );
                    }
                }
            }
        }
    }
}

fn drop_first_branch_by_tree_id(
    graph: &mut Graph,
    node_id: git2::Oid,
    mut branch_ids: std::collections::HashSet<git2::Oid>,
    protected_tree_ids: std::collections::HashSet<git2::Oid>,
) {
    #![allow(clippy::if_same_then_else)]
    branch_ids.insert(node_id);

    let node = graph.get(node_id).expect("all children exist");
    debug_assert!(!node.action.is_protected());
    if node.commit.revert_summary().is_some() {
        // Might not *actually* be a revert or something more complicated might be going on.  Let's
        // just be cautious.
        return;
    }

    let is_branch = !node.branches.is_empty();
    let node_tree_id = node.commit.tree_id;

    if is_branch {
        if protected_tree_ids.contains(&node_tree_id) {
            for branch_id in branch_ids {
                graph.get_mut(branch_id).expect("all children exist").action =
                    crate::graph::Action::Delete;
            }
        } else {
        }
    } else {
        let node_children = graph
            .get(node_id)
            .expect("all children exist")
            .children
            .clone();
        match node_children.len() {
            0 => {}
            1 => {
                let child_id = node_children.into_iter().next().unwrap();
                drop_first_branch_by_tree_id(graph, child_id, branch_ids, protected_tree_ids);
            }
            _ => {
                for child_id in node_children {
                    drop_first_branch_by_tree_id(
                        graph,
                        child_id,
                        branch_ids.clone(),
                        protected_tree_ids.clone(),
                    );
                }
            }
        }
    }
}

pub fn fixup(graph: &mut Graph, effect: crate::config::Fixup) {
    if effect == crate::config::Fixup::Ignore {
        return;
    }

    let mut protected_queue = VecDeque::new();
    let root_action = graph.root().action;
    if root_action.is_protected() {
        protected_queue.push_back(graph.root_id());
    }
    while let Some(current_id) = protected_queue.pop_front() {
        let current_children = graph
            .get_mut(current_id)
            .expect("all children exist")
            .children
            .clone();

        for child_id in current_children {
            let child_action = graph.get(child_id).expect("all children exist").action;
            if child_action.is_protected() || child_action.is_delete() {
                protected_queue.push_back(child_id);
            } else {
                fixup_branch(graph, current_id, child_id, effect);
            }
        }
    }
}

fn fixup_branch(
    graph: &mut Graph,
    base_id: git2::Oid,
    mut node_id: git2::Oid,
    effect: crate::config::Fixup,
) {
    debug_assert_ne!(effect, crate::config::Fixup::Ignore);

    let mut outstanding = std::collections::BTreeMap::new();
    let node_children = graph
        .get_mut(node_id)
        .expect("all children exist")
        .children
        .clone();
    for child_id in node_children {
        fixup_node(graph, node_id, child_id, effect, &mut outstanding);
    }
    if !outstanding.is_empty() {
        let node = graph.get_mut(node_id).expect("all children exist");
        if let Some(fixup_ids) = outstanding.remove(&node.commit.summary) {
            if effect == crate::config::Fixup::Squash {
                for fixup_id in fixup_ids.iter().copied() {
                    let fixup = graph.get_mut(fixup_id).expect("all children exist");
                    assert!(fixup.action == crate::graph::Action::Pick);
                    fixup.action = crate::graph::Action::Squash;
                }
            }
            splice_after(graph, node_id, fixup_ids);
        }
        debug_assert_ne!(
            graph.get(node_id).expect("all children exist").action,
            crate::graph::Action::Protected,
            "Unexpected result for {}",
            base_id
        );
        for fixup_ids in outstanding.into_values() {
            node_id = splice_between(graph, base_id, node_id, fixup_ids);
        }
    }
}

fn fixup_node(
    graph: &mut Graph,
    base_id: git2::Oid,
    node_id: git2::Oid,
    effect: crate::config::Fixup,
    outstanding: &mut std::collections::BTreeMap<bstr::BString, Vec<git2::Oid>>,
) {
    debug_assert_ne!(effect, crate::config::Fixup::Ignore);

    let node_children = graph
        .get_mut(node_id)
        .expect("all children exist")
        .children
        .clone();
    for child_id in node_children {
        fixup_node(graph, node_id, child_id, effect, outstanding);
    }

    let mut patch = None;
    let mut fixup_ids = Vec::new();
    {
        let node = graph.get_mut(node_id).expect("all children exist");
        debug_assert_ne!(node.action, crate::graph::Action::Protected);
        debug_assert_ne!(node.action, crate::graph::Action::Delete);
        if let Some(summary) = node.commit.fixup_summary() {
            outstanding
                .entry(summary.to_owned())
                .or_insert_with(Default::default)
                .push(node_id);

            let mut children = Default::default();
            std::mem::swap(&mut node.children, &mut children);
            let mut branches = Default::default();
            std::mem::swap(&mut node.branches, &mut branches);
            patch = Some((children, branches));
        } else if let Some(ids) = outstanding.remove(&node.commit.summary) {
            fixup_ids = ids;
        }
    }

    if let Some((children, branches)) = patch {
        debug_assert!(fixup_ids.is_empty());

        let base = graph.get_mut(base_id).expect("all children exist");
        debug_assert_ne!(base.action, crate::graph::Action::Protected);
        debug_assert_ne!(base.action, crate::graph::Action::Delete);
        base.children.remove(&node_id);
        base.children.extend(children);
        base.branches.extend(branches);
    } else if !fixup_ids.is_empty() {
        if effect == crate::config::Fixup::Squash {
            for fixup_id in fixup_ids.iter().copied() {
                let fixup = graph.get_mut(fixup_id).expect("all children exist");
                assert!(fixup.action == crate::graph::Action::Pick);
                fixup.action = crate::graph::Action::Squash;
            }
        }
        splice_after(graph, node_id, fixup_ids);
    }
}

// Does not update references
fn splice_between(
    graph: &mut Graph,
    parent_id: git2::Oid,
    child_id: git2::Oid,
    node_ids: Vec<git2::Oid>,
) -> git2::Oid {
    let mut new_child_id = child_id;
    for node_id in node_ids.into_iter() {
        let node = graph.get_mut(node_id).expect("all children exist");
        debug_assert!(node.children.is_empty());
        node.children.insert(new_child_id);
        new_child_id = node.commit.id;
    }
    let parent = graph.get_mut(parent_id).expect("all children exist");
    parent.children.remove(&child_id);
    parent.children.insert(new_child_id);
    new_child_id
}

// Updates references
fn splice_after(graph: &mut Graph, node_id: git2::Oid, fixup_ids: Vec<git2::Oid>) {
    if fixup_ids.is_empty() {
        return;
    }

    let mut new_children = Default::default();
    let mut new_branches = Default::default();
    {
        let node = graph.get_mut(node_id).expect("all children exist");
        std::mem::swap(&mut node.children, &mut new_children);
        std::mem::swap(&mut node.branches, &mut new_branches);
    }

    let mut last_id = node_id;
    for fixup_id in fixup_ids.into_iter().rev() {
        let last = graph.get_mut(last_id).expect("all children exist");
        last.children.insert(fixup_id);
        last_id = fixup_id;
    }

    {
        let last = graph.get_mut(last_id).expect("all children exist");
        debug_assert!(last.children.is_empty());
        debug_assert!(last.branches.is_empty());
        std::mem::swap(&mut last.children, &mut new_children);
        std::mem::swap(&mut last.branches, &mut new_branches);
    }
}

pub fn to_script(graph: &Graph) -> crate::git::Script {
    let mut script = crate::git::Script::new();

    let mut protected_queue = VecDeque::new();
    if graph.root().action.is_protected() {
        protected_queue.push_back(graph.root_id());
    }
    while let Some(current_id) = protected_queue.pop_front() {
        let current = graph.get(current_id).expect("all children exist");

        for child_id in current.children.iter().copied() {
            let child_action = graph.get(child_id).expect("all children exist").action;
            if child_action.is_protected() {
                protected_queue.push_back(child_id);
            } else if let Some(mut dependent) = node_to_script(graph, child_id) {
                dependent
                    .commands
                    .insert(0, crate::git::Command::SwitchCommit(current_id));
                script.dependents.push(dependent);
            }
        }
    }

    script
}

fn node_to_script(graph: &Graph, node_id: git2::Oid) -> Option<crate::git::Script> {
    let mut script = crate::git::Script::new();

    let node = graph.get(node_id).expect("all children exist");
    match node.action {
        crate::graph::Action::Pick => {
            script
                .commands
                .push(crate::git::Command::CherryPick(node.commit.id));
            for branch in node.branches.iter() {
                script
                    .commands
                    .push(crate::git::Command::CreateBranch(branch.name.clone()));
            }

            let node_dependents: Vec<_> = node
                .children
                .iter()
                .copied()
                .filter_map(|child_id| node_to_script(graph, child_id))
                .collect();
            if !node_dependents.is_empty() {
                // End the transaction on branch boundaries
                let transaction = !node.branches.is_empty();
                extend_dependents(node, &mut script, node_dependents, transaction);
            }
        }
        crate::graph::Action::Squash => {
            script
                .commands
                .push(crate::git::Command::Squash(node.commit.id));
            // We can't re-target the branches of the commit we are squashing into, so the ops that
            // creates a `Squash` option has to handle that.
            for branch in node.branches.iter() {
                script
                    .commands
                    .push(crate::git::Command::CreateBranch(branch.name.clone()));
            }

            let node_dependents: Vec<_> = node
                .children
                .iter()
                .copied()
                .filter_map(|child_id| node_to_script(graph, child_id))
                .collect();
            if !node_dependents.is_empty() {
                // End the transaction on branch boundaries
                let transaction = !node.branches.is_empty();
                extend_dependents(node, &mut script, node_dependents, transaction);
            }
        }
        crate::graph::Action::Protected => {
            let node_dependents: Vec<_> = node
                .children
                .iter()
                .copied()
                .filter_map(|child_id| node_to_script(graph, child_id))
                .collect();
            if !node_dependents.is_empty() {
                let stack_mark = node.commit.id;
                script
                    .commands
                    .push(crate::git::Command::SwitchCommit(stack_mark));

                // No transactions needed for protected commits
                let transaction = false;
                extend_dependents(node, &mut script, node_dependents, transaction);
            }
        }
        crate::graph::Action::Delete => {
            for branch in node.branches.iter() {
                script
                    .commands
                    .push(crate::git::Command::DeleteBranch(branch.name.clone()));
            }

            let node_dependents: Vec<_> = node
                .children
                .iter()
                .copied()
                .filter_map(|child_id| node_to_script(graph, child_id))
                .collect();
            if !node_dependents.is_empty() {
                // End the transaction on branch boundaries
                let transaction = !node.branches.is_empty();
                extend_dependents(node, &mut script, node_dependents, transaction);
            }
        }
    }

    if script.is_empty() {
        None
    } else {
        Some(script)
    }
}

fn extend_dependents(
    node: &Node,
    script: &mut crate::git::Script,
    mut dependents: Vec<crate::git::Script>,
    transaction: bool,
) {
    // Create transactions at the branch boundaries
    if !transaction && dependents.len() == 1 {
        let dependent = dependents.remove(0);
        script.commands.extend(dependent.commands);
        script.dependents.extend(dependent.dependents);
    } else {
        // Ensure each dependent can pick up where needed
        let stack_mark = node.commit.id;
        script
            .commands
            .push(crate::git::Command::RegisterMark(stack_mark));
        for dependent in dependents.iter_mut() {
            dependent
                .commands
                .insert(0, crate::git::Command::SwitchMark(stack_mark));
        }
        script.dependents.extend(dependents);
    }
}
