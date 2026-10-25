//! Which sources a repository has, which this build reads, and the GitLab
//! CLI's say in whether a remote is a GitLab project.

use super::{Provider, RepoInfo, SourceKey, SourceStatus, SyncState, gitlab};

/// Makes `info`'s remote a GitLab source when glab knows its host; returns
/// whether it changed. `cli` reads glab's config, only when needed.
pub(super) fn recognize(
    info: &mut RepoInfo,
    cli: impl FnOnce() -> Vec<gitlab::cli_hosts::CliHost>,
) -> bool {
    info.remote
        .as_mut()
        .is_some_and(|remote| gitlab::recognize(&mut remote.source, cli))
}

/// Each source of `info`, syncing when this build can read it.
pub(super) fn statuses(info: &RepoInfo) -> Vec<SourceStatus> {
    sources(info)
        .into_iter()
        .map(|(key, supported)| SourceStatus {
            key,
            state: if supported {
                SyncState::Syncing
            } else {
                SyncState::Unsupported
            },
        })
        .collect()
}

/// The sources a repository has, and whether this build can read each.
pub fn sources(info: &RepoInfo) -> Vec<(SourceKey, bool)> {
    let remote = info.remote.as_ref().map(|remote| {
        let supported = match remote.source.provider {
            Provider::Github => remote.source.host == "github.com",
            Provider::Gitlab => true,
            Provider::Beads => false,
        };
        (remote.source.clone(), supported)
    });
    remote
        .into_iter()
        .chain(info.beads_source().map(|key| (key, true)))
        .collect()
}
