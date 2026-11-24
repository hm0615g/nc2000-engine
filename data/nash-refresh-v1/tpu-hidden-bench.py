"""Run with gen2_tpu from 25ed6f33 on PYTHONPATH and the local WASM Node build."""
import json
from pathlib import Path
import subprocess

import jax
import jax.numpy as jnp
from gen2_tpu import default_eval_weights
from gen2_tpu.kodoku import (
    assemble_3v3_from_rosters_jax,
    build_shared_dex_table,
    compile_roster6,
    select_shell_belief_minimax_action_jax,
)


def main():
    root = Path(__file__).resolve().parents[2]
    data = root / "data"
    teams = {}
    for path in ("team-pool-v2/team-pool.json", "nash-refresh-v1/candidate.json"):
        teams.update({t["id"]: t["sets"] for t in json.loads((data / path).read_text())["teams"]})
    own_sets = teams["deme-r1-00-s0030"]
    actual_sets = teams["sample-07"]
    opponent_ids = ["sample-07", "sample-07-beq-g1-c3", "sample-07-beq-g1-c4", "sample-07-beq-g2-c4"]
    own = compile_roster6(own_sets)
    opponents = [compile_roster6(teams[k]) for k in opponent_ids]
    own_pick = [1, 4, 6]
    opposing_picks = [[3, 1, 2], [3, 1, 5]]
    dex = build_shared_dex_table()

    @jax.jit
    def assemble(other, pick):
        return assemble_3v3_from_rosters_jax(
            own, other, jnp.asarray(own_pick) - 1, pick, jnp.array([12345, 67890], jnp.uint32), dex
        )

    @jax.jit
    def choose(state, table, candidates, seed):
        return select_shell_belief_minimax_action_jax(
            state, table, candidates, jnp.ones(4) / 4, jnp.ones(4, dtype=bool), 0,
            seed, default_eval_weights(), rollout_turns=2, rm_sweeps=32,
        )[1]

    actions = []
    for pick in opposing_picks:
        worlds = [assemble(other, jnp.asarray(pick) - 1) for other in opponents]
        candidates = jax.tree.map(lambda *xs: jnp.stack(xs), *(table for table, _ in worlds))
        true_table, state = worlds[0]
        actions.append(int(choose(state, true_table, candidates, jnp.array([1, 6], jnp.uint32))))

    # Feed each preview and first-turn frame through the same production player channel.
    js = """
const fs = require('node:fs');
const wasm = require('./crates/wasm/pkg-node/nc2000_wasm.js');
const [own, other, ownPick, otherPicks] = JSON.parse(fs.readFileSync(0, 'utf8'));
const dex = new wasm.Dex();
const [a, b] = otherPicks.map(pick => {
    const battle = new wasm.Battle(dex, JSON.stringify(own), JSON.stringify(other), '0,12345,1,2354');
    const channel = new wasm.PlayerChannel(0);
    const preview = channel.frame(battle);
    battle.applyChoice(0, 'team ' + ownPick.join(','));
    battle.applyChoice(1, 'team ' + pick.join(','));
    const turn = channel.frame(battle);
    channel.free();
    battle.free();
    return [preview, turn];
});
dex.free();
process.stdout.write(JSON.stringify(a.map((value, i) => value === b[i])));
"""
    result = subprocess.run(
        ["node", "-e", js], cwd=root, input=json.dumps([own_sets, actual_sets, own_pick, opposing_picks]),
        text=True, capture_output=True, check=True,
    )
    frame_matches = json.loads(result.stdout)
    assert frame_matches == [True, True] and actions == [0, 1]
    print(json.dumps({"preview_and_turn_frames_equal": frame_matches, "moves": [own_sets[0]["moves"][a] for a in actions]}))


if __name__ == "__main__":
    main()
