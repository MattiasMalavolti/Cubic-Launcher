#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

use crate::launcher_paths::LauncherPaths;
use crate::path_safety::validate_path_component;
use crate::mod_cache::ModCacheRecord;
use crate::modrinth::{ModrinthClient, ModrinthVersion};
use crate::process_streaming::ProcessLogStream;
use crate::resolver::{
    version_rules_conflict, FailureReason, ModLoader, ResolutionResult, ResolutionTarget,
    RuleOutcome,
};
use crate::rules::{ModList, ModSource, Rule, RULES_FILENAME};

use super::{
    embedded_minecraft_requirements_match, emit_log, ensure_remote_version_cached,
    load_cached_file_hashes_for_selected, load_cached_mod_record_for_target,
    read_embedded_fabric_requirements, SelectedMod,
};

pub(super) struct TopLevelVersionCandidates {
    selected_mod_id: String,
    project_id: String,
    candidates: Vec<ModrinthVersion>,
}

#[derive(Debug, Clone)]
pub(super) enum RemoteArtifact {
    Live(ModrinthVersion),
    Cached(ModCacheRecord),
}
/// Pick the preferred candidate by channel first, then by `date_published`
/// (RFC3339 UTC, so lexicographic comparison is chronological). Candidates are
/// already filtered to the exact/wildcard-compatible set by
/// `fetch_project_versions`.
fn select_preferred_version(versions: Vec<ModrinthVersion>) -> Option<ModrinthVersion> {
    versions.into_iter().max_by(|left, right| {
        left.channel_rank()
            .cmp(&right.channel_rank())
            .then_with(|| left.date_published.cmp(&right.date_published))
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DownloadArtifact {
    pub(super) filename: String,
    pub(super) url: String,
    pub(super) destination_path: PathBuf,
    pub(super) file_hash: Option<String>,
    pub(super) file_size: u64,
}

/// Extracts the artifact name from a jar filename by stripping the version suffix.
/// e.g. "asm-9.6.jar" → "asm", "fabric-loader-0.16.jar" → "fabric-loader"
pub(in crate::launch_preview) fn extract_artifact_name(filename: &str) -> String {
    let stem = filename.strip_suffix(".jar").unwrap_or(filename);
    // Find the last '-' followed by a digit — everything before it is the artifact name
    if let Some(pos) = stem.rfind(|c: char| c == '-').and_then(|i| {
        if stem[i + 1..].starts_with(|c: char| c.is_ascii_digit()) {
            Some(i)
        } else {
            None
        }
    }) {
        stem[..pos].to_string()
    } else {
        stem.to_string()
    }
}

pub(super) fn parse_mod_loader(value: &str) -> Result<ModLoader> {
    match value.trim().to_ascii_lowercase().as_str() {
        "fabric" => Ok(ModLoader::Fabric),
        "forge" => Ok(ModLoader::Forge),
        "neoforge" => Ok(ModLoader::NeoForge),
        "vanilla" => Ok(ModLoader::Vanilla),
        "quilt" => bail!("Quilt is no longer supported by Cubic Launcher"),
        other => bail!("unsupported mod loader '{other}'"),
    }
}

pub(super) fn load_modlist(launcher_paths: &LauncherPaths, modlist_name: &str) -> Result<ModList> {
    validate_path_component(modlist_name)?;
    let rules_path = launcher_paths
        .modlists_dir()
        .join(modlist_name)
        .join(RULES_FILENAME);

    ModList::read_from_file(&rules_path).with_context(|| {
        format!(
            "failed to read mod-list '{}' from {}",
            modlist_name,
            rules_path.display()
        )
    })
}

/// Fetch compatible versions only for the mods that were actually selected by resolution.
/// Network/API errors for individual mods are treated as "no compatible version"
/// so that the re-resolution pass can disable them and try alternatives.
pub(super) async fn prefetch_compatible_versions_for_selected(
    app_handle: &tauri::AppHandle,
    _launcher_paths: &LauncherPaths,
    _http_client: &reqwest::Client,
    selected_mods: &[SelectedMod],
    client: &ModrinthClient,
    target: &ResolutionTarget,
) -> Result<HashMap<String, ModrinthVersion>> {
    let mut versions = HashMap::new();

    for selected in selected_mods {
        if !matches!(selected.source, ModSource::Modrinth) {
            continue;
        }
        if versions.contains_key(&selected.mod_id) {
            continue;
        }
        match client
            .fetch_project_versions(&selected.mod_id, target)
            .await
        {
            Ok(candidate_versions) => {
                if let Some(version) = select_preferred_version(candidate_versions) {
                    versions.insert(selected.mod_id.clone(), version);
                }
            }
            Err(err) => {
                let _ = emit_log(
                    app_handle,
                    ProcessLogStream::Stderr,
                    format!(
                        "[Launch] skipping mod '{}': failed to query Modrinth ({:#})",
                        selected.mod_id, err
                    ),
                );
            }
        }
    }

    Ok(versions)
}

/// Selected Modrinth mods split by whether `mod_cache` knows a sha1 for them on
/// this target. One entry per mod id, in resolution order.
#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct CachedHashSplit {
    /// `(mod_id, lowercased sha1)` — the bulk path.
    pub(super) hashed: Vec<(String, String)>,
    /// No row for this target, or a row whose `file_hash` is NULL — the
    /// per-project path.
    pub(super) unhashed: Vec<String>,
}

/// What the bulk stage answered, and what it left to the per-project stage.
pub(super) struct HashStageOutcome {
    pub(super) resolved: HashMap<String, ModrinthVersion>,
    pub(super) per_project_mod_ids: Vec<String>,
    /// Mod ids that had a cached hash the bulk response omitted, and are
    /// therefore retried per project (D22). Subset of `per_project_mod_ids`.
    pub(super) omitted_mod_ids: Vec<String>,
    /// Set when the bulk call itself failed and its whole set fell back.
    pub(super) bulk_error: Option<String>,
}

pub(super) fn split_selected_by_cached_hash(
    selected_mods: &[SelectedMod],
    hash_by_mod_id: &HashMap<String, String>,
) -> CachedHashSplit {
    let mut split = CachedHashSplit::default();
    let mut seen = HashSet::new();

    for selected in selected_mods {
        if !matches!(selected.source, ModSource::Modrinth) {
            continue;
        }
        if !seen.insert(selected.mod_id.as_str()) {
            continue;
        }

        match hash_by_mod_id.get(&selected.mod_id) {
            Some(hash) => split.hashed.push((selected.mod_id.clone(), hash.clone())),
            None => split.unhashed.push(selected.mod_id.clone()),
        }
    }

    split
}

/// Re-key a hash-keyed bulk response by mod id.
///
/// A hash the response omitted yields no entry; the caller retries that mod per
/// project instead of declaring it unavailable, because the response cannot
/// distinguish "no version for this target" from "this hash is no longer
/// known". Two mod ids sharing one hash both resolve.
pub(super) fn versions_by_mod_id(
    hashed: &[(String, String)],
    versions_by_hash: &HashMap<String, ModrinthVersion>,
) -> HashMap<String, ModrinthVersion> {
    hashed
        .iter()
        .filter_map(|(mod_id, hash)| {
            versions_by_hash
                .get(hash)
                .map(|version| (mod_id.clone(), version.clone()))
        })
        .collect()
}

/// Bulk results win; the per-project stage only fills mod ids the bulk stage
/// left out. The two key sets are disjoint in the normal path and overlap only
/// after a failed bulk call sent its whole set to the fallback.
pub(super) fn merge_resolved_versions(
    mut bulk: HashMap<String, ModrinthVersion>,
    per_project: HashMap<String, ModrinthVersion>,
) -> HashMap<String, ModrinthVersion> {
    for (mod_id, version) in per_project {
        bulk.entry(mod_id).or_insert(version);
    }
    bulk
}

/// The bulk stage: one cascade for every mod that has a cached hash.
///
/// Generic over the request so both fallbacks are testable without network.
///
/// Two things go back to the per-project path. An **omitted hash** (D22): the
/// endpoint answers `200` with the hash simply absent, and that covers both "no
/// version exists for this target" — where the per-project path finds nothing
/// either, so the retry costs one wasted request — and "Modrinth no longer
/// knows this hash", where the per-project path still finds the version and the
/// mod keeps working exactly as it does today. A **failed call**: a per-project
/// error costs one mod today (`prefetch_compatible_versions_for_selected`),
/// while a bulk error would cost the whole modlist, so the set falls back
/// instead of shrinking the launch. Both are slow paths, and both are rare.
pub(super) async fn resolve_versions_for_cached_hashes<Fetch, Fut>(
    split: &CachedHashSplit,
    fetch_bulk: Fetch,
) -> HashStageOutcome
where
    Fetch: FnOnce(Vec<String>) -> Fut,
    Fut: Future<Output = Result<HashMap<String, ModrinthVersion>>>,
{
    if split.hashed.is_empty() {
        return HashStageOutcome {
            resolved: HashMap::new(),
            per_project_mod_ids: split.unhashed.clone(),
            omitted_mod_ids: Vec::new(),
            bulk_error: None,
        };
    }

    let hashes = split
        .hashed
        .iter()
        .map(|(_, hash)| hash.clone())
        .collect::<Vec<_>>();

    match fetch_bulk(hashes).await {
        Ok(versions_by_hash) => {
            let resolved = versions_by_mod_id(&split.hashed, &versions_by_hash);
            let omitted_mod_ids = split
                .hashed
                .iter()
                .map(|(mod_id, _)| mod_id)
                .filter(|mod_id| !resolved.contains_key(mod_id.as_str()))
                .cloned()
                .collect::<Vec<_>>();

            let mut per_project_mod_ids = omitted_mod_ids.clone();
            per_project_mod_ids.extend(split.unhashed.iter().cloned());

            HashStageOutcome {
                resolved,
                per_project_mod_ids,
                omitted_mod_ids,
                bulk_error: None,
            }
        }
        Err(error) => {
            let mut per_project_mod_ids = split
                .hashed
                .iter()
                .map(|(mod_id, _)| mod_id.clone())
                .collect::<Vec<_>>();
            per_project_mod_ids.extend(split.unhashed.iter().cloned());

            HashStageOutcome {
                resolved: HashMap::new(),
                per_project_mod_ids,
                omitted_mod_ids: Vec::new(),
                bulk_error: Some(format!("{error:#}")),
            }
        }
    }
}

/// Same contract as `prefetch_compatible_versions_for_selected` — mod id →
/// preferred `ModrinthVersion` — reached through `mod_cache` + the bulk
/// endpoint, falling back to one request per mod only for the mods no cached
/// hash covers.
pub(super) async fn resolve_compatible_versions_hybrid(
    app_handle: &tauri::AppHandle,
    launcher_paths: &LauncherPaths,
    http_client: &reqwest::Client,
    selected_mods: &[SelectedMod],
    client: &ModrinthClient,
    target: &ResolutionTarget,
) -> Result<HashMap<String, ModrinthVersion>> {
    let hash_by_mod_id =
        load_cached_file_hashes_for_selected(launcher_paths, selected_mods, target)?;
    let split = split_selected_by_cached_hash(selected_mods, &hash_by_mod_id);
    let outcome = resolve_versions_for_cached_hashes(&split, |hashes| async move {
        client.fetch_latest_versions_by_hash(&hashes, target).await
    })
    .await;

    if let Some(error) = &outcome.bulk_error {
        let _ = emit_log(
            app_handle,
            ProcessLogStream::Stderr,
            format!(
                "[Launch] bulk version lookup failed for {} cached hashes; falling back to one request per mod ({error})",
                split.hashed.len()
            ),
        );
    } else if !outcome.omitted_mod_ids.is_empty() {
        // The endpoint reports neither "unknown hash" nor "no version for this
        // target": both are omissions. Naming them is the only diagnostic
        // available for a case that should not happen (D23).
        let _ = emit_log(
            app_handle,
            ProcessLogStream::Stderr,
            format!(
                "[Launch] bulk version lookup omitted {}; retrying one request per mod",
                outcome.omitted_mod_ids.join(", ")
            ),
        );
    }

    let per_project_mod_ids = outcome
        .per_project_mod_ids
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let per_project_selection = selected_mods
        .iter()
        .filter(|selected| per_project_mod_ids.contains(selected.mod_id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let per_project = prefetch_compatible_versions_for_selected(
        app_handle,
        launcher_paths,
        http_client,
        &per_project_selection,
        client,
        target,
    )
    .await?;

    Ok(merge_resolved_versions(outcome.resolved, per_project))
}

pub(super) async fn prefetch_ranked_versions_for_selected(
    app_handle: &tauri::AppHandle,
    selected_mods: &[SelectedMod],
    client: &ModrinthClient,
    target: &ResolutionTarget,
) -> Result<HashMap<String, Vec<ModrinthVersion>>> {
    let mut versions = HashMap::new();

    for selected in selected_mods {
        if !matches!(selected.source, ModSource::Modrinth)
            || versions.contains_key(&selected.mod_id)
        {
            continue;
        }

        match client
            .fetch_project_versions(&selected.mod_id, target)
            .await
        {
            Ok(mut candidate_versions) => {
                crate::modrinth::sort_versions_by_target_preference(
                    &mut candidate_versions,
                    target,
                );
                if !candidate_versions.is_empty() {
                    versions.insert(selected.mod_id.clone(), candidate_versions);
                }
            }
            Err(err) => {
                let _ = emit_log(
                    app_handle,
                    ProcessLogStream::Stderr,
                    format!(
                        "[Launch] skipping mod '{}': failed to query Modrinth ({:#})",
                        selected.mod_id, err
                    ),
                );
            }
        }
    }

    Ok(versions)
}

pub(super) async fn select_latest_launch_compatible_version(
    app_handle: &tauri::AppHandle,
    launcher_paths: &LauncherPaths,
    http_client: &reqwest::Client,
    client: &ModrinthClient,
    project_id_or_slug: &str,
    target: &ResolutionTarget,
) -> Result<Option<ModrinthVersion>> {
    Ok(select_launch_compatible_versions(
        app_handle,
        launcher_paths,
        http_client,
        client,
        project_id_or_slug,
        target,
    )
    .await?
    .into_iter()
    .next())
}

pub(super) async fn select_launch_compatible_versions(
    app_handle: &tauri::AppHandle,
    launcher_paths: &LauncherPaths,
    http_client: &reqwest::Client,
    client: &ModrinthClient,
    project_id_or_slug: &str,
    target: &ResolutionTarget,
) -> Result<Vec<ModrinthVersion>> {
    let mut versions = client
        .fetch_project_versions(project_id_or_slug, target)
        .await?;
    crate::modrinth::sort_versions_by_target_preference(&mut versions, target);

    let mut compatible_versions = Vec::new();
    for version in versions {
        let jar_path = ensure_remote_version_cached(http_client, launcher_paths, &version, target)
            .await
            .with_context(|| {
                format!(
                    "failed to cache '{}' candidate version '{}'",
                    project_id_or_slug, version.id
                )
            })?;
        let requirements = read_embedded_fabric_requirements(&jar_path)?;
        if requirements.entries.is_empty()
            || embedded_minecraft_requirements_match(&requirements, target)
        {
            compatible_versions.push(version);
            continue;
        }

        let _ = emit_log(
            app_handle,
            ProcessLogStream::Stdout,
            format!(
                "[Launch] skipping remote version '{}' for '{}': embedded metadata is incompatible with {} / {}",
                version.version_number,
                project_id_or_slug,
                target.minecraft_version,
                target.mod_loader.as_modrinth_loader()
            ),
        );
    }

    Ok(compatible_versions)
}

pub(super) fn log_resolution(
    app_handle: &tauri::AppHandle,
    resolution: &ResolutionResult,
) -> Result<()> {
    for rule in &resolution.resolved_rules {
        match &rule.outcome {
            RuleOutcome::Resolved { resolved_id } => emit_log(
                app_handle,
                ProcessLogStream::Stdout,
                format!("[Resolver] {} -> {}", rule.mod_id, resolved_id,),
            )?,
            RuleOutcome::Unresolved { reason } => emit_log(
                app_handle,
                ProcessLogStream::Stdout,
                format!(
                    "[Resolver] {} unresolved ({})",
                    rule.mod_id,
                    describe_failure_reason(*reason)
                ),
            )?,
        }
    }

    Ok(())
}

pub(super) fn describe_failure_reason(reason: FailureReason) -> &'static str {
    match reason {
        FailureReason::ExcludedByActiveMod => "excluded by already-selected mods",
        FailureReason::RequiredModMissing => "required mod not active",
        FailureReason::IncompatibleVersion => "incompatible version/loader",
        FailureReason::NoOptionAvailable => "no compatible option remained",
    }
}

/// Collect mods for launch.  When a rule resolves (primary or alternative),
/// include whichever mod was selected by the resolver.
pub(super) fn collect_selected_mods(
    modlist: &ModList,
    resolution: &ResolutionResult,
    _target: &ResolutionTarget,
) -> Vec<SelectedMod> {
    let mut selected = Vec::new();

    for (i, resolved) in resolution.resolved_rules.iter().enumerate() {
        let Some(top_rule) = modlist.rules.get(i) else {
            continue;
        };

        if let RuleOutcome::Resolved { resolved_id } = &resolved.outcome {
            // Find the actual rule (primary or any nested alternative) to get its source.
            let rule = modlist.find_rule(resolved_id).unwrap_or(top_rule);
            selected.push(SelectedMod {
                mod_id: resolved_id.clone(),
                source: rule.source.clone(),
            });
        }
    }

    selected
}

pub(super) fn remote_artifact_project_id(artifact: &RemoteArtifact) -> &str {
    match artifact {
        RemoteArtifact::Live(version) => &version.project_id,
        RemoteArtifact::Cached(record) => &record.modrinth_project_id,
    }
}

pub(super) fn split_remote_artifacts(
    artifacts: &[RemoteArtifact],
) -> (Vec<ModrinthVersion>, Vec<ModCacheRecord>) {
    let mut live_versions = Vec::new();
    let mut cached_records = Vec::new();
    let mut seen_version_ids = HashSet::new();

    for artifact in artifacts {
        match artifact {
            RemoteArtifact::Live(version) => {
                if seen_version_ids.insert(version.id.clone()) {
                    live_versions.push(version.clone());
                }
            }
            RemoteArtifact::Cached(record) => {
                if seen_version_ids.insert(record.modrinth_version_id.clone()) {
                    cached_records.push(record.clone());
                }
            }
        }
    }

    (live_versions, cached_records)
}

pub(super) async fn resolve_selected_remote_artifacts(
    launcher_paths: &LauncherPaths,
    selected_mods: &[SelectedMod],
    target: &ResolutionTarget,
) -> Result<HashMap<String, RemoteArtifact>> {
    let mut artifacts = HashMap::new();

    for selected in selected_mods {
        if !matches!(selected.source, ModSource::Modrinth)
            || artifacts.contains_key(&selected.mod_id)
        {
            continue;
        }

        if let Some(record) =
            load_cached_mod_record_for_target(launcher_paths, &selected.mod_id, target)?
        {
            artifacts.insert(selected.mod_id.clone(), RemoteArtifact::Cached(record));
        }
    }

    Ok(artifacts)
}

pub(super) fn alt_viable_for_launch(
    rule: &Rule,
    active_mods: &HashSet<String>,
    target: &ResolutionTarget,
) -> bool {
    if !rule.enabled {
        return false;
    }
    if rule.exclude_if.iter().any(|id| active_mods.contains(id)) {
        return false;
    }
    if rule.requires.iter().any(|id| !active_mods.contains(id)) {
        return false;
    }
    !version_rules_conflict(&rule.version_rules, target)
}

pub(super) fn collect_resolved_parent_versions(
    selected_mods: &[SelectedMod],
    compatible_versions: &HashMap<String, Vec<ModrinthVersion>>,
) -> Vec<ModrinthVersion> {
    let mut versions = Vec::new();
    let mut seen_projects = HashSet::new();

    for selected in selected_mods {
        if !matches!(selected.source, ModSource::Modrinth)
            || !seen_projects.insert(selected.mod_id.clone())
        {
            continue;
        }

        // Skip mods that have no compatible version on Modrinth for this
        // target — they were resolved by local rules but simply don't have
        // a release for this MC version + loader.
        if let Some(version) = compatible_versions
            .get(&selected.mod_id)
            .and_then(|versions| versions.first())
        {
            versions.push(version.clone());
        }
    }

    versions
}

pub(super) fn collect_top_level_version_candidates(
    selected_mods: &[SelectedMod],
    compatible_versions: &HashMap<String, Vec<ModrinthVersion>>,
) -> Vec<TopLevelVersionCandidates> {
    let mut top_level_candidates = Vec::new();
    let mut seen_selected_mod_ids = HashSet::new();

    for selected in selected_mods {
        if !matches!(selected.source, ModSource::Modrinth)
            || !seen_selected_mod_ids.insert(selected.mod_id.clone())
        {
            continue;
        }

        let Some(candidates) = compatible_versions.get(&selected.mod_id) else {
            continue;
        };
        let Some(first_candidate) = candidates.first() else {
            continue;
        };

        top_level_candidates.push(TopLevelVersionCandidates {
            selected_mod_id: selected.mod_id.clone(),
            project_id: first_candidate.project_id.clone(),
            candidates: candidates.clone(),
        });
    }

    top_level_candidates
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_version(id: &str, date_published: &str) -> ModrinthVersion {
        test_version_with_channel(id, date_published, "release")
    }

    fn test_version_with_channel(
        id: &str,
        date_published: &str,
        version_type: &str,
    ) -> ModrinthVersion {
        ModrinthVersion {
            id: id.into(),
            project_id: "example-project".into(),
            version_number: id.into(),
            name: id.into(),
            game_versions: vec!["26.1".into()],
            loaders: vec!["fabric".into()],
            version_type: version_type.into(),
            dependencies: Vec::new(),
            files: Vec::new(),
            date_published: date_published.into(),
        }
    }

    #[test]
    fn select_preferred_version_uses_release_before_newer_beta() {
        let release = test_version_with_channel(
            "iris-release",
            "2025-02-20T00:00:00Z",
            "release",
        );
        let beta =
            test_version_with_channel("iris-beta", "2026-06-13T00:00:00Z", "beta");

        assert_eq!(
            select_preferred_version(vec![beta, release.clone()]),
            Some(release)
        );
    }

    #[test]
    fn select_preferred_version_uses_newest_alpha_when_only_alphas_exist() {
        let older = test_version_with_channel("c2me-older", "2025-01-01T00:00:00Z", "alpha");
        let newest =
            test_version_with_channel("c2me-newest", "2026-06-13T00:00:00Z", "alpha");

        assert_eq!(
            select_preferred_version(vec![newest.clone(), older]),
            Some(newest)
        );
    }

    #[test]
    fn select_preferred_version_uses_beta_before_alpha_without_release() {
        let beta = test_version_with_channel("beta", "2025-01-01T00:00:00Z", "beta");
        let alpha = test_version_with_channel("alpha", "2026-06-13T00:00:00Z", "alpha");

        assert_eq!(
            select_preferred_version(vec![alpha, beta.clone()]),
            Some(beta)
        );
    }

    #[test]
    fn select_preferred_version_is_date_order_independent_within_channel() {
        let january = test_version("january", "2024-01-01T00:00:00Z");
        let june = test_version("june", "2024-06-01T00:00:00Z");
        let march = test_version("march", "2024-03-01T00:00:00Z");

        let first =
            select_preferred_version(vec![january.clone(), june.clone(), march.clone()]);
        let second = select_preferred_version(vec![march, january, june.clone()]);

        assert_eq!(first, Some(june.clone()));
        assert_eq!(second, Some(june));
    }

    fn modrinth_mod(mod_id: &str) -> SelectedMod {
        SelectedMod {
            mod_id: mod_id.into(),
            source: ModSource::Modrinth,
        }
    }

    fn local_mod(mod_id: &str) -> SelectedMod {
        SelectedMod {
            mod_id: mod_id.into(),
            source: ModSource::Local,
        }
    }

    fn cached_hashes(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(mod_id, hash)| ((*mod_id).to_string(), (*hash).to_string()))
            .collect()
    }

    #[test]
    fn split_sends_every_mod_with_a_cached_hash_to_the_bulk_path() {
        let selected = vec![modrinth_mod("sodium"), modrinth_mod("lithium")];
        let split = split_selected_by_cached_hash(
            &selected,
            &cached_hashes(&[("sodium", "aaa1"), ("lithium", "bbb2")]),
        );

        assert_eq!(
            split.hashed,
            vec![
                ("sodium".to_string(), "aaa1".to_string()),
                ("lithium".to_string(), "bbb2".to_string()),
            ]
        );
        assert!(split.unhashed.is_empty());
    }

    #[test]
    fn split_sends_every_mod_to_the_per_project_path_without_cached_hashes() {
        let selected = vec![modrinth_mod("polytone"), modrinth_mod("connector-extras")];
        let split = split_selected_by_cached_hash(&selected, &HashMap::new());

        assert!(split.hashed.is_empty());
        assert_eq!(split.unhashed, vec!["polytone", "connector-extras"]);
    }

    #[test]
    fn split_separates_mixed_coverage_and_ignores_local_and_repeated_mods() {
        let selected = vec![
            modrinth_mod("sodium"),
            local_mod("my-own.jar"),
            modrinth_mod("polytone"),
            modrinth_mod("sodium"),
        ];
        let split =
            split_selected_by_cached_hash(&selected, &cached_hashes(&[("sodium", "aaa1")]));

        assert_eq!(split.hashed, vec![("sodium".to_string(), "aaa1".to_string())]);
        assert_eq!(split.unhashed, vec!["polytone"]);
    }

    #[tokio::test]
    async fn a_hash_omitted_by_the_bulk_response_is_retried_per_project() {
        let selected = vec![
            modrinth_mod("sodium"),
            modrinth_mod("lithium"),
            modrinth_mod("polytone"),
        ];
        let split = split_selected_by_cached_hash(
            &selected,
            &cached_hashes(&[("sodium", "aaa1"), ("lithium", "bbb2")]),
        );

        let outcome = resolve_versions_for_cached_hashes(&split, |_hashes| async {
            Ok(HashMap::from([(
                "aaa1".to_string(),
                test_version("sodium-version", "2026-01-01T00:00:00Z"),
            )]))
        })
        .await;

        assert_eq!(outcome.resolved.len(), 1);
        assert!(outcome.resolved.contains_key("sodium"));
        // An omission cannot be told apart from "no version for this target",
        // so the mod goes back on the per-project path instead of being
        // declared unavailable — ahead of the mods that never had a hash.
        assert_eq!(outcome.omitted_mod_ids, vec!["lithium"]);
        assert_eq!(outcome.per_project_mod_ids, vec!["lithium", "polytone"]);
        assert!(outcome.bulk_error.is_none());

        // ...and it resolves when the per-project stage finds the version, the
        // case where an unknown hash would otherwise silently drop a mod that
        // works today.
        let from_per_project = test_version("lithium-version", "2026-01-01T00:00:00Z");
        let merged = merge_resolved_versions(
            outcome.resolved,
            HashMap::from([("lithium".to_string(), from_per_project.clone())]),
        );
        assert_eq!(merged.get("lithium"), Some(&from_per_project));
        assert_eq!(merged.len(), 2);
    }

    #[tokio::test]
    async fn two_mod_ids_sharing_a_cached_hash_both_resolve() {
        let selected = vec![modrinth_mod("sodium"), modrinth_mod("AANobbMI")];
        let split = split_selected_by_cached_hash(
            &selected,
            &cached_hashes(&[("sodium", "aaa1"), ("AANobbMI", "aaa1")]),
        );

        let outcome = resolve_versions_for_cached_hashes(&split, |hashes| async move {
            assert_eq!(hashes, vec!["aaa1".to_string(), "aaa1".to_string()]);
            Ok(HashMap::from([(
                "aaa1".to_string(),
                test_version("sodium-version", "2026-01-01T00:00:00Z"),
            )]))
        })
        .await;

        assert_eq!(outcome.resolved.len(), 2);
        assert_eq!(
            outcome.resolved.get("sodium"),
            outcome.resolved.get("AANobbMI")
        );
    }

    #[tokio::test]
    async fn a_failed_bulk_call_sends_its_whole_set_to_the_per_project_path() {
        let selected = vec![
            modrinth_mod("sodium"),
            modrinth_mod("lithium"),
            modrinth_mod("polytone"),
        ];
        let split = split_selected_by_cached_hash(
            &selected,
            &cached_hashes(&[("sodium", "aaa1"), ("lithium", "bbb2")]),
        );

        let outcome = resolve_versions_for_cached_hashes(&split, |_hashes| async {
            Err(anyhow::anyhow!("connection reset"))
        })
        .await;

        assert!(outcome.resolved.is_empty());
        assert_eq!(
            outcome.per_project_mod_ids,
            vec!["sodium", "lithium", "polytone"]
        );
        assert_eq!(outcome.bulk_error.as_deref(), Some("connection reset"));
    }

    #[tokio::test]
    async fn an_empty_bulk_set_costs_no_request() {
        let selected = vec![modrinth_mod("polytone")];
        let split = split_selected_by_cached_hash(&selected, &HashMap::new());

        let outcome = resolve_versions_for_cached_hashes(&split, |_hashes| async {
            panic!("the bulk endpoint must not be called without hashes");
        })
        .await;

        assert!(outcome.resolved.is_empty());
        assert_eq!(outcome.per_project_mod_ids, vec!["polytone"]);
    }

    #[test]
    fn per_project_results_only_fill_mod_ids_the_bulk_left_out() {
        let from_bulk = test_version("from-bulk", "2026-01-01T00:00:00Z");
        let from_per_project = test_version("from-per-project", "2026-01-01T00:00:00Z");

        let merged = merge_resolved_versions(
            HashMap::from([("sodium".to_string(), from_bulk.clone())]),
            HashMap::from([
                ("sodium".to_string(), from_per_project.clone()),
                ("polytone".to_string(), from_per_project.clone()),
            ]),
        );

        assert_eq!(merged.get("sodium"), Some(&from_bulk));
        assert_eq!(merged.get("polytone"), Some(&from_per_project));
        assert_eq!(merged.len(), 2);
    }
}
