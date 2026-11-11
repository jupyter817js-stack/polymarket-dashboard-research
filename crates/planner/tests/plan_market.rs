//! Unit tests for the planner — verifying every bug-fix from §5 of the handoff.

use rn1_planner::*;

fn cfg() -> PlannerConfig {
    PlannerConfig::default()
}

fn afford(c: f64) -> Affordability {
    Affordability { event_headroom: c, wallet_headroom: c, kill_switch: false }
}

fn ps(q: [f64; 2], vwap: [f64; 2], bid: [f64; 2], ask: [f64; 2]) -> PairState {
    PairState {
        condition_id: "cid".into(),
        token_id: ["t0".into(), "t1".into()],
        game_started: true,
        endgame: false,
        secs_to_game_start: Some(-60),
        outcome_prob: None,
        q, vwap, bid, ask,
        collapse: [false, false],
        resting: vec![],
    }
}

#[test]
fn ladder_places_below_aim_even_when_bidsum_above_aim() {
    // bid_sum = 0.98 > aim (0.90) — old outer-gate would Idle. We must accumulate.
    // (Set game_started=false so 2026-05-25 in-play suppression doesn't fire.)
    let mut s = ps([0.0,0.0], [0.0,0.0], [0.50, 0.48], [0.51, 0.49]);
    s.game_started = false;
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert!(!plan.add.is_empty(), "Step 3 must ladder below market even with bid_sum > aim");
    for d in &plan.add {
        assert!(matches!(d.kind, OrderKind::Maker));
        assert!(matches!(d.tag, PlanTag::NewPair));
    }
    // every accepted rung has pair-sum ≤ aim (playbook §4.2)
    for d in plan.add.iter().filter(|d| d.leg == 0) {
        let p1 = s.bid[1] - (s.bid[0] - d.price);
        let sum = (d.price * 100.0).round() / 100.0 + (p1 * 100.0).round() / 100.0;
        assert!(sum <= cfg().cumulative_aim + 1e-9, "rung pair-sum {} > aim", sum);
    }
}

#[test]
fn complete_places_at_cap_when_market_bid_above() {
    // ahead leg vwap=0.50; aim=0.90 → cap=0.40. lagging market bid=0.60 (above cap).
    // Old logic: nothing placed. New: place AT 0.40.
    // (Post 2026-05-24 Step3 refactor — Step 3 may add extra dip-pair rungs;
    // we only require the Completion bid at price 0.40 exists.)
    let mut s = ps([10.0, 0.0], [0.50, 0.0], [0.50, 0.60], [0.52, 0.62]);
    s.game_started = false;     // skip sweep
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert_eq!(plan.mode, PlanMode::Complete);
    let comp: Vec<_> = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::Completion)).collect();
    assert_eq!(comp.len(), 1, "must place exactly one completion bid");
    let d = comp[0];
    assert_eq!(d.leg, 1, "complete the lagging leg");
    assert!((d.price - 0.40).abs() < 1e-9, "expected 0.40, got {}", d.price);
}

#[test]
fn cap_breach_does_not_fire_on_empty_other_leg() {
    // Resting bid at 0.45 on leg 0; leg 1 q==0, market bid 0.60. Old cap_breach
    // would compare 0.45 + 0.60 = 1.05 > 0.95 and cancel. New: q[1]==0 → no breach.
    let mut s = ps([0.0, 0.0], [0.0, 0.0], [0.50, 0.60], [0.52, 0.62]);
    s.resting.push(RestingOrder { qid: 1, leg: 0, price: 0.45, remaining_shares: 6.0,
                                  side: Side::Buy, kind: OrderKind::Maker, age_s: 30 });
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert!(!plan.cancel.contains(&1), "must not cancel new-pair rung on empty other leg");
}

#[test]
fn sweep_does_not_pile_into_over_held_leg() {
    // Collapse on leg 0 but we already hold MORE of leg 0 than leg 1 — must NOT sweep.
    let mut s = ps([20.0, 5.0], [0.50, 0.40], [0.35, 0.55], [0.37, 0.57]);
    s.collapse = [true, false];
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    // Sweep must not fire — falls through to completion of lagging leg 1.
    assert_ne!(plan.mode, PlanMode::Sweep);
}

#[test]
fn kill_switch_cancels_everything_no_new_orders() {
    let mut s = ps([0.0, 0.0], [0.0, 0.0], [0.50, 0.48], [0.51, 0.49]);
    s.resting.push(RestingOrder { qid: 7, leg: 0, price: 0.45, remaining_shares: 6.0,
                                  side: Side::Buy, kind: OrderKind::Maker, age_s: 30 });
    let aff = Affordability { event_headroom: 100.0, wallet_headroom: 100.0, kill_switch: true };
    let plan = plan_market(&cfg(), &s, &aff);
    assert!(plan.add.is_empty());
    assert_eq!(plan.cancel, vec![7]);
    assert_eq!(plan.mode, PlanMode::Idle);
}

#[test]
fn bail_fires_on_single_held_leg_with_collapsed_market() {
    // q[0]=24 @ vwap 0.555, q[1]=0. Market collapsed: bid[0]=0.02 (≤ 0.05 threshold).
    // 4 stale completion bids resting on lag (leg 1) at 0.29-0.32. Must cancel ALL
    // and return Bail mode (no new orders).
    let mut s = ps([24.0, 0.0], [0.555, 0.0], [0.02, 0.95], [0.05, 0.99]);
    s.game_started = false;
    for (i, price) in [0.29, 0.30, 0.31, 0.32].iter().enumerate() {
        s.resting.push(RestingOrder {
            qid: (i + 1) as i64, leg: 1, price: *price, remaining_shares: 6.0,
            side: Side::Buy, kind: OrderKind::Maker, age_s: 100,
        });
    }
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert_eq!(plan.mode, PlanMode::Bail, "must enter Bail mode");
    assert!(plan.add.is_empty(), "no new orders in Bail");
    assert_eq!(plan.cancel.len(), 4, "must cancel all 4 stale Navarro bids");
    for qid in 1..=4_i64 { assert!(plan.cancel.contains(&qid)); }
}

#[test]
fn bail_does_not_fire_on_balanced_pair() {
    // Both legs held, both bids healthy → no bail even if one bid is low.
    let mut s = ps([10.0, 10.0], [0.50, 0.50], [0.02, 0.95], [0.05, 0.99]);
    s.game_started = false;
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert_ne!(plan.mode, PlanMode::Bail, "balanced pair must not bail");
}

#[test]
fn bail_does_not_fire_on_new_pair() {
    // No position yet — Bail does not apply.
    let s = ps([0.0, 0.0], [0.0, 0.0], [0.02, 0.95], [0.05, 0.99]);
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert_ne!(plan.mode, PlanMode::Bail, "new pair (no holdings) must not bail");
}

#[test]
fn bail_disabled_via_config_does_not_fire() {
    let mut c = cfg();
    c.bail_enabled = false;
    let mut s = ps([24.0, 0.0], [0.555, 0.0], [0.02, 0.95], [0.05, 0.99]);
    s.game_started = false;
    let plan = plan_market(&c, &s, &afford(100.0));
    assert_ne!(plan.mode, PlanMode::Bail);
}

#[test]
fn stale_maker_far_from_market_is_cancelled() {
    // Resting maker at 0.32 on leg 1; market bid[1]=0.95 (distance 0.63 > 0.10
    // threshold) AND age 6h > 4h. Must be cancelled.
    let mut s = ps([24.0, 0.0], [0.555, 0.0], [0.50, 0.95], [0.55, 0.97]);
    s.game_started = false;
    s.resting.push(RestingOrder {
        qid: 99, leg: 1, price: 0.32, remaining_shares: 6.0,
        side: Side::Buy, kind: OrderKind::Maker, age_s: 6 * 3600,
    });
    // Disable bail so we only test the stale-maker path on bid[0]=0.50 (above threshold).
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert!(plan.cancel.contains(&99), "stale maker (age 6h, dist 0.63) must be cancelled");
}

#[test]
fn stale_maker_close_to_market_is_kept() {
    // Resting maker at 0.50 on leg 0; market bid[0]=0.49 (dist 0.01 < threshold).
    // Even though aged, the order is still reachable — keep it.
    let mut s = ps([0.0, 0.0], [0.0, 0.0], [0.49, 0.50], [0.51, 0.52]);
    s.resting.push(RestingOrder {
        qid: 42, leg: 0, price: 0.50, remaining_shares: 6.0,
        side: Side::Buy, kind: OrderKind::Maker, age_s: 6 * 3600,
    });
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert!(!plan.cancel.contains(&42), "near-market stale maker must be kept");
}

#[test]
fn stale_maker_under_max_age_is_kept() {
    // Far from market (dist 0.30) but YOUNG (1h < 4h). Must not be cancelled.
    let mut s = ps([0.0, 0.0], [0.0, 0.0], [0.50, 0.80], [0.52, 0.82]);
    s.resting.push(RestingOrder {
        qid: 7, leg: 1, price: 0.50, remaining_shares: 6.0,
        side: Side::Buy, kind: OrderKind::Maker, age_s: 3600,
    });
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert!(!plan.cancel.contains(&7), "young maker must be kept regardless of distance");
}

#[test]
fn step3_runs_alongside_step2_with_naked_position() {
    // q=[12, 0] naked Mboko-style. Old planner returned after Step 2 (completion
    // bid only). New planner must ALSO place new dip-pair rungs.
    let mut s = ps([12.0, 0.0], [0.45, 0.0], [0.55, 0.40], [0.56, 0.42]);
    s.game_started = false;
    let plan = plan_market(&cfg(), &s, &afford(200.0));
    let completion = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::Completion)).count();
    let new_pair  = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::NewPair)).count();
    assert!(completion >= 1, "completion bid expected on naked-1 case");
    assert!(new_pair  >= 2, "Step 3 must add new_pair ladder alongside Step 2");
    assert_eq!(plan.mode, PlanMode::Complete);
}

#[test]
fn rung_blocked_when_existing_vwap_pinned_above_target() {
    // Huge anchor at vwap 0.51 each leg → matched_vwap = 1.02 (over ceiling).
    // No dip is deep enough (even bottom rung) to drag weighted vwap below
    // target 0.95 — every rung must be blocked.
    let s = ps([1000.0, 1000.0], [0.51, 0.51], [0.50, 0.50], [0.52, 0.52]);
    let plan = plan_market(&cfg(), &s, &afford(2000.0));
    let new_pair = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::NewPair)).count();
    assert_eq!(new_pair, 0, "weighted-vwap guard must block all rungs when existing matched_vwap is already over target");
}

#[test]
fn rung_allowed_when_post_fill_matched_vwap_dips_below_target() {
    // existing small position pulls average down with new cheap rung.
    // q=[6,6] @ 0.50/0.45 → matched_vwap=0.95 (exactly target). Add deep
    // dip 0.20/0.20 (size 6) → post-vwap each ≈ (6*0.50+6*0.20)/12 = 0.35,
    // sum 0.70 < target. Must allow.
    let mut s = ps([6.0, 6.0], [0.50, 0.45], [0.20, 0.20], [0.22, 0.22]);
    s.game_started = false;
    let plan = plan_market(&cfg(), &s, &afford(200.0));
    let new_pair = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::NewPair)).count();
    assert!(new_pair > 0, "deep-dip rung that pulls matched_vwap down must be allowed");
}

#[test]
fn merge_emitted_when_cap_blocks_with_matched_pairs() {
    // q=[100, 100] @ vwap 0.45 each → matched_vwap=0.90. cap is exhausted
    // (afford 0.5 — below min_order_usdc=1.0). Planner must emit MergeAction
    // to recycle 99 shares (matched - buffer=1) and free $89.10 of headroom.
    let mut s = ps([100.0, 100.0], [0.45, 0.45], [0.40, 0.40], [0.42, 0.42]);
    s.game_started = false;
    let plan = plan_market(&cfg(), &s, &afford(0.5));
    assert_eq!(plan.merges.len(), 1, "must emit one MergeAction when cap is tight");
    let m = &plan.merges[0];
    assert!((m.shares - 99.0).abs() < 1e-6, "merge shares = matched - buffer (1.0), got {}", m.shares);
    assert!((m.matched_vwap - 0.90).abs() < 0.01, "matched_vwap snapshot ≈ 0.90, got {}", m.matched_vwap);
    // With $89.10 freed, the ladder should place at least one new_pair rung.
    let new_pair = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::NewPair)).count();
    assert!(new_pair > 0, "post-merge capital should allow at least one new_pair rung");
}

#[test]
fn merge_does_not_fire_below_eager_threshold_with_headroom() {
    // 2026-05-25: eager threshold dropped 10 → 5. Use matched=3 (< 5) to keep
    // this test's intent: below eager threshold + ample cap → no merge.
    let mut s = ps([3.0, 3.0], [0.45, 0.45], [0.40, 0.40], [0.42, 0.42]);
    s.game_started = false;
    let plan = plan_market(&cfg(), &s, &afford(200.0));
    assert!(plan.merges.is_empty(),
        "no merge when matched is below eager threshold AND cap is not tight");
}

#[test]
fn merge_does_not_fire_when_matched_below_threshold() {
    // Only 2 matched pairs < min_merge_shares=5. Don't merge.
    let mut s = ps([2.0, 2.0], [0.45, 0.45], [0.40, 0.40], [0.42, 0.42]);
    s.game_started = false;
    let plan = plan_market(&cfg(), &s, &afford(0.5));
    assert!(plan.merges.is_empty(), "matched below min_merge_shares — no merge");
}

#[test]
fn merge_disabled_via_config_does_not_fire() {
    let mut c = cfg();
    c.merge_enabled = false;
    let mut s = ps([100.0, 100.0], [0.45, 0.45], [0.40, 0.40], [0.42, 0.42]);
    s.game_started = false;
    let plan = plan_market(&c, &s, &afford(0.5));
    assert!(plan.merges.is_empty(), "merge_enabled=false → no merge");
}

#[test]
fn cap_breach_uses_weighted_vwap_keeps_dip_rung() {
    // Post 2026-05-25 fix: a resting rung that, after filling, would pull leg
    // vwap DOWN (so combined matched_vwap stays ≤ target) must NOT be cancelled.
    // Old single-leg compare (price + vwap[other] > target) would cancel this
    // → infinite churn. Weighted projection keeps it.
    let mut s = ps([10.0, 10.0], [0.50, 0.50], [0.40, 0.40], [0.42, 0.42]);
    s.game_started = false;
    s.resting.push(RestingOrder {
        qid: 1, leg: 0, price: 0.30, remaining_shares: 18.0,
        side: Side::Buy, kind: OrderKind::Maker, age_s: 60,
    });
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    // post-fill leg-0 vwap = (10*0.5 + 18*0.3)/28 = (5 + 5.4)/28 = 0.371
    // matched_vwap ≈ 0.371 + 0.50 = 0.871 ≤ target 0.95 → keep
    assert!(!plan.cancel.contains(&1),
        "weighted-vwap cap_breach must keep deep-dip rung that drags vwap down");
}

#[test]
fn cap_breach_still_cancels_truly_breaching_rung() {
    // A small rung at high price doesn't move vwap much — properly breaches.
    let mut s = ps([10.0, 10.0], [0.50, 0.50], [0.40, 0.40], [0.42, 0.42]);
    s.game_started = false;
    s.resting.push(RestingOrder {
        qid: 2, leg: 0, price: 0.80, remaining_shares: 5.0,
        side: Side::Buy, kind: OrderKind::Maker, age_s: 60,
    });
    // post-fill leg-0 vwap = (10*0.5 + 5*0.8)/15 = (5+4)/15 = 0.60
    // matched_vwap = 0.60 + 0.50 = 1.10 > target 0.95 → cancel
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert!(plan.cancel.contains(&2), "truly-breaching rung must still cancel");
}

#[test]
fn in_play_suppresses_step3_ladder() {
    // Live match (game_started=true): Step 3 must not place new pair rungs.
    // Step 1 (Sweep) + Step 2 (Completion) still run.
    let mut s = ps([0.0, 0.0], [0.0, 0.0], [0.50, 0.48], [0.51, 0.49]);
    s.game_started = true;
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    let new_pair = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::NewPair)).count();
    assert_eq!(new_pair, 0, "in-play Step 3 must be suppressed");
    assert_eq!(plan.mode, PlanMode::Idle);
}

#[test]
fn pre_match_still_allows_step3_ladder() {
    // Pre-match (game_started=false): Step 3 ladders normally.
    let mut s = ps([0.0, 0.0], [0.0, 0.0], [0.50, 0.40], [0.51, 0.42]);
    s.game_started = false;
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    let new_pair = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::NewPair)).count();
    assert!(new_pair > 0, "pre-match Step 3 must still ladder");
}

#[test]
fn in_play_suppress_can_be_disabled_via_config() {
    let mut c = cfg();
    c.step3_suppress_in_play = false;
    let mut s = ps([0.0, 0.0], [0.0, 0.0], [0.50, 0.40], [0.51, 0.42]);
    s.game_started = true;
    let plan = plan_market(&c, &s, &afford(100.0));
    let new_pair = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::NewPair)).count();
    assert!(new_pair > 0, "with suppression off, ladder must run even in-play");
}

#[test]
fn merge_eager_fires_without_cap_pressure() {
    // 20 matched pairs, plenty of headroom — old logic skipped merge.
    // New eager trigger: matched ≥ 10 → recycle anyway.
    let mut s = ps([20.0, 20.0], [0.45, 0.45], [0.40, 0.40], [0.42, 0.42]);
    s.game_started = false;
    let plan = plan_market(&cfg(), &s, &afford(200.0));   // 200 = ample headroom
    assert_eq!(plan.merges.len(), 1, "eager merge should fire on matched ≥ 10 even with headroom");
    let m = &plan.merges[0];
    assert!((m.shares - 19.0).abs() < 1e-6, "merge = matched(20) - buffer(1) = 19, got {}", m.shares);
}

#[test]
fn merge_eager_below_threshold_holds_off() {
    // 2026-05-25: eager threshold dropped to 5. Use matched=3 to stay below.
    let mut s = ps([3.0, 3.0], [0.45, 0.45], [0.40, 0.40], [0.42, 0.42]);
    s.game_started = false;
    let plan = plan_market(&cfg(), &s, &afford(200.0));
    assert!(plan.merges.is_empty(), "below eager threshold + ample cap → no merge");
}

#[test]
fn asymmetric_bail_fires_on_over_loaded_loser_side() {
    // q=[80, 8] → naked=72, matched=8. naked/matched=9 > 0.5. Leg 0 bid
    // collapsed to 0.05 (≤ 0.10 asymmetric threshold) → BAIL.
    // This mirrors the Kecmano-Marozsa failure (70.96 loser shares + 1 matched).
    let mut s = ps([80.0, 8.0], [0.48, 0.48], [0.05, 0.95], [0.08, 0.99]);
    s.game_started = false;
    s.resting.push(RestingOrder {
        qid: 1, leg: 0, price: 0.20, remaining_shares: 6.0,
        side: Side::Buy, kind: OrderKind::Maker, age_s: 30,
    });
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert_eq!(plan.mode, PlanMode::Bail, "asymmetric BAIL must fire");
    assert!(plan.cancel.contains(&1), "all resting must be cancelled in BAIL");
    assert!(plan.add.is_empty(), "no new orders in BAIL");
}

#[test]
fn asymmetric_bail_does_not_fire_on_healthy_imbalance() {
    // 2026-05-25: bail_strong_threshold = 0.50 now triggers at mid bids.
    // Truly "healthy" means bid > 0.50. Use 0.60.
    let mut s = ps([80.0, 8.0], [0.48, 0.48], [0.60, 0.35], [0.62, 0.37]);
    s.game_started = false;
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert_ne!(plan.mode, PlanMode::Bail, "healthy bid (> bail_strong_threshold) blocks BAIL");
}

#[test]
fn emergency_close_sweeps_lagging_leg_pre_match() {
    // The Rinderknech-Rodionov pattern: pre-match ladder filled leg 0 only.
    // q=[24, 0], game in 3 minutes (180s ≤ 300s threshold). MUST taker-sweep
    // Rodionov at ask + slippage to pair up before in-play.
    let mut s = ps([24.0, 0.0], [0.675, 0.0], [0.75, 0.24], [0.76, 0.25]);
    s.game_started = false;
    s.secs_to_game_start = Some(180);
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert_eq!(plan.mode, PlanMode::Sweep, "emergency close must enter Sweep mode");
    assert_eq!(plan.add.len(), 1, "single sweep order on lagging leg");
    let d = &plan.add[0];
    assert_eq!(d.leg, 1, "sweep the lagging (empty) leg");
    assert!(matches!(d.kind, OrderKind::Taker), "must be a TAKER order");
    // ask 0.25 + 2¢ slippage = 0.27
    assert!((d.price - 0.27).abs() < 1e-9, "sweep at ask+slip, got {}", d.price);
    // size = full naked (24)
    assert!((d.size_shares - 24.0).abs() < 1e-6, "sweep full naked size, got {}", d.size_shares);
}

#[test]
fn emergency_close_does_not_fire_if_game_far_away() {
    // Same imbalance but match is 1 hour away → no emergency, normal Step 2.
    let mut s = ps([24.0, 0.0], [0.675, 0.0], [0.75, 0.24], [0.76, 0.25]);
    s.game_started = false;
    s.secs_to_game_start = Some(3600);
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert_ne!(plan.mode, PlanMode::Sweep, "no emergency 1 hour out");
}

#[test]
fn emergency_close_does_not_fire_after_game_started() {
    // Game already started — Step 2 (completion) takes over.
    let mut s = ps([24.0, 0.0], [0.675, 0.0], [0.75, 0.24], [0.76, 0.25]);
    s.game_started = true;
    s.secs_to_game_start = Some(-120);
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert_ne!(plan.mode, PlanMode::Sweep, "no emergency after game start");
}

#[test]
fn emergency_close_can_be_disabled() {
    let mut c = cfg();
    c.emergency_close_enabled = false;
    let mut s = ps([24.0, 0.0], [0.675, 0.0], [0.75, 0.24], [0.76, 0.25]);
    s.game_started = false;
    s.secs_to_game_start = Some(180);
    let plan = plan_market(&c, &s, &afford(100.0));
    assert_ne!(plan.mode, PlanMode::Sweep, "disabled config must skip emergency");
}

#[test]
fn emergency_close_requires_naked_above_min_shares() {
    // Tiny naked (2 < min 5) → no emergency sweep.
    let mut s = ps([2.0, 0.0], [0.675, 0.0], [0.75, 0.24], [0.76, 0.25]);
    s.game_started = false;
    s.secs_to_game_start = Some(180);
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert_ne!(plan.mode, PlanMode::Sweep, "below min_order_shares → no sweep");
}

#[test]
fn step2_completion_skipped_when_lag_is_dead_leg() {
    // Busta-Lehecka pattern: Lehecka is dead (outcome_prob 0.005).
    // Completion would normally fill lag = Lehecka. With D-rule, skip.
    let mut s = ps([10.0, 0.0], [0.50, 0.0], [0.97, 0.40], [0.99, 0.42]);
    s.game_started = true;
    s.outcome_prob = Some([0.995, 0.005]);
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    let completion = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::Completion)).count();
    assert_eq!(completion, 0, "no completion onto dead leg");
}

#[test]
fn step2_completion_skipped_when_in_play_lag_bid_too_low() {
    // In-play AND lag bid < step2_in_play_min_bid (0.30 default).
    // Lehecka 13:1 scenario without outcome_prob: lag bid 0.10 < 0.30 → skip.
    let mut s = ps([10.0, 0.0], [0.50, 0.0], [0.90, 0.10], [0.92, 0.12]);
    s.game_started = true;
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    let completion = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::Completion)).count();
    assert_eq!(completion, 0, "no completion when in-play lag bid below threshold");
}

#[test]
fn step2_completion_runs_pre_match_even_with_low_lag_bid() {
    // Pre-match: even low lag bid is OK (price discovery still in progress).
    let mut s = ps([10.0, 0.0], [0.50, 0.0], [0.90, 0.10], [0.92, 0.12]);
    s.game_started = false;
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    let completion = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::Completion)).count();
    assert!(completion >= 1, "pre-match completion still works at low bids");
}

#[test]
fn sweep_skipped_into_dead_leg() {
    // Collapse on leg 1 (Lehecka-style), but Polymarket says leg 1 is dead.
    let mut s = ps([10.0, 5.0], [0.50, 0.40], [0.95, 0.05], [0.97, 0.08]);
    s.collapse = [false, true];
    s.outcome_prob = Some([0.99, 0.01]);
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert_ne!(plan.mode, PlanMode::Sweep, "no sweep into dead leg");
}

#[test]
fn strong_asymmetric_bail_fires_at_low_bid() {
    // 13:1 imbalance with leg-0 bid at 0.25 (below 0.30 threshold).
    // Lehecka-class late-stage: large naked, dropping bid → BAIL.
    let mut s = ps([13.0, 1.0], [0.585, 0.29], [0.25, 0.75], [0.27, 0.77]);
    s.game_started = true;
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert_eq!(plan.mode, PlanMode::Bail, "strong asymmetric BAIL should fire");
}

#[test]
fn strong_asymmetric_bail_does_not_fire_on_healthy_pair() {
    // Symmetric position with mid bids → no BAIL.
    let s = ps([10.0, 10.0], [0.50, 0.40], [0.45, 0.55], [0.47, 0.57]);
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert_ne!(plan.mode, PlanMode::Bail);
}

#[test]
fn strong_asymmetric_bail_no_fire_on_underdog_holding_steady() {
    // Svitolina-Bondar 2026-05-25 false-positive: 12 Bondar @ vwap 0.25,
    // market bid 0.33 (above 0.30 threshold) → no BAIL despite 12:1.29 ratio.
    let mut s = ps([1.29, 12.0], [0.70, 0.25], [0.66, 0.33], [0.67, 0.34]);
    s.game_started = true;
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert_ne!(plan.mode, PlanMode::Bail,
        "Bondar @ 0.33 is not losing — no BAIL");
}

#[test]
fn exit_fires_on_naked_leg_when_price_rebounds_above_target() {
    // 2026-05-26: defaults relaxed (vwap × 1.75 → 1.5, min 0.50 → 0.40).
    // Bought 12 of leg 0 @ vwap 0.40. Target = 0.40 × 1.5 = 0.60. Ask 0.65 ≥ 0.60.
    let mut s = ps([12.0, 0.0], [0.40, 0.0], [0.63, 0.30], [0.65, 0.32]);
    s.game_started = true;
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    let exits: Vec<_> = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::Exit)).collect();
    assert_eq!(exits.len(), 1, "expected 1 exit");
    let e = exits[0];
    assert_eq!(e.leg, 0);
    assert!(matches!(e.side, Side::Sell));
    assert!((e.price - 0.60).abs() < 1e-9, "exit price = vwap×1.5 = 0.60, got {}", e.price);
    assert!((e.size_shares - 12.0).abs() < 1e-6, "sell full naked, got {}", e.size_shares);
}

#[test]
fn exit_does_not_fire_on_matched_portion() {
    // Matched 10/10, both vwap 0.40. naked = 0 → no exit.
    let mut s = ps([10.0, 10.0], [0.40, 0.40], [0.80, 0.20], [0.82, 0.22]);
    s.game_started = true;
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    let exits: Vec<_> = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::Exit)).collect();
    assert!(exits.is_empty(), "matched portion goes to MERGE, not exit");
}

#[test]
fn exit_blocked_when_ask_below_target() {
    // 2026-05-26 default: vwap×1.5 = 0.60 target. Ask 0.55 < 0.60 → wait.
    let mut s = ps([12.0, 0.0], [0.40, 0.0], [0.53, 0.40], [0.55, 0.42]);
    s.game_started = true;
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    let exits: Vec<_> = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::Exit)).collect();
    assert!(exits.is_empty(), "ask below target → wait");
}

#[test]
fn exit_blocked_by_exit_min_price_floor() {
    // 2026-05-26: floor 0.40. vwap 0.20 × 1.5 = 0.30 < floor 0.40.
    // Ask 0.45 ≥ 0.40 → fires AT FLOOR 0.40, not 0.30.
    let mut s = ps([12.0, 0.0], [0.20, 0.0], [0.43, 0.50], [0.45, 0.52]);
    s.game_started = true;
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    let e = plan.add.iter().find(|d| matches!(d.tag, PlanTag::Exit)).expect("exit fires");
    assert!((e.price - 0.40).abs() < 1e-9, "floor enforced, got {}", e.price);
}

#[test]
fn exit_can_be_disabled_via_config() {
    let mut c = cfg();
    c.exit_enabled = false;
    let mut s = ps([12.0, 0.0], [0.40, 0.0], [0.74, 0.20], [0.75, 0.22]);
    s.game_started = true;
    let plan = plan_market(&c, &s, &afford(100.0));
    let exits = plan.add.iter().filter(|d| matches!(d.tag, PlanTag::Exit)).count();
    assert_eq!(exits, 0);
}

#[test]
fn step2_completion_does_not_cancel_existing_sell_exit() {
    // 2026-05-25 bug: Marti-Shimizu had 12 Shimizu @ vwap 0.51 (naked).
    // S2 exit placed SELL @ 0.89 (resting). Each tick, Step 2 completion
    // would put that SELL in plan.cancel because it's on ahead leg → cancel.
    // Next tick, exit_planner re-emits → 885 cycle churn.
    //
    // Fix: cancel only BUY-side resting on ahead leg.
    let mut s = ps([0.0, 12.0], [0.0, 0.51], [0.50, 0.82], [0.52, 0.84]);
    s.game_started = true;
    s.resting.push(RestingOrder {
        qid: 999, leg: 1, side: Side::Sell, price: 0.89, remaining_shares: 12.0,
        kind: OrderKind::Maker, age_s: 30,
    });
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert!(!plan.cancel.contains(&999),
        "Step 2 completion must NOT cancel SELL exit on ahead leg");
}

#[test]
fn sweep_does_not_cancel_existing_sell_exit_on_other_leg() {
    // Collapse on leg 0 + sweep on leg 0 → cancel other-leg (1) resting.
    // BUT a SELL exit on leg 1 should stay.
    let mut s = ps([0.0, 5.0], [0.0, 0.50], [0.30, 0.50], [0.32, 0.52]);
    s.collapse = [true, false];
    s.resting.push(RestingOrder {
        qid: 555, leg: 1, side: Side::Sell, price: 0.88, remaining_shares: 5.0,
        kind: OrderKind::Maker, age_s: 30,
    });
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert!(!plan.cancel.contains(&555),
        "Sweep must NOT cancel SELL exit on the OTHER leg");
}

#[test]
fn cap_breach_does_not_apply_to_sell_orders() {
    // SELL exit at 0.89 on leg 0 with q=[10, 10] — old logic would project
    // post-fill vwap (treating as BUY) → spurious breach. New: skip SELLs.
    let mut s = ps([10.0, 10.0], [0.50, 0.50], [0.40, 0.40], [0.42, 0.42]);
    s.resting.push(RestingOrder {
        qid: 777, leg: 0, side: Side::Sell, price: 0.89, remaining_shares: 5.0,
        kind: OrderKind::Maker, age_s: 30,
    });
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert!(!plan.cancel.contains(&777),
        "cap_breach must skip SELL orders");
}

#[test]
fn endgame_completion_relaxes_aim_to_target() {
    // ahead vwap=0.55; aim=0.90 → cap=0.35; target=0.95 → cap=0.40.
    let mut s = ps([10.0, 0.0], [0.55, 0.0], [0.50, 0.50], [0.52, 0.52]);
    s.game_started = false;
    s.endgame = true;
    let plan = plan_market(&cfg(), &s, &afford(100.0));
    assert_eq!(plan.mode, PlanMode::Complete);
    let d = &plan.add[0];
    assert!((d.price - 0.40).abs() < 1e-9, "endgame must use target (0.95): cap=0.40, got {}", d.price);
}

