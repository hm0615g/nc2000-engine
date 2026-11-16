//! Sound root action exclusion masks: turn-order-independent public no-ops
//! (`public_noop_reason`), full-world dominance (`dominated_reason` / `noop_reason`),
//! and strategic wait-turn exemptions (`wait_exempt`).

use nc2000_engine::battle::SearchChoice;
use nc2000_engine::dex::Dex;
use nc2000_engine::state::{Battle, PokeId};

use crate::eval::type_immune_to;

pub(crate) fn effective_move_type(
    b: &Battle,
    dex: &Dex,
    att: PokeId,
    key: &str,
    ms: &nc2000_engine::dex::MoveStatic,
) -> nc2000_engine::dex::TypeId {
    if key == "hiddenpower" {
        if b.poke(att).transformed {
            dex.known_types.unknown
        } else {
            b.poke(att).hp_type
        }
    } else {
        ms.move_type
    }
}

/// A self-KO move used by the side's LAST mon is an unconditional immediate
/// loss in this format: the user faints even on a miss, and when it also
/// faints the foe's last mon the Stadium Self-KO clause still rules the
/// user the loser. Root argmax and rollouts exclude such actions while any
/// alternative exists.
pub(crate) fn certain_self_loss(b: &Battle, dex: &Dex, side: usize, c: SearchChoice) -> bool {
    match c {
        SearchChoice::Move(id) => {
            b.sides[side].pokemon_left == 1 && dex.move_static(id).selfdestruct
        }
        _ => false,
    }
}

#[cfg(test)]
pub(crate) fn certain_noop(
    b: &Battle,
    dex: &Dex,
    side: usize,
    c: SearchChoice,
    rules: MaskRules,
) -> bool {
    noop_reason(b, dex, side, c, rules).is_some()
}

/// Which of the optional [`noop_reason`] rules are live. Every field defaults
/// to the SHIPPED value, so `MaskRules::default()` is exactly the mask the
/// ladder client plays; a field is added here only when an arm is needed to
/// turn one rule OFF for an A/B, never to ship a rule half-on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaskRules {
    pub destiny_bond_wait: bool,
    pub residual_damage_wait: bool,
    pub own_recovery_wait: bool,
    pub priority_heal: bool,
    /// Sleep Talk / Snore selected by an AWAKE, strictly faster user
    /// (`moveexec.rs:521`). `false` = the pre-2026-08-19 mask.
    pub sleep_talk_awake: bool,
    /// Refuse a type-immune move aimed at the mon ACTUALLY IN FRONT even
    /// when the foe could switch out first — i.e. drop the `foe_can_switch`
    /// gate for the type-immunity arm alone.
    ///
    /// **This one can be wrong**, and that is the point of having it: the
    /// move might be aimed at the replacement (Earthquake into a Flying foe
    /// that is about to bring in a Ground type). What makes it worth an A/B
    /// is that the read it deletes has no measured value — over the
    /// 570-battle corpus the class-A opportunity set switches on 20.9% of
    /// decisions against a 23.3% base rate, and across the whole gate the
    /// switch read lifts nothing at all (23.2% vs 23.3%); masking would have
    /// deleted a vindicated read on 21 of 1,410 decisions while removing 53
    /// wasted turns (`examples/perish_switch_census.rs`, round-3 brief).
    /// OFF by default: the shipped ladder mask keeps the gate.
    pub immunity_ignores_switch_read: bool,
    /// Refuse a type-immune move only when it is immune against the mon in
    /// front AND against every mon the foe could bring in
    /// ([`foe_switchin_candidates`]). Sound wherever that candidate set is a
    /// superset of the true one, which is what the helper's doc argues from
    /// public state; it can therefore only ever refuse a turn that was going
    /// to do nothing whatever the foe does.
    ///
    /// Strictly weaker than `immunity_ignores_switch_read` — everything this
    /// refuses, that one refuses too — so the pair brackets the question
    /// "how much of the class is recoverable without ever being wrong".
    /// ON by default.
    pub immunity_all_switchins: bool,
    /// Extended provable no-op masks (`moonlight`/`morningsun`/`synthesis` at full HP,
    /// last-mon `batonpass`, `bellydrum` at `<=50%` HP or `+6 Atk`, healthy-party `healbell`,
    /// `disable`/`encore`/`spite`/`mirrormove`/`conversion2`/`mimic` vs stranded foe with
    /// `last_move == None`, and narrowed `own_recovery_wait`).
    pub extended_noops: bool,
}

impl Default for MaskRules {
    fn default() -> Self {
        MaskRules {
            destiny_bond_wait: true,
            residual_damage_wait: true,
            own_recovery_wait: true,
            priority_heal: true,
            sleep_talk_awake: true,
            immunity_ignores_switch_read: false,
            immunity_all_switchins: true,
            extended_noops: true,
        }
    }
}

/// Final-choice exclusions and their reasons, including forfeit risk and no-ops.
pub fn dominated_actions(b: &Battle, dex: &Dex, side: usize) -> Vec<(SearchChoice, &'static str)> {
    dominated_actions_with(b, dex, side, MaskRules::default())
}

/// [`dominated_actions`] under an explicit rule set. `noop_census`,
/// `analysis::report`, `human_agreement` and the postmortems all call the
/// default-rules wrapper above, so every instrument reports the SHIPPED mask.
pub fn dominated_actions_with(
    b: &Battle,
    dex: &Dex,
    side: usize,
    rules: MaskRules,
) -> Vec<(SearchChoice, &'static str)> {
    b.clone()
        .legal_choices(dex, side)
        .into_iter()
        .filter_map(|c| dominated_reason(b, dex, side, c, rules).map(|why| (c, why)))
        .collect()
}


pub(crate) fn wait_exempt(b: &Battle, dex: &Dex, side: usize, rules: MaskRules) -> bool {
    if b.active_id(side).is_some_and(|id| {
        let me = b.poke(id);
        dex.conds_id("encore")
            .is_some_and(|cond| me.has_volatile(cond))
            && (dex
                .conds_id("substitute")
                .is_some_and(|cond| me.has_volatile(cond))
                || me.boosts.iter().take(5).any(|&x| x > 0)
                || b.active_id(1 - side).is_some_and(|f| b.poke(f).trapped))
    }) {
        return true;
    }
    if rules.destiny_bond_wait
        && b.active_id(1 - side).is_some_and(|id| {
            dex.conds_id("destinybond")
                .is_some_and(|cond| b.poke(id).has_volatile(cond))
        })
    {
        return true;
    }
    if b.active_id(1 - side)
        .and_then(|id| {
            dex.conds_id("perishsong")
                .and_then(|cond| b.poke(id).volatile(cond))
        })
        .and_then(|v| v.duration)
        == Some(1)
    {
        return true;
    }
    if rules.residual_damage_wait && foe_has_residual_damage(b, dex, side) {
        return true;
    }
    if rules.own_recovery_wait
        && b.active_id(side).is_some_and(|id| {
            let me = b.poke(id);
            !me.fainted
                && me.hp > 0
                && me.hp < me.maxhp
                && me.item.is_some()
                && me.item == dex.items.id("leftovers")
        })
        && (!rules.extended_noops
            || b.active_id(1 - side).is_some_and(|f| {
                let fp = b.poke(f);
                fp.status == nc2000_engine::state::Status::Slp
                    || fp.status == nc2000_engine::state::Status::Frz
                    || fp.transformed
                    || fp.move_slots.iter().any(|slot| !slot.shared)
                    || crate::belief::format_learnsets()
                        .species(dex.species.key(fp.species))
                        .map_or(true, |ls| {
                            ls.moves.iter().any(|m| {
                                matches!(
                                    m.as_str(),
                                    "counter"
                                        | "mirrorcoat"
                                        | "metronome"
                                        | "mirrormove"
                                        | "sketch"
                                )
                            })
                        })
            }))
    {
        return true;
    }
    false
}

pub(crate) fn dominated_reason(
    b: &Battle,
    dex: &Dex,
    side: usize,
    c: SearchChoice,
    rules: MaskRules,
) -> Option<&'static str> {
    if certain_self_loss(b, dex, side, c) {
        return Some("self-KO with the last mon");
    }
    if let SearchChoice::Move(id) = c {
        let ms = dex.move_static(id);
        if ms.status.as_deref() == Some("slp")
            && matches!(ms.target, "normal" | "allAdjacentFoes" | "allAdjacent")
            && faster_than_foe(b, dex, side)
            && b.has_sleeping_pokemon(1 - side)
        {
            let opp = &b.sides[1 - side];
            let bench_asleep = opp.party.iter().skip(1).any(|&slot| {
                let p = &opp.roster[slot as usize];
                !p.fainted && p.status == nc2000_engine::state::Status::Slp
            });
            if bench_asleep || foe_can_switch(b, side) {
                return Some("inflicting sleep would forfeit under Sleep Clause");
            }
        }
    }
    if wait_exempt(b, dex, side, rules) {
        return None;
    }
    noop_reason(b, dex, side, c, rules)
}

fn foe_has_residual_damage(b: &Battle, dex: &Dex, side: usize) -> bool {
    use nc2000_engine::state::Status;
    let Some(id) = b.active_id(1 - side) else {
        return false;
    };
    let foe = b.poke(id);
    if foe.fainted || foe.hp == 0 {
        return false;
    }
    // A failing move can avoid retaliation while public residual effects advance.
    matches!(foe.status, Status::Psn | Status::Tox | Status::Brn)
        || ["curse", "leechseed", "partiallytrapped"]
            .iter()
            .any(|key| dex.conds_id(key).is_some_and(|cond| foe.has_volatile(cond)))
        || (foe.status == Status::Slp
            && dex
                .conds_id("nightmare")
                .is_some_and(|cond| foe.has_volatile(cond)))
        || (dex
            .conds_id("sandstorm")
            .is_some_and(|cond| b.field.weather == Some(cond))
            && !foe
                .types
                .iter()
                .any(|t| dex.status_key_immune("sandstorm", t)))
}

/// Whether the foe can leave before the move lands. **Switches resolve before
/// moves**, so every rule that reads the CURRENT foe — its types, its status,
/// its volatiles, its Substitute — is only a proof when this is false.
/// Otherwise the move is a legitimate switch read: Toxic into an already-
/// poisoned mon poisons whatever comes in, and Earthquake into a Levitating
/// foe is aimed at its replacement. Mirrors `search.rs::legal_move_choices`:
/// a trapped mon cannot switch, and neither can one with no living bench.
fn foe_can_switch(b: &Battle, side: usize) -> bool {
    let opp = 1 - side;
    let Some(active) = b.active_id(opp) else {
        return false;
    };
    if b.poke(active).trapped {
        return false;
    }
    let s = &b.sides[opp];
    s.party
        .iter()
        .skip(1)
        .any(|&slot| !s.roster[slot as usize].fainted)
}

/// Whether `side`'s active moves first on speed alone (no tie, no priority
/// read — the foe's move is unknown in blind play). Used by the rules that
/// depend on the user's own state surviving until its move resolves.
///
/// **Quick Claw voids the proof.** The item preempts on a per-turn coin
/// (`turn.rs` endTurn, 60/256) that `get_pokemon_action_speed` reads as a
/// 65535 speed. Inside the engine that coin is already rolled at the request
/// point, but the live client cannot observe it: the importer leaves
/// `quick_claw_roll` false in every reconstructed state (measured over the
/// 570-battle corpus: 0 true out of 20,765 decisions, with a Quick Claw on
/// the field in 3.1% of active slots). So a foe holding one moves first
/// nearly a quarter of the time with nothing in the state to say so, and the
/// rules gated on this would refuse moves the preempt makes live. Claim the
/// proof only when the foe cannot preempt at all.
fn faster_than_foe(b: &Battle, dex: &Dex, side: usize) -> bool {
    let (Some(me), Some(foe)) = (b.active_id(side), b.active_id(1 - side)) else {
        return false;
    };
    if !b.quick_claw_roll
        && b.poke(foe).item.is_some()
        && b.poke(foe).item == dex.known_items.quickclaw
    {
        return false;
    }
    b.get_pokemon_action_speed(dex, me) > b.get_pokemon_action_speed(dex, foe)
}

fn foe_may_preempt_matching(
    b: &Battle,
    dex: &Dex,
    side: usize,
    priority: i8,
    damage_only: bool,
) -> bool {
    let Some(id) = b.active_id(1 - side) else {
        return true;
    };
    let foe = b.poke(id);
    // Copied move slots are public overlays; their identities and PP are not needed.
    if foe.transformed || foe.move_slots.iter().any(|slot| !slot.shared) {
        return true;
    }
    let Some(learnset) = crate::belief::format_learnsets().species(dex.species.key(foe.species))
    else {
        return true;
    };
    learnset.moves.iter().any(|key| {
        dex.moves.id(key).is_some_and(|id| {
            let ms = dex.move_static(id);
            ms.priority > priority
                && (!damage_only || ms.category != nc2000_engine::dex::Category::Status)
        })
    })
}

fn foe_may_preempt_with_damage(b: &Battle, dex: &Dex, side: usize, priority: i8) -> bool {
    foe_may_preempt_matching(b, dex, side, priority, true)
}

fn foe_may_preempt_with_any_move(b: &Battle, dex: &Dex, side: usize, priority: i8) -> bool {
    foe_may_preempt_matching(b, dex, side, priority, false)
}

/// Every foe mon that could be standing there when our move resolves, other
/// than the one in front — read off PUBLIC state only, and deliberately a
/// SUPERSET of the true set, so "immune to all of these" is a proof.
///
/// The information argument, since this is the whole soundness of
/// [`MaskRules::immunity_all_switchins`]:
///
/// * **Species are public.** The team-preview `|poke|` lines carry species,
///   level and gender for all six, which is why `Observer::new` reads them
///   straight off the roster and calls them "public at team preview". Types
///   follow from the species, and a benched mon's types are its species'
///   (`clearVolatile` restores base types on switch-out, so Conversion and
///   Transform only ever touch the active).
/// * **Which three were PICKED is not public** until they appear. So this
///   never reads party membership for a mon that has not appeared: on the
///   ladder those party slots are imputed in roster order
///   (`import.rs`, "then imputed hidden picks"), and reading them would be
///   reading a guess. It reads `appeared` instead — `previously_switched_in
///   > 0 || is_active`, the same predicate `Observer::observe` and
///   `Belief::determinize` use — and admits EVERY never-appeared roster mon
///   while any pick is still unrevealed. That is exactly the support the
///   determinizer samples hidden identities from (uniform over the
///   not-yet-appeared roster mons), so it is also "whatever the belief
///   admits", and a pinned/open-sheet belief is a subset of it.
/// * Once every pick has appeared, the set collapses to the real bench, and
///   the rule becomes as tight as perfect information would make it.
///
/// Fainted mons are dropped (public), and a never-appeared mon can never be
/// fainted. Nothing here can hand the foe a Pokemon it did not have, and a
/// bench only ever shrinks, so a superset at the decision is still a
/// superset at resolution.
fn foe_switchin_candidates(b: &Battle, side: usize) -> Vec<PokeId> {
    let opp = 1 - side;
    let s = &b.sides[opp];
    let active = b.active_id(opp);
    let appeared = |p: &nc2000_engine::state::Pokemon| p.previously_switched_in > 0 || p.is_active;
    let seen = s.roster.iter().filter(|p| appeared(p)).count();
    // `party.len()` is the pick count (3 in this format); every appeared mon
    // is in it, so `seen >= party.len()` means no hidden pick is left.
    let all_picks_revealed = seen >= s.party.len();
    let mut out = Vec::new();
    for (slot, p) in s.roster.iter().enumerate() {
        let id = PokeId {
            side: opp as u8,
            slot: slot as u8,
        };
        if Some(id) == active || p.fainted || p.hp <= 0 {
            continue;
        }
        let could_come_in = if appeared(p) {
            s.party.contains(&(slot as u8))
        } else {
            !all_picks_revealed
        };
        if could_come_in {
            out.push(id);
        }
    }
    out
}

/// Proven move failures, excluding actions whose successful effect causes a forfeit.
///
/// **What "certain" means here.** Every rule is read off the position as it
/// stands at the decision, and a foe switch (switches resolve before moves)
/// or a foe self-cure can make a refused action live before it resolves.
/// `noop_census` measures that error rate against the engine over corpus
/// positions; it is the number to re-check whenever a rule is added.
/// Subset of [`noop_reason`] that depends strictly on public / own-certain
fn foe_may_change_type(b: &Battle, dex: &Dex, def: PokeId) -> bool {
    let foe = b.poke(def);
    foe.transformed
        || foe.move_slots.iter().any(|s| !s.shared)
        || crate::belief::format_learnsets()
            .species(dex.species.key(foe.species))
            .map_or(true, |ls| {
                ls.moves.iter().any(|m| {
                    matches!(
                        m.as_str(),
                        "conversion" | "conversion2" | "transform" | "sketch"
                    )
                })
            })
}

fn raw_public_noop_reason(
    b: &Battle,
    dex: &Dex,
    side: usize,
    c: SearchChoice,
    rules: MaskRules,
) -> Option<&'static str> {
    use nc2000_engine::dex::Category;
    let SearchChoice::Move(id) = c else {
        return None;
    };
    let att = b.active_id(side)?;
    let ms = dex.move_static(id);
    let key = dex.moves.key(id);
    // Our own screens/Safeguard/Mist cannot be removed in Gen 2 and expire at
    // upkeep after moves, so casting them while already active always fails.
    if let Some(sc) = ms.side_condition.as_deref() {
        if ms.target != "foeSide"
            && dex
                .conds_id(sc)
                .is_some_and(|cid| b.sides[side].has_side_condition(cid))
        {
            return Some("that side condition is already up");
        }
    }
    // Phazing when the opponent has 0 non-fainted bench Pokemon remaining.
    if ms.force_switch && !b.can_switch((1 - side) as u8) {
        return Some("there is nobody left to phaze in");
    }
    // Perish Song / Destiny Bond fail (`onPrepareHit`) from the last own mon.
    if matches!(key, "perishsong" | "destinybond") && b.sides[side].pokemon_left == 1 {
        return Some("Perish Song and Destiny Bond fail from the last mon");
    }
    // Baton Pass fails when our own bench has no non-fainted switch target.
    if rules.extended_noops && ms.self_switch.is_some() && !b.can_switch(side as u8) {
        return Some("Baton Pass fails with nobody left to switch to");
    }
    let foe_targeted = matches!(ms.target, "normal" | "allAdjacentFoes" | "allAdjacent");
    // Safeguard is a SIDE condition: it survives a switch, so it blocks
    // status and non-Swagger confusion whoever is in.
    if ms.category == Category::Status
        && foe_targeted
        && dex
            .conds_id("safeguard")
            .is_some_and(|cid| b.sides[1 - side].has_side_condition(cid))
    {
        if ms.status.is_some() {
            return Some("Safeguard blocks foe-inflicted status");
        }
        if !(key == "curse" && !b.poke(att).has_type(dex.known_types.ghost))
            && ms.volatile_status.as_deref() == Some("confusion")
            && key != "swagger"
        {
            return Some("Safeguard blocks confusion");
        }
    }
    // Refuse when active foe cannot change type and is immune AND every public
    // switch-in candidate is immune, so the turn is dead whatever they do.
    let foe_here = b.active_id(1 - side).filter(|&d| {
        let p = b.poke(d);
        !p.fainted && p.hp > 0
    });
    let move_type = effective_move_type(b, dex, att, key, ms);
    if rules.immunity_all_switchins
        && foe_targeted
        && !ms.ignore_immunity
        && !ms.selfdestruct
        && move_type != dex.known_types.unknown
        && foe_here.is_some_and(|def| {
            !foe_may_change_type(b, dex, def) && type_immune_to(b, dex, move_type, def)
        })
        && foe_switchin_candidates(b, side)
            .into_iter()
            .all(|cand| type_immune_to(b, dex, move_type, cand))
    {
        return Some("immune to the move's type, and so is every possible switch-in");
    }
    None
}

/// Subset of [`dominated_actions`] that depends strictly on public / own-certain
/// battle state (never guessed speed, items, hidden foe sets, or HP rolls),
/// so every world compatible with the public protocol agrees on the exclusion.
pub fn public_dominated_actions(
    b: &Battle,
    dex: &Dex,
    side: usize,
) -> Vec<(SearchChoice, &'static str)> {
    public_dominated_actions_with(b, dex, side, MaskRules::default())
}

/// [`public_dominated_actions`] under an explicit [`MaskRules`] configuration.
pub fn public_dominated_actions_with(
    b: &Battle,
    dex: &Dex,
    side: usize,
    rules: MaskRules,
) -> Vec<(SearchChoice, &'static str)> {
    b.clone()
        .legal_choices(dex, side)
        .into_iter()
        .filter_map(|c| public_noop_reason(b, dex, side, c, rules).map(|why| (c, why)))
        .collect()
}

pub(crate) fn public_noop_reason(
    b: &Battle,
    dex: &Dex,
    side: usize,
    c: SearchChoice,
    rules: MaskRules,
) -> Option<&'static str> {
    if wait_exempt(b, dex, side, rules) {
        return None;
    }
    raw_public_noop_reason(b, dex, side, c, rules)
}

fn noop_reason(
    b: &Battle,
    dex: &Dex,
    side: usize,
    c: SearchChoice,
    rules: MaskRules,
) -> Option<&'static str> {
    use nc2000_engine::dex::Category;
    use nc2000_engine::state::Status;
    macro_rules! yes {
        ($why:expr) => {
            return Some($why)
        };
    }
    macro_rules! verdict {
        ($cond:expr, $why:expr) => {
            return if $cond { Some($why) } else { None }
        };
    }
    if let Some(why) = raw_public_noop_reason(b, dex, side, c, rules) {
        return Some(why);
    }
    let SearchChoice::Move(id) = c else {
        return None;
    };
    let ms = dex.move_static(id);
    let Some(att) = b.active_id(side) else {
        return None;
    };
    let key = dex.moves.key(id);
    let full_hp = {
        let me = b.poke(att);
        me.hp >= me.maxhp
    };
    if ms.heal.is_some()
        || key == "rest"
        || (rules.extended_noops && matches!(key, "moonlight" | "morningsun" | "synthesis"))
    {
        verdict!(
            full_hp
                && faster_than_foe(b, dex, side)
                && (!rules.priority_heal
                    || !foe_may_preempt_with_damage(b, dex, side, ms.priority)),
            "healing at full HP, and faster"
        );
    }
    if let Some(sc) = ms.side_condition.as_deref() {
        let on_foe = ms.target == "foeSide";
        let target_side = if on_foe { 1 - side } else { side };
        let up = dex
            .conds_id(sc)
            .is_some_and(|cid| b.sides[target_side].has_side_condition(cid));
        // Hazards on the FOE's side can be spun away first, so that half needs us to move first.
        verdict!(
            up && (!on_foe || faster_than_foe(b, dex, side)),
            "that side condition is already up"
        );
    }
    let foe_may_raise_hp = || {
        b.active_id(1 - side).is_some_and(|f| {
            let fp = b.poke(f);
            fp.transformed
                || fp.move_slots.iter().any(|slot| !slot.shared)
                || crate::belief::format_learnsets()
                    .species(dex.species.key(fp.species))
                    .map_or(true, |ls| {
                        ls.moves.iter().any(|m| {
                            matches!(
                                m.as_str(),
                                "painsplit" | "present" | "metronome" | "mirrormove" | "sketch"
                            )
                        })
                    })
        })
    };
    if rules.extended_noops && key == "bellydrum" {
        let me = b.poke(att);
        let max_leftovers_gain = if me.item.is_some() && me.item == dex.items.id("leftovers") {
            (me.maxhp / 16).max(1)
        } else {
            0
        };
        let hp_too_low_strict = (me.hp as f64) <= (me.maxhp as f64) / 2.0
            && (faster_than_foe(b, dex, side)
                || (!foe_may_raise_hp()
                    && ((me.hp + max_leftovers_gain) as f64) <= (me.maxhp as f64) / 2.0));
        let atk_capped_strict = me.boosts[0] >= 6 && faster_than_foe(b, dex, side);
        if hp_too_low_strict || atk_capped_strict {
            yes!("Belly Drum needs over half HP and uncapped Attack");
        }
    }
    if rules.extended_noops
        && key == "healbell"
        && faster_than_foe(b, dex, side)
        && !foe_may_preempt_with_damage(b, dex, side, ms.priority)
        && b.sides[side]
            .party
            .iter()
            .all(|&slot| b.sides[side].roster[slot as usize].status == Status::None)
    {
        yes!("Heal Bell has no status to cure");
    }
    // ---- Sleep Talk / Snore chosen while AWAKE. `moveexec.rs:521`
    // (`("sleeptalk","onTry") | ("snore","onTry")` => `RV::from_bool(status ==
    // Slp)`) hands a false onTry to `moveexec.rs:1799`, which returns
    // `MoveOutcome::Fail` and — unlike the PrepareHit branch two lines above
    // it — emits NO `-fail`. The PP is already gone by then
    // (`moveexec.rs:1492`, before `use_move`), so the turn is spent, nothing
    // happens, and the protocol says nothing at all: this is the one mask rule
    // whose firings `noop_census` can only score as SILENT agreement.
    //
    // `sleep_usable` is exactly {sleeptalk, snore} in this dex, the same pair
    // the engine arm covers, so the flag mirrors the engine instead of
    // transcribing a key list. Checked ABOVE the `category != Status` early
    // return below, because Snore is physical.
    //
    // Needs the speed gate, and nothing else. The proof object is OUR OWN
    // status, and the one thing that can change it before our move resolves is
    // a foe that moves first and lands a sleep: `conditions.rs:159`
    // (slp/onBeforeMove) decrements the counter and then returns `Undef` for a
    // `sleep_usable` move, so a user put to sleep this very turn still gets a
    // working Sleep Talk. In-distribution, not theoretical — 15 of the 32
    // meta-pool teams carry a sleep move. No switch gate: the rule never reads
    // the foe. A mon that wakes on its OWN turn is untouched by construction:
    // the sleep counter is hidden, so at the decision its status still reads
    // `Slp` (24 such Sleep Talks in the 570-battle corpus).
    //
    // Found by battle-4070 (2026-08-19): Suicune, alone against a Rest-
    // stalling Umbreon, spent 6 of its 64 last-mon turns on an awake Sleep
    // Talk. Corpus rate: 54 of 863 Sleep Talk selections (6.3%) were made by a
    // plainly awake mon.
    //
    // Deliberately `if`, not `verdict!`: a false verdict here returns None and
    // would shadow every rule below for this move. Snore is a Normal-typed
    // attack, so an ASLEEP user aiming it at a stranded Ghost still has to
    // reach the type-immunity arm.
    if rules.sleep_talk_awake
        && ms.sleep_usable
        && b.poke(att).status != Status::Slp
        && faster_than_foe(b, dex, side)
    {
        yes!("Sleep Talk and Snore need the user asleep");
    }
    // Every rule below that reads the foe is conditioned on the foe being
    // stuck with the mon it has: otherwise the move is aimed at whatever
    // switches in, and refusing it would delete a real option.
    let foe_here = b.active_id(1 - side).filter(|&d| {
        let p = b.poke(d);
        !p.fainted && p.hp > 0
    });
    let foe = foe_here.filter(|_| !foe_can_switch(b, side));
    // In singles every "adjacent" target resolves to the one foe, so
    // Earthquake and Poison Gas are as foe-directed as Body Slam.
    let foe_targeted = matches!(ms.target, "normal" | "allAdjacentFoes" | "allAdjacent");

    // ---- type immunity (`pokemon.rs::run_move_immunity`). Damaging moves
    // too: an immune target ends the move before any effect, so Earthquake
    // into a Flying foe is as much a wasted turn as Thunder Wave into a
    // Ground one. `ignore_immunity` is carried per move in the dex (it is why
    // Ghost-typed Confuse Ray still reaches a Normal type), and Ground is the
    // one type the engine resolves by groundedness rather than the chart.
    // A self-KO move is never a no-op even when the foe is immune: the user
    // faints on use regardless, which is `certain_self_loss`'s business.
    // Hidden Power's dex entry says Normal; its real type comes from the
    // user's DVs (`moveexec.rs:340`). Reading the static field refused Hidden
    // Power into every Ghost — caught by `noop_census`, 7 refusals that the
    // engine damaged straight through.
    let move_type = effective_move_type(b, dex, att, key, ms);
    if foe_targeted
        && !ms.ignore_immunity
        && !ms.selfdestruct
        && move_type != dex.known_types.unknown
    {
        // The shipped proof: the foe is stuck with the mon in front, and
        // that mon is immune. `foe` is already `foe_can_switch`-gated.
        if foe.is_some_and(|def| type_immune_to(b, dex, move_type, def)) {
            yes!("the target is immune to the move's type");
        }
        // Reached when the gate above dropped the proof (when the foe CAN leave).
        // `if`, never `verdict!`, so a miss does not shadow later rules.
        let front_immune = || foe_here.is_some_and(|def| type_immune_to(b, dex, move_type, def));
        // Arm 1: refuse the mon in front regardless of the switch read.
        if rules.immunity_ignores_switch_read && front_immune() {
            yes!("the mon in front is immune (switch read ignored)");
        }
    }
    // ---- Dream Eater needs a sleeping, un-substituted target
    // (`moveexec.rs` "dreameater"/onTryImmunity). Damaging, so it is checked
    // before the status-only gate below.
    if key == "dreameater" {
        if let Some(def) = foe {
            let d = b.poke(def);
            let subbed = dex
                .conds_id("substitute")
                .is_some_and(|sid| d.has_volatile(sid));
            verdict!(
                (d.status != Status::Slp && faster_than_foe(b, dex, side)) || subbed,
                "Dream Eater needs a sleeping, unsubstituted target"
            );
        }
    }
    if ms.category != Category::Status {
        return None;
    }

    // Public state of the foe's protections, shared by the rules below.
    let sub_up = foe.is_some_and(|d| {
        dex.conds_id("substitute")
            .is_some_and(|sid| b.poke(d).has_volatile(sid))
    });

    if ms.status.is_some() && foe_targeted {
        if let Some(def) = foe {
            let d = b.poke(def);
            // One major status at a time (`pokemon.rs::set_status`) — but a
            // foe that moves first can Rest, eat a berry or wake up, and the
            // census caught exactly that. Only a proof when we act first.
            if d.status != Status::None && faster_than_foe(b, dex, side) {
                yes!("the target already carries a major status");
            }
            // a Substitute blocks every foe-inflicted status
            // (`conditions.rs` substitute/onTryPrimaryHit)
            if sub_up {
                yes!("a Substitute blocks foe-inflicted status");
            }
            // type-based status immunity (`pokemon.rs::run_status_immunity`,
            // built from the typechart's non-type `damageTaken` keys — in
            // this dex that is Poison/tox onto a Poison type). tox is checked
            // as psn, exactly as `set_status` does.
            let st = ms.status.as_deref().unwrap_or("");
            let check = if st == "tox" { "psn" } else { st };
            if d.types.iter().any(|t| dex.status_key_immune(check, t)) {
                yes!("the target's type cannot take that status");
            }
        }
        return None;
    }

    // ---- own Substitute: already up, or not enough HP to pay for it
    // (`moveexec.rs` substitute/onTryHit).
    if key == "substitute" {
        let me = b.poke(att);
        let up = dex
            .conds_id("substitute")
            .is_some_and(|sid| me.has_volatile(sid));
        let me_has_heal_berry = me
            .item
            .is_some_and(|id| matches!(dex.items.key(id), "berry" | "goldberry"));
        let no_hp = (me.hp <= me.maxhp / 4 || me.maxhp == 1)
            && (faster_than_foe(b, dex, side)
                || (!me_has_heal_berry && !foe_may_raise_hp()));
        let already_up = up
            && faster_than_foe(b, dex, side)
            && !foe_may_preempt_with_damage(b, dex, side, ms.priority);
        verdict!(
            no_hp || already_up,
            "Substitute is already up, or there is not enough HP to pay for one"
        );
    }

    if rules.extended_noops
        && matches!(
            key,
            "disable" | "encore" | "spite" | "mirrormove" | "conversion2" | "mimic"
        )
    {
        if let Some(def) = foe {
            let d = b.poke(def);
            let no_last = if key == "encore" {
                d.last_move_encore.is_none()
            } else {
                d.last_move.is_none()
            };
            if no_last
                && faster_than_foe(b, dex, side)
                && !foe_may_preempt_with_any_move(b, dex, side, ms.priority)
            {
                yes!("the target has not used a move yet");
            }
        }
    }
    if let Some(v) = ms.volatile_status.as_deref() {
        // Non-Ghost Curse boosts the user (+1 Atk, +1 Def, -1 Spe) and clears
        // `volatile_status` (`moveexec.rs:576`), so it only fails when both
        // Attack and Defense are already at +6.
        if key == "curse" && !b.poke(att).has_type(dex.known_types.ghost) {
            let boosts = b.poke(att).boosts;
            if boosts[0] >= 6 && boosts[1] >= 6 && faster_than_foe(b, dex, side) {
                yes!("every stat this move moves is already capped");
            }
            return None;
        }
        let tgt = if ms.target == "self" { Some(att) } else { foe };
        // Only when the volatile IS the move. Swagger also boosts, and the
        // engine lands that boost even when the confusion fails — masking it
        // as a no-op was wrong twice over (`noop_census` caught the engine
        // logging `-boost atk 2` on a refused Swagger). A move with a second
        // payload is the eval's problem, not the mask's.
        let volatile_is_everything =
            !ms.has_boosts && ms.status.is_none() && ms.heal.is_none() && ms.damage.is_none();
        if let (true, Some(t), Some(vid)) = (volatile_is_everything, tgt, dex.conds_id(v)) {
            // the volatile is already on the target — re-applying fails,
            // since none of these conditions carries an `onRestart`
            // A volatile can also run out on its owner's own move (a
            // confusion clock is decremented before the move it hinders), so
            // the foe-directed half needs us to move first. `destinybond` on
            // `att` is also cleared by `onFoeAfterMoveSelf` and `onFoeSwitchOut`
            // (`conditions.rs:808-818`), so a slower user or switchable foe
            // clears it before our move resolves and re-arms Destiny Bond.
            let stable = if v == "destinybond" {
                !foe_can_switch(b, side)
                    && faster_than_foe(b, dex, side)
                    && !foe_may_preempt_with_any_move(b, dex, side, ms.priority)
            } else {
                tgt == Some(att) || faster_than_foe(b, dex, side)
            };
            if stable && b.poke(t).has_volatile(vid) {
                yes!("the target already has that volatile");
            }
        }
        if foe_targeted && foe.is_some() {
            // Confusion is blocked by both a Substitute and Safeguard;
            // Swagger is the documented exception — the engine strips its
            // confusion behind a Substitute but still lands the +2 Attack,
            // so it is NOT a no-op there.
            if v == "confusion" && key != "swagger" && sub_up {
                yes!("a Substitute blocks confusion");
            }
            // moves the Substitute rejects wholesale
            // (`conditions.rs` substitute/onTryPrimaryHit SUB_BLOCKED)
            const SUB_BLOCKED: [&str; 6] = [
                "leechseed",
                "lockon",
                "mindreader",
                "nightmare",
                "painsplit",
                "sketch",
            ];
            if sub_up && SUB_BLOCKED.contains(&key) {
                yes!("a Substitute rejects this move outright");
            }
            // Leech Seed does not take on a Grass type
            // (`moveexec.rs` leechseed/onTryImmunity)
            if key == "leechseed" && foe.is_some_and(|d| b.poke(d).has_type(dex.known_types.grass))
            {
                yes!("Leech Seed does not take on a Grass type");
            }
            // Attract needs opposite, known genders
            // (`moveexec.rs` attract/onTryImmunity)
            if key == "attract" {
                if let Some(def) = foe {
                    use nc2000_engine::state::Gender;
                    let (tg, sg) = (b.poke(def).gender, b.poke(att).gender);
                    let ok = (tg == Gender::M && sg == Gender::F)
                        || (tg == Gender::F && sg == Gender::M);
                    if !ok {
                        yes!("Attract needs opposite, known genders");
                    }
                }
            }
        }
    }

    // ---- a stat move that cannot move a stat (`dmg.rs::boost` reports no
    // change, and `moveexec.rs`'s didSomething chain then fails the move).
    // Only for moves whose whole payload is the boost table.
    if ms.has_boosts
        && ms.status.is_none()
        && ms.volatile_status.is_none()
        && ms.heal.is_none()
        && ms.self_effect.is_none()
        && ms.secondaries.is_empty()
    {
        // Which mon the table lands on is read from the move's own target,
        // never inferred from the sign: `curse` is target=normal and boosts
        // its user, and it stays out of this rule only because its stat
        // changes come from a callback rather than the static table. An
        // unrecognised target claims nothing.
        let self_targeted = ms.target == "self";
        let foe_directed = (ms.target == "normal" || ms.target == "allAdjacentFoes")
            && ms.boosts.iter().all(|&(_, a)| a < 0);
        let tgt = match (self_targeted, foe_directed) {
            (true, _) => Some(att),
            (_, true) => foe,
            _ => None,
        };
        if let Some(t) = tgt {
            // Mist deletes every negative entry coming from the foe
            // (`conditions.rs` mist/onTryBoost), so a pure stat-drop move
            // into Mist changes nothing.
            let misted = !self_targeted
                && dex
                    .conds_id("mist")
                    .is_some_and(|cid| b.poke(t).has_volatile(cid));
            let subbed = !self_targeted && sub_up && key != "swagger";
            if misted {
                yes!("Mist deletes foe-sourced stat drops");
            }
            if subbed {
                yes!("a Substitute blocks foe-directed stat drops");
            }
            let boosts = b.poke(t).boosts;
            let capped = ms.boosts.iter().all(|&(stat, amount)| {
                let cur = boosts[stat];
                (amount > 0 && cur >= 6) || (amount < 0 && cur <= -6) || amount == 0
            });
            // A faster foe can un-cap us first (Screech into our +6 defence
            // is the census's example), so our own stages are only fixed if we
            // act first; the foe's are covered by the switch gate above.
            if capped && faster_than_foe(b, dex, side) {
                yes!("every stat this move moves is already capped");
            }
        }
    }
    None
}


#[cfg(test)]
mod dominated_action_tests {
    use super::*;
    use crate::smmcts::{RmConfig, SelRule, SkuctSearch};
    use nc2000_engine::battle::{EffectHandle, PokemonSet};
    use nc2000_engine::state::{PokeId, Status};

    /// Faint `side`'s bench so its active cannot leave. Every rule that reads
    /// the CURRENT foe is conditioned on this: switches resolve before moves,
    /// so with a bench available the same move is a legitimate switch read.
    fn strand(b: &mut Battle, side: usize) {
        let party = b.sides[side].party.clone();
        for &slot in party.iter().skip(1) {
            let id = PokeId {
                side: side as u8,
                slot,
            };
            b.poke_mut(id).hp = 0;
            b.poke_mut(id).fainted = true;
        }
    }

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

    /// Every test here asserts the SHIPPED mask, so it reads
    /// [`super::certain_noop`] at `MaskRules::default()`. This shadows the
    /// parent item deliberately; the ablation rule set is exercised by name in
    /// `mask_rules_ablation_isolates_the_sleep_talk_rule`.
    fn certain_noop(b: &Battle, dex: &Dex, side: usize, c: SearchChoice) -> bool {
        super::certain_noop(b, dex, side, c, MaskRules::default())
    }

    #[test]
    fn noop_mask_matches_engine_failures() {
        let (dex, b) = setup();
        // side 0 active: Exeggutor; side 1 active: Snorlax
        let noop = |b: &Battle, side: usize, key: &str| certain_noop(b, &dex, side, mv(&dex, key));

        // baseline: nothing up — only Rest at full HP is a no-op
        assert!(!noop(&b, 0, "sleeppowder"));
        assert!(!noop(&b, 0, "reflect"));
        assert!(!noop(&b, 0, "spikes"));
        assert!(noop(&b, 0, "rest"), "Rest at full HP");

        // Rest becomes live once hurt
        {
            let mut b = b.clone();
            let att = b.active_id(0).unwrap();
            b.poke_mut(att).hp -= 10;
            assert!(!noop(&b, 0, "rest"));
        }

        // screens/spikes re-cast (single layer)
        {
            let mut b = b.clone();
            let att = b.active_id(0).unwrap();
            b.add_side_condition(&dex, 0, "reflect", Some(att), EffectHandle::None);
            b.add_side_condition(&dex, 1, "spikes", Some(att), EffectHandle::None);
            assert!(
                noop(&b, 0, "reflect"),
                "Reflect already up (report B turn 6)"
            );
            assert!(noop(&b, 0, "spikes"), "Spikes already laid");
        }

        // status move onto an existing status / through a Substitute
        {
            let mut b = b.clone();
            strand(&mut b, 1);
            let def = b.active_id(1).unwrap();
            b.set_status(&dex, def, "par", None, EffectHandle::None, true);
            assert!(noop(&b, 0, "sleeppowder"), "status onto statused foe");
        }
        {
            let mut b = b.clone();
            strand(&mut b, 1);
            let def = b.active_id(1).unwrap();
            b.add_volatile(&dex, def, "substitute", None, EffectHandle::None);
            assert!(noop(&b, 0, "sleeppowder"), "Sleep Powder into a Substitute");
            // damaging moves stay live through the sub
            assert!(!noop(&b, 1, "bodyslam"));
        }

        // re-inflicting a volatile / Substitute behind its own sub
        {
            let mut b = b.clone();
            strand(&mut b, 0);
            let att0 = b.active_id(0).unwrap();
            let def = b.active_id(1).unwrap();
            b.add_volatile(&dex, att0, "confusion", None, EffectHandle::None);
            b.add_volatile(&dex, def, "substitute", None, EffectHandle::None);
            // A confusion clock ticks down on its owner's own move, so this
            // rule needs the caster to move first.
            assert!(
                !noop(&b, 1, "confuseray"),
                "slower: the clock may run out first"
            );
            b.poke_mut(def).boosts[4] = 6;
            assert!(
                noop(&b, 1, "confuseray"),
                "Confuse Ray onto confused foe, moving first"
            );
            assert!(noop(&b, 1, "substitute"), "Substitute behind its own sub");
        }
    }

    #[test]
    fn sleep_forfeit_is_masked_as_risk_including_rest_sleep() {
        let (dex, base) = setup();
        let me = base.active_id(0).unwrap();
        let bench = PokeId {
            side: 1,
            slot: base.sides[1].party[1],
        };
        let sleep = mv(&dex, "sleeppowder");
        for rest in [false, true] {
            let mut b = base.clone();
            b.set_status(
                &dex,
                bench,
                "slp",
                Some(if rest { bench } else { me }),
                EffectHandle::None,
                true,
            );
            assert!(!certain_noop(&b, &dex, 0, sleep));
            assert_eq!(
                dominated_reason(&b, &dex, 0, sleep, MaskRules::default()),
                Some("inflicting sleep would forfeit under Sleep Clause")
            );
            let mut after = b.clone();
            after.choose(&dex, 0, "move sleeppowder").unwrap();
            after.choose(&dex, 1, "move bodyslam").unwrap();
            assert_eq!(after.outcome(), Some(nc2000_engine::battle::Outcome::P2Win));

            let def = b.active_id(1).unwrap();
            let mut slow = b.clone();
            slow.poke_mut(def).boosts[4] = 6;
            assert!(dominated_reason(&slow, &dex, 0, sleep, MaskRules::default()).is_none());
            b.poke_mut(bench).hp = 0;
            b.poke_mut(bench).fainted = true;
            assert!(dominated_reason(&b, &dex, 0, sleep, MaskRules::default()).is_none());
        }
    }

    /// Quick Claw preempts on a coin the client cannot see, so it voids every
    /// rule that needs us to move first (`faster_than_foe`).
    #[test]
    fn quick_claw_voids_the_speed_proof() {
        let (dex, b) = setup();
        let noop = |b: &Battle, side: usize, key: &str| certain_noop(b, &dex, side, mv(&dex, key));
        let mut b = b.clone();
        strand(&mut b, 1);
        let me = b.active_id(0).unwrap();
        let def = b.active_id(1).unwrap();
        b.set_status(&dex, def, "par", Some(me), EffectHandle::None, true);
        assert!(noop(&b, 0, "sleeppowder"), "statused foe, and we act first");

        b.poke_mut(def).item = dex.known_items.quickclaw;
        assert!(
            !noop(&b, 0, "sleeppowder"),
            "a Quick Claw foe may move first"
        );
        // …and when the coin is up it is the speed comparison itself that
        // says we are second.
        b.quick_claw_roll = true;
        assert!(
            !noop(&b, 0, "sleeppowder"),
            "coin up: the foe is faster outright"
        );

        // Same contract for the awake-Sleep-Talk rule, on the fixture that
        // carries the pair: a Quick Claw foe can preempt with a sleep move,
        // and the client cannot see the coin.
        let (dex, st) = st_setup();
        let st_noop = |b: &Battle, key: &str| certain_noop(b, &dex, 0, mv(&dex, key));
        let st_foe = st.active_id(1).unwrap();
        for key in ["sleeptalk", "snore"] {
            assert!(st_noop(&st, key), "awake {key}, and we act first");
        }
        let mut claw = st.clone();
        claw.poke_mut(st_foe).item = dex.known_items.quickclaw;
        for key in ["sleeptalk", "snore"] {
            assert!(
                !st_noop(&claw, key),
                "a Quick Claw foe may sleep us first ({key})"
            );
        }
        claw.quick_claw_roll = true;
        for key in ["sleeptalk", "snore"] {
            assert!(
                !st_noop(&claw, key),
                "coin up: the foe is faster outright ({key})"
            );
        }
    }

    // ---- 2026-08-19: Sleep Talk / Snore chosen while awake --------------
    //
    // Battle 4070: Suicune, alone against a Rest-stalling Umbreon, spent 6 of
    // its 64 last-mon turns on a Sleep Talk it was awake for. The engine fails
    // the move at `moveexec.rs:521` (onTry = `status == Slp`) and — uniquely
    // among the masked rules — says NOTHING in the protocol while doing it.

    fn st_team() -> Vec<PokemonSet> {
        // from_fixture does not validate movesets. Snore sits next to Sleep
        // Talk on purpose: it is PHYSICAL, so refusing it proves the rule is
        // reached before `noop_reason`'s `category != Status` early return.
        serde_json::from_str(
            r#"[
            {"name":"Suicune","species":"Suicune","item":"","ability":"No Ability",
             "moves":["Surf","Sleep Talk","Snore","Rest"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"N","level":50},
            {"name":"Snorlax","species":"Snorlax","item":"","ability":"No Ability",
             "moves":["Splash","Sleep Powder","Body Slam","Rest"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Exeggutor","species":"Exeggutor","item":"","ability":"No Ability",
             "moves":["Splash","Sleep Powder","Psychic","Rest"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}
        ]"#,
        )
        .unwrap()
    }

    /// Side 0 leads Suicune (base Spe 85), side 1 leads Snorlax (base Spe 30),
    /// so the speed proof holds with nothing to argue about.
    fn st_setup() -> (Dex, Battle) {
        let dex = conformance::load_dex();
        let t = st_team();
        let mut b = Battle::from_fixture(&dex, "7,8,9,10", &t, &t).unwrap();
        b.set_log_enabled(false);
        b.choose(&dex, 0, "team 1, 2, 3").unwrap();
        b.choose(&dex, 1, "team 2, 1, 3").unwrap();
        (dex, b)
    }

    #[test]
    fn noop_mask_covers_awake_sleep_talk() {
        let (dex, b) = st_setup();
        let noop = |b: &Battle, key: &str| certain_noop(b, &dex, 0, mv(&dex, key));
        let me = b.active_id(0).unwrap();
        let foe = b.active_id(1).unwrap();
        assert!(
            b.get_pokemon_action_speed(&dex, me) > b.get_pokemon_action_speed(&dex, foe),
            "fixture must put side 0 first on speed"
        );

        // awake and faster ⇒ both halves of the engine's onTry arm are refused
        assert!(
            noop(&b, "sleeptalk"),
            "awake Sleep Talk cannot call anything"
        );
        assert!(noop(&b, "snore"), "awake Snore, and it is physical");
        assert!(!noop(&b, "surf"), "the real move stays live");

        // asleep ⇒ exactly what the move is for
        {
            let mut b = b.clone();
            b.set_status(&dex, me, "slp", Some(me), EffectHandle::None, true);
            assert!(!noop(&b, "sleeptalk"), "asleep: Sleep Talk works");
            assert!(!noop(&b, "snore"), "asleep: Snore works");
        }

        // moving second is not a proof: a foe that sleeps us this turn hands
        // us a working Sleep Talk on the very turn the sleep lands
        // (`conditions.rs:159` returns Undef for a sleep_usable move after
        // decrementing). 15 of the 32 meta-pool teams carry a sleep move.
        {
            let mut b = b.clone();
            b.poke_mut(foe).boosts[4] = 6;
            assert!(!noop(&b, "sleeptalk"), "slower: the foe can sleep us first");
            assert!(!noop(&b, "snore"), "slower: the foe can sleep us first");
        }

        // the foe's bench is irrelevant — the rule reads our own status only
        {
            let mut stuck = b.clone();
            strand(&mut stuck, 1);
            assert!(noop(&stuck, "sleeptalk"), "a stranded foe changes nothing");
            let mut open = b.clone();
            open.poke_mut(foe).trapped = false;
            assert!(noop(&open, "sleeptalk"), "a switchable foe changes nothing");
        }

        // engine cross-check. Play the turn: the PP is gone, nothing happened,
        // and — unlike every other masked rule — the log carries NO marker,
        // so this asserts on state, not on a log string.
        {
            let st_id = dex.moves.id("sleeptalk").unwrap();
            let pp_before = b.poke(me).get_move_slot(st_id).unwrap().pp;
            let mut after = b.clone();
            after.set_log_enabled(true);
            after.choose(&dex, 0, "move sleeptalk").unwrap();
            after.choose(&dex, 1, "move splash").unwrap();
            assert_eq!(
                after.poke(me).get_move_slot(st_id).unwrap().pp,
                pp_before - 1,
                "the PP is spent anyway"
            );
            assert_eq!(
                after.poke(me).hp,
                after.poke(me).maxhp,
                "no Rest was called"
            );
            assert_eq!(
                after.poke(foe).hp,
                after.poke(foe).maxhp,
                "no attack was called"
            );
            assert_eq!(after.poke(me).status, Status::None, "and no self-status");
            let ms = after.poke_str(me);
            assert!(
                !after
                    .log
                    .iter()
                    .any(|l| l.starts_with(&format!("|-fail|{ms}"))),
                "the engine fails it silently, with no -fail: {:?}",
                after.log
            );
        }
    }

    /// The mask vetoes, it does not forbid: when every legal action is
    /// refused, `best()` falls back to the unfiltered argmax and still
    /// submits one. Battle 4070's turns 89-99 are the real instance — Rest at
    /// full HP was the only move with PP left.
    #[test]
    fn best_still_submits_the_masked_noop_when_it_is_the_only_option() {
        let dex = conformance::load_dex();
        let t: Vec<PokemonSet> = serde_json::from_str(
            r#"[
            {"name":"Suicune","species":"Suicune","item":"","ability":"No Ability",
             "moves":["Sleep Talk"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"N","level":50},
            {"name":"Snorlax","species":"Snorlax","item":"","ability":"No Ability",
             "moves":["Splash","Body Slam","Rest","Sleep Powder"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Exeggutor","species":"Exeggutor","item":"","ability":"No Ability",
             "moves":["Splash","Psychic","Rest","Sleep Powder"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}
        ]"#,
        )
        .unwrap();
        let mut b = Battle::from_fixture(&dex, "7,8,9,10", &t, &t).unwrap();
        b.set_log_enabled(false);
        b.choose(&dex, 0, "team 1, 2, 3").unwrap();
        b.choose(&dex, 1, "team 2, 1, 3").unwrap();
        strand(&mut b, 0);
        let acts = b.clone().legal_choices(&dex, 0);
        assert_eq!(acts.len(), 1, "the fixture must leave exactly one action");
        assert_eq!(
            dominated_actions(&b, &dex, 0).len(),
            1,
            "and the mask must refuse it"
        );

        let cfg = RmConfig {
            rule: SelRule::Ucb,
            iterations: 200,
            ..Default::default()
        };
        for seed in 1..=5u64 {
            let mut s = SkuctSearch::new(&b, &dex, cfg.clone(), seed);
            s.step(&dex, 200);
            assert_eq!(
                s.best(0).map(|c| c.to_input(&dex)),
                Some("move sleeptalk".to_string()),
                "best() must still submit something (seed {seed})"
            );
        }
    }

    /// The A/B seam: `MaskRules::default()` is the shipped mask, and the
    /// ablation arm (arena `blindlegacy`) sees exactly one rule fewer.
    #[test]
    fn mask_rules_ablation_isolates_the_sleep_talk_rule() {
        let (dex, b) = st_setup();
        let legacy = MaskRules {
            sleep_talk_awake: false,
            ..MaskRules::default()
        };
        assert!(
            certain_noop(&b, &dex, 0, mv(&dex, "sleeptalk")),
            "shipped refuses it"
        );
        assert!(
            !super::certain_noop(&b, &dex, 0, mv(&dex, "sleeptalk"), legacy),
            "the ablation arm does not"
        );
        // …and nothing else moves: same position, same other verdicts.
        let shipped: Vec<&str> = dominated_actions(&b, &dex, 0)
            .into_iter()
            .map(|(_, why)| why)
            .collect();
        let ablated: Vec<&str> = dominated_actions_with(&b, &dex, 0, legacy)
            .into_iter()
            .map(|(_, why)| why)
            .collect();
        assert_eq!(
            shipped
                .iter()
                .filter(|w| !w.starts_with("Sleep Talk"))
                .count(),
            ablated.len(),
            "the flag must move only its own rule: {shipped:?} vs {ablated:?}"
        );
    }

    // ---- 2026-07-27: the same class, everywhere else it occurs -----------
    //
    // The mask existed but covered four rules; the engine fails a move for
    // many more reasons that are all readable off public state. Each case
    // below cites the engine site it mirrors, and the ones with a state
    // consequence are cross-checked by actually running the turn and
    // asserting the effect did not land.

    fn imm_team() -> Vec<PokemonSet> {
        serde_json::from_str(
            r#"[
            {"name":"Zapdos","species":"Zapdos","item":"","ability":"No Ability",
             "moves":["Thunder Wave","Toxic","Leech Seed","Swords Dance"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"N","level":50},
            {"name":"Nidoking","species":"Nidoking","item":"","ability":"No Ability",
             "moves":["Earthquake","Substitute","Screech","Dream Eater"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Exeggutor","species":"Exeggutor","item":"","ability":"No Ability",
             "moves":["Confuse Ray","Safeguard","Mist","Swagger"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}
        ]"#,
        )
        .unwrap()
    }

    /// `lead1` picks which of the three leads side 1 sends out. Both benches
    /// are fainted: the foe-state rules only apply to a foe that cannot
    /// leave, and `switchable_foe_disarms_every_foe_state_rule` covers the
    /// other half.
    fn imm_setup(lead1: &str) -> (Dex, Battle) {
        let (dex, mut b) = imm_setup_open(lead1);
        strand(&mut b, 0);
        strand(&mut b, 1);
        (dex, b)
    }

    /// The same position with both benches intact.
    fn imm_setup_open(lead1: &str) -> (Dex, Battle) {
        let dex = conformance::load_dex();
        let t = imm_team();
        let mut b = Battle::from_fixture(&dex, "7,8,9,10", &t, &t).unwrap();
        b.set_log_enabled(false);
        b.choose(&dex, 0, "team 1, 2, 3").unwrap();
        b.choose(&dex, 1, lead1).unwrap();
        (dex, b)
    }

    /// The rule the owner caught missing: a foe with somewhere to go makes
    /// every current-foe reading provisional, because switches resolve before
    /// moves. Thunder Wave at a Ground type is aimed at its replacement.
    #[test]
    fn switchable_foe_disarms_every_foe_state_rule() {
        let (dex, open) = imm_setup_open("team 2, 1, 3");
        let mut stuck = open.clone();
        strand(&mut stuck, 1);
        let noop = |b: &Battle, side: usize, key: &str| certain_noop(b, &dex, side, mv(&dex, key));
        for key in ["thunderwave", "toxic"] {
            assert!(
                !noop(&open, 0, key),
                "{key} is a switch read while the foe has a bench"
            );
            assert!(
                noop(&stuck, 0, key),
                "{key} is refused once the foe is stuck"
            );
        }
        // Trapping is the other way to be stuck, and the engine's own flag is
        // what both the mask and `legal_choices` read.
        {
            let mut trapped = open.clone();
            let def = trapped.active_id(1).unwrap();
            trapped.poke_mut(def).trapped = true;
            assert!(
                noop(&trapped, 0, "thunderwave"),
                "a trapped foe cannot leave either"
            );
        }
        // Own-state rules are unaffected: a switch cannot heal me, refill my
        // HP, or move my stat stages.
        {
            let mut b = open.clone();
            let att = b.active_id(0).unwrap();
            b.poke_mut(att).boosts[0] = 6;
            assert!(
                noop(&b, 0, "swordsdance"),
                "capped self-boost does not care"
            );
        }
        // Safeguard is a SIDE condition: it survives the switch, so it still
        // refuses whoever comes in.
        {
            let mut b = open.clone();
            let def = b.active_id(1).unwrap();
            b.add_side_condition(&dex, 1, "safeguard", Some(def), EffectHandle::None);
            assert!(noop(&b, 0, "toxic"), "Safeguard covers the replacement too");
        }
    }

    /// A phaze is a no-op exactly when the foe has nobody to be dragged in.
    /// The trapped case is the false positive this rule has to dodge:
    /// `foe_can_switch` calls a trapped foe stuck, but the engine's
    /// `can_switch` never reads `trapped` and the drag lands anyway.
    #[test]
    fn noop_mask_covers_phaze_with_no_bench() {
        let dex = conformance::load_dex();
        let phazer: Vec<PokemonSet> = serde_json::from_str(
            r#"[
            {"name":"Skarmory","species":"Skarmory","item":"","ability":"No Ability",
             "moves":["Whirlwind","Drill Peck","Rest","Curse"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Exeggutor","species":"Exeggutor","item":"","ability":"No Ability",
             "moves":["Sleep Powder","Reflect","Rest","Spikes"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Zapdos","species":"Zapdos","item":"","ability":"No Ability",
             "moves":["Thunder Wave","Toxic","Leech Seed","Swords Dance"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"N","level":50}
        ]"#,
        )
        .unwrap();
        let foes: Vec<PokemonSet> = serde_json::from_str(
            r#"[
            {"name":"Snorlax","species":"Snorlax","item":"","ability":"No Ability",
             "moves":["Body Slam","Substitute","Confuse Ray","Rest"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Nidoking","species":"Nidoking","item":"","ability":"No Ability",
             "moves":["Earthquake","Substitute","Screech","Dream Eater"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Exeggutor","species":"Exeggutor","item":"","ability":"No Ability",
             "moves":["Confuse Ray","Safeguard","Mist","Swagger"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}
        ]"#,
        )
        .unwrap();
        let mut open = Battle::from_fixture(&dex, "7,8,9,10", &phazer, &foes).unwrap();
        open.set_log_enabled(false);
        open.choose(&dex, 0, "team 1, 2, 3").unwrap();
        open.choose(&dex, 1, "team 1, 2, 3").unwrap();
        // Gen 2 phazes also fail when the user moved FIRST (`moveexec.rs:619`),
        // a separate speed-dependent class this rule does not claim. Skarmory
        // outruns Snorlax, so slow it down to isolate the bench question.
        let att = open.active_id(0).unwrap();
        open.poke_mut(att).boosts[4] = -6;

        let noop = |b: &Battle, side: usize, key: &str| certain_noop(b, &dex, side, mv(&dex, key));

        // A live foe bench: not a no-op, and the engine really does drag.
        assert!(!noop(&open, 0, "whirlwind"), "a full bench is a live phaze");
        let dragged = play(&dex, &open, 0, 1);
        assert_ne!(
            dragged.active_id(1).unwrap(),
            open.active_id(1).unwrap(),
            "the engine dragged a bench mon in"
        );

        // Trapped, but the bench is alive: `can_switch` ignores `trapped`, so
        // the drag still lands and the mask must NOT claim it.
        {
            let mut trapped = open.clone();
            let def = trapped.active_id(1).unwrap();
            trapped.poke_mut(def).trapped = true;
            assert!(
                !noop(&trapped, 0, "whirlwind"),
                "a trapped foe is still draggable"
            );
            let after = play(&dex, &trapped, 0, 1);
            assert_ne!(
                after.active_id(1).unwrap(),
                trapped.active_id(1).unwrap(),
                "trapping does not stop a phaze"
            );
        }

        // Nobody left to drag in: refused, and the engine agrees nothing moved.
        {
            let mut stuck = open.clone();
            strand(&mut stuck, 1);
            assert!(noop(&stuck, 0, "whirlwind"), "phaze into an empty bench");
            let after = play(&dex, &stuck, 0, 1);
            assert_eq!(
                after.active_id(1).unwrap(),
                stuck.active_id(1).unwrap(),
                "the foe's active is untouched"
            );
        }
    }

    /// The Stadium 2 last-mon rule (`moveexec.rs:499`). Reported from ladder
    /// battle-4040 (2026-08-16): the bot's Gengar carries Ice Punch / Mean
    /// Look / Destiny Bond / Perish Song, and as the last mon it spent 8 of
    /// its 22 remaining turns on the two moves that cannot do anything.
    #[test]
    fn noop_mask_covers_last_mon_perish_song_and_destiny_bond() {
        let dex = conformance::load_dex();
        let ghost: Vec<PokemonSet> = serde_json::from_str(
            r#"[
            {"name":"Gengar","species":"Gengar","item":"","ability":"No Ability",
             "moves":["Ice Punch","Mean Look","Destiny Bond","Perish Song"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"F","level":50},
            {"name":"Exeggutor","species":"Exeggutor","item":"","ability":"No Ability",
             "moves":["Sleep Powder","Reflect","Rest","Spikes"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Cloyster","species":"Cloyster","item":"","ability":"No Ability",
             "moves":["Surf","Ice Beam","Screech","Toxic"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}
        ]"#,
        )
        .unwrap();
        let foes = team();
        let mut open = Battle::from_fixture(&dex, "7,8,9,10", &ghost, &foes).unwrap();
        open.set_log_enabled(false);
        open.choose(&dex, 0, "team 1, 2, 3").unwrap();
        open.choose(&dex, 1, "team 2, 1, 3").unwrap();
        let noop = |b: &Battle, side: usize, key: &str| certain_noop(b, &dex, side, mv(&dex, key));

        // The engine's own verdict, read off the turn it logs. Destiny Bond's
        // volatile is stripped again by `onFoeAfterMoveSelf` once the foe has
        // moved (`conditions.rs:828`), so a post-turn volatile check can only
        // speak for Perish Song — the hint line speaks for both.
        let hinted = |b: &Battle, slot: usize| -> bool {
            let mut b = b.clone();
            b.set_log_enabled(true);
            b.log.clear();
            let after = play(&dex, &b, 0, slot);
            after
                .log
                .iter()
                .any(|l| l.contains("used by your last Pokemon"))
        };

        // A live bench: both moves work, and the engine really does apply them.
        for key in ["perishsong", "destinybond"] {
            assert!(!noop(&open, 0, key), "{key} works while the bench is alive");
        }
        let ps_cond = dex.conds_id("perishsong").unwrap();
        let perished = play(&dex, &open, 0, 4);
        assert!(
            perished
                .poke(perished.active_id(1).unwrap())
                .has_volatile(ps_cond),
            "the engine set the perish counter"
        );
        assert!(!hinted(&open, 4), "no Stadium refusal with a bench");
        assert!(!hinted(&open, 3), "no Stadium refusal with a bench");

        // Last mon: refused, and the engine agrees nothing landed.
        let mut last = open.clone();
        strand(&mut last, 0);
        // `strand` plants the faints directly, so it does not run the
        // decrement at `turn.rs:297`. The rule and the engine both read
        // `pokemon_left`, so the fixture has to carry it.
        last.sides[0].pokemon_left = 1;
        for key in ["perishsong", "destinybond"] {
            assert!(noop(&last, 0, key), "{key} fails from the last mon");
        }
        let after_ps = play(&dex, &last, 0, 4);
        assert!(
            !after_ps
                .poke(after_ps.active_id(1).unwrap())
                .has_volatile(ps_cond),
            "no perish counter from the last mon"
        );
        assert!(hinted(&last, 4), "the engine refused Perish Song");
        assert!(hinted(&last, 3), "the engine refused Destiny Bond");

        // The rule is about our OWN bench, not the foe's, and it claims
        // nothing about the rest of the kit.
        let mut foe_last = open.clone();
        strand(&mut foe_last, 1);
        for key in ["perishsong", "destinybond"] {
            assert!(
                !noop(&foe_last, 0, key),
                "{key} does not read the foe's bench"
            );
        }
        assert!(!noop(&last, 0, "icepunch"), "the damaging move stays live");
        assert!(
            !noop(&last, 0, "meanlook"),
            "Mean Look still applies its volatile"
        );

        // …and it is the argmax consequence that matters: with the mask in
        // place `best` can never submit one of the two while Ice Punch exists,
        // however flat the root values are.
        let acts = last.clone().legal_choices(&dex, 0);
        let refused: Vec<&'static str> = dominated_actions(&last, &dex, 0)
            .into_iter()
            .map(|(_, why)| why)
            .collect();
        assert_eq!(
            refused.len(),
            2,
            "exactly the two moves, out of {}",
            acts.len()
        );
    }

    /// Run one turn with `mover` using move slot `slot` (1-based) and the
    /// other side using slot 1, then hand back the resolved battle.
    fn play(dex: &Dex, b: &Battle, mover: usize, slot: usize) -> Battle {
        let mut b = b.clone();
        let other = 1 - mover;
        b.choose(dex, mover, &format!("move {slot}")).unwrap();
        b.choose(dex, other, "move 1").unwrap();
        b
    }

    #[test]
    fn noop_mask_covers_type_and_status_immunities() {
        // side 0 Zapdos (Electric/Flying) vs side 1 Nidoking (Poison/Ground)
        let (dex, b) = imm_setup("team 2, 1, 3");
        let noop = |b: &Battle, side: usize, key: &str| certain_noop(b, &dex, side, mv(&dex, key));
        let def = b.active_id(1).unwrap();
        let att = b.active_id(0).unwrap();

        // Thunder Wave is Electric and Ground types are immune to the TYPE
        // (`pokemon.rs::run_move_immunity`), so the move never reaches its
        // status at all.
        assert!(
            noop(&b, 0, "thunderwave"),
            "Thunder Wave into a Ground type"
        );
        assert_eq!(play(&dex, &b, 0, 1).poke(def).status, Status::None);

        // Toxic is not type-immune here (Poison into Poison is a resist, not
        // an immunity) — it fails on the STATUS immunity instead
        // (`pokemon.rs::run_status_immunity`, typechart key `psn`/`tox`).
        assert!(noop(&b, 0, "toxic"), "Toxic onto a Poison type");
        assert_eq!(play(&dex, &b, 0, 2).poke(def).status, Status::None);

        // The same rule read the other way: a damaging move into an immune
        // target is just as wasted. Earthquake from Nidoking cannot touch a
        // Flying Zapdos.
        assert!(noop(&b, 1, "earthquake"), "Earthquake into a Flying type");
        let after = play(&dex, &b, 1, 1);
        assert_eq!(after.poke(att).hp, after.poke(att).maxhp, "no damage dealt");

        // Hidden Power's dex type is Normal, which would refuse it into
        // every Ghost; its real type comes from the user's DVs. The census
        // found the engine damaging Misdreavus through 7 such refusals.
        {
            let ghost: Vec<PokemonSet> = serde_json::from_str(
                r#"[
                {"name":"Misdreavus","species":"Misdreavus","item":"","ability":"No Ability",
                 "moves":["Confuse Ray","Pain Split","Perish Song","Protect"],
                 "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"F","level":50},
                {"name":"Gengar","species":"Gengar","item":"","ability":"No Ability",
                 "moves":["Night Shade","Hypnosis","Explosion","Thunderbolt"],
                 "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
                {"name":"Haunter","species":"Haunter","item":"","ability":"No Ability",
                 "moves":["Night Shade","Hypnosis","Explosion","Thunderbolt"],
                 "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}
            ]"#,
            )
            .unwrap();
            let mine: Vec<PokemonSet> = serde_json::from_str(
                r#"[
                {"name":"Zapdos","species":"Zapdos","item":"","ability":"No Ability",
                 "moves":["Hidden Power","Body Slam","Swords Dance","Thunderbolt"],
                 "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"N","level":50},
                {"name":"Nidoking","species":"Nidoking","item":"","ability":"No Ability",
                 "moves":["Earthquake","Substitute","Screech","Dream Eater"],
                 "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
                {"name":"Exeggutor","species":"Exeggutor","item":"","ability":"No Ability",
                 "moves":["Confuse Ray","Safeguard","Mist","Swagger"],
                 "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}
            ]"#,
            )
            .unwrap();
            let dex3 = conformance::load_dex();
            let mut b3 = Battle::from_fixture(&dex3, "7,8,9,10", &mine, &ghost).unwrap();
            b3.set_log_enabled(false);
            b3.choose(&dex3, 0, "team 1, 2, 3").unwrap();
            b3.choose(&dex3, 1, "team 1, 2, 3").unwrap();
            strand(&mut b3, 1); // the foe-state rules need a foe that cannot leave
            let me = b3.active_id(0).unwrap();
            assert_ne!(
                b3.poke(me).hp_type,
                dex3.known_types.normal,
                "the fixture's Hidden Power must not be Normal-typed"
            );
            assert!(
                !certain_noop(&b3, &dex3, 0, mv(&dex3, "hiddenpower")),
                "Hidden Power is typed by the user's DVs, not the dex's Normal placeholder"
            );
            // A genuinely Normal-typed attack into the same Ghost IS refused.
            assert!(
                certain_noop(&b3, &dex3, 0, mv(&dex3, "bodyslam")),
                "Body Slam into a Ghost"
            );
        }

        // Controls: the same moves are live against a legal target.
        let (dex2, b2) = imm_setup("team 3, 1, 2"); // side 1 leads Exeggutor
        let noop2 =
            |b: &Battle, side: usize, key: &str| certain_noop(b, &dex2, side, mv(&dex2, key));
        assert!(
            !noop2(&b2, 0, "thunderwave"),
            "Thunder Wave is live vs Grass/Psychic"
        );
        assert!(
            !noop2(&b2, 0, "toxic"),
            "Toxic is live vs a non-Poison type"
        );
        // ...but Leech Seed does not take on a Grass type
        // (`moveexec.rs` leechseed/onTryImmunity).
        assert!(noop2(&b2, 0, "leechseed"), "Leech Seed into a Grass type");
        let seeded = play(&dex2, &b2, 0, 3);
        let ls = dex2.conds_id("leechseed").unwrap();
        assert!(!seeded.poke(seeded.active_id(1).unwrap()).has_volatile(ls));
    }

    // ---- 2026-08-20: the two A/B arms over the type-immunity gate --------
    //
    // `noop_reason`'s foe-reading rules are dropped whenever the foe can
    // leave, because switches resolve before moves. Class A (a provably dead
    // attack aimed at the mon actually in front) is 7.53% of corpus
    // decisions and the shipped 30k search plays the dead move on 6.08% of
    // them, so the gate's price is worth measuring. Both arms default OFF;
    // these tests pin what each one changes and, as importantly, what it
    // does not.

    fn only(rule: fn(&mut MaskRules)) -> MaskRules {
        let mut r = MaskRules::default();
        rule(&mut r);
        r
    }

    /// Arm 1 refuses the mon in front and stops there: it never consults the
    /// bench, so it fires exactly where the shipped gate is silent.
    #[test]
    fn immunity_ignores_switch_read_refuses_the_mon_in_front() {
        // side 0 Zapdos (Electric/Flying) vs side 1 Nidoking (Poison/Ground),
        // BOTH benches intact — the shipped mask's switch-read case.
        let (dex, b) = imm_setup_open("team 2, 1, 3");
        let aggressive = only(|r| r.immunity_ignores_switch_read = true);
        let sound = only(|r| r.immunity_all_switchins = true);
        let tw = mv(&dex, "thunderwave");

        assert!(
            !certain_noop(&b, &dex, 0, tw),
            "shipped: a switchable foe disarms the rule"
        );
        assert!(
            super::certain_noop(&b, &dex, 0, tw, aggressive),
            "arm 1: the mon in front is Ground, refuse it anyway"
        );
        // Arm 2 must NOT fire here: the foe's bench holds an Exeggutor, which
        // Thunder Wave hits perfectly well. This is the exact case where the
        // aggressive arm can be wrong and the sound one cannot.
        assert!(
            !super::certain_noop(&b, &dex, 0, tw, sound),
            "arm 2: a hittable switch-in keeps the move live"
        );
        // and the engine agrees the move works on that switch-in
        {
            let mut after = b.clone();
            after.set_log_enabled(true);
            after.choose(&dex, 0, "move thunderwave").unwrap();
            after.choose(&dex, 1, "switch 3").unwrap(); // Exeggutor
            let def = after.active_id(1).unwrap();
            assert_eq!(
                after.poke(def).status,
                Status::Par,
                "the replacement was paralysed"
            );
        }

        // Control: nothing fires when the mon in front is not immune.
        let (dex2, b2) = imm_setup_open("team 3, 1, 2"); // Exeggutor in front
        for r in [aggressive, sound] {
            assert!(
                !super::certain_noop(&b2, &dex2, 0, mv(&dex2, "thunderwave"), r),
                "Thunder Wave is live vs Grass/Psychic under every rule set"
            );
        }

        // Each flag moves only its own rule: every OTHER verdict at this root
        // is untouched, so an A/B is measuring one thing.
        let base: Vec<&str> = dominated_actions(&b, &dex, 0)
            .into_iter()
            .map(|(_, w)| w)
            .collect();
        for r in [aggressive, sound] {
            let arm: Vec<&str> = dominated_actions_with(&b, &dex, 0, r)
                .into_iter()
                .map(|(_, w)| w)
                .collect();
            let added = arm.iter().filter(|w| !base.contains(w)).count();
            assert_eq!(
                arm.len() - added,
                base.len(),
                "a flag deleted an existing refusal: {base:?} vs {arm:?}"
            );
            assert!(
                arm.iter().all(|w| base.contains(w) || w.contains("immune")),
                "a flag moved a rule that is not the immunity one: {arm:?}"
            );
        }
    }

    /// Arm 2 is the sound one: it needs the mon in front AND every mon the
    /// foe could bring in to be immune. Ghost-vs-Normal is the class-A case
    /// battle 4069 actually played (Miltank's Return, then Snorlax's Body
    /// Slam, into a Misdreavus that could still leave).
    fn ghost_setup(bench3: &str) -> (Dex, Battle) {
        let mine: Vec<PokemonSet> = serde_json::from_str(
            r#"[
            {"name":"Snorlax","species":"Snorlax","item":"","ability":"No Ability",
             "moves":["Body Slam","Earthquake","Rest","Curse"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Nidoking","species":"Nidoking","item":"","ability":"No Ability",
             "moves":["Earthquake","Substitute","Screech","Dream Eater"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Exeggutor","species":"Exeggutor","item":"","ability":"No Ability",
             "moves":["Confuse Ray","Safeguard","Mist","Swagger"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}
        ]"#,
        )
        .unwrap();
        let foe_json = format!(
            r#"[
            {{"name":"Misdreavus","species":"Misdreavus","item":"","ability":"No Ability",
             "moves":["Confuse Ray","Pain Split","Perish Song","Mean Look"],
             "nature":"Serious","evs":{{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255}},"gender":"F","level":50}},
            {{"name":"Gengar","species":"Gengar","item":"","ability":"No Ability",
             "moves":["Night Shade","Hypnosis","Thunderbolt","Psychic"],
             "nature":"Serious","evs":{{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255}},"gender":"M","level":50}},
            {{"name":"{bench3}","species":"{bench3}","item":"","ability":"No Ability",
             "moves":["Night Shade","Hypnosis","Thunderbolt","Psychic"],
             "nature":"Serious","evs":{{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255}},"gender":"M","level":50}}
        ]"#
        );
        let foes: Vec<PokemonSet> = serde_json::from_str(&foe_json).unwrap();
        let dex = conformance::load_dex();
        let mut b = Battle::from_fixture(&dex, "7,8,9,10", &mine, &foes).unwrap();
        b.set_log_enabled(false);
        b.choose(&dex, 0, "team 1, 2, 3").unwrap();
        b.choose(&dex, 1, "team 1, 2, 3").unwrap();
        (dex, b)
    }

    #[test]
    fn immunity_all_switchins_needs_every_switch_in_immune() {
        let aggressive = only(|r| r.immunity_ignores_switch_read = true);
        let disabled = only(|r| r.immunity_all_switchins = false);
        let sound = only(|r| r.immunity_all_switchins = true);

        // (a) an all-Ghost opponent: Body Slam is dead whatever they do.
        {
            let (dex, b) = ghost_setup("Haunter");
            let bs = mv(&dex, "bodyslam");
            assert!(
                !super::certain_noop(&b, &dex, 0, bs, disabled),
                "disabled control: the foe can still leave"
            );
            assert!(
                certain_noop(&b, &dex, 0, bs),
                "shipped default: every switch-in is a Ghost"
            );
            assert!(
                super::certain_noop(&b, &dex, 0, bs, aggressive),
                "arm 1 refuses"
            );
            assert!(
                super::certain_noop(&b, &dex, 0, bs, sound),
                "arm 2 refuses: every switch-in is a Ghost"
            );
            // Earthquake, on the same board, is live under BOTH arms — the
            // rule is about the move's type, not about the position.
            assert!(!super::certain_noop(
                &b,
                &dex,
                0,
                mv(&dex, "earthquake"),
                sound
            ));
            assert!(!super::certain_noop(
                &b,
                &dex,
                0,
                mv(&dex, "earthquake"),
                aggressive
            ));
            // engine cross-check on the branch arm 2 claims to have proved:
            // the foe switches, and the Normal attack still does nothing.
            let mut after = b.clone();
            after.set_log_enabled(true);
            after.choose(&dex, 0, "move bodyslam").unwrap();
            after.choose(&dex, 1, "switch 2").unwrap(); // Gengar
            let def = after.active_id(1).unwrap();
            assert_eq!(
                after.poke(def).hp,
                after.poke(def).maxhp,
                "no damage to the switch-in"
            );
        }

        // (b) one hittable mon on the bench and arm 2 goes quiet, while the
        // aggressive arm still refuses — this is the difference between them.
        {
            let (dex, b) = ghost_setup("Snorlax");
            let bs = mv(&dex, "bodyslam");
            assert!(
                super::certain_noop(&b, &dex, 0, bs, aggressive),
                "arm 1 does not care"
            );
            assert!(
                !super::certain_noop(&b, &dex, 0, bs, sound),
                "arm 2: the Snorlax switch-in is hittable"
            );
            // …and once that mon is dead the proof is back (fainted mons are
            // dropped from the candidate set).
            let mut dead = b.clone();
            let slot = dead.sides[1].party[2];
            let id = PokeId { side: 1, slot };
            dead.poke_mut(id).hp = 0;
            dead.poke_mut(id).fainted = true;
            assert!(
                super::certain_noop(&dead, &dex, 0, bs, sound),
                "arm 2: the only hittable switch-in has fainted"
            );
        }
    }

    /// The soundness question that only a 6-mon roster can ask: with a pick
    /// still unrevealed, "the bench" is not public. Party slots that have
    /// never appeared are imputed on the ladder (`import.rs`), so arm 2 must
    /// admit EVERY not-yet-appeared roster mon — and go quiet if any of them
    /// is hittable — until the last pick has shown itself.
    #[test]
    fn immunity_all_switchins_admits_unrevealed_picks() {
        let disabled = only(|r| r.immunity_all_switchins = false);
        let sound = only(|r| r.immunity_all_switchins = true);
        let mine: Vec<PokemonSet> = serde_json::from_str(
            r#"[
            {"name":"Snorlax","species":"Snorlax","item":"","ability":"No Ability",
             "moves":["Body Slam","Earthquake","Rest","Curse"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Nidoking","species":"Nidoking","item":"","ability":"No Ability",
             "moves":["Earthquake","Substitute","Screech","Dream Eater"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Exeggutor","species":"Exeggutor","item":"","ability":"No Ability",
             "moves":["Confuse Ray","Safeguard","Mist","Swagger"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}
        ]"#,
        )
        .unwrap();
        // Six: three Ghosts (the picks) and three that are not.
        let foes: Vec<PokemonSet> = serde_json::from_str(
            r#"[
            {"name":"Misdreavus","species":"Misdreavus","item":"","ability":"No Ability",
             "moves":["Confuse Ray","Pain Split","Perish Song","Mean Look"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"F","level":50},
            {"name":"Gengar","species":"Gengar","item":"","ability":"No Ability",
             "moves":["Night Shade","Hypnosis","Thunderbolt","Psychic"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Haunter","species":"Haunter","item":"","ability":"No Ability",
             "moves":["Night Shade","Hypnosis","Thunderbolt","Psychic"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Snorlax","species":"Snorlax","item":"","ability":"No Ability",
             "moves":["Body Slam","Earthquake","Rest","Curse"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Exeggutor","species":"Exeggutor","item":"","ability":"No Ability",
             "moves":["Confuse Ray","Safeguard","Mist","Swagger"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50},
            {"name":"Nidoking","species":"Nidoking","item":"","ability":"No Ability",
             "moves":["Earthquake","Substitute","Screech","Dream Eater"],
             "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}
        ]"#,
        )
        .unwrap();
        let dex = conformance::load_dex();
        let mut b = Battle::from_fixture(&dex, "7,8,9,10", &mine, &foes).unwrap();
        b.set_log_enabled(false);
        b.choose(&dex, 0, "team 1, 2, 3").unwrap();
        b.choose(&dex, 1, "team 1, 2, 3").unwrap(); // the three Ghosts
        assert_eq!(
            b.sides[1].roster.len(),
            6,
            "the fixture must carry a full roster"
        );
        assert_eq!(b.sides[1].party.len(), 3);

        let bs = mv(&dex, "bodyslam");
        // Only the lead has appeared, so two picks are unknown and every
        // never-appeared roster mon is admitted — including the Snorlax that
        // was not even picked. Refusing here would be reading the imputed
        // party, and on the ladder that party is a guess.
        assert!(
            !super::certain_noop(&b, &dex, 0, bs, sound),
            "a hidden pick could be anything not yet seen"
        );
        assert_eq!(
            super::foe_switchin_candidates(&b, 0).len(),
            5,
            "candidates = every alive roster mon but the active"
        );

        // Reveal the other two picks (the same predicate the observer and
        // the determinizer use) and the candidate set collapses to the real
        // bench, which is all Ghost — so now the proof holds.
        for pos in 1..3 {
            let slot = b.sides[1].party[pos];
            b.poke_mut(PokeId { side: 1, slot }).previously_switched_in = 1;
        }
        assert_eq!(
            super::foe_switchin_candidates(&b, 0).len(),
            2,
            "the real bench, exactly"
        );
        assert!(
            super::certain_noop(&b, &dex, 0, bs, sound),
            "every pick is public now and all three are Ghosts"
        );
        assert!(
            certain_noop(&b, &dex, 0, bs),
            "shipped default: every pick is public now and all three are Ghosts"
        );
        // The disabled control still says nothing, in every one of these states.
        assert!(
            !super::certain_noop(&b, &dex, 0, bs, disabled),
            "disabled control gate is unchanged"
        );
    }

    #[test]
    fn public_noop_reason_respects_wait_exempt_and_type_change_guards() {
        // 1. Transformed Hidden Power and Conversion/Sketch-capable foes keep immunity_all_switchins quiet.
        let (dex, b) = ghost_setup("Haunter");
        let hp = mv(&dex, "hiddenpower");
        let mut transformed_att = b.clone();
        transformed_att
            .poke_mut(transformed_att.active_id(0).unwrap())
            .transformed = true;
        assert!(
            super::public_noop_reason(
                &transformed_att,
                &dex,
                0,
                hp,
                MaskRules::default()
            )
            .is_none(),
            "transformed user Hidden Power type is not publicly certain"
        );
        let porygon: Vec<PokemonSet> = serde_json::from_str(
            r#"[{"name":"Porygon2","species":"Porygon2","item":"","ability":"No Ability",
              "moves":["Tri Attack","Recover","Conversion 2","Ice Beam"],
              "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"N","level":50}]"#,
        )
        .unwrap();
        let snorlax_only: Vec<PokemonSet> = serde_json::from_str(
            r#"[{"name":"Snorlax","species":"Snorlax","item":"","ability":"No Ability",
              "moves":["Body Slam","Earthquake"],
              "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}]"#,
        )
        .unwrap();
        let mut bp = Battle::from_fixture(&dex, "1,2,3,4", &snorlax_only, &porygon).unwrap();
        bp.choose(&dex, 0, "team 1").unwrap();
        bp.choose(&dex, 1, "team 1").unwrap();
        let mut b_ghost = bp.clone();
        let f_id = b_ghost.active_id(1).unwrap();
        b_ghost.poke_mut(f_id).types = nc2000_engine::dex::TypeList::one(dex.known_types.ghost);
        assert!(
            super::public_noop_reason(
                &b_ghost,
                &dex,
                0,
                mv(&dex, "bodyslam"),
                MaskRules::default()
            )
            .is_none(),
            "foe with Conversion 2 in format learnset may change type"
        );

        // 2. wait_exempt unmasks last-mon Perish Song in public_noop_reason when foe is poisoned,
        //    while raw certain_noop still reports the move mechanics failure.
        let gengar_last: Vec<PokemonSet> = serde_json::from_str(
            r#"[{"name":"Gengar","species":"Gengar","item":"","ability":"No Ability",
              "moves":["Ice Punch","Perish Song"],
              "nature":"Serious","evs":{"hp":255,"atk":255,"def":255,"spa":255,"spd":255,"spe":255},"gender":"M","level":50}]"#,
        )
        .unwrap();
        let mut bw = Battle::from_fixture(&dex, "1,2,3,4", &gengar_last, &snorlax_only).unwrap();
        bw.choose(&dex, 0, "team 1").unwrap();
        bw.choose(&dex, 1, "team 1").unwrap();
        let ps = mv(&dex, "perishsong");
        assert!(super::public_noop_reason(&bw, &dex, 0, ps, MaskRules::default()).is_some());
        let fw = bw.active_id(1).unwrap();
        bw.poke_mut(fw).status = Status::Tox;
        assert!(super::public_noop_reason(&bw, &dex, 0, ps, MaskRules::default()).is_none());
        assert!(certain_noop(&bw, &dex, 0, ps));
    }

    #[test]
    fn noop_mask_covers_substitute_safeguard_and_mist() {
        // side 0 Zapdos vs side 1 Exeggutor (holds Confuse Ray/Safeguard/Mist/Swagger)
        let (dex, b) = imm_setup("team 3, 1, 2");
        let noop = |b: &Battle, side: usize, key: &str| certain_noop(b, &dex, side, mv(&dex, key));
        let att = b.active_id(0).unwrap();
        let def = b.active_id(1).unwrap();

        // Safeguard blocks foe-inflicted status AND confusion
        // (`conditions.rs` safeguard/onSetStatus + the confusion branch).
        {
            let mut b = b.clone();
            b.add_side_condition(&dex, 1, "safeguard", Some(def), EffectHandle::None);
            assert!(noop(&b, 0, "toxic"), "Toxic into Safeguard");
            assert!(!noop(&b, 1, "confuseray"), "Side 1 Safeguard must not block side 1 Confuse Ray");
            let after = play(&dex, &b, 0, 2);
            assert_eq!(after.poke(def).status, Status::None);
        }
        // A Substitute blocks confusion and stat drops, but NOT Swagger: the
        // engine strips Swagger's confusion behind a sub and still lands the
        // +2 Attack (`conditions.rs` substitute/onTryPrimaryHit).
        {
            let mut b = b.clone();
            b.add_volatile(&dex, att, "substitute", None, EffectHandle::None);
            assert!(noop(&b, 1, "confuseray"), "Confuse Ray into a Substitute");
            assert!(
                !noop(&b, 1, "swagger"),
                "Swagger still boosts through a Substitute"
            );
            let cf = dex.conds_id("confusion").unwrap();
            let after = play(&dex, &b, 1, 1);
            assert!(!after.poke(att).has_volatile(cf), "no confusion landed");
        }
        // Mist deletes foe-sourced stat drops (`conditions.rs` mist/onTryBoost).
        {
            let (dex, b) = imm_setup("team 2, 1, 3"); // side 1 Nidoking has Screech
            let att = b.active_id(0).unwrap();
            let mut b = b.clone();
            b.add_volatile(&dex, att, "mist", None, EffectHandle::None);
            assert!(
                certain_noop(&b, &dex, 1, mv(&dex, "screech")),
                "Screech into Mist changes nothing"
            );
            let after = play(&dex, &b, 1, 3);
            assert_eq!(after.poke(att).boosts[1], 0, "defence untouched");
        }
    }

    #[test]
    fn noop_mask_covers_capped_boosts_and_weak_substitute() {
        let (dex, b) = imm_setup("team 2, 1, 3");
        let noop = |b: &Battle, side: usize, key: &str| certain_noop(b, &dex, side, mv(&dex, key));
        let att = b.active_id(0).unwrap();
        let def = b.active_id(1).unwrap();

        // A stat move that cannot move its stat fails outright (`dmg.rs::boost`
        // reports no change and moveexec's didSomething chain fails the move).
        assert!(!noop(&b, 0, "swordsdance"), "live at +0");
        {
            let mut b = b.clone();
            b.poke_mut(att).boosts[0] = 6;
            assert!(noop(&b, 0, "swordsdance"), "Swords Dance at +6");
            let after = play(&dex, &b, 0, 4);
            assert_eq!(after.poke(att).boosts[0], 6);
        }
        {
            let mut b = b.clone();
            b.poke_mut(att).boosts[1] = -6;
            assert!(
                !noop(&b, 1, "screech"),
                "slower Screech must stay live when faster foe can raise Def first"
            );
            b.poke_mut(def).boosts[4] = 6;
            assert!(noop(&b, 1, "screech"), "faster Screech at -6 defence");
            let after = play(&dex, &b, 1, 3);
            assert_eq!(after.poke(att).boosts[1], -6);
        }
        // Substitute needs more than a quarter of max HP
        // (`moveexec.rs` substitute/onTryHit).
        assert!(!noop(&b, 1, "substitute"), "live at full HP");
        {
            let mut b = b.clone();
            let quarter = b.poke(def).maxhp / 4;
            b.poke_mut(def).hp = quarter;
            // Against Zapdos (whose learnset has no HP-raising move like Pain Split/Present),
            // Substitute at <= 25% HP is a guaranteed no-op even when moving second.
            assert!(
                noop(&b, 1, "substitute"),
                "slower against a foe that cannot raise our HP: still a guaranteed no-op"
            );
            // If the faster foe is transformed (or could carry Pain Split / Present),
            // our HP is only guaranteed to stay <= 25% when we act first.
            b.poke_mut(att).transformed = true;
            assert!(
                !noop(&b, 1, "substitute"),
                "slower against a transformed foe that could raise our HP first"
            );
            b.poke_mut(def).boosts[4] = 6;
            assert!(
                noop(&b, 1, "substitute"),
                "Substitute at exactly a quarter, moving first"
            );
            let sub = dex.conds_id("substitute").unwrap();
            let after = play(&dex, &b, 1, 2);
            assert!(!after.poke(after.active_id(1).unwrap()).has_volatile(sub));
        }
        // Dream Eater needs a sleeping target (`moveexec.rs` dreameater/onTryImmunity),
        // proved only when acting first so a faster awake foe cannot Rest before us.
        {
            let mut b = b.clone();
            assert!(!noop(&b, 1, "dreameater"), "slower Dream Eater into awake Rest learner stays live");
            b.poke_mut(def).boosts[4] = 6;
            assert!(noop(&b, 1, "dreameater"), "faster Dream Eater on an awake foe");
        }
        {
            let mut b = b.clone();
            b.set_status(&dex, att, "slp", None, EffectHandle::None, true);
            assert!(!noop(&b, 1, "dreameater"), "live once the foe sleeps");
        }
    }

    #[test]
    fn best_never_picks_masked_noop() {
        let (dex, mut b) = setup();
        let att = b.active_id(0).unwrap();
        b.add_side_condition(&dex, 0, "reflect", Some(att), EffectHandle::None);
        let cfg = RmConfig {
            rule: SelRule::Ucb,
            iterations: 200,
            ..Default::default()
        };
        for seed in 1..=10u64 {
            let mut s = SkuctSearch::new(&b, &dex, cfg.clone(), seed);
            s.step(&dex, 200);
            let best = s.best(0).map(|c| c.to_input(&dex)).unwrap();
            assert_ne!(
                best, "move reflect",
                "masked no-op won argmax (seed {seed})"
            );
            assert_ne!(
                best, "move rest",
                "rest at full HP won argmax (seed {seed})"
            );
        }
    }

    #[test]
    fn non_ghost_curse_and_slower_destiny_bond_and_encored_stayin_and_stranded_sleep_edges() {
        let (dex, mut b) = ghost_setup("Haunter");
        strand(&mut b, 1);
        let snorlax = b.active_id(0).unwrap();
        let misdreavus = b.active_id(1).unwrap();

        // 1. Non-Ghost Curse vs a stranded foe that already carries `curse` volatile:
        // Snorlax's Curse boosts (+1 Atk, +1 Def, -1 Spe) instead of applying `curse`.
        b.add_volatile(
            &dex,
            misdreavus,
            "curse",
            Some(misdreavus),
            EffectHandle::None,
        );
        b.poke_mut(snorlax).boosts[4] = 6; // make Snorlax faster
        assert!(
            !certain_noop(&b, &dex, 0, mv(&dex, "curse")),
            "non-Ghost Curse boosts user even when foe has curse volatile"
        );
        b.poke_mut(snorlax).boosts[0] = 6;
        b.poke_mut(snorlax).boosts[1] = 6;
        assert!(
            certain_noop(&b, &dex, 0, mv(&dex, "curse")),
            "non-Ghost Curse at +6 Atk/+6 Def when faster is capped"
        );

        // 2. Slower Destiny Bond re-arm: when slower Misdreavus still carries
        // `destinybond` from moving second last turn, the faster foe's move
        // clears `destinybond` via `onFoeAfterMoveSelf` before Misdreavus acts.
        let (_, mut db) = st_setup();
        strand(&mut db, 1);
        let def1 = db.active_id(1).unwrap();
        db.add_volatile(&dex, def1, "destinybond", Some(def1), EffectHandle::None);
        assert!(
            !certain_noop(&db, &dex, 1, mv(&dex, "destinybond")),
            "slower holder re-arms Destiny Bond after faster foe acts"
        );
        db.poke_mut(def1).boosts[4] = 6; // now side 1 is faster and side 0 is stranded
        strand(&mut db, 0);
        // Side 0 is Suicune, whose learnset includes Protect/Endure (+2 priority):
        // it can still move before side 1 and strip Destiny Bond first.
        assert!(
            !certain_noop(&db, &dex, 1, mv(&dex, "destinybond")),
            "faster holder vs stranded Protect/Endure learner may still re-arm Destiny Bond"
        );
        let sui = db.active_id(0).unwrap();
        db.poke_mut(sui).species = dex.species.id("unown").unwrap();
        assert!(
            certain_noop(&db, &dex, 1, mv(&dex, "destinybond")),
            "faster holder vs stranded zero-priority foe cannot re-apply Destiny Bond"
        );

        // 3. Encore lock on a no-op move:
        //    - Without switch-lost state (Substitute / positive boosts / trapped foe),
        //      it remains masked so flat-root argmax cannot stay in on a dead move.
        //    - With switch-lost state (e.g. Substitute or +1 boost), it is unmasked so
        //      the holder is not forced to switch out and forfeit its state.
        let (_, mut enc) = setup();
        let e_att = enc.active_id(0).unwrap();
        enc.add_side_condition(&dex, 0, "reflect", Some(e_att), EffectHandle::None);
        assert!(
            dominated_reason(&enc, &dex, 0, mv(&dex, "reflect"), MaskRules::default()).is_some()
        );
        enc.poke_mut(e_att).last_move_encore = dex.moves.id("reflect");
        enc.add_volatile(&dex, e_att, "encore", None, EffectHandle::None);
        assert!(
            dominated_reason(&enc, &dex, 0, mv(&dex, "reflect"), MaskRules::default()).is_some(),
            "Encored active mon without Substitute/boosts/trap still masks dead move"
        );
        enc.poke_mut(e_att).boosts[0] = 1;
        assert!(
            dominated_reason(&enc, &dex, 0, mv(&dex, "reflect"), MaskRules::default()).is_none(),
            "Encored active mon with positive boosts or Substitute must not be forced to Switch"
        );

        // 4. Stranded active-only sleeping foe: sleep move cannot trigger Sleep Clause forfeit.
        let (_, mut slp_b) = setup();
        strand(&mut slp_b, 1);
        let s_def = slp_b.active_id(1).unwrap();
        slp_b.set_status(&dex, s_def, "slp", Some(e_att), EffectHandle::None, true);
        assert_eq!(
            dominated_reason(
                &slp_b,
                &dex,
                0,
                mv(&dex, "sleeppowder"),
                MaskRules::default()
            ),
            Some("the target already carries a major status")
        );
        slp_b.add_volatile(&dex, s_def, "leechseed", Some(e_att), EffectHandle::None);
        assert_eq!(
            dominated_reason(
                &slp_b,
                &dex,
                0,
                mv(&dex, "sleeppowder"),
                MaskRules::default()
            ),
            None,
            "harmless sleep move into stranded sleeping foe is unmasked during residual wait"
        );
    }

    #[test]
    fn belly_drum_baton_pass_heal_bell_morning_sun_and_own_recovery_wait_edges() {
        let (dex, mut b) = setup();
        let me = b.active_id(0).unwrap();
        let foe = b.active_id(1).unwrap();
        let rules = MaskRules::default();
        b.poke_mut(me).boosts[4] = 6; // side 0 acts first

        // 1. Moonlight / Morning Sun / Synthesis at full HP are provable no-ops
        for key in ["moonlight", "morningsun", "synthesis"] {
            assert_eq!(
                noop_reason(&b, &dex, 0, mv(&dex, key), rules),
                Some("healing at full HP, and faster"),
                "{key} at full HP must be masked as a no-op"
            );
        }
        b.poke_mut(me).hp -= 1;
        for key in ["moonlight", "morningsun", "synthesis"] {
            assert_eq!(noop_reason(&b, &dex, 0, mv(&dex, key), rules), None);
        }
        b.poke_mut(me).hp = b.poke(me).maxhp;

        // 2. Baton Pass with bench alive is live; when stranded (last mon), it is a no-op
        assert_eq!(noop_reason(&b, &dex, 0, mv(&dex, "batonpass"), rules), None);
        strand(&mut b, 0);
        assert_eq!(
            noop_reason(&b, &dex, 0, mv(&dex, "batonpass"), rules),
            Some("Baton Pass fails with nobody left to switch to")
        );

        // 3. Belly Drum when HP <= 50% or Attack already at +6 is a no-op
        assert_eq!(noop_reason(&b, &dex, 0, mv(&dex, "bellydrum"), rules), None);
        b.poke_mut(me).hp = b.poke(me).maxhp / 2;
        assert_eq!(
            noop_reason(&b, &dex, 0, mv(&dex, "bellydrum"), rules),
            Some("Belly Drum needs over half HP and uncapped Attack")
        );
        // Even with Leftovers and slower, if hp + leftovers <= maxhp/2 and foe has no Pain Split/Present/Metronome, still no-op
        let orig_foe_species = b.poke(foe).species;
        let orig_me_item = b.poke(me).item;
        b.poke_mut(foe).species = dex.species.id("cloyster").unwrap();
        b.poke_mut(me).boosts[4] = -6;
        b.poke_mut(me).item = dex.items.id("leftovers");
        b.poke_mut(me).hp = b.poke(me).maxhp / 4;
        assert_eq!(
            noop_reason(&b, &dex, 0, mv(&dex, "bellydrum"), rules),
            Some("Belly Drum needs over half HP and uncapped Attack")
        );
        // Narrowed own_recovery_wait must NOT unmask dead Belly Drum against Cloyster (no Counter/Mirror Coat/Metronome)
        assert!(
            dominated_reason(&b, &dex, 0, mv(&dex, "bellydrum"), rules).is_some(),
            "Leftovers alone must not unmask dead Belly Drum against Cloyster"
        );
        b.poke_mut(foe).species = orig_foe_species;
        b.poke_mut(me).item = orig_me_item;
        // At full HP and +6 Attack while faster, Belly Drum also fails
        b.poke_mut(me).hp = b.poke(me).maxhp;
        b.poke_mut(me).boosts[4] = 6;
        b.poke_mut(me).boosts[0] = 6;
        assert_eq!(
            noop_reason(&b, &dex, 0, mv(&dex, "bellydrum"), rules),
            Some("Belly Drum needs over half HP and uncapped Attack")
        );
        b.poke_mut(me).boosts[0] = 0;

        // 4. Heal Bell when no party member has a major status
        assert_eq!(
            noop_reason(&b, &dex, 0, mv(&dex, "healbell"), rules),
            Some("Heal Bell has no status to cure")
        );
        b.set_status(&dex, me, "par", Some(foe), EffectHandle::None, true);
        assert_eq!(noop_reason(&b, &dex, 0, mv(&dex, "healbell"), rules), None);
        b.pokemon_clear_status(&dex, me);

        // 5. Disable / Encore / Spite / Mirror Move / Conversion2 / Mimic vs stranded foe with last_move == None
        strand(&mut b, 1);
        b.poke_mut(foe).last_move = None;
        b.poke_mut(foe).last_move_encore = None;
        b.poke_mut(me).boosts[4] = 6;
        // Against Snorlax (which can preempt via Endure/Protect), these stay unmasked.
        for key in [
            "disable",
            "encore",
            "spite",
            "mirrormove",
            "conversion2",
            "mimic",
        ] {
            assert_eq!(
                noop_reason(&b, &dex, 0, mv(&dex, key), rules),
                None,
                "{key} vs stranded Endure/Protect learner must not be masked"
            );
        }
        // Against a foe with zero priority moves in its learnset (e.g., Unown), they are masked.
        b.poke_mut(foe).species = dex.species.id("unown").unwrap();
        for key in [
            "disable",
            "encore",
            "spite",
            "mirrormove",
            "conversion2",
            "mimic",
        ] {
            assert_eq!(
                noop_reason(&b, &dex, 0, mv(&dex, key), rules),
                Some("the target has not used a move yet"),
                "{key} vs stranded non-priority foe with no last_move must be masked"
            );
        }
    }

    #[test]
    fn preemption_and_speed_gates_protect_encore_destiny_bond_dreameater_and_charm() {
        let (dex, mut b) = setup();
        strand(&mut b, 1);
        let me = b.active_id(0).unwrap();
        let foe = b.active_id(1).unwrap();
        let rules = MaskRules::default();

        // 1. Faster Encore vs slower stranded foe that can learn Endure/Protect.
        b.poke_mut(me).boosts[4] = 6;
        b.poke_mut(foe).last_move_encore = None;
        assert_eq!(
            noop_reason(&b, &dex, 0, mv(&dex, "encore"), rules),
            None,
            "faster Encore must stay live against slower Endure/Protect learner"
        );

        // 2. Faster Destiny Bond re-arm vs slower stranded Protect/Endure learner.
        b.add_volatile(&dex, me, "destinybond", Some(me), EffectHandle::None);
        assert_eq!(
            noop_reason(&b, &dex, 0, mv(&dex, "destinybond"), rules),
            None,
            "faster Destiny Bond re-arm stays live if foe can preempt with Protect/Endure"
        );

        // 3. Slower Dream Eater vs faster awake foe (which can Rest first).
        b.poke_mut(me).boosts[4] = -6;
        assert_eq!(
            noop_reason(&b, &dex, 0, mv(&dex, "dreameater"), rules),
            None,
            "slower Dream Eater stays live vs awake foe that can Rest first"
        );
        b.poke_mut(me).boosts[4] = 6;
        assert_eq!(
            noop_reason(&b, &dex, 0, mv(&dex, "dreameater"), rules),
            Some("Dream Eater needs a sleeping, unsubstituted target"),
            "faster Dream Eater vs awake foe is a no-op"
        );

        // 4. Slower Charm vs faster foe already at -6 Atk (which can Swords Dance first).
        b.poke_mut(foe).boosts[0] = -6;
        b.poke_mut(me).boosts[4] = -6;
        assert_eq!(
            noop_reason(&b, &dex, 0, mv(&dex, "charm"), rules),
            None,
            "slower Charm stays live when faster foe can boost Atk first"
        );
        b.poke_mut(me).boosts[4] = 6;
        assert_eq!(
            noop_reason(&b, &dex, 0, mv(&dex, "charm"), rules),
            Some("every stat this move moves is already capped"),
            "faster Charm into -6 Atk foe is a no-op"
        );
    }

}
