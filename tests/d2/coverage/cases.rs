// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Registry of the coverage cases: every case is one test function, tied to
//! the rows of the coverage table it exercises.
//!
//! Test functions of cases that need a particular system are compiled only on
//! that system; the registry itself lists every case on every system, so that
//! the resolution check covers the whole table everywhere.

use crate::{journal_rows, procedure};

/// Rows exercised by a case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Rows {
    /// Rows of the coverage table, by command and action.
    Table(&'static [(&'static str, Option<&'static str>)]),
    /// Parameter common to every command (`save_to`, `preview`, `script`,
    /// `cursor`, `validate`), outside the table.
    Common(&'static str),
}

/// Condition under which a case runs the operation of its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Variant {
    /// Default conditions.
    Plain,
    /// The operation touches the given number of files.
    Bulk(usize),
    /// The target is a directory of a lower overlay layer.
    LowerOverlayLayer,
    /// The volume has no space for a copy of the previous content.
    NoSpace,
    /// Files exceed the size limits of preservation before a launched
    /// program.
    PreserveLimit,
    /// Preservation before a launched program exceeds its time budget.
    PreserveBudget,
    /// The operation spans a second volume.
    SecondVolume,
    /// The link is a hard link.
    HardLink,
}

/// One coverage case.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Case {
    /// Name of the test function.
    pub(crate) name: &'static str,
    /// Rows exercised.
    pub(crate) rows: Rows,
    /// Condition.
    pub(crate) variant: Variant,
}

/// Case with the given test function name.
///
/// # Panics
///
/// Panics when no case has that name.
pub(crate) fn case(name: &str) -> &'static Case {
    CASES
        .iter()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("no coverage case is named {name}"))
}

macro_rules! coverage_cases {
    ($($(#[$attr:meta])* $name:ident => $rows:expr, $variant:expr, $run:path;)*) => {
        /// Every coverage case, in the order of the coverage table.
        pub(crate) const CASES: &[Case] = &[
            $(Case { name: stringify!($name), rows: $rows, variant: $variant },)*
        ];

        $(
            $(#[$attr])*
            #[test]
            fn $name() {
                $run(case(stringify!($name)));
            }
        )*
    };
}

use Rows::{Common, Table};
use Variant::{
    Bulk, HardLink, LowerOverlayLayer, NoSpace, Plain, PreserveBudget, PreserveLimit, SecondVolume,
};

coverage_cases! {
    list_changes_nothing => Table(&[("list", None)]), Plain, procedure::round_trip;
    find_changes_nothing => Table(&[("find", None)]), Plain, procedure::round_trip;
    info_changes_nothing => Table(&[("info", None)]), Plain, procedure::round_trip;
    read_changes_nothing => Table(&[("read", None)]), Plain, procedure::round_trip;
    lines_changes_nothing => Table(&[("lines", None)]), Plain, procedure::round_trip;
    search_changes_nothing => Table(&[("search", None)]), Plain, procedure::round_trip;
    filter_changes_nothing => Table(&[("filter", None)]), Plain, procedure::round_trip;
    select_changes_nothing => Table(&[("select", None)]), Plain, procedure::round_trip;
    sort_changes_nothing => Table(&[("sort", None)]), Plain, procedure::round_trip;
    first_changes_nothing => Table(&[("first", None)]), Plain, procedure::round_trip;
    last_changes_nothing => Table(&[("last", None)]), Plain, procedure::round_trip;
    skip_changes_nothing => Table(&[("skip", None)]), Plain, procedure::round_trip;
    count_changes_nothing => Table(&[("count", None)]), Plain, procedure::round_trip;
    assert_changes_nothing => Table(&[("assert", None)]), Plain, procedure::round_trip;
    verify_failure_undoes_entry_as_undo_entry => Table(&[("verify", None)]), Plain, journal_rows::verify_failure_undoes_entry_as_undo_entry;
    write_create_is_undone_and_redone => Table(&[("write", Some("create"))]), Plain, procedure::round_trip;
    write_overwrite_is_undone_and_redone => Table(&[("write", Some("overwrite"))]), Plain, procedure::round_trip;
    write_append_is_undone_and_redone => Table(&[("write", Some("append"))]), Plain, procedure::round_trip;
    write_prepend_is_undone_and_redone => Table(&[("write", Some("prepend"))]), Plain, procedure::round_trip;
    write_files_is_undone_and_redone => Table(&[("write", Some("files"))]), Plain, procedure::round_trip;
    write_truncate_is_undone_and_redone => Table(&[("write", Some("truncate"))]), Plain, procedure::round_trip;
    write_extend_is_undone_and_redone => Table(&[("write", Some("extend"))]), Plain, procedure::round_trip;
    write_offset_is_undone_and_redone => Table(&[("write", Some("data"))]), Plain, procedure::round_trip;
    #[cfg(target_os = "windows")]
    write_registry_is_undone_and_redone => Table(&[("write", Some("registry"))]), Plain, procedure::round_trip;
    write_special_is_marked_irreversible => Table(&[("write", Some("special_file"))]), Plain, procedure::round_trip;
    replace_text_is_undone_and_redone => Table(&[("replace", None)]), Plain, procedure::round_trip;
    replace_bulk_1e4_is_undone_and_redone => Table(&[("replace", None)]), Bulk(10_000), procedure::round_trip;
    replace_item_is_undone_and_redone => Table(&[("replace", Some("item"))]), Plain, procedure::round_trip;
    insert_text_is_undone_and_redone => Table(&[("insert", None)]), Plain, procedure::round_trip;
    insert_end_is_undone_and_redone => Table(&[("insert", Some("end"))]), Plain, procedure::round_trip;
    edit_is_undone_and_redone => Table(&[("edit", None)]), Plain, procedure::round_trip;
    copy_new_is_undone_and_redone => Table(&[("copy", None)]), Plain, procedure::round_trip;
    copy_overwrite_is_undone_and_redone => Table(&[("copy", Some("overwrite"))]), Plain, procedure::round_trip;
    #[cfg(target_os = "windows")]
    copy_registry_is_undone_and_redone => Table(&[("copy", Some("registry"))]), Plain, procedure::round_trip;
    move_same_volume_is_undone_and_redone => Table(&[("move", None)]), Plain, procedure::round_trip;
    move_tree_1e5_is_undone_and_redone => Table(&[("move", None)]), Bulk(100_000), procedure::round_trip;
    move_overwrite_is_undone_and_redone => Table(&[("move", Some("overwrite"))]), Plain, procedure::round_trip;
    move_cross_volume_is_undone_and_redone => Table(&[("move", Some("cross_volume"))]), SecondVolume, procedure::round_trip;
    #[cfg(target_os = "linux")]
    move_overlay_exdev_is_undone_and_redone => Table(&[("move", Some("cross_volume"))]), LowerOverlayLayer, procedure::round_trip;
    #[cfg(target_os = "windows")]
    move_registry_is_undone_and_redone => Table(&[("move", Some("registry"))]), Plain, procedure::round_trip;
    remove_entry_is_undone_and_redone => Table(&[("remove", None)]), Plain, procedure::round_trip;
    #[ignore = "runs at full scale on request"]
    remove_tree_1e6_is_undone_and_redone => Table(&[("remove", None)]), Bulk(1_000_000), procedure::round_trip;
    #[cfg(target_os = "linux")]
    remove_overlay_exdev_is_undone_and_redone => Table(&[("remove", None)]), LowerOverlayLayer, procedure::round_trip;
    remove_content_is_undone_and_redone => Table(&[("remove", Some("records"))]), Plain, procedure::round_trip;
    remove_item_is_undone_and_redone => Table(&[("remove", Some("item"))]), Plain, procedure::round_trip;
    remove_job_changes_nothing => Table(&[("remove", Some("job"))]), Plain, procedure::round_trip;
    #[cfg(target_os = "windows")]
    remove_registry_is_undone_and_redone => Table(&[("remove", Some("registry"))]), Plain, procedure::round_trip;
    rename_is_undone_and_redone => Table(&[("rename", None)]), Plain, procedure::round_trip;
    rename_overwrite_is_undone_and_redone => Table(&[("rename", Some("overwrite"))]), Plain, procedure::round_trip;
    create_file_dir_is_undone_and_redone => Table(&[("create", None)]), Plain, procedure::round_trip;
    create_link_is_undone_and_redone => Table(&[("create", Some("link"))]), Plain, procedure::round_trip;
    create_hardlink_is_undone_and_redone => Table(&[("create", Some("link"))]), HardLink, procedure::round_trip;
    #[cfg(target_os = "windows")]
    create_junction_is_undone_and_redone => Table(&[("create", Some("link"))]), Plain, procedure::round_trip;
    create_temporary_is_undone_and_redone => Table(&[("create", Some("temporary"))]), Plain, procedure::round_trip;
    create_touch_is_undone_and_redone => Table(&[("create", Some("touch"))]), Plain, procedure::round_trip;
    #[cfg(unix)]
    attributes_posix_is_undone_and_redone => Table(&[("attributes", None)]), Plain, procedure::round_trip;
    #[cfg(target_os = "windows")]
    attributes_windows_is_undone_and_redone => Table(&[("attributes", Some("system_attributes"))]), Plain, procedure::round_trip;
    #[cfg(target_os = "macos")]
    attributes_macos_is_undone_and_redone => Table(&[("attributes", Some("system_attributes"))]), Plain, procedure::round_trip;
    run_watch_p2_is_undone_or_marked_irreversible => Table(&[("run", None)]), Plain, procedure::round_trip;
    run_p2_limit_is_undone_or_marked_irreversible => Table(&[("run", None)]), PreserveLimit, procedure::round_trip;
    run_p2_budget_is_undone_or_marked_irreversible => Table(&[("run", None)]), PreserveBudget, procedure::round_trip;
    run_background_is_undone_or_marked_irreversible => Table(&[("run", Some("background"))]), Plain, procedure::round_trip;
    run_batch_is_undone_or_marked_irreversible => Table(&[("run", Some("background"))]), Plain, procedure::round_trip;
    run_external_is_marked_irreversible => Table(&[("run", Some("external"))]), Plain, procedure::round_trip;
    jobs_changes_nothing => Table(&[("jobs", None)]), Plain, procedure::round_trip;
    stop_process_is_marked_irreversible => Table(&[("stop", None)]), Plain, procedure::round_trip;
    stop_service_is_marked_irreversible => Table(&[("stop", Some("service"))]), Plain, procedure::round_trip;
    retry_is_undone_or_marked_irreversible => Table(&[("retry", None)]), Plain, procedure::round_trip;
    history_changes_nothing => Table(&[("history", None)]), Plain, journal_rows::history_changes_nothing;
    undo_last_is_undone_and_redone => Table(&[("undo", None)]), Plain, journal_rows::undo_last_is_undone_and_redone;
    undo_id_is_undone_and_redone => Table(&[("undo", None)]), Plain, journal_rows::undo_id_is_undone_and_redone;
    undo_to_is_undone_and_redone => Table(&[("undo", Some("to"))]), Plain, journal_rows::undo_to_is_undone_and_redone;
    undo_to_sessions_is_undone_and_redone => Table(&[("undo", Some("to"))]), Plain, journal_rows::undo_to_sessions_is_undone_and_redone;
    undo_file_is_undone_and_redone => Table(&[("undo", Some("file"))]), Plain, journal_rows::undo_file_is_undone_and_redone;
    undo_paths_is_undone_and_redone => Table(&[("undo", Some("paths"))]), Plain, journal_rows::undo_paths_is_undone_and_redone;
    undo_force_is_undone_and_redone => Table(&[("undo", Some("on_conflict"))]), Plain, journal_rows::undo_force_is_undone_and_redone;
    undo_include_later_is_undone_and_redone => Table(&[("undo", Some("on_conflict"))]), Plain, journal_rows::undo_include_later_is_undone_and_redone;
    undo_partial_is_undone_and_redone => Table(&[("undo", Some("partial"))]), Plain, journal_rows::undo_partial_is_undone_and_redone;
    redo_is_undone_and_redone => Table(&[("redo", None)]), Plain, journal_rows::redo_is_undone_and_redone;
    redo_paths_is_undone_and_redone => Table(&[("redo", None)]), Plain, journal_rows::redo_paths_is_undone_and_redone;
    help_changes_nothing => Table(&[("help", None)]), Plain, procedure::round_trip;
    next_changes_nothing => Table(&[("next", None)]), Plain, procedure::round_trip;
    set_existing_is_undone_and_redone => Table(&[("set", None)]), Plain, procedure::round_trip;
    set_values_is_undone_and_redone => Table(&[("set", None)]), Plain, procedure::round_trip;
    set_create_is_undone_and_redone => Table(&[("set", Some("create"))]), Plain, procedure::round_trip;
    query_changes_nothing => Table(&[("query", None)]), Plain, procedure::round_trip;
    sql_changes_nothing => Table(&[("sql", None)]), Plain, procedure::round_trip;
    sql_write_is_undone_or_marked_irreversible => Table(&[("sql", Some("write"))]), Plain, procedure::round_trip;
    sql_write_wal_is_undone_or_marked_irreversible => Table(&[("sql", Some("write"))]), Plain, procedure::round_trip;
    sql_write_nospace_is_undone_or_marked_irreversible => Table(&[("sql", Some("write"))]), NoSpace, procedure::round_trip;
    diff_changes_nothing => Table(&[("diff", None)]), Plain, procedure::round_trip;
    patch_modify_is_undone_and_redone => Table(&[("patch", None)]), Plain, procedure::round_trip;
    patch_create_is_undone_and_redone => Table(&[("patch", Some("create"))]), Plain, procedure::round_trip;
    patch_delete_is_undone_and_redone => Table(&[("patch", Some("delete"))]), Plain, procedure::round_trip;
    patch_rename_is_undone_and_redone => Table(&[("patch", Some("rename"))]), Plain, procedure::round_trip;
    patch_mode_is_undone_and_redone => Table(&[("patch", Some("mode"))]), Plain, procedure::round_trip;
    hash_changes_nothing => Table(&[("hash", None)]), Plain, procedure::round_trip;
    convert_is_undone_and_redone => Table(&[("convert", None)]), Plain, procedure::round_trip;
    evaluate_changes_nothing => Table(&[("evaluate", None)]), Plain, procedure::round_trip;
    group_changes_nothing => Table(&[("group", None)]), Plain, procedure::round_trip;
    summarize_changes_nothing => Table(&[("summarize", None)]), Plain, procedure::round_trip;
    distinct_changes_nothing => Table(&[("distinct", None)]), Plain, procedure::round_trip;
    describe_changes_nothing => Table(&[("describe", None)]), Plain, procedure::round_trip;
    parse_changes_nothing => Table(&[("parse", None)]), Plain, procedure::round_trip;
    split_changes_nothing => Table(&[("split", None)]), Plain, procedure::round_trip;
    code_symbols_changes_nothing => Table(&[("code.symbols", None)]), Plain, procedure::round_trip;
    code_outline_changes_nothing => Table(&[("code.outline", None)]), Plain, procedure::round_trip;
    code_definition_changes_nothing => Table(&[("code.definition", None)]), Plain, procedure::round_trip;
    code_references_changes_nothing => Table(&[("code.references", None)]), Plain, procedure::round_trip;
    code_hover_changes_nothing => Table(&[("code.hover", None)]), Plain, procedure::round_trip;
    code_diagnostics_changes_nothing => Table(&[("code.diagnostics", None)]), Plain, procedure::round_trip;
    code_rename_lsp_is_undone_and_redone => Table(&[("code.rename", None)]), Plain, procedure::round_trip;
    code_rename_syntax_is_undone_and_redone => Table(&[("code.rename", None)]), Plain, procedure::round_trip;
    code_rename_fileops_is_undone_and_redone => Table(&[("code.rename", Some("file_operations"))]), Plain, procedure::round_trip;
    code_languages_changes_nothing => Table(&[("code.languages", None)]), Plain, procedure::round_trip;
    code_implementations_changes_nothing => Table(&[("code.implementations", None)]), Plain, procedure::round_trip;
    code_type_definition_changes_nothing => Table(&[("code.type_definition", None)]), Plain, procedure::round_trip;
    code_call_hierarchy_changes_nothing => Table(&[("code.call_hierarchy", None)]), Plain, procedure::round_trip;
    code_actions_changes_nothing => Table(&[("code.actions", None)]), Plain, procedure::round_trip;
    code_profile_changes_nothing => Table(&[("code.profile", None)]), Plain, procedure::round_trip;
    notebook_cells_changes_nothing => Table(&[("notebook.cells", None)]), Plain, procedure::round_trip;
    notebook_insert_is_undone_and_redone => Table(&[("notebook.insert", None)]), Plain, procedure::round_trip;
    notebook_replace_is_undone_and_redone => Table(&[("notebook.replace", None)]), Plain, procedure::round_trip;
    notebook_move_is_undone_and_redone => Table(&[("notebook.move", None)]), Plain, procedure::round_trip;
    notebook_copy_is_undone_and_redone => Table(&[("notebook.copy", None)]), Plain, procedure::round_trip;
    notebook_normalize_is_undone_and_redone => Table(&[("notebook.normalize", None)]), Plain, procedure::round_trip;
    notebook_ops_is_undone_and_redone => Table(&[("notebook.ops", None)]), Plain, procedure::round_trip;
    notebook_remove_is_undone_and_redone => Table(&[("notebook.remove", None)]), Plain, procedure::round_trip;
    notebook_clear_outputs_is_undone_and_redone => Table(&[("notebook.clear_outputs", None)]), Plain, procedure::round_trip;
    notebook_export_new_is_undone_and_redone => Table(&[("notebook.export", None)]), Plain, procedure::round_trip;
    notebook_export_overwrite_is_undone_and_redone => Table(&[("notebook.export", Some("overwrite"))]), Plain, procedure::round_trip;
    git_status_changes_nothing => Table(&[("git.status", None)]), Plain, procedure::round_trip;
    git_diff_changes_nothing => Table(&[("git.diff", None)]), Plain, procedure::round_trip;
    git_log_changes_nothing => Table(&[("git.log", None)]), Plain, procedure::round_trip;
    git_show_changes_nothing => Table(&[("git.show", None)]), Plain, procedure::round_trip;
    git_blame_changes_nothing => Table(&[("git.blame", None)]), Plain, procedure::round_trip;
    git_branch_create_is_undone_and_redone => Table(&[("git.branch", Some("create"))]), Plain, procedure::round_trip;
    git_branch_force_is_undone_and_redone => Table(&[("git.branch", Some("create"))]), Plain, procedure::round_trip;
    git_branch_create_switch_is_undone_and_redone => Table(&[("git.branch", Some("switch"))]), Plain, procedure::round_trip;
    git_branch_checkpoint_is_undone_and_redone => Table(&[("git.branch", Some("switch"))]), Plain, procedure::round_trip;
    git_branch_rename_is_undone_and_redone => Table(&[("git.branch", Some("rename"))]), Plain, procedure::round_trip;
    git_branch_delete_is_undone_and_redone => Table(&[("git.branch", Some("delete"))]), Plain, procedure::round_trip;
    git_branch_delete_packed_is_undone_and_redone => Table(&[("git.branch", Some("delete"))]), Plain, procedure::round_trip;
    git_branch_purge_is_undone_and_redone => Table(&[("git.branch", Some("purge"))]), Plain, procedure::round_trip;
    git_branch_changes_nothing => Table(&[("git.branch", None)]), Plain, procedure::round_trip;
    git_stage_add_is_undone_and_redone => Table(&[("git.stage", Some("add"))]), Plain, procedure::round_trip;
    git_stage_hunks_is_undone_and_redone => Table(&[("git.stage", Some("add"))]), Plain, procedure::round_trip;
    git_stage_unstage_is_undone_and_redone => Table(&[("git.stage", Some("unstage"))]), Plain, procedure::round_trip;
    git_stage_changes_nothing => Table(&[("git.stage", Some("list"))]), Plain, procedure::round_trip;
    git_checkpoint_create_is_undone_and_redone => Table(&[("git.checkpoint", Some("create"))]), Plain, procedure::round_trip;
    git_checkpoint_keep_is_undone_and_redone => Table(&[("git.checkpoint", Some("keep"))]), Plain, procedure::round_trip;
    git_checkpoint_restore_is_undone_and_redone => Table(&[("git.checkpoint", Some("restore"))]), Plain, procedure::round_trip;
    git_checkpoint_restore_index_is_undone_and_redone => Table(&[("git.checkpoint", Some("restore"))]), Plain, procedure::round_trip;
    git_checkpoint_delete_is_undone_and_redone => Table(&[("git.checkpoint", Some("delete"))]), Plain, procedure::round_trip;
    git_checkpoint_changes_nothing => Table(&[("git.checkpoint", Some("list"))]), Plain, procedure::round_trip;
    git_review_changes_nothing => Table(&[("git.review", None)]), Plain, procedure::round_trip;
    git_worktree_add_is_undone_and_redone => Table(&[("git.worktree", Some("add"))]), Plain, procedure::round_trip;
    git_worktree_remove_is_undone_and_redone => Table(&[("git.worktree", Some("remove"))]), Plain, procedure::round_trip;
    git_worktree_remove_othervol_is_undone_and_redone => Table(&[("git.worktree", Some("remove"))]), SecondVolume, procedure::round_trip;
    git_worktree_changes_nothing => Table(&[("git.worktree", Some("list"))]), Plain, procedure::round_trip;
    git_commit_is_undone_and_redone => Table(&[("git.commit", None)]), Plain, procedure::round_trip;
    git_commit_paths_is_undone_and_redone => Table(&[("git.commit", None)]), Plain, procedure::round_trip;
    git_commit_signed_is_undone_and_redone => Table(&[("git.commit", None)]), Plain, procedure::round_trip;
    git_commit_amend_is_undone_and_redone => Table(&[("git.commit", Some("amend"))]), Plain, procedure::round_trip;
    git_commit_hooks_is_undone_or_marked_irreversible => Table(&[("git.commit", Some("hooks"))]), Plain, procedure::round_trip;
    git_tag_create_is_undone_and_redone => Table(&[("git.tag", Some("name"))]), Plain, procedure::round_trip;
    git_tag_delete_is_undone_and_redone => Table(&[("git.tag", Some("delete"))]), Plain, procedure::round_trip;
    git_tag_changes_nothing => Table(&[("git.tag", None)]), Plain, procedure::round_trip;
    git_fetch_is_undone_and_redone => Table(&[("git.fetch", None)]), Plain, procedure::round_trip;
    git_fetch_prune_is_undone_and_redone => Table(&[("git.fetch", Some("prune"))]), Plain, procedure::round_trip;
    git_clone_is_undone_and_redone => Table(&[("git.clone", None)]), Plain, procedure::round_trip;
    git_switch_is_undone_and_redone => Table(&[("git.switch", None)]), Plain, procedure::round_trip;
    git_switch_create_is_undone_and_redone => Table(&[("git.switch", Some("create"))]), Plain, procedure::round_trip;
    git_restore_is_undone_and_redone => Table(&[("git.restore", None)]), Plain, procedure::round_trip;
    git_restore_hunks_is_undone_and_redone => Table(&[("git.restore", Some("hunks"))]), Plain, procedure::round_trip;
    git_restore_staged_is_undone_and_redone => Table(&[("git.restore", Some("staged"))]), Plain, procedure::round_trip;
    git_reset_mixed_is_undone_and_redone => Table(&[("git.reset", None)]), Plain, procedure::round_trip;
    git_reset_soft_is_undone_and_redone => Table(&[("git.reset", Some("soft"))]), Plain, procedure::round_trip;
    git_reset_hard_is_undone_and_redone => Table(&[("git.reset", Some("hard"))]), Plain, procedure::round_trip;
    git_merge_ff_is_undone_and_redone => Table(&[("git.merge", None)]), Plain, procedure::round_trip;
    git_merge_commit_is_undone_and_redone => Table(&[("git.merge", None)]), Plain, procedure::round_trip;
    git_merge_conflict_is_undone_and_redone => Table(&[("git.merge", None)]), Plain, procedure::round_trip;
    git_rebase_is_undone_and_redone => Table(&[("git.rebase", None)]), Plain, procedure::round_trip;
    git_rebase_conflict_is_undone_and_redone => Table(&[("git.rebase", None)]), Plain, procedure::round_trip;
    git_stash_push_is_undone_and_redone => Table(&[("git.stash", Some("push"))]), Plain, procedure::round_trip;
    git_stash_push_untracked_is_undone_and_redone => Table(&[("git.stash", Some("push"))]), Plain, procedure::round_trip;
    git_stash_push_paths_is_undone_and_redone => Table(&[("git.stash", Some("push"))]), Plain, procedure::round_trip;
    git_stash_pop_is_undone_and_redone => Table(&[("git.stash", Some("pop"))]), Plain, procedure::round_trip;
    git_stash_apply_is_undone_and_redone => Table(&[("git.stash", Some("apply"))]), Plain, procedure::round_trip;
    git_stash_drop_is_undone_and_redone => Table(&[("git.stash", Some("drop"))]), Plain, procedure::round_trip;
    git_stash_clear_is_undone_and_redone => Table(&[("git.stash", Some("clear"))]), Plain, procedure::round_trip;
    git_stash_changes_nothing => Table(&[("git.stash", Some("list"))]), Plain, procedure::round_trip;
    git_pull_is_undone_and_redone => Table(&[("git.pull", None)]), Plain, procedure::round_trip;
    git_pull_conflict_is_undone_and_redone => Table(&[("git.pull", None)]), Plain, procedure::round_trip;
    git_push_is_marked_irreversible => Table(&[("git.push", None)]), Plain, procedure::round_trip;
    git_push_force_is_marked_irreversible => Table(&[("git.push", None)]), Plain, procedure::round_trip;
    archive_list_changes_nothing => Table(&[("archive.list", None)]), Plain, procedure::round_trip;
    archive_extract_new_is_undone_and_redone => Table(&[("archive.extract", None)]), Plain, procedure::round_trip;
    archive_extract_1e5_is_undone_and_redone => Table(&[("archive.extract", None)]), Bulk(100_000), procedure::round_trip;
    archive_extract_overwrite_is_undone_and_redone => Table(&[("archive.extract", Some("overwrite"))]), Plain, procedure::round_trip;
    archive_create_is_undone_and_redone => Table(&[("archive.create", None)]), Plain, procedure::round_trip;
    archive_create_overwrite_is_undone_and_redone => Table(&[("archive.create", Some("overwrite"))]), Plain, procedure::round_trip;
    compress_is_undone_and_redone => Table(&[("compress", None)]), Plain, procedure::round_trip;
    compress_nokeep_is_undone_and_redone => Table(&[("compress", Some("remove_source"))]), Plain, procedure::round_trip;
    decompress_is_undone_and_redone => Table(&[("decompress", None)]), Plain, procedure::round_trip;
    decompress_nokeep_is_undone_and_redone => Table(&[("decompress", Some("remove_source"))]), Plain, procedure::round_trip;
    processes_changes_nothing => Table(&[("processes", None)]), Plain, procedure::round_trip;
    ports_changes_nothing => Table(&[("ports", None)]), Plain, procedure::round_trip;
    disks_changes_nothing => Table(&[("disks", None)]), Plain, procedure::round_trip;
    host_changes_nothing => Table(&[("host", None)]), Plain, procedure::round_trip;
    environment_changes_nothing => Table(&[("environment", None)]), Plain, procedure::round_trip;
    open_files_changes_nothing => Table(&[("open_files", None)]), Plain, procedure::round_trip;
    accounts_changes_nothing => Table(&[("accounts", None)]), Plain, procedure::round_trip;
    sleep_changes_nothing => Table(&[("sleep", None)]), Plain, procedure::round_trip;
    suspend_is_marked_irreversible => Table(&[("suspend", None)]), Plain, procedure::round_trip;
    resume_is_marked_irreversible => Table(&[("resume", None)]), Plain, procedure::round_trip;
    logs_changes_nothing => Table(&[("logs", None)]), Plain, procedure::round_trip;
    services_changes_nothing => Table(&[("services", None)]), Plain, procedure::round_trip;
    start_is_marked_irreversible => Table(&[("start", None)]), Plain, procedure::round_trip;
    restart_is_marked_irreversible => Table(&[("restart", None)]), Plain, procedure::round_trip;
    enable_is_marked_irreversible => Table(&[("enable", None)]), Plain, procedure::round_trip;
    disable_is_marked_irreversible => Table(&[("disable", None)]), Plain, procedure::round_trip;
    #[cfg(target_os = "windows")]
    windows_cim_changes_nothing => Table(&[("windows.cim", None)]), Plain, procedure::round_trip;
    package_list_changes_nothing => Table(&[("package.list", None)]), Plain, procedure::round_trip;
    package_search_changes_nothing => Table(&[("package.search", None)]), Plain, procedure::round_trip;
    package_install_is_marked_irreversible => Table(&[("package.install", None)]), Plain, procedure::round_trip;
    package_remove_is_marked_irreversible => Table(&[("package.remove", None)]), Plain, procedure::round_trip;
    package_upgrade_is_marked_irreversible => Table(&[("package.upgrade", None)]), Plain, procedure::round_trip;
    http_request_changes_nothing => Table(&[("http.request", None)]), Plain, procedure::round_trip;
    http_request_save_is_undone_and_redone => Table(&[("http.request", Some("save"))]), Plain, procedure::round_trip;
    http_request_network_is_marked_irreversible => Table(&[("http.request", Some("post")), ("http.request", Some("put")), ("http.request", Some("patch")), ("http.request", Some("delete")), ("http.request", Some("options"))]), Plain, procedure::round_trip;
    http_download_new_is_undone_and_redone => Table(&[("http.download", None)]), Plain, procedure::round_trip;
    http_download_overwrite_is_undone_and_redone => Table(&[("http.download", Some("overwrite"))]), Plain, procedure::round_trip;
    http_download_resume_is_undone_and_redone => Table(&[("http.download", Some("resume"))]), Plain, procedure::round_trip;
    network_ping_changes_nothing => Table(&[("network.ping", None)]), Plain, procedure::round_trip;
    network_resolve_changes_nothing => Table(&[("network.resolve", None)]), Plain, procedure::round_trip;
    network_interfaces_changes_nothing => Table(&[("network.interfaces", None)]), Plain, procedure::round_trip;
    schedule_create_is_undone_and_redone => Table(&[("schedule.create", None)]), Plain, procedure::round_trip;
    schedule_create_run_now_is_undone_and_redone => Table(&[("schedule.create", Some("run_now"))]), Plain, procedure::round_trip;
    schedule_list_changes_nothing => Table(&[("schedule.list", None)]), Plain, procedure::round_trip;
    schedule_remove_is_undone_and_redone => Table(&[("schedule.remove", None)]), Plain, procedure::round_trip;
    schedule_remove_all_is_undone_and_redone => Table(&[("schedule.remove", None)]), Plain, procedure::round_trip;
    schedule_pause_is_undone_and_redone => Table(&[("schedule.pause", None)]), Plain, procedure::round_trip;
    schedule_resume_is_undone_and_redone => Table(&[("schedule.resume", None)]), Plain, procedure::round_trip;
    schedule_run_files_is_undone_or_marked_irreversible => Table(&[("schedule.run", None)]), Plain, procedure::round_trip;
    schedule_run_run_is_undone_or_marked_irreversible => Table(&[("schedule.run", None)]), Plain, procedure::round_trip;
    schedule_history_changes_nothing => Table(&[("schedule.history", None)]), Plain, procedure::round_trip;
    journal_purge_up_to_is_marked_irreversible => Table(&[("journal.purge", None)]), Plain, journal_rows::journal_purge_up_to_is_marked_irreversible;
    journal_purge_all_is_marked_irreversible => Table(&[("journal.purge", None)]), Plain, journal_rows::journal_purge_all_is_marked_irreversible;
    journal_purge_id_is_marked_irreversible => Table(&[("journal.purge", Some("id"))]), Plain, journal_rows::journal_purge_id_is_marked_irreversible;
    journal_space_changes_nothing => Table(&[("journal.space", None)]), Plain, journal_rows::journal_space_changes_nothing;
    journal_status_changes_nothing => Table(&[("journal.status", None)]), Plain, journal_rows::journal_status_changes_nothing;
    journal_pin_is_undone_and_redone => Table(&[("journal.pin", None)]), Plain, journal_rows::journal_pin_is_undone_and_redone;
    journal_pin_release_is_undone_or_marked_irreversible => Table(&[("journal.pin", Some("release"))]), Plain, journal_rows::journal_pin_release_is_undone_or_marked_irreversible;
    journal_limits_changes_nothing => Table(&[("journal.limits", None)]), Plain, journal_rows::journal_limits_changes_nothing;
    journal_limits_session_is_undone_or_marked_irreversible => Table(&[("journal.limits", Some("session"))]), Plain, journal_rows::journal_limits_session_is_undone_or_marked_irreversible;
    journal_limits_user_is_undone_or_marked_irreversible => Table(&[("journal.limits", Some("user"))]), Plain, journal_rows::journal_limits_user_is_undone_or_marked_irreversible;
    program_find_changes_nothing => Table(&[("program.find", None)]), Plain, procedure::round_trip;
    program_search_changes_nothing => Table(&[("program.search", None)]), Plain, procedure::round_trip;
    program_describe_changes_nothing => Table(&[("program.describe", None)]), Plain, procedure::round_trip;
    program_catalog_changes_nothing => Table(&[("program.catalog", None)]), Plain, procedure::round_trip;
    test_run_is_undone_or_marked_irreversible => Table(&[("test.run", None)]), Plain, procedure::round_trip;
    test_failed_is_undone_or_marked_irreversible => Table(&[("test.failed", None)]), Plain, procedure::round_trip;
    test_show_changes_nothing => Table(&[("test.show", None)]), Plain, procedure::round_trip;
    test_list_changes_nothing => Table(&[("test.list", None)]), Plain, procedure::round_trip;
    analyze_check_tools_is_undone_or_marked_irreversible => Table(&[("analyze.check", None)]), Plain, procedure::round_trip;
    analyze_check_changes_nothing => Table(&[("analyze.check", Some("complexity"))]), Plain, procedure::round_trip;
    analyze_show_changes_nothing => Table(&[("analyze.show", None)]), Plain, procedure::round_trip;
    analyze_format_stdin_is_undone_and_redone => Table(&[("analyze.format", None)]), Plain, procedure::round_trip;
    analyze_format_inplace_is_undone_or_marked_irreversible => Table(&[("analyze.format", Some("in_place"))]), Plain, procedure::round_trip;
    analyze_tools_changes_nothing => Table(&[("analyze.tools", None)]), Plain, procedure::round_trip;
    firewall_rules_changes_nothing => Table(&[("firewall.rules", None)]), Plain, procedure::round_trip;
    firewall_allow_is_undone_and_redone => Table(&[("firewall.allow", None)]), Plain, procedure::round_trip;
    firewall_deny_is_undone_and_redone => Table(&[("firewall.deny", None)]), Plain, procedure::round_trip;
    firewall_remove_is_undone_and_redone => Table(&[("firewall.remove", None)]), Plain, procedure::round_trip;
    network_route_add_is_undone_and_redone => Table(&[("network.route_add", None)]), Plain, procedure::round_trip;
    network_route_remove_is_undone_and_redone => Table(&[("network.route_remove", None)]), Plain, procedure::round_trip;
    network_dns_set_is_undone_and_redone => Table(&[("network.dns_set", None)]), Plain, procedure::round_trip;
    clock_timezone_set_is_undone_and_redone => Table(&[("clock.timezone_set", None)]), Plain, procedure::round_trip;
    clock_ntp_is_undone_and_redone => Table(&[("clock.ntp", None)]), Plain, procedure::round_trip;
    clock_set_is_marked_irreversible => Table(&[("clock.set", None)]), Plain, procedure::round_trip;
    accounts_create_is_marked_irreversible => Table(&[("accounts.create", None)]), Plain, procedure::round_trip;
    accounts_modify_is_marked_irreversible => Table(&[("accounts.modify", None)]), Plain, procedure::round_trip;
    accounts_remove_is_undone_or_marked_irreversible => Table(&[("accounts.remove", None)]), Plain, procedure::round_trip;
    accounts_password_is_marked_irreversible => Table(&[("accounts.password", None)]), Plain, procedure::round_trip;
    groups_create_is_marked_irreversible => Table(&[("groups.create", None)]), Plain, procedure::round_trip;
    groups_remove_is_marked_irreversible => Table(&[("groups.remove", None)]), Plain, procedure::round_trip;
    groups_add_member_is_marked_irreversible => Table(&[("groups.add_member", None)]), Plain, procedure::round_trip;
    groups_remove_member_is_marked_irreversible => Table(&[("groups.remove_member", None)]), Plain, procedure::round_trip;
    mount_is_undone_or_marked_irreversible => Table(&[("mount", None)]), Plain, procedure::round_trip;
    unmount_is_undone_or_marked_irreversible => Table(&[("unmount", None)]), Plain, procedure::round_trip;
    sysctl_set_is_undone_or_marked_irreversible => Table(&[("sysctl.set", None)]), Plain, procedure::round_trip;
    modules_load_is_marked_irreversible => Table(&[("modules.load", None)]), Plain, procedure::round_trip;
    modules_unload_is_marked_irreversible => Table(&[("modules.unload", None)]), Plain, procedure::round_trip;
    power_restart_is_marked_irreversible => Table(&[("power.restart", None)]), Plain, procedure::round_trip;
    power_shutdown_is_marked_irreversible => Table(&[("power.shutdown", None)]), Plain, procedure::round_trip;
    power_cancel_is_marked_irreversible => Table(&[("power.cancel", None)]), Plain, procedure::round_trip;
    disks_partition_is_marked_irreversible => Table(&[("disks.partition", None)]), Plain, procedure::round_trip;
    disks_format_is_marked_irreversible => Table(&[("disks.format", None)]), Plain, procedure::round_trip;
    save_to_new_file_is_undone_and_redone => Common("save_to"), Plain, journal_rows::save_to_new_file_is_undone_and_redone;
    save_to_existing_file_is_undone_and_redone => Common("save_to"), Plain, journal_rows::save_to_existing_file_is_undone_and_redone;
    save_to_of_changing_command_joins_its_entry => Common("save_to"), Plain, journal_rows::save_to_of_changing_command_joins_its_entry;
    preview_changes_nothing => Common("preview"), Plain, journal_rows::preview_changes_nothing;
    script_records_one_entry_per_pipeline => Common("script"), Plain, journal_rows::script_records_one_entry_per_pipeline;
    cursor_page_does_not_rerun_changes => Common("cursor"), Plain, journal_rows::cursor_page_does_not_rerun_changes;
    validation_failure_rolls_back_without_new_entry => Common("validate"), Plain, journal_rows::validation_failure_rolls_back_without_new_entry;
    if_match_mismatch_rolls_back_without_new_entry => Common("validate"), Plain, journal_rows::if_match_mismatch_rolls_back_without_new_entry;
}
