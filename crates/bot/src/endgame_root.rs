//! Post-search root regime classification (`FastWin` / `SafeWin` / `ComebackLoss`)
//! and unified root decision selection (`EndgameRootStats` + Bayesian equilibrium gate).

use nc2000_engine::battle::SearchChoice;
use nc2000_engine::dex::Dex;
use nc2000_engine::state::{Battle, PokeId};

use crate::eval::type_immune_to;
use crate::mask::effective_move_type;

/// Optional root decision & early-stop rules for won, near-won, and lost positions.
///
/// 1. `fast_win`: in completely winning positions ($\mu^* \ge 0.985$), prefer actions
///    that minimize expected turns to win (`win_turns`) without increasing loss/downside risk.
/// 2. `safe_win`: in near-winning positions ($\mu^* \ge 0.85$), penalize quadratic downside
///    risk $E[(1 - R)^2]$, terminal loss rate, and worst-reply/worst-type drops to crush
///    opponent comeback lines.
/// 3. `comeback_loss`: in losing positions ($\mu^* \le 0.12$), weight opponent-mistake EV
///    (payoff when the opponent misses their single best punishing reply), upside tail
///    $\sqrt{E[R^2]}$, survival turns, and a static desperation tie-breaker.
/// 4. `early_stop`: stop the search loop early when a 1-turn 100%-win KO or a fully
///    saturated terminal root is locked in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EndgameRules {
    pub fast_win: bool,
    pub safe_win: bool,
    pub comeback_loss: bool,
    pub early_stop: bool,
}

impl Default for EndgameRules {
    #[inline]
    fn default() -> Self {
        Self::all_decision()
    }
}

impl EndgameRules {
    #[inline]
    pub const fn none() -> Self {
        Self {
            fast_win: false,
            safe_win: false,
            comeback_loss: false,
            early_stop: false,
        }
    }

    #[inline]
    pub const fn any(self) -> bool {
        self.fast_win || self.safe_win || self.comeback_loss || self.early_stop
    }

    #[inline]
    pub const fn decision_any(self) -> bool {
        self.fast_win || self.safe_win || self.comeback_loss
    }

    #[inline]
    pub const fn all_decision() -> Self {
        Self {
            fast_win: true,
            safe_win: true,
            comeback_loss: true,
            early_stop: false,
        }
    }

    #[inline]
    pub const fn all_with_early_stop() -> Self {
        Self {
            fast_win: true,
            safe_win: true,
            comeback_loss: true,
            early_stop: true,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndgameRegime {
    Normal,
    FastWin,
    SafeWin,
    ComebackLoss,
}

/// Compare two root actions by effective visits first, breaking ties by higher
/// empirical mean reward (`rewards / total_visits`) and finally total visits.
pub(crate) fn cmp_visits_then_mean(
    effective_visits: &[u32],
    total_visits: &[u32],
    rewards: &[f64],
    a: usize,
    b: usize,
) -> std::cmp::Ordering {
    let mean = |i: usize| {
        let n = total_visits[i];
        if n > 0 {
            rewards[i] / f64::from(n)
        } else {
            f64::NEG_INFINITY
        }
    };
    effective_visits[a]
        .cmp(&effective_visits[b])
        .then_with(|| mean(a).total_cmp(&mean(b)))
        .then_with(|| total_visits[a].cmp(&total_visits[b]))
}

#[derive(Clone, Debug)]
pub struct EndgameActionMetrics {
    pub action: SearchChoice,
    pub visits: u32,
    pub mean: f64,
    pub downside: f64,
    pub win_rate: f64,
    pub loss_rate: f64,
    pub win_turns: f64,
    pub survive_turns: f64,
    pub trap_risk: f64,
    pub mistake_ev: f64,
    pub desperation: f64,
    pub score: f64,
}

#[derive(Clone, Debug)]
pub struct EndgameDecisionInfo {
    pub regime: EndgameRegime,
    pub mu_star: f64,
    pub legacy_idx: usize,
    pub chosen_idx: usize,
    pub actions: Vec<EndgameActionMetrics>,
}

/// Per-root-action auxiliary statistics accumulated alongside `w` without altering
/// RNG draws or tree exploration.
#[derive(Clone, Debug, Default)]
pub struct EndgameRootStats {
    pub base_turn_offset: u16,
    pub w2: Vec<f64>,
    pub win_n: Vec<u32>,
    pub loss_n: Vec<u32>,
    pub win_turns: Vec<f64>,
    pub survive_turns: Vec<f64>,
    pub desperation: Vec<f64>,
}

impl EndgameRootStats {
    pub fn new(b: &Battle, dex: &Dex, side: usize, acts: &[SearchChoice]) -> Self {
        let k = acts.len();
        let base_turn_offset = u16::from(matches!(
            b.request_state,
            nc2000_engine::state::RequestState::Switch
        ));
        let desperation = if matches!(
            b.request_state,
            nc2000_engine::state::RequestState::TeamPreview
        ) {
            vec![0.0; k]
        } else {
            acts.iter()
                .map(|&c| tactical_desperation_bonus(b, dex, side, c))
                .collect()
        };
        EndgameRootStats {
            base_turn_offset,
            w2: vec![0.0; k],
            win_n: vec![0; k],
            loss_n: vec![0; k],
            win_turns: vec![0.0; k],
            survive_turns: vec![0.0; k],
            desperation,
        }
    }

    #[inline]
    pub fn record(
        &mut self,
        root_turn: u16,
        side: usize,
        act_idx: usize,
        my_reward: f64,
        end: crate::mcts::SimEndInfo,
    ) {
        let Some(w2) = self.w2.get_mut(act_idx) else {
            return;
        };
        let mine = my_reward;
        *w2 += mine * mine;
        let my_win_code = if side == 0 { 1 } else { 2 };
        let my_loss_code = if side == 0 { 2 } else { 1 };
        let is_win = end.outcome == my_win_code || mine >= 0.999_999;
        let is_loss = end.outcome == my_loss_code || mine <= 0.000_001;
        let base_turn = root_turn.saturating_add(self.base_turn_offset);
        let completed_turns = end.turn.saturating_sub(base_turn) as f64;
        let opp_alive = end.alive[1 - side] as f64;
        let my_alive = end.alive[side] as f64;
        let win_t = if is_win {
            completed_turns + 1.0
        } else {
            completed_turns + 2.0 + 3.0 * opp_alive + 6.0 * (1.0 - mine)
        };
        let survive_t = if is_loss {
            completed_turns + 1.0
        } else {
            completed_turns + 2.0 + 3.0 * my_alive + 6.0 * mine
        };
        if is_win {
            self.win_n[act_idx] += 1;
        }
        if is_loss {
            self.loss_n[act_idx] += 1;
        }
        self.win_turns[act_idx] += win_t;
        self.survive_turns[act_idx] += survive_t;
    }

    pub fn should_early_stop(
        &self,
        rules: EndgameRules,
        done: u32,
        target_iters: u32,
        is_preview: bool,
        total_visits: &[u32],
        rewards: &[f64],
        eligible: impl Fn(usize) -> bool,
    ) -> bool {
        if !rules.early_stop || is_preview || target_iters < 128 {
            return false;
        }
        let min_done = (target_iters / 6).clamp(96, 1536);
        if done < min_done || done % 32 != 0 {
            return false;
        }
        let elig: Vec<usize> = (0..total_visits.len()).filter(|&i| eligible(i)).collect();
        if elig.len() <= 1 {
            return true;
        }
        let Some(&legacy_idx) = elig.iter().max_by_key(|&&i| total_visits[i]) else {
            return false;
        };
        let min_ko_v = (target_iters / 30).clamp(32, 256);
        // 1) Guaranteed 1-turn terminal KO lock: the leading action (or a fast_win
        //    action when all alternatives are explored) has >= min_ko_v visits, 100%
        //    terminal win rate, and mean win turns <= 1.001 (every simulation KOs immediately).
        let n_leg = total_visits[legacy_idx];
        if n_leg >= min_ko_v
            && self.win_n[legacy_idx] == n_leg
            && self.loss_n[legacy_idx] == 0
            && (rewards[legacy_idx] / n_leg as f64) >= 0.999_999
            && (self.win_turns[legacy_idx] / n_leg as f64) <= 1.001
        {
            return true;
        }
        if rules.fast_win && elig.iter().all(|&i| total_visits[i] >= 16) {
            let leg_mean = rewards[legacy_idx] / n_leg.max(1) as f64;
            if leg_mean >= 0.985 {
                for &i in &elig {
                    let n = total_visits[i];
                    if n >= min_ko_v
                        && self.win_n[i] == n
                        && self.loss_n[i] == 0
                        && (rewards[i] / n as f64) >= 0.999_999
                        && (self.win_turns[i] / n as f64) <= 1.001
                    {
                        return true;
                    }
                }
            }
        }
        // 2) Full root saturation after 50% budget: every eligible action has >= 24 visits
        //    and all are 100% immediate/1-turn terminal losses or 100% immediate wins.
        if done >= (target_iters / 2).max(128) && elig.iter().all(|&i| total_visits[i] >= 24) {
            let all_dead_now = elig.iter().all(|&i| {
                let n = total_visits[i];
                self.loss_n[i] == n
                    && self.win_n[i] == 0
                    && rewards[i] == 0.0
                    && (self.survive_turns[i] / n as f64) <= 1.5
            });
            if all_dead_now {
                return true;
            }
            let all_won_fast = elig.iter().all(|&i| {
                let n = total_visits[i];
                self.loss_n[i] == 0
                    && self.win_n[i] == n
                    && (rewards[i] / n as f64) >= 0.999
                    && (self.win_turns[i] / n as f64) <= 2.0
            });
            if all_won_fast {
                return true;
            }
        }
        false
    }

    pub fn analyze(
        &self,
        rules: EndgameRules,
        acts: &[SearchChoice],
        total_visits: &[u32],
        effective_visits: &[u32],
        rewards: &[f64],
        eligible: impl Fn(usize) -> bool,
        reply_metrics: impl Fn(usize, u32, f64) -> (f64, f64),
    ) -> Option<EndgameDecisionInfo> {
        let k = acts.len();
        let elig: Vec<usize> = (0..k).filter(|&i| eligible(i)).collect();
        if elig.is_empty() {
            return None;
        }
        let legacy_idx = *elig
            .iter()
            .max_by(|&&a, &&b| cmp_visits_then_mean(effective_visits, total_visits, rewards, a, b))?;
        let n_max = elig.iter().map(|&i| total_visits[i]).max().unwrap_or(0);
        let mut metrics: Vec<EndgameActionMetrics> = (0..k)
            .map(|i| {
                let n = total_visits[i];
                let mean = if n > 0 { rewards[i] / n as f64 } else { 0.5 };
                let w2_i = self.w2.get(i).copied().unwrap_or(0.0);
                let win_n_i = self.win_n.get(i).copied().unwrap_or(0);
                let loss_n_i = self.loss_n.get(i).copied().unwrap_or(0);
                let win_t_i = self.win_turns.get(i).copied().unwrap_or(0.0);
                let surv_t_i = self.survive_turns.get(i).copied().unwrap_or(0.0);
                let m2 = if n > 0 { w2_i / n as f64 } else { 0.25 };
                let downside = (1.0 - 2.0 * mean + m2).max(0.0);
                let win_rate = if n > 0 {
                    win_n_i as f64 / n as f64
                } else {
                    0.0
                };
                let loss_rate = if n > 0 {
                    loss_n_i as f64 / n as f64
                } else {
                    0.0
                };
                let win_turns = if n > 0 { win_t_i / n as f64 } else { 20.0 };
                let survive_turns = if n > 0 { surv_t_i / n as f64 } else { 1.0 };
                let (trap_risk, mistake_ev) = if n > 0 && eligible(i) {
                    reply_metrics(i, n, mean)
                } else {
                    (0.0, mean)
                };
                EndgameActionMetrics {
                    action: acts[i],
                    visits: n,
                    mean,
                    downside,
                    win_rate,
                    loss_rate,
                    win_turns,
                    survive_turns,
                    trap_risk,
                    mistake_ev,
                    desperation: self.desperation.get(i).copied().unwrap_or(0.0),
                    score: mean,
                }
            })
            .collect();

        if elig.len() <= 1 || n_max == 0 || !rules.decision_any() {
            return Some(EndgameDecisionInfo {
                regime: EndgameRegime::Normal,
                mu_star: metrics[legacy_idx].mean,
                legacy_idx,
                chosen_idx: legacy_idx,
                actions: metrics,
            });
        }

        let min_v = (n_max / 8).clamp(8, 256).min(n_max);
        let well_sampled: Vec<usize> = elig
            .iter()
            .copied()
            .filter(|&i| total_visits[i] >= min_v)
            .collect();
        let mu_star = well_sampled
            .iter()
            .map(|&i| metrics[i].mean)
            .fold(f64::NEG_INFINITY, f64::max);
        let mu_legacy = metrics[legacy_idx].mean;
        let emp_var = |i: usize| -> f64 {
            let n = total_visits[i];
            if n == 0 {
                return 0.25;
            }
            let m = rewards[i] / n as f64;
            let m2 = self.w2.get(i).copied().unwrap_or(0.0) / n as f64;
            (m2 - m * m).max(1e-4)
        };
        let se_vs_legacy = |i: usize| -> f64 {
            let n_i = total_visits[i].max(1) as f64;
            let n_l = total_visits[legacy_idx].max(1) as f64;
            (emp_var(i) / n_i + emp_var(legacy_idx) / n_l).sqrt()
        };

        // ---- Regime 1: Completely Winning (fast_win)
        let max_win_rate = well_sampled
            .iter()
            .map(|&i| metrics[i].win_rate)
            .fold(0.0_f64, f64::max);
        let min_loss_rate = well_sampled
            .iter()
            .map(|&i| metrics[i].loss_rate)
            .fold(1.0_f64, f64::min);
        let is_fast_win_regime = (mu_star >= 0.985 && mu_legacy >= 0.970)
            || (mu_star >= 0.93
                && mu_legacy >= 0.91
                && max_win_rate >= 0.85
                && min_loss_rate <= 0.002);
        if rules.fast_win && is_fast_win_regime {
            let top_win: Vec<usize> = well_sampled
                .iter()
                .copied()
                .filter(|&i| {
                    let slack = (1.5 * se_vs_legacy(i)).clamp(0.0025, 0.012);
                    metrics[i].mean + slack >= mu_star
                })
                .collect();
            let min_loss = top_win
                .iter()
                .map(|&i| metrics[i].loss_rate)
                .fold(f64::INFINITY, f64::min);
            let min_down = top_win
                .iter()
                .map(|&i| metrics[i].downside)
                .fold(f64::INFINITY, f64::min);
            let min_trap = top_win
                .iter()
                .map(|&i| metrics[i].trap_risk)
                .fold(f64::INFINITY, f64::min);

            for &i in &elig {
                let m = &metrics[i];
                let switch_turn_pen = if matches!(acts[legacy_idx], SearchChoice::Move(_))
                    && matches!(m.action, SearchChoice::Switch(_))
                {
                    0.45
                } else {
                    0.0
                };
                metrics[i].score = m.mean + 0.08 * m.win_rate
                    - 0.50 * m.downside
                    - 0.80 * m.loss_rate
                    - 0.015 * (m.win_turns + switch_turn_pen);
            }
            let leg_win = metrics[legacy_idx].win_rate;
            let leg_wt = metrics[legacy_idx].win_turns;
            let safe_winners: Vec<usize> = top_win
                .into_iter()
                .filter(|&i| {
                    metrics[i].win_rate + 0.005 >= leg_win
                        && metrics[i].loss_rate <= min_loss + 0.003
                        && metrics[i].downside <= min_down + 0.006
                        && metrics[i].trap_risk <= min_trap + 0.020
                })
                .collect();
            if let Some(&best_idx) = safe_winners
                .iter()
                .max_by(|&&a, &&b| metrics[a].score.total_cmp(&metrics[b].score))
            {
                let min_wt_gain = if matches!(acts[legacy_idx], SearchChoice::Move(_))
                    && matches!(acts[best_idx], SearchChoice::Switch(_))
                {
                    0.80
                } else {
                    0.45
                };
                let chosen_idx = if metrics[best_idx].score > metrics[legacy_idx].score + 0.003
                    && metrics[best_idx].win_turns + min_wt_gain <= leg_wt
                {
                    best_idx
                } else {
                    legacy_idx
                };
                return Some(EndgameDecisionInfo {
                    regime: EndgameRegime::FastWin,
                    mu_star,
                    legacy_idx,
                    chosen_idx,
                    actions: metrics,
                });
            }
        }

        // ---- Regime 2: Almost Winning (safe_win)
        // Guard against Jensen's inequality on 8-turn horizon cutoffs: a stalling move
        // that ends in a non-terminal heuristic leaf (R ~ 0.85) has lower variance than
        // a move that resolves terminally (90% R=1, 10% R=0), so we must never trade
        // terminal `win_rate` for unresolved horizon cutoffs. Only prefer an alternative
        // when it is well-sampled, preserves `mean` and terminal `win_rate`, and strictly
        // reduces terminal `loss_rate` or `trap_risk`.
        if rules.safe_win && mu_star >= 0.75 && mu_legacy >= 0.72 {
            let turn_w = if rules.fast_win && mu_star >= 0.90 {
                0.004
            } else {
                0.0
            };
            for &i in &elig {
                let m = &metrics[i];
                metrics[i].score = m.mean + 0.18 * m.win_rate
                    - 1.15 * m.loss_rate
                    - 0.30 * m.trap_risk
                    - turn_w * m.win_turns;
            }
            let min_safe_v = (n_max / 4).clamp(16, 160).min(n_max);
            let leg = &metrics[legacy_idx];
            let leg_score = leg.score;
            let leg_loss = leg.loss_rate;
            let leg_win = leg.win_rate;
            let leg_trap = leg.trap_risk;
            let candidates: Vec<usize> = elig
                .iter()
                .copied()
                .filter(|&i| {
                    if i == legacy_idx {
                        return true;
                    }
                    let slack = (1.25 * se_vs_legacy(i)).clamp(0.0020, 0.008);
                    total_visits[i] >= min_safe_v
                        && metrics[i].mean + slack >= mu_legacy
                        && metrics[i].win_rate + 0.015 >= leg_win
                        && metrics[i].loss_rate <= leg_loss
                        && (metrics[i].loss_rate + 0.004 <= leg_loss
                            || (metrics[i].trap_risk + 0.035 <= leg_trap
                                && metrics[i].win_rate >= leg_win + 0.005)
                            || (metrics[i].win_rate >= leg_win + 0.020
                                && metrics[i].mean >= mu_legacy))
                })
                .collect();
            if let Some(&best_idx) = candidates
                .iter()
                .max_by(|&&a, &&b| metrics[a].score.total_cmp(&metrics[b].score))
            {
                let cand = &metrics[best_idx];
                let overrides = cand.score > leg_score + 0.004;
                let chosen_idx = if overrides { best_idx } else { legacy_idx };
                return Some(EndgameDecisionInfo {
                    regime: EndgameRegime::SafeWin,
                    mu_star,
                    legacy_idx,
                    chosen_idx,
                    actions: metrics,
                });
            }
        }

        // ---- Regime 3: Losing (comeback_loss)
        // "失うもの無しにわずかな勝率を回収": only when truly losing, NEVER reduce
        // terminal `win_rate` or increase terminal `loss_rate`, never sacrifice true MCTS
        // mean beyond standard error, and only use static `desperation` when every root
        // action has 0 terminal wins and 100% terminal losses.
        let is_comeback_regime =
            mu_star <= 0.08 || (mu_star <= 0.12 && max_win_rate <= 0.015 && min_loss_rate >= 0.85);
        if rules.comeback_loss && is_comeback_regime {
            let all_guaranteed_loss = elig.iter().all(|&i| {
                self.win_n.get(i).copied().unwrap_or(0) == 0
                    && self.loss_n.get(i).copied().unwrap_or(0) == total_visits[i]
            });
            let alpha = ((0.10 - mu_star.min(0.10)) / 0.10).clamp(0.25, 1.00);
            let desp_scale = if all_guaranteed_loss { 1.0 } else { 0.0 };
            for &i in &elig {
                let m = &metrics[i];
                let m2 = if m.visits > 0 {
                    (m.downside - 1.0 + 2.0 * m.mean).max(0.0)
                } else {
                    0.0
                };
                let upside = m2.sqrt();
                let survive_norm = (m.survive_turns / 10.0).min(1.5);
                metrics[i].score = m.mean
                    + alpha
                        * (0.22 * m.mistake_ev + 0.20 * upside + 0.25 * m.win_rate
                            - 0.20 * m.loss_rate
                            + 0.030 * survive_norm
                            + 0.025 * desp_scale * m.desperation);
            }
            let min_loss_v = (n_max / 6).clamp(10, 128).min(n_max);
            let leg_win = metrics[legacy_idx].win_rate;
            let leg_loss = metrics[legacy_idx].loss_rate;
            let leg_mev = metrics[legacy_idx].mistake_ev;
            let leg_surv = metrics[legacy_idx].survive_turns;
            let candidates: Vec<usize> = elig
                .iter()
                .copied()
                .filter(|&i| {
                    if i == legacy_idx {
                        return true;
                    }
                    let slack = (1.25 * se_vs_legacy(i)).clamp(0.0015, 0.005);
                    let leg_win_n = self.win_n.get(legacy_idx).copied().unwrap_or(0);
                    let i_win_n = self.win_n.get(i).copied().unwrap_or(0);
                    let i_loss_n = self.loss_n.get(i).copied().unwrap_or(0);
                    let never_lose_all_wins =
                        leg_win_n == 0 || (i_win_n > 0 && i_loss_n < total_visits[i]);
                    let se_win = ((leg_win * (1.0 - leg_win))
                        / (total_visits[legacy_idx].max(1) as f64))
                        .sqrt();
                    let no_terminal_regression = never_lose_all_wins
                        && metrics[i].win_rate + se_win >= leg_win
                        && metrics[i].loss_rate <= leg_loss + 0.0015;
                    let has_gain = if !all_guaranteed_loss {
                        metrics[i].mistake_ev >= leg_mev + 0.075
                            || (metrics[i].win_rate >= leg_win + 0.003
                                && metrics[i].mean + 0.0002 >= mu_legacy
                                && metrics[i].mistake_ev + 0.020 >= leg_mev)
                            || (metrics[i].loss_rate + 0.005 <= leg_loss
                                && metrics[i].survive_turns >= leg_surv + 1.0)
                    } else {
                        metrics[i].mistake_ev >= leg_mev + 0.040
                            || metrics[i].survive_turns >= leg_surv + 1.0
                            || (metrics[i].mistake_ev + 0.005 >= leg_mev
                                && metrics[i].desperation >= metrics[legacy_idx].desperation + 0.15)
                    };
                    total_visits[i] >= min_loss_v
                        && metrics[i].mean + slack >= mu_legacy
                        && no_terminal_regression
                        && has_gain
                })
                .collect();
            if let Some(&best_idx) = candidates
                .iter()
                .max_by(|&&a, &&b| metrics[a].score.total_cmp(&metrics[b].score))
            {
                let chosen_idx = if metrics[best_idx].score > metrics[legacy_idx].score + 0.001 {
                    best_idx
                } else {
                    legacy_idx
                };
                return Some(EndgameDecisionInfo {
                    regime: EndgameRegime::ComebackLoss,
                    mu_star,
                    legacy_idx,
                    chosen_idx,
                    actions: metrics,
                });
            }
        }

        Some(EndgameDecisionInfo {
            regime: EndgameRegime::Normal,
            mu_star,
            legacy_idx,
            chosen_idx: legacy_idx,
            actions: metrics,
        })
    }

    pub fn select_best(
        &self,
        rules: EndgameRules,
        acts: &[SearchChoice],
        total_visits: &[u32],
        effective_visits: &[u32],
        rewards: &[f64],
        eligible: impl Fn(usize) -> bool,
        reply_metrics: impl Fn(usize, u32, f64) -> (f64, f64),
    ) -> Option<usize> {
        let k = acts.len();
        if !rules.decision_any() {
            return (0..k)
                .filter(|&i| eligible(i))
                .max_by(|&a, &b| cmp_visits_then_mean(effective_visits, total_visits, rewards, a, b));
        }
        self.analyze(
            rules,
            acts,
            total_visits,
            effective_visits,
            rewards,
            eligible,
            reply_metrics,
        )
        .map(|info| info.chosen_idx)
    }
}

pub fn reply_risk_and_mistake_ev(
    n_i: u32,
    mean_i: f64,
    cells: impl Iterator<Item = (u32, f64)>,
) -> (f64, f64) {
    let min_cell_n = (n_i / 25).clamp(5, 32);
    let mut worst_drop = 0.0_f64;
    let mut smoothed_replies: Vec<(u32, f64)> = Vec::new();
    for (cn, cw) in cells {
        if cn == 0 {
            continue;
        }
        if cn >= min_cell_n {
            let shrunk_low = (cw + 4.0 * mean_i) / (cn as f64 + 4.0);
            worst_drop = worst_drop.max((mean_i - shrunk_low).max(0.0));
        }
        let shrunk_up = (cw + 2.5 * mean_i) / (cn as f64 + 2.5);
        smoothed_replies.push((cn, shrunk_up));
    }
    let mistake_ev = if smoothed_replies.len() >= 2 {
        let min_idx = smoothed_replies
            .iter()
            .enumerate()
            .filter(|(_, (cn, _))| *cn >= 3)
            .min_by(|(_, a), (_, b)| a.1.total_cmp(&b.1))
            .or_else(|| {
                smoothed_replies
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| a.1.total_cmp(&b.1))
            })
            .map(|(idx, _)| idx)
            .unwrap_or(0);
        // Cap visit weight at `cn <= 25` so a second opponent KO move with thousands
        // of UCB visits at 27k budget does not drown out non-KO opponent mistakes.
        let mut sum_v = 0.0;
        let mut sum_w = 0.0;
        let mut max_v = mean_i;
        for (idx, &(cn, val)) in smoothed_replies.iter().enumerate() {
            if idx == min_idx {
                continue;
            }
            let w = (cn as f64).clamp(1.0, 25.0).powf(0.25);
            sum_v += w * val;
            sum_w += w;
            if val > max_v {
                max_v = val;
            }
        }
        if sum_w > 0.0 {
            0.70 * (sum_v / sum_w) + 0.30 * max_v
        } else {
            mean_i
        }
    } else {
        mean_i
    };
    (worst_drop, mistake_ev)
}


fn foe_status_or_volatile_blocked(
    b: &Battle,
    dex: &Dex,
    side: usize,
    att: PokeId,
    def: PokeId,
    key: &str,
    ms: &nc2000_engine::dex::MoveStatic,
) -> bool {
    use nc2000_engine::state::Status;
    let me = b.poke(att);
    let foe = b.poke(def);
    let foe_targeted = matches!(ms.target, "normal" | "allAdjacentFoes" | "allAdjacent");
    let foe_directed = foe_targeted && !(key == "curse" && !me.has_type(dex.known_types.ghost));
    if !foe_directed {
        return false;
    }
    let move_type = effective_move_type(b, dex, att, key, ms);
    if !ms.ignore_immunity
        && move_type != dex.known_types.unknown
        && type_immune_to(b, dex, move_type, def)
    {
        return true;
    }
    let sub_up = dex
        .conds_id("substitute")
        .is_some_and(|sid| foe.has_volatile(sid));
    if sub_up && key != "swagger" {
        return true;
    }
    let safeguard = dex
        .conds_id("safeguard")
        .is_some_and(|cid| b.sides[1 - side].has_side_condition(cid));
    if safeguard
        && (ms.status.is_some()
            || (ms.volatile_status.as_deref() == Some("confusion") && key != "swagger"))
    {
        return true;
    }
    if let Some(st) = ms.status.as_deref() {
        let check = if st == "tox" { "psn" } else { st };
        if foe.status != Status::None
            || foe.types.iter().any(|t| dex.status_key_immune(check, t))
        {
            return true;
        }
    }
    if let Some(v) = ms.volatile_status.as_deref() {
        let pure_volatile =
            !ms.has_boosts && ms.status.is_none() && ms.heal.is_none() && ms.damage.is_none();
        if (pure_volatile && dex.conds_id(v).is_some_and(|vid| foe.has_volatile(vid)))
            || (key == "leechseed" && foe.has_type(dex.known_types.grass))
            || (key == "nightmare" && foe.status != Status::Slp)
        {
            return true;
        }
    }
    false
}

/// Static desperation tie-breaker in `[0, 1]` used when a losing root is flat zero.
pub fn tactical_desperation_bonus(b: &Battle, dex: &Dex, side: usize, c: SearchChoice) -> f64 {
    use nc2000_engine::state::Status;
    let (Some(att), Some(def)) = (b.active_id(side), b.active_id(1 - side)) else {
        return 0.0;
    };
    let me = b.poke(att);
    let foe = b.poke(def);
    if me.fainted || me.hp <= 0 || foe.fainted || foe.hp <= 0 {
        return 0.0;
    }
    match c {
        SearchChoice::Move(id) => {
            let ms = dex.move_static(id);
            let key = dex.moves.key(id);
            if me.status == Status::Slp {
                return if ms.sleep_usable { 0.90 } else { 0.10 };
            }
            let mut bonus = 0.0_f64;
            if me.status == Status::Frz && matches!(key, "flamewheel" | "sacredfire") {
                bonus += 0.70;
            }
            let dmg =
                crate::eval::expected_hit_fraction(b, dex, att, def, id, true).clamp(0.0, 1.5);
            bonus += 0.45 * dmg.min(1.0);
            if ms.priority > 0 && dmg > 0.0 {
                bonus += 0.25;
            }
            if matches!(key, "protect" | "detect" | "endure") {
                bonus += 0.20;
            }
            if (ms.status.is_some() || ms.volatile_status.is_some())
                && !foe_status_or_volatile_blocked(b, dex, side, att, def, key, ms)
            {
                bonus += 0.25;
            }
            if ms.has_boosts || matches!(key, "curse" | "bellydrum") {
                bonus += 0.20;
            }
            if !ms.secondaries.is_empty() && dmg > 0.0 {
                bonus += 0.10;
            }
            bonus.clamp(0.0, 1.0)
        }
        SearchChoice::Switch(pos) => {
            let Some(&slot) = b.sides[side].party.get(pos as usize - 1) else {
                return 0.0;
            };
            let sub = &b.sides[side].roster[slot as usize];
            if sub.fainted || sub.hp <= 0 {
                return 0.0;
            }
            let has_spikes = dex
                .conds_id("spikes")
                .is_some_and(|cid| b.sides[side].has_side_condition(cid));
            let spikes_dmg = if has_spikes && !sub.has_type(dex.known_types.flying) {
                (sub.maxhp / 8).max(1)
            } else {
                0
            };
            let net_hp = sub.hp - spikes_dmg;
            if net_hp <= 0 {
                return 0.0;
            }
            let hp_frac = (net_hp as f64 / sub.maxhp.max(1) as f64).clamp(0.0, 1.0);
            (0.10 + 0.15 * hp_frac).clamp(0.0, 0.25)
        }
        SearchChoice::Team(_) | SearchChoice::Pass => 0.0,
    }
}


/// Unified post-search root decision selector for non-preview imperfect-information roots.
///
/// Evaluates `EndgameRootStats` once:
/// 1. `FastWin`, `SafeWin`, and `ComebackLoss` regimes return `chosen_idx` directly
///    (protecting certified/near-won/desperate roots from equilibrium override).
/// 2. `Normal` regime (or `EndgameRules::none()`) computes `base_idx` via
///    `cmp_visits_then_mean`, then allows `equilibrium_override` to replace it only
///    when `eq.override_value > eq.pure_guarantees[base_idx] + MIN_MIX_MARGIN`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn select_blind_root_choice(
    stats: &EndgameRootStats,
    rules: EndgameRules,
    acts: &[SearchChoice],
    total_visits: &[u32],
    effective_visits: &[u32],
    rewards: &[f64],
    eligible: impl Fn(usize) -> bool,
    reply_metrics: impl Fn(usize, u32, f64) -> (f64, f64),
    equilibrium_override: impl FnOnce(usize) -> Option<SearchChoice>,
) -> Option<SearchChoice> {
    let base_idx = if rules.decision_any() {
        let info = stats.analyze(
            rules,
            acts,
            total_visits,
            effective_visits,
            rewards,
            &eligible,
            reply_metrics,
        )?;
        if matches!(
            info.regime,
            EndgameRegime::FastWin | EndgameRegime::SafeWin | EndgameRegime::ComebackLoss
        ) {
            return Some(acts[info.chosen_idx]);
        }
        info.chosen_idx
    } else {
        (0..acts.len())
            .filter(|&i| eligible(i))
            .max_by(|&a, &b| cmp_visits_then_mean(effective_visits, total_visits, rewards, a, b))?
    };
    if let Some(choice) = equilibrium_override(base_idx) {
        return Some(choice);
    }
    Some(acts[base_idx])
}

#[cfg(test)]
mod tests {
    use super::*;
    use nc2000_engine::battle::{EffectHandle, PokemonSet};

    fn team() -> Vec<PokemonSet> {
        // from_fixture does not validate movesets — purpose-built slots
        serde_json::from_str(
            r#"[
            {"name":"Exeggutor","species":"Exeggutor","item":"","ability":"No Ability",
             "moves":["Sleep Powder","Reflect","Rest","Spikes"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Snorlax","species":"Snorlax","item":"","ability":"No Ability",
             "moves":["Body Slam","Substitute","Confuse Ray","Explosion"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Cloyster","species":"Cloyster","item":"","ability":"No Ability",
             "moves":["Surf","Ice Beam","Screech","Toxic"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}
        ]"#,
        )
        .unwrap()
    }

    fn setup() -> (Dex, Battle) {
        let dex = conformance::load_dex();
        let t = team();
        let mut b = Battle::from_fixture(&dex, "7,8,9,10", &t, &t).unwrap();
        b.set_log_enabled(false);
        b.choose(&dex, 0, "team 1, 2, 3").unwrap();
        b.choose(&dex, 1, "team 2, 1, 3").unwrap();
        (dex, b)
    }

    fn mv(dex: &Dex, key: &str) -> SearchChoice {
        SearchChoice::Move(dex.moves.id(key).unwrap())
    }

    #[test]
    fn tactical_desperation_bonus_blocks_useless_status_and_accounts_for_spikes() {
        let mine: Vec<PokemonSet> = serde_json::from_str(
            r#"[
            {"name":"Zapdos","species":"Zapdos","item":"","ability":"No Ability",
             "moves":["Thunder Wave","Confuse Ray","Swagger","Thunderbolt"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"N","level":50},
            {"name":"Snorlax","species":"Snorlax","item":"","ability":"No Ability",
             "moves":["Body Slam"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Skarmory","species":"Skarmory","item":"","ability":"No Ability",
             "moves":["Drill Peck"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}
        ]"#,
        )
        .unwrap();
        let foes: Vec<PokemonSet> = serde_json::from_str(
            r#"[{"name":"Nidoking","species":"Nidoking","item":"","ability":"No Ability",
              "moves":["Earthquake"],
              "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}]"#,
        )
        .unwrap();
        let dex = conformance::load_dex();
        let mut b = Battle::from_fixture(&dex, "1,2,3,4", &mine, &foes).unwrap();
        b.choose(&dex, 0, "team 1,2,3").unwrap();
        b.choose(&dex, 1, "team 1").unwrap();

        // Thunder Wave vs Ground Nidoking is visibly blocked -> bonus == 0.0.
        assert_eq!(
            super::tactical_desperation_bonus(&b, &dex, 0, mv(&dex, "thunderwave")),
            0.0
        );
        // Unblocked Confuse Ray vs Nidoking gets +0.25.
        assert_eq!(
            super::tactical_desperation_bonus(&b, &dex, 0, mv(&dex, "confuseray")),
            0.25
        );
        // Under Safeguard, Confuse Ray drops to 0.0 while Swagger keeps +0.25.
        let att = b.active_id(0).unwrap();
        let def = b.active_id(1).unwrap();
        let mut b_sg = b.clone();
        b_sg.add_side_condition(&dex, 1, "safeguard", Some(def), EffectHandle::None);
        assert_eq!(
            super::tactical_desperation_bonus(&b_sg, &dex, 0, mv(&dex, "confuseray")),
            0.0
        );
        assert_eq!(
            super::tactical_desperation_bonus(&b_sg, &dex, 0, mv(&dex, "swagger")),
            0.45
        );

        // Switch bonus ceiling is 0.25 and Spikes penalizes grounded Snorlax (and returns 0.0 on lethal).
        let sw_lax = super::tactical_desperation_bonus(&b, &dex, 0, SearchChoice::Switch(2));
        assert!((sw_lax - 0.25).abs() < 1e-9);
        b.add_side_condition(&dex, 0, "spikes", Some(att), EffectHandle::None);
        let sw_lax_spikes =
            super::tactical_desperation_bonus(&b, &dex, 0, SearchChoice::Switch(2));
        let sw_skarm_spikes =
            super::tactical_desperation_bonus(&b, &dex, 0, SearchChoice::Switch(3));
        assert!(sw_lax_spikes < 0.25);
        assert!((sw_skarm_spikes - 0.25).abs() < 1e-9);

        let lax_slot = b.sides[0].party[1] as usize;
        b.sides[0].roster[lax_slot].hp = 1;
        assert_eq!(
            super::tactical_desperation_bonus(&b, &dex, 0, SearchChoice::Switch(2)),
            0.0
        );
    }

    #[test]
    fn endgame_rules_fast_win_safe_win_comeback_loss_and_early_stop() {
        let (dex, b) = setup();
        let acts = vec![mv(&dex, "agility"), mv(&dex, "thunderbolt")];

        // 1. FastWin: both moves win 100% (mean = 1.0, loss = 0), but action 0 (Agility)
        //    wins in 4 turns with 155 visits while action 1 (Thunderbolt) wins in 1 turn
        //    with 145 visits.
        let mut stats = EndgameRootStats::new(&b, &dex, 0, &acts);
        for _ in 0..155 {
            stats.record(
                b.turn,
                0,
                0,
                1.0,
                crate::mcts::SimEndInfo {
                    turn: b.turn + 3,
                    outcome: 1,
                    alive: [2, 0],
                },
            );
        }
        for _ in 0..145 {
            stats.record(
                b.turn,
                0,
                1,
                1.0,
                crate::mcts::SimEndInfo {
                    turn: b.turn,
                    outcome: 1,
                    alive: [2, 0],
                },
            );
        }
        let n = [155u32, 145];
        let w = [155.0f64, 145.0];
        // `EndgameRules::none()` (ablation) preserves legacy argmax-visits (index 0),
        // while `EndgameRules::default()` (shipped) selects the 1-turn KO (index 1).
        assert_eq!(
            stats.select_best(
                EndgameRules::none(),
                &acts,
                &n,
                &n,
                &w,
                |_| true,
                |_, _, m| (0.0, m)
            ),
            Some(0)
        );
        assert_eq!(
            stats.select_best(
                EndgameRules::default(),
                &acts,
                &n,
                &n,
                &w,
                |_| true,
                |_, _, m| (0.0, m)
            ),
            Some(1)
        );
        // FastWin picks index 1 (1-turn KO)
        let info = stats
            .analyze(
                EndgameRules {
                    fast_win: true,
                    ..EndgameRules::none()
                },
                &acts,
                &n,
                &n,
                &w,
                |_| true,
                |_, _, m| (0.0, m),
            )
            .unwrap();
        assert_eq!(info.regime, EndgameRegime::FastWin);
        assert_eq!(info.legacy_idx, 0);
        assert_eq!(info.chosen_idx, 1);
        assert!(
            stats.should_early_stop(
                EndgameRules {
                    fast_win: true,
                    early_stop: true,
                    ..EndgameRules::none()
                },
                128,
                300,
                false,
                &n,
                &w,
                |_| true,
            ),
            "early_stop triggers when a 1-turn 100%-win KO is locked in"
        );

        // 2. SafeWin: action 0 has slightly higher mean (0.9125) and visits (160) but 8.75%
        //    catastrophic terminal losses, whereas action 1 has mean 0.907 (140 visits) with
        //    90% terminal wins and 0% terminal losses.
        let mut safe_stats = EndgameRootStats::new(&b, &dex, 0, &acts);
        for k in 0..160 {
            let win = k < 146;
            safe_stats.record(
                b.turn,
                0,
                0,
                if win { 1.0 } else { 0.0 },
                crate::mcts::SimEndInfo {
                    turn: b.turn + 2,
                    outcome: if win { 1 } else { 2 },
                    alive: if win { [1, 0] } else { [0, 1] },
                },
            );
        }
        for k in 0..140 {
            let r = if k < 126 { 1.0 } else { 0.07 };
            safe_stats.record(
                b.turn,
                0,
                1,
                r,
                crate::mcts::SimEndInfo {
                    turn: b.turn + 2,
                    outcome: if k < 126 { 1 } else { 0 },
                    alive: [2, 1],
                },
            );
        }
        let sn = [160u32, 140];
        let sw = [146.0f64, 126.0 + 14.0 * 0.07];
        let safe_info = safe_stats
            .analyze(
                EndgameRules {
                    safe_win: true,
                    ..EndgameRules::none()
                },
                &acts,
                &sn,
                &sn,
                &sw,
                |_| true,
                |i, _, m| if i == 0 { (0.25, m) } else { (0.0, m) },
            )
            .unwrap();
        assert_eq!(safe_info.regime, EndgameRegime::SafeWin);
        assert_eq!(safe_info.legacy_idx, 0);
        assert_eq!(
            safe_info.chosen_idx, 1,
            "SafeWin eliminates the 8.75% loss path"
        );

        // 3. ComebackLoss: both moves have 0.02 mean against opponent best play,
        //    but action 1 survives 5 turns and recovers 0.45 when opponent errs.
        let mut loss_stats = EndgameRootStats::new(&b, &dex, 0, &acts);
        for _ in 0..155 {
            loss_stats.record(
                b.turn,
                0,
                0,
                0.02,
                crate::mcts::SimEndInfo {
                    turn: b.turn,
                    outcome: 2,
                    alive: [0, 2],
                },
            );
        }
        for _ in 0..145 {
            loss_stats.record(
                b.turn,
                0,
                1,
                0.02,
                crate::mcts::SimEndInfo {
                    turn: b.turn + 4,
                    outcome: 2,
                    alive: [0, 1],
                },
            );
        }
        let ln = [155u32, 145];
        let lw = [155.0 * 0.02, 145.0 * 0.02];
        let loss_info = loss_stats
            .analyze(
                EndgameRules {
                    comeback_loss: true,
                    ..EndgameRules::none()
                },
                &acts,
                &ln,
                &ln,
                &lw,
                |_| true,
                |i, _, _| if i == 0 { (0.0, 0.02) } else { (0.0, 0.45) },
            )
            .unwrap();
        assert_eq!(loss_info.regime, EndgameRegime::ComebackLoss);
        assert_eq!(loss_info.legacy_idx, 0);
        assert_eq!(
            loss_info.chosen_idx, 1,
            "ComebackLoss picks the high-mistake-EV line"
        );

        // 4. Side 1 symmetry: verify `my_reward` is not double-flipped for side == 1.
        let mut s1_stats = EndgameRootStats::new(&b, &dex, 1, &acts);
        for _ in 0..80 {
            s1_stats.record(
                b.turn,
                1,
                0,
                1.0,
                crate::mcts::SimEndInfo {
                    turn: b.turn + 3,
                    outcome: 2,
                    alive: [0, 2],
                },
            );
            s1_stats.record(
                b.turn,
                1,
                1,
                1.0,
                crate::mcts::SimEndInfo {
                    turn: b.turn,
                    outcome: 2,
                    alive: [0, 2],
                },
            );
        }
        let s1_n = [80u32, 80];
        let s1_w = [80.0f64, 80.0];
        let s1_info = s1_stats
            .analyze(
                EndgameRules::all_decision(),
                &acts,
                &s1_n,
                &s1_n,
                &s1_w,
                |_| true,
                |_, _, m| (0.0, m),
            )
            .unwrap();
        assert_eq!(s1_info.regime, EndgameRegime::FastWin);
        assert_eq!(
            s1_info.chosen_idx, 1,
            "Side 1 FastWin selects the 1-turn KO"
        );
        assert!((s1_info.actions[1].win_rate - 1.0).abs() < 1e-9);
        assert_eq!(s1_info.actions[1].loss_rate, 0.0);
    }
}
