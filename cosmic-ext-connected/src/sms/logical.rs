//! Logical conversation: wraps one-or-more underlying SMS thread IDs that
//! represent the same user-perceived conversation.
//!
//! M7 lights up reaction-bucket merging via [`merge_into_logical`], which
//! collapses split threads (same canonical address-set + same subID) into
//! a single `LogicalConversation` carrying multiple `merged_thread_ids`.
//! See `Reaction Thread Splitting - Investigation and Fix Approach.md`
//! for the design rationale and Phase 1B-validated heuristic.

use std::collections::{BTreeSet, HashMap};

use kdeconnect_dbus::plugins::ConversationSummary;

/// A user-perceived conversation, possibly composed of multiple underlying
/// SMS thread IDs that AOSP has split apart (e.g. by iOS reaction echoes
/// arriving with slightly-different address-sets).
#[derive(Debug, Clone)]
pub struct LogicalConversation {
    /// Thread ID used for `replyToConversation` and view subscriptions. For
    /// merged groups this is the most-recently-active sibling within the
    /// canonical merge set, matching AOSP's outgoing-reply canonicalization
    /// (Phase 1B Pair 4 finding).
    pub primary_thread_id: i64,
    /// All underlying thread IDs composing this logical conversation.
    /// Always contains `primary_thread_id`. Single-element for non-merged.
    pub merged_thread_ids: Vec<i64>,
    /// SIM subscription ID. Merge never crosses subID boundaries, so all
    /// threads in `merged_thread_ids` share this value.
    pub subscription_id: i64,
    /// Union of every underlying thread's address-set, deduplicated by
    /// canonical (digit-only) form. Original carrier formatting preserved
    /// for the first occurrence of each canonical address.
    pub addresses: Vec<String>,
    /// Preview of the most recent message body across all merged threads.
    pub last_message_preview: String,
    /// Unix-millis timestamp of the most recent message across all merged threads.
    pub last_message_timestamp: i64,
    /// Whether the most recent message is an MMS with attachments.
    pub has_attachments: bool,
    /// Sum of underlying threads currently flagged unread.
    #[allow(dead_code)] // Read at M11 (mark-as-read fan-out).
    pub unread_count: usize,
}

impl LogicalConversation {
    /// Wrap a single underlying conversation 1:1.
    pub fn from_single(cs: ConversationSummary) -> Self {
        Self {
            primary_thread_id: cs.thread_id,
            merged_thread_ids: vec![cs.thread_id],
            subscription_id: cs.sub_id,
            addresses: cs.addresses,
            last_message_preview: cs.last_message,
            last_message_timestamp: cs.timestamp,
            has_attachments: cs.has_attachments,
            unread_count: if cs.unread { 1 } else { 0 },
        }
    }
}

/// Strip non-digit characters; if the result has more than 10 digits and
/// starts with `1` (US country code), drop the leading `1`. Mirrors
/// `analyze.py::normalize_addr`.
pub(crate) fn normalize_addr(addr: &str) -> String {
    let digits: String = addr.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() > 10 && digits.starts_with('1') {
        digits[1..].to_string()
    } else {
        digits
    }
}

/// Canonical address set: normalize each address, drop empties, deduplicate.
/// Returned as a sorted `Vec` so equal sets compare equal and the value is
/// hashable as a `HashMap` key.
pub(crate) fn canonical_set(addresses: &[String]) -> Vec<String> {
    let mut set = BTreeSet::new();
    for addr in addresses {
        let norm = normalize_addr(addr);
        if !norm.is_empty() {
            set.insert(norm);
        }
    }
    set.into_iter().collect()
}

/// Pairwise merge predicate from the validated heuristic
/// (`analyze.py::is_reaction_bucket`, primary equality clause only).
///
/// Returns `true` iff:
/// - both `sub_id`s are non-`-1` and equal, AND
/// - canonical address-sets are equal.
///
/// The subset clause is intentionally omitted at M7: every Phase 1 capture
/// pair was symmetric, and the conversation-summary-level body field is too
/// sparse to drive the reaction-pattern check that the subset clause needs.
/// Revisit once richer per-message context is available.
#[allow(dead_code)] // Called at M9 (split-by-case reply rule); covered by tests today.
pub(crate) fn is_reaction_bucket(a: &ConversationSummary, b: &ConversationSummary) -> bool {
    if a.sub_id == -1 || b.sub_id == -1 {
        return false;
    }
    if a.sub_id != b.sub_id {
        return false;
    }
    canonical_set(&a.addresses) == canonical_set(&b.addresses)
}

/// Group raw conversations into `LogicalConversation`s, merging threads
/// that share the same canonical address-set and the same `sub_id`
/// (the [`is_reaction_bucket`] precondition). Conversations with
/// `sub_id == -1` or an empty canonical address-set are passed through
/// as 1:1 wrappers — they cannot satisfy the merge precondition.
///
/// Output is sorted by `last_message_timestamp` descending, matching the
/// convention used by the existing single-thread display order.
pub(crate) fn merge_into_logical(raw: &[ConversationSummary]) -> Vec<LogicalConversation> {
    let mut groups: HashMap<(Vec<String>, i64), Vec<&ConversationSummary>> = HashMap::new();
    let mut standalone: Vec<&ConversationSummary> = Vec::new();

    for cs in raw {
        let canon = canonical_set(&cs.addresses);
        if cs.sub_id == -1 || canon.is_empty() {
            standalone.push(cs);
        } else {
            groups.entry((canon, cs.sub_id)).or_default().push(cs);
        }
    }

    let mut result: Vec<LogicalConversation> = Vec::with_capacity(groups.len() + standalone.len());

    for (_, threads) in groups {
        if threads.len() == 1 {
            result.push(LogicalConversation::from_single(threads[0].clone()));
        } else {
            result.push(merge_group(threads));
        }
    }
    for cs in standalone {
        result.push(LogicalConversation::from_single(cs.clone()));
    }

    result.sort_by_key(|lc| std::cmp::Reverse(lc.last_message_timestamp));
    result
}

/// Build one `LogicalConversation` from a group of 2+ `ConversationSummary`
/// values known to share the same canonical address-set and `sub_id`.
fn merge_group(mut threads: Vec<&ConversationSummary>) -> LogicalConversation {
    // Most-recently-active sibling becomes the primary, per the symmetric-
    // split tiebreak rule (Phase 1B: matches AOSP's outgoing-reply
    // canonicalization, so the echo lands on the threadId we passed).
    threads.sort_by_key(|cs| std::cmp::Reverse(cs.timestamp));
    let primary = threads[0];

    let merged_thread_ids: Vec<i64> = threads.iter().map(|cs| cs.thread_id).collect();
    let subscription_id = primary.sub_id;

    // Address union deduplicated by canonical form, preserving the first-
    // seen original (carrier-formatted) string for display.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut addresses: Vec<String> = Vec::new();
    for cs in &threads {
        for addr in &cs.addresses {
            let norm = normalize_addr(addr);
            if !norm.is_empty() && seen.insert(norm) {
                addresses.push(addr.clone());
            }
        }
    }

    let unread_count = threads.iter().filter(|cs| cs.unread).count();

    LogicalConversation {
        primary_thread_id: primary.thread_id,
        merged_thread_ids,
        subscription_id,
        addresses,
        last_message_preview: primary.last_message.clone(),
        last_message_timestamp: primary.timestamp,
        has_attachments: primary.has_attachments,
        unread_count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cs(
        thread_id: i64,
        sub_id: i64,
        addresses: &[&str],
        timestamp: i64,
        last_message: &str,
        unread: bool,
        has_attachments: bool,
    ) -> ConversationSummary {
        ConversationSummary {
            thread_id,
            addresses: addresses.iter().map(|s| (*s).to_string()).collect(),
            last_message: last_message.to_string(),
            timestamp,
            unread,
            has_attachments,
            sub_id,
        }
    }

    #[test]
    fn normalize_addr_strips_non_digits() {
        assert_eq!(normalize_addr("(555) 123-4567"), "5551234567");
        assert_eq!(normalize_addr("+1 555-123-4567"), "5551234567");
        assert_eq!(normalize_addr("555.123.4567"), "5551234567");
    }

    #[test]
    fn normalize_addr_drops_leading_us_country_code() {
        assert_eq!(normalize_addr("+15551234567"), "5551234567");
        assert_eq!(normalize_addr("15551234567"), "5551234567");
    }

    #[test]
    fn normalize_addr_keeps_short_strings_intact() {
        // "1234" is 4 digits — no leading-1 strip (length not > 10).
        assert_eq!(normalize_addr("1234"), "1234");
        // Exactly 10 digits — no leading-1 strip.
        assert_eq!(normalize_addr("1234567890"), "1234567890");
    }

    #[test]
    fn normalize_addr_handles_empty_and_non_numeric() {
        assert_eq!(normalize_addr(""), "");
        assert_eq!(normalize_addr("abc"), "");
    }

    #[test]
    fn canonical_set_dedupes_and_sorts() {
        let addrs = vec![
            "5551234567".to_string(),
            "+15551234567".to_string(),
            "(555) 123-4567".to_string(),
        ];
        assert_eq!(canonical_set(&addrs), vec!["5551234567".to_string()]);
    }

    #[test]
    fn canonical_set_drops_empty_addresses() {
        let addrs = vec!["5551234567".to_string(), "".to_string(), "abc".to_string()];
        assert_eq!(canonical_set(&addrs), vec!["5551234567".to_string()]);
    }

    #[test]
    fn canonical_set_orders_results() {
        let addrs = vec!["5559999999".to_string(), "5551111111".to_string()];
        assert_eq!(
            canonical_set(&addrs),
            vec!["5551111111".to_string(), "5559999999".to_string()]
        );
    }

    #[test]
    fn is_reaction_bucket_merges_equal_canonical_sets() {
        let a = cs(100, 3, &["+15551234567"], 1000, "hi", false, false);
        let b = cs(101, 3, &["5551234567"], 2000, "Loved 'hi'", false, false);
        assert!(is_reaction_bucket(&a, &b));
    }

    #[test]
    fn is_reaction_bucket_rejects_different_addresses() {
        let a = cs(100, 3, &["5551234567"], 1000, "hi", false, false);
        let b = cs(101, 3, &["5559999999"], 2000, "yo", false, false);
        assert!(!is_reaction_bucket(&a, &b));
    }

    #[test]
    fn is_reaction_bucket_rejects_different_subids() {
        // Same canonical set but different subIDs — different SIM cards.
        let a = cs(100, 3, &["5551234567"], 1000, "hi", false, false);
        let b = cs(101, 5, &["5551234567"], 2000, "hi", false, false);
        assert!(!is_reaction_bucket(&a, &b));
    }

    #[test]
    fn is_reaction_bucket_rejects_subid_minus_one() {
        let a = cs(100, -1, &["5551234567"], 1000, "hi", false, false);
        let b = cs(101, -1, &["5551234567"], 2000, "hi", false, false);
        assert!(!is_reaction_bucket(&a, &b));
        // Even one being -1 is a hard reject.
        let c = cs(102, 3, &["5551234567"], 3000, "hi", false, false);
        assert!(!is_reaction_bucket(&a, &c));
    }

    #[test]
    fn is_reaction_bucket_rejects_strict_subset() {
        // Subset clause is omitted at M7; one strictly contained in the
        // other should NOT merge under primary-equality-only.
        let a = cs(100, 3, &["5551111111", "5552222222"], 1000, "hi", false, false);
        let b = cs(101, 3, &["5551111111"], 2000, "yo", false, false);
        assert!(!is_reaction_bucket(&a, &b));
    }

    #[test]
    fn merge_into_logical_passes_through_singletons() {
        let raw = vec![
            cs(100, 3, &["5551111111"], 1000, "a", false, false),
            cs(200, 3, &["5552222222"], 2000, "b", false, false),
        ];
        let logical = merge_into_logical(&raw);
        assert_eq!(logical.len(), 2);
        // Sorted by recency descending.
        assert_eq!(logical[0].primary_thread_id, 200);
        assert_eq!(logical[1].primary_thread_id, 100);
        assert_eq!(logical[0].merged_thread_ids, vec![200]);
        assert_eq!(logical[1].merged_thread_ids, vec![100]);
    }

    #[test]
    fn merge_into_logical_collapses_pair() {
        // Phase 1A Pair 1 shape: 1108 ↔ 1217, same addresses, same subID.
        let raw = vec![
            cs(1108, 3, &["+15551234567"], 5000, "original", false, false),
            cs(1217, 3, &["5551234567"], 6000, "❤ to \"original\"", true, false),
        ];
        let logical = merge_into_logical(&raw);
        assert_eq!(logical.len(), 1);
        let lc = &logical[0];
        // Most-recent activity is 1217 (timestamp 6000) → primary.
        assert_eq!(lc.primary_thread_id, 1217);
        let mut merged = lc.merged_thread_ids.clone();
        merged.sort();
        assert_eq!(merged, vec![1108, 1217]);
        assert_eq!(lc.subscription_id, 3);
        assert_eq!(lc.last_message_preview, "❤ to \"original\"");
        assert_eq!(lc.last_message_timestamp, 6000);
        assert_eq!(lc.unread_count, 1);
    }

    #[test]
    fn merge_into_logical_collapses_triple() {
        // Phase 1A Pair 2 shape: 1048 ↔ 655 ↔ 1047, all same canonical set.
        let raw = vec![
            cs(1048, 3, &["5551111111", "5552222222"], 4000, "older", false, false),
            cs(655,  3, &["5551111111", "5552222222"], 5000, "middle", true, false),
            cs(1047, 3, &["5551111111", "5552222222"], 6000, "newest", true, true),
        ];
        let logical = merge_into_logical(&raw);
        assert_eq!(logical.len(), 1);
        let lc = &logical[0];
        assert_eq!(lc.primary_thread_id, 1047);
        let mut merged = lc.merged_thread_ids.clone();
        merged.sort();
        assert_eq!(merged, vec![655, 1047, 1048]);
        assert_eq!(lc.last_message_preview, "newest");
        assert_eq!(lc.last_message_timestamp, 6000);
        assert!(lc.has_attachments);
        // Two of three threads were unread.
        assert_eq!(lc.unread_count, 2);
    }

    #[test]
    fn merge_into_logical_does_not_merge_across_subids() {
        // Same addresses but different SIMs — must remain two logical convos.
        let raw = vec![
            cs(100, 3, &["5551111111"], 1000, "sim a", false, false),
            cs(200, 5, &["5551111111"], 2000, "sim b", false, false),
        ];
        let logical = merge_into_logical(&raw);
        assert_eq!(logical.len(), 2);
        assert!(logical.iter().all(|lc| lc.merged_thread_ids.len() == 1));
    }

    #[test]
    fn merge_into_logical_does_not_merge_subid_minus_one() {
        // Two threads with subID=-1, same addresses — heuristic rejects.
        let raw = vec![
            cs(100, -1, &["5551111111"], 1000, "a", false, false),
            cs(200, -1, &["5551111111"], 2000, "b", false, false),
        ];
        let logical = merge_into_logical(&raw);
        assert_eq!(logical.len(), 2);
    }

    #[test]
    fn merge_into_logical_unions_addresses() {
        // Same canonical set but different formatting per thread.
        let raw = vec![
            cs(100, 3, &["+15551111111", "+15552222222"], 1000, "a", false, false),
            cs(200, 3, &["5551111111", "5552222222"], 2000, "b", false, false),
        ];
        let logical = merge_into_logical(&raw);
        assert_eq!(logical.len(), 1);
        assert_eq!(logical[0].addresses.len(), 2);
        // First-seen formatting wins; 200 sorts as primary (newer), so its
        // addresses come first in iteration during merge_group.
        let canon = canonical_set(&logical[0].addresses);
        assert_eq!(canon, vec!["5551111111".to_string(), "5552222222".to_string()]);
    }

    #[test]
    fn merge_into_logical_output_sorted_by_recency() {
        // One merged group (timestamp 5000), one standalone (timestamp 9000),
        // one merged group (timestamp 1000). Expect order: 9000, 5000, 1000.
        let raw = vec![
            cs(10, 3, &["5551111111"], 1000, "old a", false, false),
            cs(11, 3, &["5551111111"], 1500, "old b", false, false),
            cs(20, 3, &["5552222222"], 9000, "lone", false, false),
            cs(30, 3, &["5553333333"], 4000, "mid a", false, false),
            cs(31, 3, &["5553333333"], 5000, "mid b", false, false),
        ];
        let logical = merge_into_logical(&raw);
        assert_eq!(logical.len(), 3);
        assert_eq!(logical[0].last_message_timestamp, 9000);
        assert_eq!(logical[1].last_message_timestamp, 5000);
        assert_eq!(logical[2].last_message_timestamp, 1500);
    }
}
