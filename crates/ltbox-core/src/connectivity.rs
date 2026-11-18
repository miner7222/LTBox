//! Startup reachability probe — advisory only, nothing gates on it.
//!
//! LTBox pulls EDL loaders, root providers, driver packages and updates from
//! GitHub, which some regions block while the rest of the internet works.
//! Probing the link and GitHub separately lets the startup notice name which
//! is missing.
//!
//! Distinct from [`ltbox_device::driver::probe_connectivity`], the GitHub-only
//! gate for the Qualcomm driver buttons.

use std::time::Duration;

use crate::downloader::USER_AGENT;

/// Fixed-body link check. Microsoft's NCSI host stays reachable where GitHub
/// is blocked, and the exact body rejects a captive portal answering 200 with
/// a login page.
const NCSI_URL: &str = "http://www.msftconnecttest.com/connecttest.txt";
const NCSI_BODY: &str = "Microsoft Connect Test";
/// Fallback for networks that block Microsoft: Firefox's portal-detection host.
const PORTAL_URL: &str = "http://detectportal.firefox.com/success.txt";
const PORTAL_BODY: &str = "success";
/// GitHub canary: the host every release / artifact lookup starts from.
const GITHUB_URL: &str = "https://api.github.com/";

/// Per-probe budget; at most three sequential probes, run off the UI thread.
const PROBE_TIMEOUT: Duration = Duration::from_secs(6);

/// Outcome of the two startup reachability checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnectivityReport {
    /// The link itself works — a fixed-body probe a captive portal fails.
    pub internet: bool,
    /// `api.github.com` answered. Can be `false` while [`Self::internet`]
    /// is `true` wherever GitHub specifically is blocked.
    pub github: bool,
}

impl ConnectivityReport {
    /// Nothing to tell the user about.
    pub fn all_reachable(&self) -> bool {
        self.internet && self.github
    }
}

fn probe_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .user_agent(USER_AGENT)
        .timeout_global(Some(PROBE_TIMEOUT))
        .build()
        .new_agent()
}

/// Whether `url` answered with a body starting with `expected`; a captive
/// portal's 200 HTML fails the match.
fn body_matches(agent: &ureq::Agent, url: &str, expected: &str) -> bool {
    agent
        .get(url)
        .call()
        .ok()
        .and_then(|mut resp| resp.body_mut().read_to_string().ok())
        .is_some_and(|body| body.trim_start().starts_with(expected))
}

/// Run both checks. Never fails: a transport error is just `false`.
pub fn probe() -> ConnectivityReport {
    let agent = probe_agent();
    let internet =
        body_matches(&agent, NCSI_URL, NCSI_BODY) || body_matches(&agent, PORTAL_URL, PORTAL_BODY);
    let github = agent.get(GITHUB_URL).call().is_ok();
    // A GitHub hit proves the link even when both fixed-body hosts are
    // blocked, so never report "no internet" alongside a reachable GitHub.
    ConnectivityReport {
        internet: internet || github,
        github,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_reachable_requires_both() {
        assert!(
            ConnectivityReport {
                internet: true,
                github: true
            }
            .all_reachable()
        );
        assert!(
            !ConnectivityReport {
                internet: true,
                github: false
            }
            .all_reachable()
        );
        assert!(
            !ConnectivityReport {
                internet: false,
                github: false
            }
            .all_reachable()
        );
    }
}
