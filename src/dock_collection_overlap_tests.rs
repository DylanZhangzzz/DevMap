use super::*;

fn git(root: &Path, args: &[&str]) {
    let output =
        crate::git_process::output(Command::new("git").arg("-C").arg(root).args(args)).unwrap();
    assert!(
        output.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn parallel_collection_matches_serial_inputs_across_configuration_and_dirty_changes() {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-b", "main"]);
    git(
        root.path(),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-m",
            "fixture",
        ],
    );
    let linked_parent = tempfile::tempdir().unwrap();
    let linked = linked_parent.path().join("linked");
    git(
        root.path(),
        &["worktree", "add", "-b", "dev", linked.to_str().unwrap()],
    );
    for target in ["main", "dev", "refs/heads/missing", "invalid target"] {
        git(
            root.path(),
            &["config", "--local", "devmap.developmentTarget", target],
        );
        for source in [root.path(), linked.as_path()] {
            let workspace = SourceGitInspector::open(source)
                .unwrap()
                .workspace_allow_unborn()
                .unwrap();
            let actual = DockProjectionContext::collect(&workspace, &[]).unwrap();
            let worktrees = WorktreeScanner::scan(&workspace).unwrap();
            let topology = GitTopologyCollector::scan(&workspace, &worktrees).unwrap();
            let configured =
                GitRelationshipResolver::development_configuration(&workspace).unwrap();
            let relationships = GitRelationshipResolver::resolve_with_configuration(
                &workspace,
                &worktrees,
                configured.as_deref(),
            )
            .unwrap();
            assert_eq!(actual.worktrees, worktrees);
            assert_eq!(actual.topology, topology);
            assert_eq!(actual.relationships, relationships);
            assert_eq!(
                actual.configuration_key,
                configured.as_deref().map(|v| sha256_hex(v.as_bytes()))
            );
            assert!(actual.targets.is_empty());
        }
        std::fs::write(linked.join("dirty.txt"), target).unwrap();
    }
}
