use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Identifies a content item across all verticals.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentKey {
    pub content_type: String,
    pub content_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LikeCount {
    pub content_type: String,
    pub content_id: Uuid,
    pub count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LikeStatus {
    pub liked: bool,
    pub liked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserIdentity {
    pub user_id: String,
    pub display_name: Option<String>,
}

/// Supported leaderboard windows.
///
/// Spec: window must be one of `24h`, `7d`, `30d`, `all`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LeaderboardWindow {
    H24,
    D7,
    D30,
    All,
}

impl LeaderboardWindow {
    pub fn as_str(&self) -> &'static str {
        match self {
            LeaderboardWindow::H24 => "24h",
            LeaderboardWindow::D7 => "7d",
            LeaderboardWindow::D30 => "30d",
            LeaderboardWindow::All => "all",
        }
    }

    /// Parses a query parameter value into a `LeaderboardWindow`.
    ///
    /// Accepts case-insensitive values.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "24h" => Some(LeaderboardWindow::H24),
            "7d" => Some(LeaderboardWindow::D7),
            "30d" => Some(LeaderboardWindow::D30),
            "all" => Some(LeaderboardWindow::All),
            _ => None,
        }
    }

    /// Returns the earliest timestamp (inclusive) for time-bounded windows.
    ///
    /// For `all`, returns `None`.
    pub fn since(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self {
            LeaderboardWindow::H24 => Some(now - Duration::hours(24)),
            LeaderboardWindow::D7 => Some(now - Duration::days(7)),
            LeaderboardWindow::D30 => Some(now - Duration::days(30)),
            LeaderboardWindow::All => None,
        }
    }
}

impl std::fmt::Display for LeaderboardWindow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_parse_roundtrip() {
        for (raw, w) in [
            ("24h", LeaderboardWindow::H24),
            ("7d", LeaderboardWindow::D7),
            ("30d", LeaderboardWindow::D30),
            ("all", LeaderboardWindow::All),
        ] {
            assert_eq!(LeaderboardWindow::parse(raw), Some(w));
            assert_eq!(w.to_string(), raw);
        }

        assert_eq!(
            LeaderboardWindow::parse(" 7D "),
            Some(LeaderboardWindow::D7)
        );
        assert_eq!(LeaderboardWindow::parse("bad"), None);
    }
}
