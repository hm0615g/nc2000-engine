"""Independent small-game witnesses for information-constrained solving."""
import copy

import numpy as np
import pytest

from gen2_tpu.ico import arrays, baseline_game, best_response, compare, evaluate, realization, solve, validate


def matrix_game(matrix, baseline=(1., 0.)):
    m = np.asarray(matrix)
    players = []
    for side, n in enumerate(m.shape):
        info = dict(key=f"player-{side}-observation", parent=0,
                    sequences=list(range(1, n+1)), actions=[str(i) for i in range(n)])
        if side == 0:
            info["baseline"] = list(baseline)
        players.append(dict(sequence_count=n+1, infosets=[info]))
    return dict(format="nc2000-ico-sequence-v1", players=players,
                payoffs=[[i+1, j+1, 1., float(m[i,j]), float(m[i,j]), float(m[i,j])]
                         for i in range(m.shape[0]) for j in range(m.shape[1])])


def test_matching_pennies_and_baseline_loss():
    g = matrix_game([[1, 0], [0, 1]])
    r = compare(g, 1024)
    assert r["unrestricted"]["lower"] == pytest.approx(.5)
    assert r["unrestricted"]["upper"] == pytest.approx(.5)
    lo, hi = r["estimated_root_loss_interval"]
    assert lo > .49 and hi <= .501
    assert r["sampled_game_frontier_interval"] == pytest.approx([.5, .5])


def test_known_asymmetric_matrix_value_and_exploitability():
    g = matrix_game([[.9, .2], [.1, .8]])
    r = solve(g, 4096)
    assert r["lower"] <= .5 <= r["upper"]
    assert r["saddle_gap"] < .01
    assert r["policies"][0][0] == pytest.approx([.5, .5], abs=.01)
    assert r["policies"][1][0] == pytest.approx([3/7, 4/7], abs=.01)


def hidden_bit_game(revealed=False):
    # Chance chooses an unobservable bit, then player 0 guesses it. Solving
    # each world independently (the forbidden full-state teacher) scores 1.
    infos = [dict(key="same-visible-history", parent=0,
                  sequences=[1,2], actions=["guess-0", "guess-1"])]
    if revealed:
        infos.append(dict(key="bit-1-revealed", parent=0,
                          sequences=[3,4], actions=["guess-0", "guess-1"]))
    rows = []
    for bit in range(2):
        for guess in range(2):
            score = float(bit == guess)
            seq = 1+guess+(2*bit if revealed else 0)
            rows.append([seq, 0, .5, score, score, score])
    return dict(format="nc2000-ico-sequence-v1", players=[
        dict(sequence_count=5 if revealed else 3, infosets=infos),
        dict(sequence_count=1, infosets=[])], payoffs=rows)


def test_hidden_worlds_cannot_choose_different_actions_but_reveals_can():
    hidden, revealed = solve(hidden_bit_game(), 128), solve(hidden_bit_game(True), 128)
    assert hidden["value"] == pytest.approx(.5)
    assert revealed["value"] > .999
    # Unseen-world enumeration order must not change policy.
    g = hidden_bit_game()
    g["payoffs"].reverse()
    assert solve(g, 128)["policies"] == hidden["policies"]


def test_sequential_recall_and_best_response():
    # Player 0 chooses branch L/R, then chooses a/b remembering that branch.
    g = hidden_bit_game()
    g["players"][0] = dict(sequence_count=7, infosets=[
        dict(key="root", parent=0, sequences=[1,2], actions=["L","R"]),
        dict(key="remember-L", parent=1, sequences=[3,4], actions=["a","b"]),
        dict(key="remember-R", parent=2, sequences=[5,6], actions=["a","b"]),
    ])
    g["payoffs"] = [[s,0,1.,v,v,v] for s,v in [(3,.1),(4,.7),(5,.9),(6,.2)]]
    r = solve(g, 512)
    assert r["lower"] > .899
    assert r["upper"] == pytest.approx(.9)
    a = arrays(g["players"][0])
    own = realization(a, np.array(r["policies"][0]))
    assert own[1]+own[2] == pytest.approx(1)
    assert own[3]+own[4] == pytest.approx(own[1])
    assert own[5]+own[6] == pytest.approx(own[2])


def test_frontier_uncertainty_is_not_a_full_game_certificate():
    g = matrix_game([[.9,.2],[.1,.8]])
    for row in g["payoffs"]:
        row[4:6] = [0.,1.]
    r = compare(g, 256)
    assert r["sampled_game_frontier_interval"] == pytest.approx([0,1])
    assert not r["full_game_certified"]


def test_reject_duplicate_information_sets_and_bad_sequences():
    g = matrix_game([[1,0],[0,1]])
    bad = copy.deepcopy(g)
    bad["players"][0]["infosets"].append(copy.deepcopy(bad["players"][0]["infosets"][0]))
    with pytest.raises(ValueError, match="duplicate information key"):
        validate(bad)
    bad = copy.deepcopy(g)
    bad["players"][0]["infosets"][0]["parent"] = 2
    with pytest.raises(ValueError, match="perfect recall"):
        validate(bad)


def test_best_response_accounts_for_all_infosets_sharing_parent():
    a = arrays(hidden_bit_game(True)["players"][0])
    # The two chance observations both follow empty sequence; SUM their
    # counterfactual values, never choose which observation to receive.
    assert best_response(a, np.array([0,.5,0,0,.5])) == 1.


def test_kuhn_poker_known_value_and_perfect_recall():
    # Exact six-deal Kuhn poker, value -1/18 antes for player 0. The utility
    # transform (net+2)/4 gives 35/72. This checks private types, public
    # reveals, sequential decisions, reach weighting, and best responses.
    players = [dict(sequence_count=1, infosets=[]) for _ in range(2)]
    indices = [{}, {}]
    rows = []
    def visit(cards, history, seq):
        if history in ("cc", "bc", "cbc", "bf", "cbf"):
            if history == "bf":
                net = 1
            elif history == "cbf":
                net = -1
            else:
                net = (1 if cards[0] > cards[1] else -1) * (1 if history == "cc" else 2)
            v = (net+2)/4
            rows.append([*seq, 1/6, v, v, v])
            return
        side = len(history) % 2
        actions = ["c", "b"] if history in ("", "c") else ["c", "f"]
        key = f"card={cards[side]};public={history}"
        if key not in indices[side]:
            p = players[side]
            sequences = list(range(p["sequence_count"], p["sequence_count"]+2))
            p["sequence_count"] += 2
            indices[side][key] = len(p["infosets"])
            p["infosets"].append(dict(key=key, parent=seq[side], actions=actions, sequences=sequences))
        info = players[side]["infosets"][indices[side][key]]
        assert info["parent"] == seq[side]
        for action, child in zip(actions, info["sequences"]):
            nxt = list(seq)
            nxt[side] = child
            visit(cards, history+action, nxt)
    for c0 in range(3):
        for c1 in range(3):
            if c0 != c1:
                visit((c0,c1), "", [0,0])
    g = dict(format="nc2000-ico-sequence-v1", players=players, payoffs=rows)
    r = solve(g, 8192, checkpoints=[128,1024])
    assert r["lower"] <= 35/72 <= r["upper"]
    assert r["saddle_gap"] < .001
    assert r["checkpoints"][-1]["saddle_gap"] < r["checkpoints"][0]["saddle_gap"]


def test_actual_native_baseline_is_not_silently_projected_onto_legal_policy():
    g = hidden_bit_game()
    g["players"][0]["infosets"][0]["baseline"] = [1.,0.]
    # An intentionally cheating native baseline guesses the hidden bit in
    # both worlds. ICO must remain at .5 and report the negative difference.
    g["baseline_payoffs"] = [row for row in g["payoffs"] if row[3] == 1.]
    r = compare(g, 128)
    assert r["baseline_root"]["lower"] == pytest.approx(1.)
    assert r["estimated_root_loss_interval"] == pytest.approx([-.5,-.5])
    assert baseline_game(g)["players"][0]["sequence_count"] == 1
