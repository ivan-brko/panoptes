//! How long an agent's prompt cache stays warm
//!
//! Returning to an idle Claude session after its prompt cache has expired
//! re-sends the whole context uncached: on a long session that is slow and
//! expensive, and nothing on screen said so. This is the figure the session
//! row shows - `cache 42m` while the cache is warm, `cold` once it is not.
//!
//! ## Where the clock starts
//!
//! A cache entry lives for its TTL from the last request that *used* it, and a
//! cache hit refreshes the TTL as much as a cache write does. So the clock
//! starts at the latest request that read or wrote the cache, not the latest
//! one that wrote it: an idle session whose last turn was all cache reads is
//! as warm as one whose last turn wrote a fresh breakpoint.
//!
//! It starts at the request, not the response. The server stamps the entry
//! when the request arrives; the transcript record is written once a content
//! block has streamed back, seconds - or, behind a long think, minutes -
//! later. One request writes several records (one per content block, sharing
//! a `requestId`), so the earliest of them stands for the request, less its
//! `thinkingDurationMs` when it carries one, less [`REQUEST_MARGIN`] for the
//! time to the first token. Every error here errs early: a cache shown cold a
//! minute too soon costs nothing, one shown warm a minute too late costs a
//! full uncached resend.
//!
//! ## Which TTL
//!
//! Claude's `usage.cache_creation` splits what a request wrote into
//! `ephemeral_1h_input_tokens` and `ephemeral_5m_input_tokens`. Any 1-hour
//! tokens means a 1-hour cache (typical on subscriptions), otherwise 5-minute
//! tokens mean five minutes (typical with API keys). A request that wrote
//! nothing - all reads - says nothing about the tier, so the last known tier
//! carries over; a session that has never written anything has no known tier,
//! and nothing is shown for it.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// How much earlier than its first record a request is assumed to have started
///
/// The time from the request reaching the API to the first content block
/// being written. Generous on purpose: see the module docs.
pub const REQUEST_MARGIN: Duration = Duration::seconds(30);

/// A prompt cache's time to live
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheTtl {
    /// `ephemeral_5m_input_tokens`: the API default, typical with API keys
    FiveMinutes,
    /// `ephemeral_1h_input_tokens`: typical on Claude subscriptions
    OneHour,
}

impl CacheTtl {
    /// How long a cache entry of this tier lives after its last use
    pub fn duration(self) -> Duration {
        match self {
            CacheTtl::FiveMinutes => Duration::minutes(5),
            CacheTtl::OneHour => Duration::hours(1),
        }
    }

    /// The tier a request wrote its cache at, from its token split
    ///
    /// `None` when it wrote nothing, which is not the same as a tier: a
    /// request of pure cache reads leaves the tier as it was.
    pub fn from_split(one_hour_tokens: u64, five_minute_tokens: u64) -> Option<Self> {
        if one_hour_tokens > 0 {
            Some(CacheTtl::OneHour)
        } else if five_minute_tokens > 0 {
            Some(CacheTtl::FiveMinutes)
        } else {
            None
        }
    }
}

/// The last request that used the prompt cache, and when that cache expires
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptCache {
    /// The cache's tier, once any request has written to it
    pub ttl: Option<CacheTtl>,
    /// When the last request that read or wrote the cache was sent
    pub requested_at: DateTime<Utc>,
    /// That request's id, so its later records do not move the clock forward
    pub request_id: Option<String>,
    /// `requested_at` plus the tier's TTL; `None` while the tier is unknown
    ///
    /// Worked out once, when a record arrives, so drawing a row only compares
    /// it with the clock.
    pub expires_at: Option<DateTime<Utc>>,
}

impl PromptCache {
    /// A request that used the cache, sent at `requested_at`
    pub fn new(
        ttl: Option<CacheTtl>,
        requested_at: DateTime<Utc>,
        request_id: Option<String>,
    ) -> Self {
        let mut cache = Self {
            ttl,
            requested_at,
            request_id,
            expires_at: None,
        };
        cache.expires_at = cache.compute_expiry();
        cache
    }

    /// Fold in a later record's view of the cache
    ///
    /// Another record of the same request keeps the earlier send time, since
    /// the request started when its first record says it did. A new request
    /// restarts the clock. A tier is only ever learned, never forgotten.
    pub fn merge(&mut self, newer: PromptCache) {
        let same_request = self.request_id.is_some() && self.request_id == newer.request_id;
        if same_request {
            self.requested_at = self.requested_at.min(newer.requested_at);
        } else {
            self.requested_at = newer.requested_at;
            self.request_id = newer.request_id;
        }
        if newer.ttl.is_some() {
            self.ttl = newer.ttl;
        }
        self.expires_at = self.compute_expiry();
    }

    /// How warm the cache is at `now`; `None` while its tier is unknown
    pub fn warmth_at(&self, now: DateTime<Utc>) -> Option<CacheWarmth> {
        let expires_at = self.expires_at?;
        Some(if expires_at > now {
            CacheWarmth::Warm {
                remaining: expires_at - now,
            }
        } else {
            CacheWarmth::Cold
        })
    }

    fn compute_expiry(&self) -> Option<DateTime<Utc>> {
        self.ttl.map(|ttl| self.requested_at + ttl.duration())
    }
}

/// Whether resuming a session now would hit its prompt cache
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheWarmth {
    /// Still cached, for this much longer
    Warm {
        /// Time until the cache expires; always positive
        remaining: Duration,
    },
    /// Expired: the next turn re-sends the context uncached
    Cold,
}

impl CacheWarmth {
    /// `cache 42m` or `cold`
    ///
    /// Minutes round up, so the last seconds of a warm cache read `cache 1m`
    /// rather than a `cache 0m` that looks expired.
    pub fn label(&self) -> String {
        match self {
            CacheWarmth::Warm { remaining } => {
                let minutes = (remaining.num_seconds() + 59) / 60;
                format!("cache {}m", minutes.max(1))
            }
            CacheWarmth::Cold => "cold".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(secs, 0).unwrap()
    }

    #[test]
    fn test_one_hour_tokens_mean_a_one_hour_cache() {
        assert_eq!(CacheTtl::from_split(5_000, 0), Some(CacheTtl::OneHour));
        // Any 1-hour tokens decide it, even beside 5-minute ones
        assert_eq!(CacheTtl::from_split(1, 9_000), Some(CacheTtl::OneHour));
        assert_eq!(CacheTtl::OneHour.duration(), Duration::hours(1));
    }

    #[test]
    fn test_five_minute_tokens_mean_a_five_minute_cache() {
        assert_eq!(CacheTtl::from_split(0, 5_000), Some(CacheTtl::FiveMinutes));
        assert_eq!(CacheTtl::FiveMinutes.duration(), Duration::minutes(5));
    }

    #[test]
    fn test_no_cache_creation_tokens_is_no_tier() {
        assert_eq!(CacheTtl::from_split(0, 0), None);
        // And without a tier there is nothing to show
        let cache = PromptCache::new(None, at(1_000), None);
        assert_eq!(cache.expires_at, None);
        assert_eq!(cache.warmth_at(at(1_000)), None);
    }

    #[test]
    fn test_expiry_counts_from_the_request() {
        let cache = PromptCache::new(Some(CacheTtl::OneHour), at(1_000), Some("r1".into()));
        assert_eq!(cache.expires_at, Some(at(1_000 + 3_600)));
        assert_eq!(
            cache.warmth_at(at(1_000 + 18 * 60)),
            Some(CacheWarmth::Warm {
                remaining: Duration::minutes(42)
            })
        );
        assert_eq!(cache.warmth_at(at(1_000 + 3_600)), Some(CacheWarmth::Cold));
    }

    #[test]
    fn test_later_records_of_one_request_keep_its_start() {
        let mut cache = PromptCache::new(Some(CacheTtl::OneHour), at(1_000), Some("r1".into()));
        cache.merge(PromptCache::new(
            Some(CacheTtl::OneHour),
            at(1_012),
            Some("r1".into()),
        ));
        assert_eq!(cache.requested_at, at(1_000));
        assert_eq!(cache.expires_at, Some(at(4_600)));

        // A new request restarts the clock
        cache.merge(PromptCache::new(
            Some(CacheTtl::OneHour),
            at(2_000),
            Some("r2".into()),
        ));
        assert_eq!(cache.expires_at, Some(at(5_600)));
    }

    #[test]
    fn test_a_cache_hit_refreshes_the_clock_and_keeps_the_tier() {
        let mut cache = PromptCache::new(Some(CacheTtl::FiveMinutes), at(1_000), Some("r1".into()));
        // A request of pure reads: no tier of its own, but it used the cache
        cache.merge(PromptCache::new(None, at(1_200), Some("r2".into())));
        assert_eq!(cache.ttl, Some(CacheTtl::FiveMinutes));
        assert_eq!(cache.expires_at, Some(at(1_500)));
    }

    /// The status line and Codex report usage with no cache figure; folding
    /// one of those in must not forget the transcript's
    #[test]
    fn test_usage_without_a_cache_figure_keeps_the_known_one() {
        use crate::agent::events::UsageSnapshot;
        let cache = PromptCache::new(Some(CacheTtl::OneHour), at(1_000), Some("r1".into()));
        let mut usage = UsageSnapshot::default();
        usage.merge(UsageSnapshot {
            prompt_cache: Some(cache.clone()),
            ..Default::default()
        });
        usage.merge(UsageSnapshot {
            total_tokens: Some(10),
            ..Default::default()
        });
        assert_eq!(usage.prompt_cache, Some(cache));

        // A pure-read request after it restarts the clock on the known tier
        usage.merge(UsageSnapshot {
            prompt_cache: Some(PromptCache::new(None, at(2_000), Some("r2".into()))),
            ..Default::default()
        });
        assert_eq!(
            usage.prompt_cache.and_then(|c| c.expires_at),
            Some(at(5_600))
        );
    }

    #[test]
    fn test_labels() {
        let warm = |secs| CacheWarmth::Warm {
            remaining: Duration::seconds(secs),
        };
        assert_eq!(warm(42 * 60).label(), "cache 42m");
        assert_eq!(warm(41 * 60 + 1).label(), "cache 42m");
        assert_eq!(warm(5).label(), "cache 1m");
        assert_eq!(warm(3_600).label(), "cache 60m");
        assert_eq!(CacheWarmth::Cold.label(), "cold");
    }
}
