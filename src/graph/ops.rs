pub use crate::graph::Node;
pub use crate::graph::Stack;

pub fn protect_branches(
    root: &mut Node,
    repo: &dyn crate::git::Repo,
    protected_branches: &crate::git::Branches,
) -> Result<(), git2::Error> {
    // Assuming the root is the base.  The base is not guaranteed to be a protected branch but
    // might be an ancestor of one.
    //
    // We can't use `descendant_protected` because a protect branch might not be in the
    // descendants, depending on what Graph the user selected.
    for protected_oid in protected_branches.oids() {
        if let Some(merge_base_oid) = repo.merge_base(root.local_commit.id, protected_oid) {
            if merge_base_oid == root.local_commit.id {
                root.action = crate::graph::Action::Protected;
                break;
            }
        }
    }

    for stack in root.stacks.iter_mut() {
        protect_branches_stack(stack, repo, protected_branches)?;
    }

    Ok(())
}

fn protect_branches_stack(
    nodes: &mut Stack,
    repo: &dyn crate::git::Repo,
    protected_branches: &crate::git::Branches,
) -> Result<bool, git2::Error> {
    let mut descendant_protected = false;
    for node in nodes.iter_mut().rev() {
        let mut stacks_protected = false;
        for stack in node.stacks.iter_mut() {
            let stack_protected = protect_branches_stack(stack, repo, protected_branches)?;
            stacks_protected |= stack_protected;
        }
        let self_protected = protected_branches.contains_oid(node.local_commit.id);
        if descendant_protected || stacks_protected || self_protected {
            node.action = crate::graph::Action::Protected;
            descendant_protected = true;
        }
    }

    Ok(descendant_protected)
}

/// Pre-requisites:
/// - Running protect_branches
///
/// # Panics
///
/// If `new_base_id` doesn't exist
pub fn rebase_branches(node: &mut Node, new_base_id: git2::Oid) -> Result<(), git2::Error> {
    let mut rebaseable = Vec::new();
    pop_rebaseable_stacks(node, &mut rebaseable);

    let new_base = node.find_commit_mut(new_base_id).unwrap();
    new_base.stacks.extend(rebaseable);

    Ok(())
}

fn pop_rebaseable_stacks(node: &mut Node, rebaseable: &mut Vec<Stack>) {
    if !node.action.is_protected() {
        // The parent should pop this node
        return;
    }

    // Rebase the full stack
    let mut full_stack = Vec::new();
    for (index, stack) in node.stacks.iter().enumerate() {
        if !stack.first().action.is_protected() {
            full_stack.push(index);
        }
    }
    full_stack.reverse();
    for index in full_stack {
        rebaseable.push(node.stacks.remove(index));
    }

    for stack in node.stacks.iter_mut() {
        let mut base_index = None;
        for (index, node) in stack.iter_mut().enumerate() {
            if node.action.is_protected() {
                pop_rebaseable_stacks(node, rebaseable);
            } else {
                base_index = Some(index);
                break;
            }
        }
        if let Some(index) = base_index {
            let remaining = stack.split_off(index).unwrap();
            rebaseable.push(remaining);
        }
    }
}

pub fn pushable(node: &mut Node) -> Result<(), git2::Error> {
    if node.action.is_protected() || node.branches.is_empty() {
        for stack in node.stacks.iter_mut() {
            pushable_stack(stack)?;
        }
    }
    Ok(())
}

fn pushable_stack(nodes: &mut Stack) -> Result<(), git2::Error> {
    let mut cause = None;
    for node in nodes.iter_mut() {
        if node.action.is_protected() {
            assert_eq!(cause, None);
            for stack in node.stacks.iter_mut() {
                pushable_stack(stack)?;
            }
            continue;
        }

        if node.local_commit.wip_summary().is_some() {
            cause = Some("contains WIP commit");
        }

        if !node.branches.is_empty() {
            let branch = &node.branches[0];
            if let Some(cause) = cause {
                log::debug!("{} isn't pushable, {}", branch.name, cause);
            } else if node.branches.iter().all(|b| Some(b.id) == b.push_id) {
                log::debug!("{} is already pushed", branch.name);
            } else {
                log::debug!("{} is pushable", branch.name);
                node.pushable = true;
            }
            // Bail out, only the first branch of a stack is up for consideration
            return Ok(());
        } else {
            for stack in node.stacks.iter_mut() {
                pushable_stack(stack)?;
            }
        }
    }

    Ok(())
}

pub fn delinearize(node: &mut Node) {
    for stack in node.stacks.iter_mut() {
        delinearize_stack(stack);
    }
}

pub(crate) fn delinearize_stack(nodes: &mut Stack) {
    for node in nodes.iter_mut() {
        for stack in node.stacks.iter_mut() {
            delinearize_stack(stack);
        }
    }

    let splits: Vec<_> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| !n.stacks.is_empty() || !n.branches.is_empty())
        .map(|(i, _)| i + 1)
        .rev()
        .collect();
    for split in splits {
        if split == nodes.len() {
            continue;
        }
        let stack = nodes.split_off(split).unwrap();
        nodes.last_mut().stacks.push(stack);
    }
}

pub fn linearize_by_size(node: &mut Node) {
    for stack in node.stacks.iter_mut() {
        linearize_stack(stack);
    }
    node.stacks.sort_by_key(|s| s.len());
}

fn linearize_stack(nodes: &mut Stack) {
    let append = {
        let last = nodes.last_mut();
        match last.stacks.len() {
            0 => {
                return;
            }
            1 => {
                let mut append = last.stacks.pop().unwrap();
                linearize_stack(&mut append);
                assert!(last.stacks.is_empty());
                append
            }
            _ => {
                for stack in last.stacks.iter_mut() {
                    linearize_stack(stack);
                }
                last.stacks.sort_by_key(|s| s.len());
                last.stacks.pop().unwrap()
            }
        }
    };
    nodes.extend(append);
}

pub fn to_script(node: &Node) -> crate::git::Script {
    let mut script = crate::git::Script::new();

    match node.action {
        // The base should be immutable, so nothing to cherry-pick
        crate::graph::Action::Pick | crate::graph::Action::Protected => {
            let stack_mark = node.local_commit.id;
            script
                .commands
                .push(crate::git::Command::SwitchCommit(stack_mark));
            script
                .commands
                .push(crate::git::Command::RegisterMark(stack_mark));
            for stack in node.stacks.iter() {
                script
                    .dependents
                    .extend(to_script_internal(stack, stack_mark));
            }
        }
        crate::graph::Action::Delete => {
            assert!(node.stacks.is_empty());
            for branch in node.branches.iter() {
                script
                    .commands
                    .push(crate::git::Command::DeleteBranch(branch.name.clone()));
            }
        }
    }

    script
}

fn to_script_internal(nodes: &[Node], base_mark: git2::Oid) -> Option<crate::git::Script> {
    let mut script = crate::git::Script::new();
    for node in nodes {
        match node.action {
            crate::graph::Action::Pick => {
                script
                    .commands
                    .push(crate::git::Command::CherryPick(node.local_commit.id));
                for branch in node.branches.iter() {
                    script
                        .commands
                        .push(crate::git::Command::CreateBranch(branch.name.clone()));
                }

                if !node.stacks.is_empty() {
                    let stack_mark = node.local_commit.id;
                    script
                        .commands
                        .push(crate::git::Command::RegisterMark(stack_mark));
                    for stack in node.stacks.iter() {
                        script
                            .dependents
                            .extend(to_script_internal(stack, stack_mark));
                    }
                }
            }
            crate::graph::Action::Protected => {
                for stack in node.stacks.iter() {
                    let stack_mark = node.local_commit.id;
                    script
                        .commands
                        .push(crate::git::Command::SwitchCommit(stack_mark));
                    script
                        .commands
                        .push(crate::git::Command::RegisterMark(stack_mark));
                    script
                        .dependents
                        .extend(to_script_internal(stack, stack_mark));
                }
            }
            crate::graph::Action::Delete => {
                assert!(node.stacks.is_empty());
                for branch in node.branches.iter() {
                    script
                        .commands
                        .push(crate::git::Command::DeleteBranch(branch.name.clone()));
                }
            }
        }
    }

    if !script.commands.is_empty() {
        script
            .commands
            .insert(0, crate::git::Command::SwitchMark(base_mark));
    }
    if script.commands.is_empty() && script.dependents.is_empty() {
        None
    } else {
        Some(script)
    }
}
