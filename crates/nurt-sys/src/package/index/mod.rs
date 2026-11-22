// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//! Readers of the local indexes of package repositories.
//!
//! The indexes are the files the managers keep after a refresh of their
//! repositories. The readers stream the entries to a sink, so that a search
//! holds only its matches in memory.

use nurt_contracts::record::PackageManager;

use super::root::HostEnv;

mod apt;
mod brew;
mod pacman;
mod repodata;
mod scoop;
mod winget;

/// Package offered by a repository.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct IndexEntry {
    /// Package name.
    pub name: String,
    /// Offered version.
    pub version: String,
    /// Architecture.
    pub architecture: Option<String>,
    /// One-line description.
    pub description: Option<String>,
    /// Installed size in bytes.
    pub size: Option<u64>,
    /// Repository the entry comes from.
    pub source: String,
    /// Percentage of systems that receive the update (apt phased updates).
    pub phased: Option<u8>,
}

/// Problem found while reading an index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexWarning {
    /// Stable code of the problem.
    pub code: &'static str,
    /// Description of the problem.
    pub detail: String,
}

impl IndexWarning {
    pub(crate) fn new(code: &'static str, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}

/// Outcome of reading the indexes of one manager.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexReport {
    /// Index files or directories that exist.
    pub sources_found: usize,
    /// Index files or directories that were read completely.
    pub sources_read: usize,
    /// Problems that did not stop the reading.
    pub warnings: Vec<IndexWarning>,
}

impl IndexReport {
    pub(crate) fn warn(&mut self, code: &'static str, detail: impl Into<String>) {
        self.warnings.push(IndexWarning::new(code, detail));
    }

    /// Returns `true` when indexes exist and none of them could be read.
    #[must_use]
    pub fn unreadable(&self) -> bool {
        self.sources_found > 0 && self.sources_read == 0
    }
}

/// Receiver of the entries of an index.
pub(crate) type EntrySink<'a> = &'a mut dyn FnMut(IndexEntry);

/// Reads the repository indexes of `manager` and passes every entry to
/// `sink`.
///
/// A missing index is reported with the code `no_index` and a damaged one
/// with `corrupt_index`; neither stops the reading of the other indexes.
pub fn read_indexes(env: &HostEnv, manager: PackageManager, sink: EntrySink<'_>) -> IndexReport {
    let mut report = IndexReport::default();
    match manager {
        PackageManager::Apt => apt::read(env, sink, &mut report),
        PackageManager::Dnf => repodata::read_dnf(env, sink, &mut report),
        PackageManager::Zypper => repodata::read_zypper(env, sink, &mut report),
        PackageManager::Pacman => pacman::read(env, sink, &mut report),
        PackageManager::Brew => brew::read(env, sink, &mut report),
        PackageManager::Scoop => scoop::read(env, sink, &mut report),
        PackageManager::Winget => winget::read(env, sink, &mut report),
        PackageManager::Choco => report.warn(
            "no_local_index",
            "chocolatey keeps no local index of its repositories",
        ),
        _ => report.warn("no_index", format!("{manager} has no repository index")),
    }
    if report.sources_found == 0 && report.warnings.is_empty() {
        report.warn("no_index", format!("no repository index of {manager} found"));
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::package::compress::testing;
    use crate::package::root::SysRoot;
    use nurt_contracts::env::Os;
    use std::fs;
    use std::path::Path;

    fn collect(env: &HostEnv, manager: PackageManager) -> (Vec<IndexEntry>, IndexReport) {
        let mut entries = Vec::new();
        let report = read_indexes(env, manager, &mut |entry| entries.push(entry));
        entries.sort_by(|a, b| a.name.cmp(&b.name).then(a.source.cmp(&b.source)));
        (entries, report)
    }

    fn put(root: &Path, path: &str, bytes: &[u8]) {
        let target = root.join(path);
        fs::create_dir_all(target.parent().expect("parent")).expect("directories");
        fs::write(target, bytes).expect("file");
    }

    const APT_ALL: &str = "Package: docs\nVersion: 2.0\nArchitecture: all\nDescription: documentation\nInstalled-Size: 10\n\n";
    const APT_AMD64: &str = "Package: tool\nVersion: 1.2-1\nArchitecture: amd64\nDescription: a tool\n more text\nInstalled-Size: 100\nPhased-Update-Percentage: 40\n\nPackage: tool32\nVersion: 1.2-1\nArchitecture: i386\n\n";

    #[test]
    fn apt_lists_parse_compressed_variants() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let lists = "var/lib/apt/lists/";
        put(dir.path(), "var/lib/dpkg/arch", b"amd64\n");
        put(
            dir.path(),
            &format!("{lists}deb.example.org_debian_dists_stable_main_binary-amd64_Packages"),
            APT_AMD64.as_bytes(),
        );
        put(
            dir.path(),
            &format!("{lists}deb.example.org_debian_dists_stable_contrib_binary-amd64_Packages.gz"),
            &testing::gzip(b"Package: g\nVersion: 1\nArchitecture: amd64\n\n"),
        );
        put(
            dir.path(),
            &format!("{lists}deb.example.org_debian_dists_stable_extra_binary-amd64_Packages.xz"),
            &testing::xz(b"Package: x\nVersion: 1\nArchitecture: amd64\n\n"),
        );
        put(
            dir.path(),
            &format!("{lists}deb.example.org_debian_dists_stable_more_binary-amd64_Packages.lz4"),
            &testing::lz4(b"Package: l\nVersion: 1\nArchitecture: amd64\n\n"),
        );
        put(
            dir.path(),
            &format!("{lists}deb.example.org_debian_dists_stable_zst_binary-amd64_Packages.zst"),
            &testing::zstd(b"Package: z\nVersion: 1\nArchitecture: amd64\n\n"),
        );
        put(
            dir.path(),
            &format!("{lists}deb.example.org_debian_dists_stable_main_i18n_Translation-en"),
            b"Package: ignored\nVersion: 1\n\n",
        );
        let env = HostEnv::new(SysRoot::at(dir.path()), Os::Linux);
        let (entries, report) = collect(&env, PackageManager::Apt);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["g", "l", "tool", "x", "z"]);
        assert_eq!(report.sources_read, 5);
        let tool = entries.iter().find(|e| e.name == "tool").expect("tool");
        assert_eq!(tool.size, Some(102_400));
        assert_eq!(tool.phased, Some(40));
        assert_eq!(tool.source, "deb.example.org/debian stable/main");
        assert_eq!(tool.description.as_deref(), Some("a tool"));
    }

    #[test]
    fn apt_lists_architecture_filter_all_and_native() {
        let dir = tempfile::tempdir().expect("temporary directory");
        put(dir.path(), "var/lib/dpkg/arch", b"arm64\n");
        let text = format!("{APT_ALL}Package: native\nVersion: 1\nArchitecture: arm64\n\n{APT_AMD64}");
        put(
            dir.path(),
            "var/lib/apt/lists/h_dists_s_main_binary-arm64_Packages",
            text.as_bytes(),
        );
        let env = HostEnv::new(SysRoot::at(dir.path()), Os::Linux);
        let (entries, _) = collect(&env, PackageManager::Apt);
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["docs", "native"]);
    }

    fn primary_xml() -> String {
        concat!(
            "<?xml version=\"1.0\"?>\n<metadata xmlns=\"http://linux.duke.edu/metadata/common\" packages=\"3\">\n",
            "<package type=\"rpm\"><name>bash</name><arch>x86_64</arch>",
            "<version epoch=\"0\" ver=\"5.2\" rel=\"1.fc40\"/><summary>The GNU &amp; shell</summary>",
            "<size package=\"100\" installed=\"7000\" archive=\"8000\"/>",
            "<format><rpm:provides><rpm:entry name=\"bash\"/></rpm:provides></format></package>\n",
            "<package type=\"rpm\"><name>perl</name><arch>x86_64</arch>",
            "<version epoch=\"4\" ver=\"5.38\" rel=\"2\"/><summary>Perl</summary><size package=\"50\" installed=\"0\"/></package>\n",
            "<package type=\"rpm\"><name>bash</name><arch>src</arch><version epoch=\"0\" ver=\"5.2\" rel=\"1\"/><summary>src</summary></package>\n",
            "</metadata>\n"
        )
        .to_owned()
    }

    fn repomd(file: &str) -> String {
        format!(
            "<?xml version=\"1.0\"?>\n<repomd xmlns=\"http://linux.duke.edu/metadata/repo\"><data type=\"filelists\"><location href=\"repodata/other-filelists.xml.gz\"/></data><data type=\"primary\"><location href=\"repodata/{file}\"/></data></repomd>"
        )
    }

    #[test]
    fn dnf_repomd_primary_zst_and_gz() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let primary = primary_xml();
        put(
            dir.path(),
            "var/cache/libdnf5/fedora-0123456789abcdef/repodata/repomd.xml",
            repomd("aa-primary.xml.zst").as_bytes(),
        );
        put(
            dir.path(),
            "var/cache/libdnf5/fedora-0123456789abcdef/repodata/aa-primary.xml.zst",
            &testing::zstd(primary.as_bytes()),
        );
        put(
            dir.path(),
            "var/cache/dnf/updates-fedcba9876543210/repodata/repomd.xml",
            repomd("bb-primary.xml.gz").as_bytes(),
        );
        put(
            dir.path(),
            "var/cache/dnf/updates-fedcba9876543210/repodata/bb-primary.xml.gz",
            &testing::gzip(primary.as_bytes()),
        );
        let env = HostEnv::new(SysRoot::at(dir.path()), Os::Linux);
        let (entries, report) = collect(&env, PackageManager::Dnf);
        assert_eq!(report.sources_read, 2);
        assert_eq!(entries.len(), 4);
        let bash = &entries[0];
        assert_eq!(bash.name, "bash");
        assert_eq!(bash.version, "5.2-1.fc40");
        assert_eq!(bash.description.as_deref(), Some("The GNU & shell"));
        assert_eq!(bash.size, Some(7000));
        assert_eq!(bash.source, "fedora");
        let perl = entries.iter().find(|e| e.name == "perl").expect("perl");
        assert_eq!(perl.version, "4:5.38-2");
        assert_eq!(perl.size, Some(50));
    }

    #[test]
    fn zypper_raw_repodata_primary() {
        let dir = tempfile::tempdir().expect("temporary directory");
        put(
            dir.path(),
            "var/cache/zypp/raw/repo-oss/repodata/repomd.xml",
            repomd("cc-primary.xml.xz").as_bytes(),
        );
        put(
            dir.path(),
            "var/cache/zypp/raw/repo-oss/repodata/cc-primary.xml.xz",
            &testing::xz(primary_xml().as_bytes()),
        );
        put(dir.path(), "var/cache/zypp/raw/repo-oss/solv/solv", b"binary");
        let env = HostEnv::new(SysRoot::at(dir.path()), Os::Linux);
        let (entries, report) = collect(&env, PackageManager::Zypper);
        assert_eq!(report.sources_read, 1);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].source, "repo-oss");
    }

    fn sync_database(compress: fn(&[u8]) -> Vec<u8>) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, desc) in [
            ("bash-5.2.026-2", "%NAME%\nbash\n\n%VERSION%\n5.2.026-2\n\n%DESC%\nThe GNU Bourne Again shell\n\n%CSIZE%\n1000\n\n%ISIZE%\n9000\n\n%ARCH%\nx86_64\n"),
            ("zlib-1:1.3-1", "%NAME%\nzlib\n\n%VERSION%\n1:1.3-1\n\n%ARCH%\nx86_64\n"),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(desc.len() as u64);
            header.set_mode(0o644);
            builder
                .append_data(&mut header, format!("{name}/desc"), desc.as_bytes())
                .expect("tar entry");
        }
        compress(&builder.into_inner().expect("tar"))
    }

    #[test]
    fn pacman_sync_db_gzip_and_zstd_tar() {
        let dir = tempfile::tempdir().expect("temporary directory");
        put(dir.path(), "var/lib/pacman/sync/core.db", &sync_database(testing::gzip));
        put(dir.path(), "var/lib/pacman/sync/extra.db", &sync_database(testing::zstd));
        put(dir.path(), "var/lib/pacman/sync/extra.db.sig", b"ignored");
        let env = HostEnv::new(SysRoot::at(dir.path()), Os::Linux);
        let (entries, report) = collect(&env, PackageManager::Pacman);
        assert_eq!(report.sources_read, 2);
        assert_eq!(entries.len(), 4);
        let bash = &entries[0];
        assert_eq!(bash.name, "bash");
        assert_eq!(bash.version, "5.2.026-2");
        assert_eq!(bash.size, Some(9000));
        assert_eq!(bash.source, "core");
        assert_eq!(bash.description.as_deref(), Some("The GNU Bourne Again shell"));
    }

    #[test]
    fn brew_api_jws_payload_parse() {
        let formulae = serde_json::json!([
            {"name": "ripgrep", "desc": "Search tool", "versions": {"stable": "14.1.0"}, "revision": 1},
            {"name": "wget", "desc": "Downloader", "versions": {"stable": "1.24.5"}, "revision": 0}
        ]);
        let casks = serde_json::json!([
            {"token": "firefox", "name": ["Firefox"], "desc": null, "version": "125.0"}
        ]);
        let jws = |payload: &serde_json::Value| {
            serde_json::json!({"payload": payload.to_string(), "signatures": []}).to_string()
        };
        let dir = tempfile::tempdir().expect("temporary directory");
        put(dir.path(), "cache/api/formula.jws.json", jws(&formulae).as_bytes());
        put(dir.path(), "cache/api/cask.jws.json", jws(&casks).as_bytes());
        put(dir.path(), "opt/homebrew/Cellar/", b"");
        let env = HostEnv::new(SysRoot::at(dir.path()), Os::Macos).with_var("HOMEBREW_CACHE", "/cache");
        let (entries, report) = collect(&env, PackageManager::Brew);
        assert_eq!(report.sources_read, 2);
        let versions: Vec<_> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.version.as_str(), e.source.as_str()))
            .collect();
        assert_eq!(
            versions,
            [
                ("firefox", "125.0", "homebrew/cask"),
                ("ripgrep", "14.1.0_1", "homebrew/core"),
                ("wget", "1.24.5", "homebrew/core"),
            ]
        );
        assert_eq!(entries[0].description.as_deref(), Some("Firefox"));
    }

    #[test]
    fn scoop_bucket_manifests_search() {
        let dir = tempfile::tempdir().expect("temporary directory");
        put(
            dir.path(),
            "Users/a/scoop/buckets/main/bucket/git.json",
            br#"{"version": "2.44.0", "description": "Distributed VCS"}"#,
        );
        put(
            dir.path(),
            "Users/a/scoop/buckets/extras/bucket/vlc.json",
            br#"{"version": "3.0.20", "description": "Media player"}"#,
        );
        put(dir.path(), "Users/a/scoop/buckets/main/bucket/broken.json", b"{not json");
        let env = HostEnv::new(SysRoot::at(dir.path()), Os::Windows)
            .with_var("USERPROFILE", "C:\\Users\\a");
        let (entries, report) = collect(&env, PackageManager::Scoop);
        let names: Vec<_> = entries.iter().map(|e| (e.name.as_str(), e.source.as_str())).collect();
        assert_eq!(names, [("git", "main"), ("vlc", "extras")]);
        assert!(report.warnings.iter().any(|w| w.code == "corrupt_index"));
    }

    #[test]
    fn corrupt_index_warns_and_skips() {
        let dir = tempfile::tempdir().expect("temporary directory");
        put(dir.path(), "var/lib/dpkg/arch", b"amd64\n");
        put(
            dir.path(),
            "var/lib/apt/lists/h_dists_s_main_binary-amd64_Packages.gz",
            b"\x1f\x8b not really gzip",
        );
        put(
            dir.path(),
            "var/lib/apt/lists/h_dists_s_good_binary-amd64_Packages",
            b"Package: ok\nVersion: 1\nArchitecture: amd64\n\n",
        );
        let env = HostEnv::new(SysRoot::at(dir.path()), Os::Linux);
        let (entries, report) = collect(&env, PackageManager::Apt);
        assert_eq!(entries.len(), 1);
        assert_eq!(report.sources_found, 2);
        assert_eq!(report.sources_read, 1);
        assert!(report.warnings.iter().any(|w| w.code == "corrupt_index"));
        assert!(!report.unreadable());
    }

    #[test]
    fn missing_indexes_are_reported_without_failure() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let env = HostEnv::new(SysRoot::at(dir.path()), Os::Linux);
        for manager in [PackageManager::Apt, PackageManager::Dnf, PackageManager::Pacman, PackageManager::Choco] {
            let (entries, report) = collect(&env, manager);
            assert!(entries.is_empty());
            assert!(!report.warnings.is_empty());
            assert!(!report.unreadable());
        }
    }

    #[test]
    fn winget_index_database_reads_ids_names_and_versions() {
        let dir = tempfile::tempdir().expect("temporary directory");
        let database = dir
            .path()
            .join("Users/a/AppData/Local/Packages/Microsoft.DesktopAppInstaller_8wekyb3d8bbwe/LocalState/Microsoft.Winget.Source_8wekyb3d8bbwe/Public/index.db");
        fs::create_dir_all(database.parent().expect("parent")).expect("directories");
        let connection = rusqlite::Connection::open(&database).expect("database");
        connection
            .execute_batch(
                "CREATE TABLE ids (id TEXT); CREATE TABLE names (name TEXT); CREATE TABLE versions (version TEXT);
                 CREATE TABLE manifest (id INTEGER, name INTEGER, version INTEGER);
                 INSERT INTO ids VALUES ('Git.Git'); INSERT INTO names VALUES ('Git'); INSERT INTO versions VALUES ('2.44.0');
                 INSERT INTO manifest VALUES (1, 1, 1);",
            )
            .expect("schema");
        drop(connection);
        let env = HostEnv::new(SysRoot::at(dir.path()), Os::Windows)
            .with_var("LOCALAPPDATA", "C:\\Users\\a\\AppData\\Local");
        let (entries, _) = collect(&env, PackageManager::Winget);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "Git.Git");
        assert_eq!(entries[0].description.as_deref(), Some("Git"));
        assert_eq!(entries[0].version, "2.44.0");
    }
}
