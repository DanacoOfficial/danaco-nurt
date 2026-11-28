// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// https://github.com/DanacoOfficial | support@danaco-group.pl
//
// Module:           nurt-core::system_services
// Responsibility:   Shared services of the running process.
// Dependencies:     nurt_code, nurt_contracts, nurt_doc, nurt_files, nurt_fs, nurt_git, nurt_proc, nurt_sys, nurt_testkit, nurt_text
// Author:           Danaco Official
// Version:          0.1.1, 2026-11-27
// License:          see the LICENSE file or https://www.danaco-group.pl

//! Shared services of the running process.
//!
//! The command crates reach each other through the service traits of
//! `nurt_contracts::services`. The engine of an executable registers the
//! implementation of every group that is linked into the build, so that the
//! commands that depend on another group (`read key:` on documents,
//! `read symbol:` on source code, the language server client, the file
//! listing of the code commands) find it. An embedding application that gives
//! its own implementation of a service keeps it.

use std::sync::Arc;

use nurt_code::service::CodeAnalysis;
use nurt_contracts::config::Config;
use nurt_contracts::env::{Clock, Host};
use nurt_contracts::fs::Fs;
use nurt_contracts::services::Services;
use nurt_doc::registry::{DocOptions, Registry};
use nurt_doc::service::DocumentService;
use nurt_files::walk::FileList;
use nurt_git::query::RepoQueries;
use nurt_sys::open_files::SystemFileHolders;
use nurt_text::TextCodec;

/// Adds the services that `services` lacks: documents, source code, process
/// host, file listing (when the engine has a file system), repository
/// queries, text codec and file holders.
///
/// The review rules have no implementation of their own; the built-in rules
/// apply without them.
pub(crate) fn complete(
    services: Services,
    fs: Option<&Arc<dyn Fs>>,
    clock: &Arc<dyn Clock>,
    host: &Arc<dyn Host>,
    config: &Config,
) -> Services {
    let mut services = services;
    if services.doc().is_none() {
        let options = DocOptions::from_config(&config.documents);
        services = services.with_doc(Arc::new(DocumentService::new(Registry::builtin(), options)));
    }
    if services.code().is_none() {
        services = services.with_code(Arc::new(CodeAnalysis::builtin()));
    }
    if services.process_host().is_none() {
        services = services.with_process_host(nurt_proc::process_host());
    }
    if services.file_list().is_none()
        && let Some(fs) = fs
    {
        services = services.with_file_list(Arc::new(FileList::new(Arc::clone(fs))));
    }
    if services.git_query().is_none() {
        services = services.with_git_query(Arc::new(RepoQueries::new(
            Arc::clone(clock),
            Arc::clone(host),
        )));
    }
    if services.text_codec().is_none() {
        let codec = TextCodec::with_legacy_encodings(&config.text.legacy_encodings);
        services = services.with_text_codec(Arc::new(codec));
    }
    if services.file_holders().is_none() {
        services = services.with_file_holders(Arc::new(SystemFileHolders::new()));
    }
    services
}

#[cfg(test)]
mod tests {
    use nurt_contracts::services::ServiceKind;
    use nurt_testkit::clock::VirtualClock;
    use nurt_testkit::host::TestHost;

    use super::*;

    fn complete_with(services: Services, fs: Option<&Arc<dyn Fs>>) -> Services {
        let clock: Arc<dyn Clock> = Arc::new(VirtualClock::new());
        let host: Arc<dyn Host> = Arc::new(TestHost::linux());
        complete(services, fs, &clock, &host, &Config::default())
    }

    #[test]
    fn every_group_of_the_build_registers_its_service() {
        let fs: Arc<dyn Fs> = nurt_fs::system_fs();
        let services = complete_with(Services::new(), Some(&fs));
        assert_eq!(
            services.available(),
            [
                ServiceKind::Doc,
                ServiceKind::Code,
                ServiceKind::ProcessHost,
                ServiceKind::FileList,
                ServiceKind::GitQuery,
                ServiceKind::TextCodec,
                ServiceKind::FileHolders,
            ]
        );
    }

    #[test]
    fn the_file_listing_needs_a_file_system() {
        let services = complete_with(Services::new(), None);
        assert!(!services.has(ServiceKind::FileList));
        assert!(services.has(ServiceKind::Doc));
    }

    #[test]
    fn a_service_given_by_the_embedding_application_is_kept() {
        let given = Arc::new(CodeAnalysis::builtin());
        let services = complete_with(Services::new().with_code(given.clone()), None);
        let kept = services.code().expect("the given service stays registered");
        assert!(std::ptr::addr_eq(kept, Arc::as_ptr(&given)));
    }
}

// Copyright (c) 2026 Danaco Official | Danaco Holding Group Sp. z o.o. All rights reserved.
// Use is subject to license terms: see the LICENSE file or https://www.danaco-group.pl
// Support: support@danaco-group.pl
