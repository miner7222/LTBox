//! GitHub API client — releases, workflow runs, artifacts.
//!
//! Blocking ureq, no auth. Process-wide 5-minute response cache keyed on URL,
//! storing raw JSON bodies so each caller reparses into its own type.

use std::sync::Arc;
use std::time::Duration;

use moka::sync::Cache;
use serde::Deserialize;

use crate::error::{LtboxError, Result};

const API_BASE: &str = "https://api.github.com";

static RESPONSE_CACHE: std::sync::LazyLock<Cache<String, Arc<String>>> =
    std::sync::LazyLock::new(|| {
        Cache::builder()
            .time_to_live(Duration::from_secs(5 * 60))
            .max_capacity(128)
            .build()
    });

/// Validate before caching and coalesce concurrent misses for the same URL.
/// Bypass requests neither read nor populate the shared cache.
fn cached_json<T: serde::de::DeserializeOwned>(
    cache: &Cache<String, Arc<String>>,
    url: &str,
    bypass: bool,
    load: impl FnOnce() -> Result<String>,
) -> Result<T> {
    let parse = |body: &str| {
        serde_json::from_str::<T>(body)
            .map_err(|error| LtboxError::Download(format!("JSON parse error: {error}")))
    };
    if bypass {
        return parse(&load()?);
    }
    let body = cache
        .try_get_with(url.to_owned(), || {
            let body = load()?;
            parse(&body)?;
            Ok::<_, LtboxError>(Arc::new(body))
        })
        .map_err(|error| {
            Arc::try_unwrap(error).unwrap_or_else(|error| LtboxError::Other(error.to_string()))
        })?;
    parse(&body)
}

pub struct GitHubClient {
    owner_repo: String,
    agent: ureq::Agent,
    bypass_cache: bool,
}

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    assets: Vec<Asset>,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    published_at: Option<String>,
    #[serde(default)]
    id: u64,
}

/// Published release shown in the root version picker, newest first.
#[derive(Debug, Clone)]
pub struct PublishedRelease {
    pub tag: String,
    pub run_id: Option<u64>,
    pub prerelease: bool,
    pub published_at: String,
}

fn recent_releases(mut releases: Vec<Release>) -> Vec<PublishedRelease> {
    releases.retain(|r| !r.draft && r.published_at.is_some());
    // GitHub timestamps are UTC ISO-8601. IDs break equal-date ties without
    // relying on API response order or assuming tags are semantic versions.
    releases.sort_by(|a, b| b.published_at.cmp(&a.published_at).then(b.id.cmp(&a.id)));
    releases
        .into_iter()
        .take(5)
        .map(|r| PublishedRelease {
            tag: r.tag_name,
            run_id: None,
            prerelease: r.prerelease,
            published_at: r.published_at.unwrap_or_default(),
        })
        .collect()
}

/// Slim public payload for the in-app update banner — see
/// [`GitHubClient::latest_stable_release`].
#[derive(Debug, Clone)]
pub struct StableRelease {
    pub tag: String,
    pub html_url: String,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Deserialize)]
struct WorkflowRunsResponse {
    workflow_runs: Vec<WorkflowRun>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowRun {
    pub id: u64,
    pub created_at: String,
    pub head_branch: Option<String>,
    pub path: Option<String>,
    #[serde(default)]
    pub conclusion: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ArtifactsResponse {
    artifacts: Vec<WorkflowArtifact>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkflowArtifact {
    pub name: String,
    #[serde(default)]
    pub digest: Option<String>,
    #[serde(default)]
    pub expired: bool,
    pub created_at: String,
    pub expires_at: String,
}

impl GitHubClient {
    pub fn new(owner_repo: &str) -> Result<Self> {
        let agent = crate::downloader::build_agent();
        Ok(Self {
            owner_repo: owner_repo.to_string(),
            agent,
            bypass_cache: false,
        })
    }

    /// Fetch fresh metadata for an explicit version-picker query or retry.
    pub fn without_cache(mut self) -> Self {
        self.bypass_cache = true;
        self
    }

    /// Parse "github.com/owner/repo" or "owner/repo" into "owner/repo".
    pub fn from_url(url: &str) -> Result<Self> {
        let repo = url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .trim_start_matches("github.com/")
            .trim_end_matches('/')
            .to_string();
        if repo.matches('/').count() != 1 {
            return Err(LtboxError::Config(format!("Invalid repo: {url}")));
        }
        Self::new(&repo)
    }

    fn get_json<T: serde::de::DeserializeOwned>(&self, endpoint: &str) -> Result<T> {
        let url = format!("{API_BASE}/repos/{}{endpoint}", self.owner_repo);
        cached_json(&RESPONSE_CACHE, &url, self.bypass_cache, || {
            self.fetch_json(&url)
        })
    }

    fn fetch_json(&self, url: &str) -> Result<String> {
        // Retry transport errors and 5xx; do not retry 4xx.
        let mut last_err: Option<LtboxError> = None;
        for attempt in 0..3_u32 {
            if attempt > 0 {
                let delay_ms = 100u64 * 4u64.pow(attempt - 1);
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            }
            let mut request = self.agent.get(url);
            if self.bypass_cache {
                request = request.header("Cache-Control", "no-cache");
            }
            // Optional CI token is sent only to api.github.com, never asset hosts.
            if let Ok(token) = std::env::var("LTBOX_GITHUB_TOKEN")
                && !token.is_empty()
            {
                request = request.header("Authorization", &format!("Bearer {token}"));
            }
            match request.call() {
                Ok(mut resp) => {
                    // GitHub release + tag JSON payloads we hit are small;
                    // `read_to_string` is bounded by ureq's default body-size
                    // limit (these endpoints never approach it).
                    let body = resp
                        .body_mut()
                        .read_to_string()
                        .map_err(|e| LtboxError::Download(format!("read body: {e}")))?;
                    return Ok(body);
                }
                Err(ureq::Error::StatusCode(code)) => {
                    if (400..500).contains(&code) {
                        return Err(LtboxError::Download(format!("GitHub API {code}: {url}")));
                    }
                    last_err = Some(LtboxError::Download(format!("GitHub API {code}: {url}")));
                }
                Err(e) => {
                    last_err = Some(LtboxError::Download(format!("Request failed: {e}")));
                }
            }
        }
        Err(last_err.unwrap_or_else(|| {
            LtboxError::Download("GitHub API exhausted retries with no recorded error".into())
        }))
    }

    /// Newest non-draft, non-prerelease release on the repo, or `Ok(None)`
    /// when the repo has nothing stable published yet.
    ///
    /// `/releases/latest` would already filter out prereleases on GitHub's
    /// side, but it 404s when **every** published release on the repo is a
    /// prerelease — a real state for this project during alpha/beta. Walk
    /// `/releases?per_page=100` instead and pick the highest semver among
    /// the `prerelease == false && draft == false` rows so the caller
    /// always gets a defined answer (`None` = no stable yet, `Some(...)`
    /// = the candidate to compare against the running build).
    pub fn latest_stable_release(&self) -> Result<Option<StableRelease>> {
        let releases: Vec<Release> = self.get_json("/releases?per_page=100")?;
        let mut best: Option<(semver::Version, StableRelease)> = None;
        for r in releases {
            if r.draft || r.prerelease {
                continue;
            }
            // Tags are conventionally `vX.Y.Z`; semver wants the bare
            // `X.Y.Z` form. Skip tags we can't parse — better to ignore a
            // weird tag than to call it "the latest" and ship a bad
            // banner pointing at it.
            let stripped = r.tag_name.trim_start_matches('v');
            let Ok(ver) = semver::Version::parse(stripped) else {
                continue;
            };
            let candidate = StableRelease {
                tag: r.tag_name.clone(),
                html_url: r.html_url.clone(),
            };
            match best.as_ref() {
                Some((cur, _)) if &ver <= cur => {}
                _ => best = Some((ver, candidate)),
            }
        }
        Ok(best.map(|(_, r)| r))
    }

    /// Latest release: `(tag, [(asset_name, browser_download_url)])`.
    pub fn latest_release_assets(&self) -> Result<(String, Vec<(String, String)>)> {
        let release: Release = self.get_json("/releases/latest")?;
        let tag = release.tag_name;
        let assets = release
            .assets
            .into_iter()
            .map(|a| (a.name, a.browser_download_url))
            .collect();
        Ok((tag, assets))
    }

    /// Latest five published releases, including prereleases, by publication date.
    pub fn recent_published_releases(&self) -> Result<Vec<PublishedRelease>> {
        let mut releases = Vec::new();
        let mut page = 1;
        loop {
            let batch: Vec<Release> =
                self.get_json(&format!("/releases?per_page=100&page={page}"))?;
            let finished = batch.len() < 100;
            releases.extend(batch);
            if finished {
                break;
            }
            page += 1;
        }
        Ok(recent_releases(releases))
    }

    /// Resolve an explicitly selected release, falling back only when unselected.
    pub fn selected_release_assets(
        &self,
        tag: Option<&str>,
    ) -> Result<(String, Vec<(String, String)>)> {
        match tag {
            Some(tag) => Ok((tag.to_owned(), self.release_by_tag(tag)?)),
            None => self.latest_release_assets(),
        }
    }

    /// First latest-release asset whose name matches `predicate` → `(name, url)`.
    pub fn latest_release_asset_where(
        &self,
        predicate: impl Fn(&str) -> bool,
    ) -> Result<(String, String)> {
        let (_tag, assets) = self.latest_release_assets()?;
        assets
            .into_iter()
            .find(|(name, _)| predicate(name))
            .ok_or_else(|| {
                LtboxError::Download(format!(
                    "No matching asset on latest release of {}",
                    self.owner_repo
                ))
            })
    }

    pub fn release_by_tag(&self, tag: &str) -> Result<Vec<(String, String)>> {
        let encoded = percent_encode(tag);
        let release: Release = self.get_json(&format!("/releases/tags/{encoded}"))?;
        Ok(release
            .assets
            .into_iter()
            .map(|a| (a.name, a.browser_download_url))
            .collect())
    }

    /// Successful runs of `workflow_file` pushed for `tag`, newest first.
    ///
    /// Scoped to one workflow: a repository-wide tag query also returns lint
    /// and other artifact-less runs, and GitHub's order among the runs a
    /// single tag push starts is not meaningful.
    pub fn workflow_runs_for_tag(&self, workflow_file: &str, tag: &str) -> Result<Vec<u64>> {
        let encoded = percent_encode(tag);
        let resp: WorkflowRunsResponse = self.get_json(&format!(
            "/actions/workflows/{workflow_file}/runs?per_page=30&status=success&branch={encoded}"
        ))?;
        if !resp.workflow_runs.is_empty() {
            return Ok(resp.workflow_runs.into_iter().map(|r| r.id).collect());
        }
        // The `status`/`branch` filters go through GitHub's run search, which
        // has returned nothing for a tag run that completed a week earlier
        // (KernelSU-Next v3.4.0). Check the unfiltered newest runs before
        // concluding the tag has none.
        let resp: WorkflowRunsResponse = self.get_json(&format!(
            "/actions/workflows/{workflow_file}/runs?per_page=100"
        ))?;
        Ok(successful_tag_runs(resp.workflow_runs, tag))
    }

    pub fn workflow_artifacts(&self, run_id: u64) -> Result<Vec<String>> {
        Ok(self
            .workflow_artifact_details(run_id)?
            .into_iter()
            .map(|a| a.name)
            .collect())
    }

    /// Workflow artifacts with the optional digest reported by GitHub.
    pub fn workflow_artifact_details(&self, run_id: u64) -> Result<Vec<WorkflowArtifact>> {
        let resp: ArtifactsResponse =
            self.get_json(&format!("/actions/runs/{run_id}/artifacts?per_page=100"))?;
        let now = chrono::Utc::now();
        Ok(resp
            .artifacts
            .into_iter()
            .filter(|a| artifact_available(a, now))
            .collect())
    }

    pub fn workflow_run_matches(
        &self,
        run_id: u64,
        workflow_file: &str,
        branch: Option<&str>,
    ) -> Result<bool> {
        let run: WorkflowRun = self.get_json(&format!("/actions/runs/{run_id}"))?;
        if let Some(b) = branch
            && run.head_branch.as_deref() != Some(b)
        {
            return Ok(false);
        }
        if !workflow_file.is_empty() {
            let expected = normalize_workflow_path(workflow_file);
            let actual = run
                .path
                .as_deref()
                .map(normalize_workflow_path)
                .unwrap_or_default();
            if actual != expected {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Successful runs less than 90 days old that still have downloadable artifacts.
    /// A shorter upstream retention period is honored as well.
    pub fn recent_available_runs(
        &self,
        workflow_file: &str,
        branch: &str,
    ) -> Result<Vec<PublishedRelease>> {
        self.recent_available_runs_matching(workflow_file, branch, |_| true)
    }

    /// Filter retained artifacts by the provider's payload requirements.
    pub fn recent_available_runs_matching(
        &self,
        workflow_file: &str,
        branch: &str,
        accepts: impl Fn(&[WorkflowArtifact]) -> bool,
    ) -> Result<Vec<PublishedRelease>> {
        let now = chrono::Utc::now();
        let mut choices = Vec::new();
        let mut page = 1;
        loop {
            let resp: WorkflowRunsResponse = self.get_json(&format!(
                "/actions/workflows/{workflow_file}/runs?status=success&per_page=100&page={page}&branch={branch}"
            ))?;
            let finished = resp.workflow_runs.len() < 100
                || resp.workflow_runs.last().is_some_and(|r| {
                    chrono::DateTime::parse_from_rfc3339(&r.created_at).is_ok_and(|date| {
                        now.signed_duration_since(date) >= chrono::Duration::days(90)
                    })
                });
            let mut runs = resp.workflow_runs;
            runs.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(b.id.cmp(&a.id)));
            for run in runs {
                if !recent_timestamp(&run.created_at, now) {
                    continue;
                }
                let artifacts = self.workflow_artifact_details(run.id)?;
                if artifacts.is_empty() || !accepts(&artifacts) {
                    continue;
                }
                choices.push(PublishedRelease {
                    tag: run.id.to_string(),
                    run_id: Some(run.id),
                    prerelease: false,
                    published_at: run.created_at,
                });
                if choices.len() == 5 {
                    return Ok(choices);
                }
            }
            if finished {
                break;
            }
            page += 1;
        }
        Ok(choices)
    }

    pub fn latest_successful_run(
        &self,
        workflow_file: &str,
        branch: Option<&str>,
    ) -> Result<Option<u64>> {
        let mut endpoint =
            format!("/actions/workflows/{workflow_file}/runs?status=success&per_page=20");
        if let Some(b) = branch {
            endpoint.push_str(&format!("&branch={b}"));
        }
        let resp: WorkflowRunsResponse = self.get_json(&endpoint)?;
        Ok(resp.workflow_runs.first().map(|r| r.id))
    }
}

/// Percent-encode a tag for a URL path segment or query value.
fn percent_encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn recent_timestamp(value: &str, now: chrono::DateTime<chrono::Utc>) -> bool {
    chrono::DateTime::parse_from_rfc3339(value).is_ok_and(|created| {
        let age = now.signed_duration_since(created);
        age >= chrono::Duration::zero() && age < chrono::Duration::days(90)
    })
}

fn artifact_available(artifact: &WorkflowArtifact, now: chrono::DateTime<chrono::Utc>) -> bool {
    !artifact.expired
        && recent_timestamp(&artifact.created_at, now)
        && chrono::DateTime::parse_from_rfc3339(&artifact.expires_at)
            .is_ok_and(|expiry| expiry > now)
}

fn normalize_workflow_path(path: &str) -> String {
    path.trim_start_matches(".github/workflows/")
        .trim_start_matches(".github/workflows\\")
        .to_lowercase()
}

/// Successful runs pushed for `tag`, keeping the listing's newest-first order.
fn successful_tag_runs(runs: Vec<WorkflowRun>, tag: &str) -> Vec<u64> {
    runs.into_iter()
        .filter(|run| {
            run.head_branch.as_deref() == Some(tag) && run.conclusion.as_deref() == Some("success")
        })
        .map(|run| run.id)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unfiltered_tag_fallback_keeps_only_successful_runs_of_that_tag() {
        let runs: WorkflowRunsResponse = serde_json::from_str(
            r#"{"workflow_runs": [
                {"id": 5, "created_at": "", "head_branch": "v3.4.0", "conclusion": "failure"},
                {"id": 4, "created_at": "", "head_branch": "dev", "conclusion": "success"},
                {"id": 3, "created_at": "", "head_branch": "v3.4.0", "conclusion": "success"},
                {"id": 2, "created_at": "", "head_branch": "v3.4.0", "conclusion": null},
                {"id": 1, "created_at": "", "head_branch": "v3.4.0", "conclusion": "success"}
            ]}"#,
        )
        .unwrap();
        assert_eq!(successful_tag_runs(runs.workflow_runs, "v3.4.0"), [3, 1]);
    }

    #[test]
    fn cache_validates_json_and_keeps_bypass_requests_isolated() {
        let cache = Cache::new(4);
        assert!(cached_json::<Vec<u8>>(&cache, "a", false, || Ok("{}".into())).is_err());
        assert!(cache.get("a").is_none());
        assert!(
            cached_json::<Vec<u8>>(&cache, "a", false, || Err(LtboxError::Download(
                "offline".into()
            )))
            .is_err()
        );
        assert_eq!(
            cached_json::<Vec<u8>>(&cache, "a", false, || Ok("[1]".into())).unwrap(),
            vec![1]
        );
        assert_eq!(
            cached_json::<Vec<u8>>(&cache, "a", true, || Ok("[2]".into())).unwrap(),
            vec![2]
        );
        assert_eq!(
            cached_json::<Vec<u8>>(&cache, "a", false, || panic!("cached")).unwrap(),
            vec![1]
        );
        assert!(cached_json::<u8>(&cache, "a", false, || panic!("cached")).is_err());
    }

    #[test]
    fn concurrent_cache_misses_share_one_loader() {
        use std::sync::{
            Barrier,
            atomic::{AtomicUsize, Ordering},
        };
        let cache = Cache::new(4);
        let barrier = Barrier::new(8);
        let loads = AtomicUsize::new(0);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    barrier.wait();
                    let value = cached_json::<u8>(&cache, "same", false, || {
                        loads.fetch_add(1, Ordering::SeqCst);
                        std::thread::sleep(Duration::from_millis(20));
                        Ok("42".into())
                    })
                    .unwrap();
                    assert_eq!(value, 42);
                });
            }
        });
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn missing_retention_metadata_is_an_api_error_not_an_empty_list() {
        assert!(
            serde_json::from_str::<WorkflowRunsResponse>(
                r#"{"workflow_runs":[{"id":34813418845,"head_branch":"main"}]}"#
            )
            .is_err()
        );
        for dates in [
            r#""created_at":"2026-09-14T06:25:31Z""#,
            r#""expires_at":"2026-12-13T06:25:31Z""#,
        ] {
            let json = format!(r#"{{"artifacts":[{{"name":"manager",{dates}}}]}}"#);
            assert!(serde_json::from_str::<ArtifactsResponse>(&json).is_err());
        }
    }

    #[test]
    fn artifact_retention_rejects_90_day_boundary_and_early_expiry() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-09-14T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let mut artifact = WorkflowArtifact {
            name: "manager".into(),
            digest: None,
            expired: false,
            created_at: (now - chrono::Duration::days(90) + chrono::Duration::seconds(1))
                .to_rfc3339(),
            expires_at: (now + chrono::Duration::days(1)).to_rfc3339(),
        };
        assert!(artifact_available(&artifact, now));
        artifact.created_at = (now - chrono::Duration::days(90)).to_rfc3339();
        assert!(!artifact_available(&artifact, now));
        artifact.created_at = (now - chrono::Duration::days(1)).to_rfc3339();
        artifact.expired = true;
        assert!(!artifact_available(&artifact, now));
        artifact.expired = false;
        artifact.expires_at = now.to_rfc3339();
        assert!(!artifact_available(&artifact, now));
        assert!(!recent_timestamp("invalid", now));
    }

    #[test]
    fn release_picker_accepts_a_prerelease_only_repository() {
        let releases: Vec<Release> = serde_json::from_str(r#"[
            {"id": 1, "tag_name": "v4.2.0-rc1", "assets": [], "prerelease": true, "published_at": "2026-09-01T00:00:00Z"},
            {"id": 2, "tag_name": "v4.2.0-rc2", "assets": [], "prerelease": true, "published_at": "2026-09-02T00:00:00Z"}
        ]"#).unwrap();
        let choices = recent_releases(releases);
        assert_eq!(choices[0].tag, "v4.2.0-rc2");
        assert!(choices.iter().all(|release| release.prerelease));
    }

    #[test]
    fn recent_release_picker_excludes_drafts_and_keeps_latest_five_by_publish_time() {
        let json = r#"[
            {"id": 1, "tag_name": "v1", "assets": [], "published_at": "2026-01-01T00:00:00Z"},
            {"id": 2, "tag_name": "v2-rc", "assets": [], "prerelease": true, "published_at": "2026-02-01T00:00:00Z"},
            {"id": 3, "tag_name": "draft", "assets": [], "draft": true, "published_at": "2026-09-01T00:00:00Z"},
            {"id": 4, "tag_name": "v4", "assets": [], "published_at": "2026-04-01T00:00:00Z"},
            {"id": 5, "tag_name": "v5", "assets": [], "published_at": "2026-05-01T00:00:00Z"},
            {"id": 6, "tag_name": "v6", "assets": [], "published_at": "2026-06-01T00:00:00Z"},
            {"id": 7, "tag_name": "v7", "assets": [], "published_at": "2026-07-01T00:00:00Z"},
            {"id": 8, "tag_name": "unpublished", "assets": []}
        ]"#;
        let releases: Vec<Release> = serde_json::from_str(json).unwrap();
        let recent = recent_releases(releases);

        assert_eq!(
            recent
                .iter()
                .map(|release| release.tag.as_str())
                .collect::<Vec<_>>(),
            ["v7", "v6", "v5", "v4", "v2-rc"]
        );
        assert!(recent.last().is_some_and(|release| release.prerelease));
    }
}
