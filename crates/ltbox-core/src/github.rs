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

pub struct GitHubClient {
    owner_repo: String,
    agent: ureq::Agent,
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
    pub head_branch: Option<String>,
    pub path: Option<String>,
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
}

impl GitHubClient {
    pub fn new(owner_repo: &str) -> Result<Self> {
        let agent = crate::downloader::build_agent();
        Ok(Self {
            owner_repo: owner_repo.to_string(),
            agent,
        })
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
        // Up to 3 attempts with exponential backoff between retries
        // (100ms → 400ms); transport errors and 5xx retry, 4xx short-circuits.
        let url = format!("{API_BASE}/repos/{}{endpoint}", self.owner_repo);

        if let Some(cached) = RESPONSE_CACHE.get(&url) {
            return serde_json::from_str::<T>(&cached)
                .map_err(|e| LtboxError::Download(format!("JSON parse error (cached): {e}")));
        }

        let mut last_err: Option<LtboxError> = None;
        for attempt in 0..3_u32 {
            if attempt > 0 {
                let delay_ms = 100u64 * 4u64.pow(attempt - 1);
                std::thread::sleep(std::time::Duration::from_millis(delay_ms));
            }
            match self.agent.get(&url).call() {
                Ok(mut resp) => {
                    // GitHub release + tag JSON payloads we hit are small;
                    // `read_to_string` is bounded by ureq's default body-size
                    // limit (these endpoints never approach it).
                    let body = resp
                        .body_mut()
                        .read_to_string()
                        .map_err(|e| LtboxError::Download(format!("read body: {e}")))?;
                    let parsed = serde_json::from_str::<T>(&body)
                        .map_err(|e| LtboxError::Download(format!("JSON parse error: {e}")))?;
                    // Only cache successful parses.
                    RESPONSE_CACHE.insert(url.clone(), Arc::new(body));
                    return Ok(parsed);
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
        let encoded: String = tag
            .bytes()
            .map(|b| {
                if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                    char::from(b).to_string()
                } else {
                    format!("%{b:02X}")
                }
            })
            .collect();
        let release: Release = self.get_json(&format!("/releases/tags/{encoded}"))?;
        Ok(release
            .assets
            .into_iter()
            .map(|a| (a.name, a.browser_download_url))
            .collect())
    }

    pub fn workflow_run_for_tag(&self, tag: &str) -> Result<u64> {
        let resp: WorkflowRunsResponse = self.get_json(&format!(
            "/actions/runs?per_page=30&status=completed&branch={tag}"
        ))?;
        resp.workflow_runs
            .first()
            .map(|r| r.id)
            .ok_or_else(|| LtboxError::Download(format!("No workflow run for tag {tag}")))
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
            self.get_json(&format!("/actions/runs/{run_id}/artifacts"))?;
        Ok(resp.artifacts)
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

fn normalize_workflow_path(path: &str) -> String {
    path.trim_start_matches(".github/workflows/")
        .trim_start_matches(".github/workflows\\")
        .to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

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
