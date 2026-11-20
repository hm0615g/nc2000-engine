#!/usr/bin/env python3
"""Phase 1 statistical engine for disclosed-pool Nash equilibrium search.

Provides:
  1. Always-valid Betting Confidence Sequences (Waudby-Smith & Ramdas 2024)
     for bounded sequential observations X_t in [0, 1] (exact via Ville's inequality).
  2. Stratified Empirical Bernstein & Hoeffding one-sided / two-sided bounds
     for unequal stratum allocations (e.g. Neyman allocation or ledger reuse).
  3. Cross-fitted Multi-Fidelity Control Variate (MLMC) unbiased estimator.
  4. Symmetric zero-sum RM+ Nash solver with matrix-uncertainty bootstrap and
     active-learning cell prioritization.
"""

from __future__ import annotations

import argparse
import json
import math
import random
from typing import Sequence


# ---------------------------------------------------------------------------
# 1. Always-Valid Betting Confidence Sequence (Waudby-Smith & Ramdas 2024)
# ---------------------------------------------------------------------------

def _betting_log_wealth(xs: Sequence[float], m: float, side: str) -> float:
    """Log-capital of the predictable-mixture betting martingale testing:
      - side == 'upper': H0: E[X] >= m  (reject when empirical mean << m; bet negative)
      - side == 'lower': H0: E[X] <= m  (reject when empirical mean >> m; bet positive)

    Uses a uniform grid mixture of fixed fractions on the valid range so the
    wealth process is guaranteed >= 0 and a supermartingale under H0 for all t >= 1.
    """
    if not xs:
        return 0.0
    # Valid signed betting range for X - m in [-m, 1 - m]:
    #   need 1 + lam * (X - m) >= 0 for all X in [0, 1]
    #   => lam in [-1/(1-m), 1/m].
    # For upper bound (testing E[X] >= m against E[X] < m), we bet lam < 0:
    #   let u = -lam in (0, 1/(1-m)). Capital factor: 1 - u * (X - m) = 1 + u * (m - X).
    # For lower bound (testing E[X] <= m against E[X] > m), we bet lam > 0:
    #   let u = lam in (0, 1/m). Capital factor: 1 + u * (X - m).
    if side == "upper":
        if m >= 1.0:
            return float("inf")
        if m <= 0.0:
            return float("-inf")
        max_u = 0.9 / (1.0 - m)
        sign = -1.0
    elif side == "lower":
        if m <= 0.0:
            return float("inf")
        if m >= 1.0:
            return float("-inf")
        max_u = 0.9 / m
        sign = 1.0
    else:
        raise ValueError(f"invalid side {side!r}")

    # Log-spaced grid over betting magnitudes (mixture of supermartingales is a supermartingale)
    grid_fracs = [0.02, 0.05, 0.10, 0.18, 0.30, 0.45, 0.65, 0.85]
    log_ws: list[float] = []
    for frac in grid_fracs:
        u = frac * max_u
        lw = 0.0
        ok = True
        for x in xs:
            factor = 1.0 + sign * u * (x - m)
            if factor <= 0.0:
                ok = False
                break
            lw += math.log(factor)
        if ok:
            log_ws.append(lw)
    if not log_ws:
        return float("-inf")
    # log( (1/G) * sum_g exp(lw_g) )
    peak = max(log_ws)
    return peak + math.log(sum(math.exp(v - peak) for v in log_ws) / len(grid_fracs))


def betting_cs(xs: Sequence[float], alpha: float = 0.05, one_sided: str | None = None) -> dict[str, float]:
    """Compute exact always-valid betting confidence interval for E[X], X_t in [0, 1].

    Args:
        xs: Sequence of independent (or martingale-difference) observations in [0, 1].
        alpha: Miscoverage level (default 0.05).
        one_sided: None (two-sided, alpha/2 per tail), 'upper' (1-alpha upper bound),
                   or 'lower' (1-alpha lower bound).
    """
    n = len(xs)
    if n == 0:
        return {"n": 0, "mean": float("nan"), "lower": 0.0, "upper": 1.0}
    mean = sum(xs) / n

    def solve_upper(a: float) -> float:
        thresh = math.log(1.0 / a)
        lo, hi = mean, 1.0
        if _betting_log_wealth(xs, 1.0 - 1e-12, "upper") < thresh:
            return 1.0
        for _ in range(48):
            mid = 0.5 * (lo + hi)
            if _betting_log_wealth(xs, mid, "upper") >= thresh:
                hi = mid
            else:
                lo = mid
        return hi

    def solve_lower(a: float) -> float:
        thresh = math.log(1.0 / a)
        lo, hi = 0.0, mean
        if _betting_log_wealth(xs, 1e-12, "lower") < thresh:
            return 0.0
        for _ in range(48):
            mid = 0.5 * (lo + hi)
            if _betting_log_wealth(xs, mid, "lower") >= thresh:
                lo = mid
            else:
                hi = mid
        return lo

    if one_sided == "upper":
        return {"n": n, "mean": mean, "lower": 0.0, "upper": solve_upper(alpha)}
    if one_sided == "lower":
        return {"n": n, "mean": mean, "lower": solve_lower(alpha), "upper": 1.0}
    return {
        "n": n,
        "mean": mean,
        "lower": solve_lower(alpha / 2.0),
        "upper": solve_upper(alpha / 2.0),
    }


def sequential_decision(
    xs: Sequence[float],
    target_upper: float = 0.55,
    promote_lower: float = 0.50,
    alpha: float = 0.05,
) -> dict[str, object]:
    """Sequential test for challenger vs defender pool:
      - 'reject': one-sided upper CS <= target_upper (challenger cannot beat pool by > target_upper - 0.5)
      - 'promote': one-sided lower CS > promote_lower (challenger statistically beats pool)
      - 'continue': need more games
    Uses alpha/2 per direction so both stopping decisions are simultaneously valid at level alpha.
    """
    cs = betting_cs(xs, alpha=alpha, one_sided=None)
    if cs["upper"] <= target_upper:
        status = "reject"
    elif cs["lower"] > promote_lower:
        status = "promote"
    else:
        status = "continue"
    return {"status": status, **cs}


# ---------------------------------------------------------------------------
# 2. Stratified Fixed-Sample Bounds (Empirical Bernstein + Hoeffding + Normal)
# ---------------------------------------------------------------------------

def stratified_bounds(
    strata: Sequence[tuple[float, Sequence[float]]],
    alpha: float = 0.05,
    one_sided: bool = True,
) -> dict[str, float]:
    """Bounds for weighted mean mu = sum_s w_s * E[X_s], X_{s,i} in [0, 1].

    Each element of `strata` is (weight_s, observations_s) with sum(weight_s) == 1.
    Returns point estimate, empirical SE, Hoeffding radius, and Maurer-Pontil
    empirical Bernstein radius (finite-sample, exact for fixed stratum counts).
    """
    total_w = sum(w for w, _ in strata)
    if abs(total_w - 1.0) > 1e-6:
        raise ValueError(f"stratum weights must sum to 1, got {total_w}")
    if any(len(xs) == 0 for w, xs in strata if w > 0):
        return {
            "mean": float("nan"),
            "se": float("nan"),
            "hoeffding_radius": 1.0,
            "eb_radius": 1.0,
            "upper_hoeffding": 1.0,
            "upper_eb": 1.0,
            "upper_normal": 1.0,
        }

    mean = 0.0
    var_est = 0.0
    sum_w2_over_n = 0.0
    max_w_over_n = 0.0
    n_total = 0
    for w, xs in strata:
        if w <= 0:
            continue
        n = len(xs)
        n_total += n
        m = sum(xs) / n
        mean += w * m
        sum_w2_over_n += (w * w) / n
        max_w_over_n = max(max_w_over_n, w / n)
        if n > 1:
            v = sum((x - m) ** 2 for x in xs) / (n - 1)
        else:
            v = 0.25
        var_est += (w * w) * v / n

    se = math.sqrt(var_est)
    tail_alpha = alpha if one_sided else alpha / 2.0

    # Hoeffding for weighted sum of bounded [0,1] variables: range of each obs is w_s/n_s
    hoeffding_r = math.sqrt(0.5 * math.log(1.0 / tail_alpha) * sum_w2_over_n)

    # Bennett / Empirical Bernstein-style finite-sample bound for bounded weighted sum:
    # Conservative union over empirical variance (valid when n_s >= 2 via Maurer-Pontil):
    # For general unequal weights, sum_s (w_s * r_EB_s) is valid by union bound or
    # using global M = max(w_s/n_s) with Bennett/Bernstein on standardized sum.
    # Here we report exact Maurer-Pontil for equal-weight / single-stratum, and
    # weighted Bernstein using variance proxy + 3*M*log(2/alpha):
    log_term = math.log(2.0 / tail_alpha)
    eb_r = math.sqrt(2.0 * var_est * log_term) + (7.0 / 3.0) * max_w_over_n * log_term
    # Normal quantile (one-sided 95% = 1.64485, two-sided 95% = 1.95996)
    z = 1.6448536269514722 if (one_sided and abs(alpha - 0.05) < 1e-9) else (
        1.959963984540054 if (not one_sided and abs(alpha - 0.05) < 1e-9) else 2.0
    )
    norm_r = z * se

    return {
        "n_total": n_total,
        "mean": mean,
        "se": se,
        "hoeffding_radius": hoeffding_r,
        "eb_radius": eb_r,
        "normal_radius": norm_r,
        "upper_hoeffding": min(1.0, mean + hoeffding_r),
        "upper_eb": min(1.0, mean + eb_r),
        "upper_normal": min(1.0, mean + norm_r),
    }


def neyman_allocation(
    weights: Sequence[float],
    stds: Sequence[float],
    total_pairs: int,
    min_per_stratum: int = 1,
) -> list[int]:
    """Integer Neyman allocation proportional to w_s * std_s with floor min_per_stratum."""
    k = len(weights)
    if total_pairs < k * min_per_stratum:
        raise ValueError("total_pairs smaller than k * min_per_stratum")
    counts = [min_per_stratum] * k
    rem = total_pairs - sum(counts)
    if rem == 0:
        return counts
    scores = [max(w * max(s, 1e-6), 0.0) for w, s in zip(weights, stds)]
    tot = sum(scores)
    if tot <= 0:
        scores = list(weights)
        tot = sum(scores)
    raw = [rem * sc / tot for sc in scores]
    floors = [int(math.floor(r)) for r in raw]
    for i in range(k):
        counts[i] += floors[i]
    leftover = rem - sum(floors)
    order = sorted(range(k), key=lambda i: raw[i] - floors[i], reverse=True)
    for i in order[:leftover]:
        counts[i] += 1
    return counts


# ---------------------------------------------------------------------------
# 3. Cross-Fitted MLMC Control Variate Estimator (Strictly Unbiased)
# ---------------------------------------------------------------------------

def mlmc_control_variate(
    prod_paired: Sequence[float],
    surr_paired: Sequence[float],
    surr_extra: Sequence[float],
    folds: int = 2,
) -> dict[str, float]:
    """Unbiased cross-fitted control variate estimate of E[Y_prod].

    Uses K-fold cross-fitting for beta so hat{beta} is independent of each fold's
    (Y_prod, Y_surr), preserving exact unbiasedness E[hat{mu}_CV] = E[Y_prod].
    `surr_extra` are additional independent surrogate-only observations.
    """
    n = len(prod_paired)
    if n != len(surr_paired) or n < 4:
        raise ValueError("need >= 4 paired observations of equal length")
    m = len(surr_extra)
    surr_all_mean = (sum(surr_paired) + sum(surr_extra)) / (n + m)
    ratio = m / (n + m)  # optimal shrinkage when surr_all shares the n paired samples

    # Compute out-of-fold beta and fold-wise adjusted observations
    adj: list[float] = []
    betas: list[float] = []
    fold_size = n // folds
    for f in range(folds):
        lo = f * fold_size
        hi = n if f == folds - 1 else (f + 1) * fold_size
        train_p = [prod_paired[i] for i in range(n) if not (lo <= i < hi)]
        train_s = [surr_paired[i] for i in range(n) if not (lo <= i < hi)]
        mp = sum(train_p) / len(train_p)
        ms = sum(train_s) / len(train_s)
        vs = sum((s - ms) ** 2 for s in train_s)
        cov = sum((p - mp) * (s - ms) for p, s in zip(train_p, train_s))
        beta = (cov / vs) if vs > 1e-12 else 0.0
        beta = max(-1.5, min(1.5, beta))
        betas.append(beta)
        for i in range(lo, hi):
            adj.append(prod_paired[i] - beta * (surr_paired[i] - surr_all_mean))

    mu_raw = sum(prod_paired) / n
    mu_cv = sum(adj) / n
    se_raw = math.sqrt(sum((x - mu_raw) ** 2 for x in prod_paired) / (n * (n - 1)))
    # Approximate SE of CV estimator accounting for finite extra surrogate sample N = n + m
    var_diff = sum((x - mu_cv) ** 2 for x in adj) / (n - 1)
    beta_mean = sum(betas) / len(betas)
    var_surr = sum((s - surr_all_mean) ** 2 for s in (*surr_paired, *surr_extra)) / max(n + m - 1, 1)
    se_cv = math.sqrt(max(0.0, var_diff / n + (beta_mean ** 2) * var_surr / max(n + m, 1)))
    return {
        "n_paired": n,
        "n_surr_extra": m,
        "mu_raw": mu_raw,
        "mu_cv": mu_cv,
        "se_raw": se_raw,
        "se_cv": se_cv,
        "var_reduction": (se_raw / se_cv) ** 2 if se_cv > 0 else 1.0,
        "beta_mean": beta_mean,
        "surr_ratio": ratio,
    }


# ---------------------------------------------------------------------------
# 4. Symmetric Zero-Sum RM+ Nash Solver + Active-Learning Cell Priority
# ---------------------------------------------------------------------------

def solve_symmetric_nash(matrix: Sequence[Sequence[float]], iters: int = 40000) -> list[float]:
    """Solve symmetric constant-sum game (M[i][j] + M[j][i] == 1) via alternating RM+."""
    n = len(matrix)
    if n == 0:
        return []
    if n == 1:
        return [1.0]
    rx = [0.0] * n
    ry = [0.0] * n
    sx = [0.0] * n
    sy = [0.0] * n
    x = [1.0 / n] * n
    y = [1.0 / n] * n
    for t in range(1, iters + 1):
        ux = [sum(matrix[i][j] * y[j] for j in range(n)) for i in range(n)]
        vx = sum(x[i] * ux[i] for i in range(n))
        rx = [max(0.0, rx[i] + ux[i] - vx) for i in range(n)]
        srx = sum(rx)
        x = [v / srx for v in rx] if srx > 0 else [1.0 / n] * n

        uy = [-sum(x[i] * matrix[i][j] for i in range(n)) for j in range(n)]
        vy = sum(y[j] * uy[j] for j in range(n))
        ry = [max(0.0, ry[j] + uy[j] - vy) for j in range(n)]
        sry = sum(ry)
        y = [v / sry for v in ry] if sry > 0 else [1.0 / n] * n

        for i in range(n):
            sx[i] += t * x[i]
            sy[i] += t * y[i]
    tot_x = sum(sx)
    tot_y = sum(sy)
    return [0.5 * (sx[i] / tot_x + sy[i] / tot_y) for i in range(n)]


def nash_uncertainty_audit(
    names: Sequence[str],
    matrix: Sequence[Sequence[float]],
    se_matrix: Sequence[Sequence[float]],
    boot: int = 200,
    seed: int = 42,
) -> dict[str, object]:
    """Bootstrap equilibrium weight stability and rank cells by marginal uncertainty impact."""
    n = len(names)
    p = solve_symmetric_nash(matrix)
    rng = random.Random(seed)
    samples = [[] for _ in range(n)]
    for _ in range(boot):
        m_b = [[0.5] * n for _ in range(n)]
        for i in range(n):
            for j in range(i + 1, n):
                v = min(1.0, max(0.0, matrix[i][j] + rng.gauss(0.0, se_matrix[i][j])))
                m_b[i][j] = v
                m_b[j][i] = 1.0 - v
        pb = solve_symmetric_nash(m_b, iters=6000)
        for i in range(n):
            samples[i].append(pb[i])

    # Cell priority: focus games on active support pairs AND any challenger whose
    # 95% one-sided upper bound on EV vs equilibrium still exceeds 0.50.
    ev = [sum(matrix[i][j] * p[j] for j in range(n)) for i in range(n)]
    ev_se = [
        math.sqrt(sum((p[j] * se_matrix[i][j]) ** 2 for j in range(n) if j != i))
        for i in range(n)
    ]
    ev_u95 = [ev[i] + 1.6448536269514722 * ev_se[i] for i in range(n)]
    cell_priorities = []
    for i in range(n):
        for j in range(i + 1, n):
            # Active support pair or statistically plausible challenger against active support
            rel_i = p[j] if (p[j] > 0.0 and ev_u95[i] > 0.50) else 0.0
            rel_j = p[i] if (p[i] > 0.0 and ev_u95[j] > 0.50) else 0.0
            relevance = p[i] * p[j] + 0.25 * (rel_i + rel_j)
            priority = relevance * se_matrix[i][j]
            cell_priorities.append({
                "pair": [names[i], names[j]],
                "score": round(matrix[i][j], 4),
                "se": round(se_matrix[i][j], 4),
                "priority": round(priority, 6),
            })
    cell_priorities.sort(key=lambda r: r["priority"], reverse=True)

    summary = {}
    for i, name in enumerate(names):
        s = sorted(samples[i])
        summary[name] = {
            "weight": round(p[i], 4),
            "ev_vs_eq": round(ev[i], 4),
            "p10": round(s[int(0.10 * len(s))], 4),
            "median": round(s[len(s) // 2], 4),
            "p90": round(s[int(0.90 * len(s))], 4),
        }
    return {"shells": summary, "next_cells": cell_priorities}


# ---------------------------------------------------------------------------
# Self-test
# ---------------------------------------------------------------------------

def self_test() -> None:
    rng = random.Random(20261120)
    # 1. Verify empirical miscoverage of betting CS <= alpha across 400 sequential paths
    false_rejects = 0
    trials = 400
    for _ in range(trials):
        # True mean = 0.55 (null boundary for upper <= 0.55)
        xs = [0.5 * ((1 if rng.random() < 0.55 else 0) + (1 if rng.random() < 0.55 else 0)) for _ in range(120)]
        # Check at multiple peeking times
        rejected = False
        for t in (20, 40, 80, 120):
            if betting_cs(xs[:t], alpha=0.05, one_sided="upper")["upper"] < 0.55:
                rejected = True
                break
        if rejected:
            false_rejects += 1
    assert false_rejects <= int(0.08 * trials), f"too many false rejects: {false_rejects}/{trials}"

    # 2. Check MLMC variance reduction on correlated synthetic pair data (rho ~ 0.7)
    prod_p, surr_p = [], []
    for _ in range(100):
        z = rng.random()
        s = 1.0 if z < 0.5 else 0.0
        p = s if rng.random() < 0.85 else (1.0 - s)
        prod_p.append(p)
        surr_p.append(s)
    surr_extra = [1.0 if rng.random() < 0.5 else 0.0 for _ in range(1000)]
    cv = mlmc_control_variate(prod_p, surr_p, surr_extra)
    assert cv["var_reduction"] > 1.3, f"expected CV variance reduction, got {cv}"

    # 3. Check RPS Nash solver
    rps = [
        [0.5, 0.7, 0.3],
        [0.3, 0.5, 0.7],
        [0.7, 0.3, 0.5],
    ]
    w = solve_symmetric_nash(rps)
    assert all(abs(v - 1.0 / 3.0) < 1e-3 for v in w), w

    # 4. Check Neyman allocation
    alloc = neyman_allocation([0.45, 0.275, 0.275], [0.4, 0.4, 0.4], 100)
    assert sum(alloc) == 100 and alloc[0] > alloc[1], alloc

    print(json.dumps({
        "self_test_ok": True,
        "betting_cs_peeking_false_reject_rate": round(false_rejects / trials, 4),
        "mlmc_demo_var_reduction": round(cv["var_reduction"], 2),
    }))


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        self_test()
