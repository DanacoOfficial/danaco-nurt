// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Changes to INI documents through the API of `IniDocument` and the
//! registered format: lists of repeated keys, global keys, sections and
//! subsections, refused changes and the positions reported by `get`.
// Helpers outside `#[test]` functions fail the test on unexpected errors.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::path::Path;

use nurt_contracts::error::{Error, ErrorCode};
use nurt_contracts::keypath::KeyPath;
use nurt_contracts::value::{Fields, Value};
use nurt_doc::formats::ini::{IniDocument, IniFormat, IniProfile};
use nurt_doc::registry::{DocFormat, DocOptions};

fn key(text: &str) -> KeyPath {
    KeyPath::parse(text).unwrap()
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn list(values: &[&str]) -> Value {
    Value::List(values.iter().map(|v| text(v)).collect())
}

fn map(entries: &[(&str, Value)]) -> Value {
    Value::Map(
        entries
            .iter()
            .map(|(name, value)| ((*name).to_owned(), value.clone()))
            .collect::<Fields>(),
    )
}

fn document(profile: IniProfile, content: &str) -> IniDocument {
    IniDocument::parse(content, profile).unwrap()
}

/// Sets `value` at `path` with `create` and returns the new text.
fn set(profile: IniProfile, content: &str, path: &str, value: &Value) -> String {
    let mut doc = document(profile, content);
    doc.set(&key(path), value, true).unwrap();
    doc.text().to_owned()
}

fn remove(profile: IniProfile, content: &str, path: &str) -> String {
    let mut doc = document(profile, content);
    doc.remove(&key(path)).unwrap();
    doc.text().to_owned()
}

fn construct(error: &Error) -> &str {
    assert_eq!(error.code(), ErrorCode::UnsupportedConstruct);
    error.found.as_deref().unwrap()
}

const UNIT: &str = "[Service]\nExecStart=/a\nExecStart=/b\nType=simple\n";

#[test]
fn systemd_list_is_replaced_in_place_when_the_length_is_kept() {
    let out = set(
        IniProfile::Systemd,
        UNIT,
        "Service.ExecStart",
        &list(&["/x", "/y"]),
    );
    assert_eq!(out, "[Service]\nExecStart=/x\nExecStart=/y\nType=simple\n");
}

#[test]
fn systemd_list_gains_occurrences_after_the_last_one() {
    let out = set(
        IniProfile::Systemd,
        UNIT,
        "Service.ExecStart",
        &list(&["/x", "/y", "/z"]),
    );
    assert_eq!(
        out,
        "[Service]\nExecStart=/x\nExecStart=/y\nExecStart=/z\nType=simple\n"
    );
}

#[test]
fn systemd_list_loses_the_surplus_occurrences() {
    let out = set(
        IniProfile::Systemd,
        UNIT,
        "Service.ExecStart",
        &list(&["/x"]),
    );
    assert_eq!(out, "[Service]\nExecStart=/x\nType=simple\n");
}

#[test]
fn systemd_list_gets_a_scalar_as_its_only_occurrence() {
    let out = set(IniProfile::Systemd, UNIT, "Service.ExecStart", &text("/x"));
    assert_eq!(out, "[Service]\nExecStart=/x\nType=simple\n");
}

#[test]
fn append_segment_and_index_equal_to_the_length_add_an_occurrence() {
    let expected = "[Service]\nExecStart=/a\nExecStart=/b\nExecStart=/c\nType=simple\n";
    assert_eq!(
        set(
            IniProfile::Systemd,
            UNIT,
            "Service.ExecStart[+]",
            &text("/c")
        ),
        expected
    );
    assert_eq!(
        set(
            IniProfile::Systemd,
            UNIT,
            "Service.ExecStart[2]",
            &text("/c")
        ),
        expected
    );
}

#[test]
fn append_to_a_single_key_of_a_list_profile_adds_a_second_occurrence() {
    let out = set(
        IniProfile::Systemd,
        "[Unit]\nAfter=a.service\n",
        "Unit.After[+]",
        &text("b.service"),
    );
    assert_eq!(out, "[Unit]\nAfter=a.service\nAfter=b.service\n");
}

#[test]
fn append_to_a_key_that_is_not_a_list_is_a_key_conflict() {
    let mut doc = document(IniProfile::Generic, "[a]\nk = 1\n");
    let error = doc.set(&key("a.k[+]"), &text("2"), true).unwrap_err();
    assert_eq!(error.code(), ErrorCode::KeyConflict);
}

#[test]
fn append_creates_a_missing_list_with_create() {
    let out = set(
        IniProfile::Systemd,
        "[Unit]\nDescription=d\n",
        "Unit.After[+]",
        &text("a.service"),
    );
    assert_eq!(out, "[Unit]\nDescription=d\nAfter=a.service\n");
}

#[test]
fn generic_repeated_keys_are_set_by_index_and_removed_by_index() {
    let content = "[a]\nk = 1\nk = 2\nk = 3\n";
    let out = set(IniProfile::Generic, content, "a.k[1]", &text("two"));
    assert_eq!(out, "[a]\nk = 1\nk = two\nk = 3\n");
    assert_eq!(
        remove(IniProfile::Generic, content, "a.k[0]"),
        "[a]\nk = 2\nk = 3\n"
    );
}

#[test]
fn global_key_is_inserted_before_the_first_section_and_its_comment() {
    let out = set(
        IniProfile::Generic,
        "; first section\n[a]\nk = 1\n",
        "name",
        &text("v"),
    );
    assert_eq!(out, "name = v\n; first section\n[a]\nk = 1\n");
}

#[test]
fn global_key_follows_the_existing_global_keys() {
    let out = set(
        IniProfile::Editorconfig,
        "# top\nroot = true\n\n[*]\nindent_size = 2\n",
        "charset",
        &text("utf-8"),
    );
    assert_eq!(
        out,
        "# top\nroot = true\ncharset = utf-8\n\n[*]\nindent_size = 2\n"
    );
}

#[test]
fn global_key_is_inserted_after_the_byte_order_mark() {
    let out = set(IniProfile::Generic, "\u{feff}[a]\nk=1\n", "g", &text("v"));
    assert_eq!(out, "\u{feff}g=v\n[a]\nk=1\n");
}

#[test]
fn global_key_is_appended_to_a_document_without_sections() {
    let out = set(IniProfile::Generic, "a = 1", "b", &text("2"));
    assert_eq!(out, "a = 1\nb = 2");
    assert_eq!(set(IniProfile::Generic, "", "b", &text("2")), "b = 2\n");
}

#[test]
fn global_keys_are_read_by_their_bare_name() {
    let doc = document(IniProfile::Generic, "root = true\n[a]\nk = 1\n");
    let found = doc.get(&key("root")).unwrap();
    assert_eq!(found[0].value, text("true"));
}

#[test]
fn removing_a_global_key_keeps_the_sections() {
    assert_eq!(
        remove(IniProfile::Generic, "g = 1\n[a]\nk = 1\n", "g"),
        "[a]\nk = 1\n"
    );
}

#[test]
fn gitconfig_section_is_replaced_by_a_map() {
    let content = "[core]\n\tbare = false\n\teditor = vim\n";
    let value = map(&[("bare", text("true")), ("pager", text("less"))]);
    let out = set(IniProfile::Gitconfig, content, "core", &value);
    assert_eq!(out, "[core]\n\tbare = true\n\tpager = less\n");
}

#[test]
fn gitconfig_subsection_is_created_with_an_escaped_name() {
    let out = set(
        IniProfile::Gitconfig,
        "[core]\n\tbare = false\n",
        "remote[\"Or\\\"igin\"].url",
        &text("x"),
    );
    assert_eq!(
        out,
        "[core]\n\tbare = false\n\n[remote \"Or\\\"igin\"]\n\turl = x\n"
    );
}

#[test]
fn gitconfig_new_key_goes_into_the_last_section_of_the_same_name() {
    let content =
        "[remote \"a\"]\n\turl = 1\n[core]\n\tbare = false\n[remote \"a\"]\n\tfetch = f\n";
    let out = set(
        IniProfile::Gitconfig,
        content,
        "remote.a.prune",
        &text("true"),
    );
    assert_eq!(
        out,
        "[remote \"a\"]\n\turl = 1\n[core]\n\tbare = false\n[remote \"a\"]\n\tfetch = f\n\tprune = true\n"
    );
}

#[test]
fn gitconfig_section_with_a_map_value_is_created_from_the_fields() {
    let value = map(&[("name", text("n")), ("email", text("e"))]);
    let out = set(
        IniProfile::Gitconfig,
        "[core]\n\tbare = true\n",
        "user",
        &value,
    );
    assert_eq!(
        out,
        "[core]\n\tbare = true\n\n[user]\n\tname = n\n\temail = e\n"
    );
}

#[test]
fn gitconfig_value_is_quoted_when_it_needs_quotes() {
    let out = set(
        IniProfile::Gitconfig,
        "[a]\n\tb = 1\n",
        "a.b",
        &text("x ; y"),
    );
    assert_eq!(out, "[a]\n\tb = \"x ; y\"\n");
}

#[test]
fn gitconfig_key_without_separator_gets_a_value_in_place() {
    let out = set(
        IniProfile::Gitconfig,
        "[core]\n\tbare\n\teditor = vim\n",
        "core.bare",
        &text("true"),
    );
    assert_eq!(out, "[core]\n\tbare = true\n\teditor = vim\n");
}

#[test]
fn gitconfig_key_without_separator_reads_as_true() {
    let doc = document(IniProfile::Gitconfig, "[core]\n\tbare\n");
    assert_eq!(
        doc.get(&key("core.bare")).unwrap()[0].value,
        Value::Bool(true)
    );
}

#[test]
fn php_list_suffix_is_kept_for_appended_elements() {
    let out = set(
        IniProfile::Php,
        "[PHP]\nextension[] = a\nextension[] = b\n",
        "PHP.extension[+]",
        &text("c"),
    );
    assert_eq!(
        out,
        "[PHP]\nextension[] = a\nextension[] = b\nextension[] = c\n"
    );
}

#[test]
fn php_value_with_special_characters_is_quoted() {
    let out = set(
        IniProfile::Php,
        "[PHP]\nmemory_limit = 128M\n",
        "PHP.memory_limit",
        &text("a=b"),
    );
    assert_eq!(out, "[PHP]\nmemory_limit = \"a=b\"\n");
}

#[test]
fn desktop_values_are_escaped() {
    let out = set(
        IniProfile::Desktop,
        "[Desktop Entry]\nName=a\nName[pl]=b\n",
        "[\"Desktop Entry\"].Name",
        &text(" x\ty"),
    );
    assert!(out.starts_with("[Desktop Entry]\nName=\\sx\\ty\n"), "{out}");
    let doc = document(IniProfile::Desktop, &out);
    let read = doc.get(&key("[\"Desktop Entry\"].Name")).unwrap();
    assert_eq!(read[0].value, text(" x\ty"));
}

#[test]
fn desktop_localized_keys_are_separate_keys() {
    let doc = document(IniProfile::Desktop, "[Desktop Entry]\nName=a\nName[pl]=b\n");
    let read = doc.get(&key("[\"Desktop Entry\"][\"Name[pl]\"]")).unwrap();
    assert_eq!(read[0].value, text("b"));
}

#[test]
fn sections_with_dots_are_addressed_by_a_quoted_segment() {
    let mut doc = document(IniProfile::Generic, "[a.b]\nk = 1\n");
    assert_eq!(doc.get(&key("[\"a.b\"].k")).unwrap()[0].value, text("1"));
    doc.set(&key("[\"a.b\"].k"), &text("2"), false).unwrap();
    assert_eq!(doc.text(), "[a.b]\nk = 2\n");
}

#[test]
fn new_section_and_key_are_appended_with_a_blank_line() {
    let out = set(IniProfile::Generic, "[a]\nk = 1\n", "b.x", &text("2"));
    assert_eq!(out, "[a]\nk = 1\n\n[b]\nx = 2\n");
    let crlf = set(IniProfile::Generic, "[a]\r\nk = 1\r\n", "b.x", &text("2"));
    assert_eq!(crlf, "[a]\r\nk = 1\r\n\r\n[b]\r\nx = 2\r\n");
}

#[test]
fn new_section_follows_a_document_without_final_line_break() {
    let out = set(IniProfile::Generic, "[a]\nk = 1", "b.x", &text("2"));
    assert_eq!(out, "[a]\nk = 1\n\n[b]\nx = 2\n");
}

#[test]
fn new_key_is_added_to_the_last_section_of_that_name() {
    let out = set(
        IniProfile::Generic,
        "[a]\nk = 1\n\n[b]\nx = 1\n",
        "a.y",
        &text("2"),
    );
    assert_eq!(out, "[a]\nk = 1\ny = 2\n\n[b]\nx = 1\n");
}

#[test]
fn removing_a_section_takes_its_comment_and_keys() {
    let content = "[a]\nk = 1\n\n# about b\n[b]\nx = 1\n[c]\ny = 2\n";
    assert_eq!(
        remove(IniProfile::Generic, content, "b"),
        "[a]\nk = 1\n\n[c]\ny = 2\n"
    );
}

#[test]
fn removing_the_last_key_without_final_line_break_takes_the_preceding_break() {
    assert_eq!(
        remove(IniProfile::Generic, "[a]\nk = 1\nx = 2", "a.x"),
        "[a]\nk = 1"
    );
}

#[test]
fn removing_keys_selected_by_a_wildcard_removes_each_of_them() {
    let out = remove(
        IniProfile::Generic,
        "[a]\nk1 = 1\nk2 = 2\n[b]\nk = 3\n",
        "a.*",
    );
    assert_eq!(out, "[a]\n[b]\nk = 3\n");
}

#[test]
fn removing_the_document_root_is_a_key_conflict() {
    let mut doc = document(IniProfile::Generic, "[a]\nk = 1\n");
    assert_eq!(
        doc.remove(&KeyPath::parse("$").unwrap())
            .unwrap_err()
            .code(),
        ErrorCode::KeyConflict
    );
}

#[test]
fn removing_a_missing_key_is_reported() {
    let mut doc = document(IniProfile::Generic, "[a]\nk = 1\n");
    let error = doc.remove(&key("a.missing")).unwrap_err();
    assert_eq!(error.code(), ErrorCode::KeyNotFound);
}

#[test]
fn setting_a_missing_key_without_create_is_reported() {
    let mut doc = document(IniProfile::Generic, "[a]\nk = 1\n");
    let error = doc.set(&key("a.missing"), &text("1"), false).unwrap_err();
    assert_eq!(error.code(), ErrorCode::KeyNotFound);
}

#[test]
fn scalar_for_a_section_is_refused_with_its_construct() {
    let mut doc = document(IniProfile::Generic, "[a]\nk = 1\n");
    let error = doc.set(&key("a"), &text("1"), false).unwrap_err();
    assert_eq!(construct(&error), "ini.section_value");
    assert_eq!(doc.text(), "[a]\nk = 1\n");
}

#[test]
fn map_for_a_key_is_refused_as_nested_value() {
    let mut doc = document(IniProfile::Generic, "[a]\nk = 1\n");
    let error = doc
        .set(&key("a.k"), &map(&[("x", text("1"))]), false)
        .unwrap_err();
    assert_eq!(construct(&error), "ini.nested_value");
}

#[test]
fn list_for_a_key_of_a_profile_without_lists_is_refused_as_nested_value() {
    let mut doc = document(IniProfile::Generic, "[a]\nk = 1\n");
    let error = doc.set(&key("a.k"), &list(&["1", "2"]), false).unwrap_err();
    assert_eq!(construct(&error), "ini.nested_value");
    let error = doc.set(&key("a.new"), &list(&["1"]), true).unwrap_err();
    assert_eq!(construct(&error), "ini.nested_value");
}

#[test]
fn list_with_a_nested_element_is_refused_as_nested_value() {
    let mut doc = document(IniProfile::Systemd, "[a]\nk=1\n");
    let nested = Value::List(vec![list(&["x"])]);
    let error = doc.set(&key("a.k"), &nested, false).unwrap_err();
    assert_eq!(construct(&error), "ini.nested_value");
}

#[test]
fn value_that_one_line_cannot_hold_is_refused_as_unrepresentable() {
    let mut doc = document(IniProfile::Generic, "[a]\nk = 1\n");
    for value in ["a\nb", " lead", "trail ", "end\\"] {
        let error = doc.set(&key("a.k"), &text(value), false).unwrap_err();
        assert_eq!(construct(&error), "ini.unrepresentable_value", "{value:?}");
    }
    let error = doc.set(&key("a.new"), &text("a\nb"), true).unwrap_err();
    assert_eq!(construct(&error), "ini.unrepresentable_value");
    assert_eq!(doc.text(), "[a]\nk = 1\n");
}

#[test]
fn key_name_that_the_profile_cannot_write_is_refused() {
    let mut doc = document(IniProfile::Generic, "[a]\nk = 1\n");
    for name in ["a=b", ";x", "[x", " x"] {
        let path = KeyPath::new(
            None,
            vec![
                nurt_contracts::keypath::KeySegment::Key("a".to_owned()),
                nurt_contracts::keypath::KeySegment::Key(name.to_owned()),
            ],
        )
        .unwrap();
        let error = doc.set(&path, &text("1"), true).unwrap_err();
        assert_eq!(construct(&error), "ini.key_name", "{name:?}");
    }
    let mut git = document(IniProfile::Gitconfig, "[core]\n\tbare = true\n");
    let error = git
        .set(&key("core[\"a b\"]"), &text("1"), true)
        .unwrap_err();
    assert_eq!(construct(&error), "ini.key_name");
}

#[test]
fn section_name_that_the_profile_cannot_write_is_refused() {
    let mut doc = document(IniProfile::Generic, "[a]\nk = 1\n");
    let path = KeyPath::new(
        None,
        vec![
            nurt_contracts::keypath::KeySegment::Key(" b".to_owned()),
            nurt_contracts::keypath::KeySegment::Key("k".to_owned()),
        ],
    )
    .unwrap();
    assert_eq!(
        construct(&doc.set(&path, &text("1"), true).unwrap_err()),
        "ini.section_name"
    );
    let mut git = document(IniProfile::Gitconfig, "[core]\n\tbare = true\n");
    let error = git
        .set(&key("[\"bad name\"].k"), &text("1"), true)
        .unwrap_err();
    assert_eq!(construct(&error), "ini.section_name");
}

#[test]
fn git_key_outside_a_section_is_refused_as_global_key() {
    let mut doc = document(IniProfile::Gitconfig, "[core]\n\tbare = true\n");
    let error = doc.set(&key("name"), &text("1"), true).unwrap_err();
    assert_eq!(construct(&error), "ini.global_key");
}

#[test]
fn key_path_deeper_than_a_section_is_refused_as_nested_value() {
    let mut doc = document(IniProfile::Generic, "[a]\nk = 1\n");
    let error = doc.set(&key("a.b.c"), &text("1"), true).unwrap_err();
    assert_eq!(construct(&error), "ini.nested_value");
}

#[test]
fn setting_the_current_value_changes_nothing() {
    let mut doc = document(IniProfile::Generic, "[a]\nk = 1 \n");
    assert_eq!(doc.set(&key("a.k"), &text("1"), false).unwrap(), 1);
    assert_eq!(doc.text(), "[a]\nk = 1 \n");
}

#[test]
fn key_naming_both_a_key_and_a_section_is_a_parse_error() {
    let error =
        IniDocument::parse("[a]\nb = 1\n[a.b]\nc = 1\n", IniProfile::Gitconfig).unwrap_err();
    assert_eq!(error.code(), ErrorCode::DocParse);
    assert!(error.to_string().contains("names both a key and a section"));
}

#[test]
fn get_reports_spans_and_comments_of_keys_and_sections() {
    let doc = document(
        IniProfile::Generic,
        "; about a\n[a]\nk = 1 ; kept in the value\n; about x\nx = 2\n\n[b]\ny = 3\n",
    );
    let section = doc.get(&key("a")).unwrap();
    assert_eq!(section[0].comment.as_deref(), Some("about a"));
    assert_eq!(section[0].span.unwrap().start.line, 2);
    assert_eq!(section[0].span.unwrap().end.line, 5);
    let entry = doc.get(&key("a.x")).unwrap();
    assert_eq!(entry[0].comment.as_deref(), Some("about x"));
    assert_eq!(entry[0].value, text("2"));
    let root = doc.get(&KeyPath::parse("$").unwrap()).unwrap();
    assert_eq!(root.len(), 1);
    assert!(matches!(root[0].value, Value::Map(_)));
    assert_eq!(doc.profile(), IniProfile::Generic);
}

#[test]
fn get_reports_repeated_keys_of_list_profiles_as_one_list() {
    let doc = document(IniProfile::Systemd, UNIT);
    let found = doc.get(&key("Service.ExecStart")).unwrap();
    assert_eq!(found[0].value, list(&["/a", "/b"]));
    let by_index = doc.get(&key("Service.ExecStart[1]")).unwrap();
    assert_eq!(by_index[0].value, text("/b"));
}

#[test]
fn get_of_a_repeated_key_without_index_is_a_key_conflict() {
    let doc = document(IniProfile::Generic, "[a]\nk = 1\nk = 2\n");
    assert_eq!(
        doc.get(&key("a.k")).unwrap_err().code(),
        ErrorCode::KeyConflict
    );
    assert_eq!(doc.to_value(), map(&[("a", map(&[("k", text("2"))]))]));
}

#[test]
fn gitconfig_names_compare_without_case_except_subsections() {
    let doc = document(
        IniProfile::Gitconfig,
        "[Remote \"Origin\"]\n\tURL = a\n[remote \"origin\"]\n\tUrl = b\n",
    );
    assert_eq!(
        doc.get(&key("REMOTE.Origin.url")).unwrap()[0].value,
        text("a")
    );
    assert_eq!(
        doc.get(&key("remote.origin.URL")).unwrap()[0].value,
        text("b")
    );
    assert_eq!(
        doc.get(&key("remote.ORIGIN.url")).unwrap_err().code(),
        ErrorCode::KeyNotFound
    );
}

#[test]
fn an_ini_document_above_the_size_limit_is_refused_before_its_tree_is_built() {
    let content: Vec<u8> = (0..1000)
        .flat_map(|index| format!("k{index} = v\n").into_bytes())
        .collect();
    let options = DocOptions {
        max_bytes: 1024,
        ..DocOptions::default()
    };
    let format = IniFormat::new();
    let error = nurt_doc::registry::open_with(&format, &content, &options).unwrap_err();
    assert_eq!(error.code(), ErrorCode::Limit);
    let within = DocOptions {
        max_bytes: u64::try_from(content.len()).unwrap(),
        ..DocOptions::default()
    };
    assert!(nurt_doc::registry::open_with(&format, &content, &within).is_ok());
}

#[test]
fn registered_format_resolves_gitconfig_subsections_by_exact_name() {
    let content = b"[remote \"Origin\"]\n\turl = a\n[remote \"origin\"]\n\turl = b\n".to_vec();
    let format = IniFormat::with_profile(IniProfile::Gitconfig);
    let mut handle = format.open(content, &DocOptions::default()).unwrap();
    let options = nurt_doc::model::SetOptions::default();
    handle
        .set(&key("remote.Origin.url"), &text("c"), &options)
        .unwrap();
    assert_eq!(
        handle.text(),
        b"[remote \"Origin\"]\n\turl = c\n[remote \"origin\"]\n\turl = b\n"
    );
    assert!(handle.get(&key("remote.ORIGIN.url")).unwrap().is_empty());
    assert_eq!(handle.get(&key("REMOTE.origin.URL")).unwrap().len(), 1);
}

#[test]
fn registered_format_takes_the_profile_from_the_file_name() {
    let content = b"[core]\nk = 1\n".to_vec();
    let format = IniFormat::new();
    let options = DocOptions::default();
    let folded =
        |handle: &dyn nurt_doc::model::DocHandle| !handle.get(&key("CORE.K")).unwrap().is_empty();
    assert!(folded(&*format.open(content.clone(), &options).unwrap()));
    for (path, expected) in [
        (Some("data/web.service"), false),
        (Some("app.ini"), false),
        (Some("notes.txt"), true),
        (None, true),
    ] {
        let handle = format
            .open_at(path.map(Path::new), content.clone(), &options)
            .unwrap();
        assert_eq!(folded(&*handle), expected, "{path:?}");
    }
}

#[test]
fn registered_format_keeps_the_profile_of_the_file_name_after_a_change() {
    let content = b"[Unit]\nAfter=a\nAfter=b\n[Install]\nWantedBy=x\n".to_vec();
    let format = IniFormat::new();
    let mut handle = format
        .open_at(
            Some(Path::new("a.service")),
            content,
            &DocOptions::default(),
        )
        .unwrap();
    let options = nurt_doc::model::SetOptions { create: true };
    handle.remove(&key("Install")).unwrap();
    handle
        .set(&key("Unit.After[+]"), &text("c"), &options)
        .unwrap();
    assert_eq!(handle.text(), b"[Unit]\nAfter=a\nAfter=b\nAfter=c\n");
}
