//! M15a protocol→state importer: reconstruct a `Battle` from PLAYER-VISIBLE
//! information only — the accumulated protocol lines of a PS-hosted battle
//! plus our side's request JSON — with the hidden opponent fields imputed
//! from an M10 belief source.
//!
//! This is the from-scratch cousin of `Belief::determinize`: the determinizer
//! overwrites a *known* true battle; here no true battle exists, so a
//! synthetic one is built that honors every public fact and leaves the
//! hidden fields to the belief. The division of labor maximizes reuse:
//!
//! - `ProtocolTracker` (new): parses the protocol vocabulary this engine
//!   itself emits (bit-exact with PS for this format, so the vocabulary is
//!   closed and known) into per-mon public state: HP (own exact / foe 1/48
//!   pixels), status (+ public counters: sleep turns, Rest, residual ticks),
//!   boosts (Baton Pass keeps them), announced volatiles, side conditions,
//!   weather (+ upkeep count), move usage counts for PP marks (charge-turn
//!   and locked-continuation `|move|` lines deduct no PP — mirrored from
//!   `run_move`'s deduction rule), picks/appearance/faint flow.
//! - `synthesize`: builds `Battle::from_fixture(own sets, belief refs)` and
//!   performs the state surgery: picks/positions (the M10 canonical party
//!   scheme), per-mon public fields, engine-API planting of statuses /
//!   volatiles / side conditions / weather (so companion state — Stadium
//!   residual counters, substitute HP, rolled hidden durations — comes from
//!   the same code the real engine runs), request bookkeeping via the
//!   engine's own `make_request`, and the mid-turn queue (pending foe move +
//!   residual) that `Belief::determinize`'s pending-move scrub expects.
//! - `Observer` (M10a, protocol mode): revealed moves + item knowledge from
//!   the same lines; `Belief` (M10a) filters candidates and provides the
//!   imputation refs; `BlindSearch` (M10c) then runs per-iteration
//!   determinization on the synthesized battle exactly as it does on a true
//!   one.
//!
//! Declared imputations (hidden by protocol, fixed at synthesis): foe HP
//! within its announced 1/48 bucket, hidden status/volatile counters (sleep
//! turns remaining, confusion/encore/disable/bind durations, substitute HP),
//! unrevealed sets/DVs (the belief's job), never-appeared pick identities
//! (resampled per iteration by the determinizer). Thrash/rollout lock
//! *inference* (never announced) is mechanics-based and calibrated against
//! the conformance corpus by `tests/import.rs`.

use nc2000_engine::battle::{
    events::{ev, EvTarget},
    EffectHandle, PokemonSet, SearchChoice,
};
use nc2000_engine::dex::{toid, Dex, MoveId, SpeciesId};
use nc2000_engine::state::{
    Action, ActionKind, Battle, Gender, MoveSlot, PokeId, PokeName, RequestState, Status,
    SwitchFlag, DK,
};

use crate::belief::Belief;
use crate::blind::BlindSearch;
use crate::observe::{move_matches, MonObs, Observer};
use crate::preview::{MetaPool, TableSet};
use crate::rng::SplitMix64;
use crate::smmcts::RmConfig;

/// NC2000 rule: Max Total Level = 155. Since the 2026-07-17 preview-space
/// fix the ENGINE enforces this at team preview (validation + enumeration;
/// certificate on `nc2000_engine::battle::MAX_TOTAL_LEVEL`), so the M15
/// root mask and the filtered table sampling below are redundant belt-and-
/// suspenders at preview — kept because they are harmless and still guard
/// table policies fed from external files.
pub const MAX_TOTAL_LEVEL: i32 = nc2000_engine::battle::MAX_TOTAL_LEVEL as i32;

// ===================================================================== lines

/// One mon's tracked public state.
#[derive(Clone, Debug)]
struct TrackMon {
    species: SpeciesId,
    level: u8,
    gender: Gender,
    preview_item: bool,
    /// Nickname, learned at first appearance ("" until then).
    name: String,
    appeared: bool,
    appear_count: i32,
    switch_in_turn: u16,
    active: bool,
    fainted: bool,
    /// Foe: last announced HP numerator (over `hp_den`). Own side is
    /// authoritative from the request, so pixels are only a fallback.
    pixels: i32,
    /// Denominator of the announced foe HP: 100 under HP Percentage Mod
    /// (this format), 48 on legacy pixel-bar streams.
    hp_den: i32,
    status: Status,
    /// Sleep came from Rest (public 2-turn counter).
    rest: bool,
    /// `|cant|...|slp` count since falling asleep.
    slept: i32,
    /// Stadium companion volatiles (public lifecycles of their own —
    /// created by their status, surviving cures/replacements, removed by
    /// boosting the dropped stat / Haze / switching):
    /// `residualdmg` = the Toxic counter (created by tox only; +1 per
    /// residual tick while brn/psn/tox; retained through Heal Bell/rests).
    comp_res: Option<i32>,
    comp_brn: bool,
    comp_par: bool,
    boosts: [i8; 7],
    vols: Vec<TVol>,
    /// PP-mark deductions by move id.
    uses: Vec<(MoveId, i32)>,
    /// PP-mark deductions while Transformed (cleared on exit).
    transformed_uses: Vec<(MoveId, i32)>,
    /// Mechanics-inferred thrash/rollout lock: (move, uses so far).
    locked: Option<(MoveId, i32)>,
    /// Two-turn move charge pending release.
    charging: Option<MoveId>,
    must_recharge: bool,
    transformed_into: Option<(usize, usize)>,
    mimic_overlay: Option<MoveId>,
    last_move: Option<MoveId>,
    stall_streak: i32,
    last_protect_turn: u16,
    protected_this_turn: bool,
}

impl TrackMon {
    fn new(species: SpeciesId, level: u8, gender: Gender, item: bool) -> TrackMon {
        TrackMon {
            species,
            level,
            gender,
            preview_item: item,
            name: String::new(),
            appeared: false,
            appear_count: 0,
            switch_in_turn: 0,
            active: false,
            fainted: false,
            pixels: 48,
            hp_den: 48,
            status: Status::None,
            rest: false,
            slept: 0,
            comp_res: None,
            comp_brn: false,
            comp_par: false,
            boosts: [0; 7],
            vols: Vec::new(),
            uses: Vec::new(),
            transformed_uses: Vec::new(),
            locked: None,
            charging: None,
            must_recharge: false,
            transformed_into: None,
            mimic_overlay: None,
            last_move: None,
            stall_streak: 0,
            last_protect_turn: 0,
            protected_this_turn: false,
        }
    }

    fn deduct(&mut self, id: MoveId) {
        match self.uses.iter_mut().find(|(m, _)| *m == id) {
            Some((_, n)) => *n += 1,
            None => self.uses.push((id, 1)),
        }
    }

    fn deduct_transformed(&mut self, id: MoveId) {
        match self.transformed_uses.iter_mut().find(|(m, _)| *m == id) {
            Some((_, n)) => *n += 1,
            None => self.transformed_uses.push((id, 1)),
        }
    }

    /// Everything a switch-out clears (mirrors `clear_volatile`).
    fn clear_on_exit(&mut self) {
        self.boosts = [0; 7];
        self.vols.clear();
        self.transformed_uses.clear();
        self.locked = None;
        self.charging = None;
        self.must_recharge = false;
        self.transformed_into = None;
        self.mimic_overlay = None;
        self.last_move = None;
        self.stall_streak = 0;
        self.protected_this_turn = false;
        self.comp_res = None;
        self.comp_brn = false;
        self.comp_par = false;
    }
}

/// Public per-mon snapshot for harness consumers — see
/// [`ProtocolTracker::snapshot`].
#[derive(Clone, Debug)]
pub struct MonSnapshot {
    pub species: SpeciesId,
    pub level: u8,
    pub gender: Gender,
    pub name: String,
    pub appeared: bool,
    pub active: bool,
    pub fainted: bool,
    /// Announced HP as a fraction of max (percentage-granular under HP% Mod).
    pub hp_frac: f64,
    /// The announced HP exactly as the protocol printed it: numerator over
    /// `hp_den` (100 under HP Percentage Mod, 48 on a legacy pixel-bar
    /// stream). Kept unrounded so a checker can re-announce an imputed HP
    /// amount and prove the rounding rule matches the one the stream used —
    /// the M18 class-A `%`-HP invariant.
    pub hp_num: i32,
    pub hp_den: i32,
    pub status: Status,
    /// Sleep came from Rest (public 2-turn clock).
    pub rest: bool,
    pub boosts: [i8; 7],
    /// Announced volatile keys (+ the locked/binding move where tracked).
    pub vols: Vec<(String, Option<MoveId>)>,
    /// PP-mark deductions by move id = revealed moves + use counts.
    pub uses: Vec<(MoveId, i32)>,
    /// Mid-action state that removes or distorts the choice (thrash lock,
    /// two-turn charge, recharge, transform).
    pub choiceless: bool,
}

/// One announced volatile.
#[derive(Clone, Debug)]
struct TVol {
    /// Engine condition key ("confusion", "substitute", ...).
    key: String,
    start_turn: u16,
    /// encore/disable: the locked move; partiallytrapped: the binding move.
    move_id: Option<MoveId>,
    /// (side, slot) of the volatile's source, when it matters.
    source: Option<(usize, usize)>,
    /// perishsong: remaining count.
    counter: Option<i64>,
}

#[derive(Clone, Debug, Default)]
struct TrackSide {
    mons: Vec<TrackMon>,
    active: Option<usize>,
    /// (condition key, start turn).
    conds: Vec<(String, u16)>,
    pending_bp: bool,
    /// The side's action this turn is spent (move / cant / switch).
    acted_this_turn: bool,
    fainted_this_turn: Option<usize>,
    fainted_last_turn: Option<usize>,
    /// Last move by this side (Stadium self-KO clause bookkeeping).
    side_last_move: Option<MoveId>,
}

/// Player-visible protocol accumulator for one battle.
pub struct ProtocolTracker {
    /// Our side (0 = p1).
    side: usize,
    sides: [TrackSide; 2],
    /// (weather key, upkeeps seen since set).
    weather: Option<(String, u16)>,
    turn: u16,
    upkeep_this_turn: bool,
    /// Transient active move context `(side, slot, move_id, is_crit)` for
    /// attributing `|-crit|` and `|-activate|...|Substitute|[damage]` hits.
    cur_move: Option<(usize, usize, MoveId, bool)>,
}

/// Locking (thrash-class) move keys: `lockedmove` continuation turns emit
/// plain `|move|` lines with NO PP deduction.
const THRASH_CLASS: [&str; 3] = ["thrash", "petaldance", "outrage"];
/// Binding move keys (`-activate |move: X|[of]` plants `partiallytrapped`).
const BIND_CLASS: [&str; 5] = ["wrap", "bind", "firespin", "clamp", "whirlpool"];

impl ProtocolTracker {
    pub(crate) fn set_hp_observations(&self, belief: &mut Belief, side: usize) {
        belief.set_hp_observations(
            self.sides[side]
                .mons
                .iter()
                .map(|m| if m.fainted { (0, m.hp_den) } else { (m.pixels, m.hp_den) })
                .collect(),
        );
        let turn = self.turn;
        let map_side = |s: usize| -> Vec<crate::belief::HiddenTimerMonObs> {
            self.sides[s]
                .mons
                .iter()
                .map(|m| {
                    let elapsed_of = |key: &str| {
                        m.vols
                            .iter()
                            .find(|v| v.key == key)
                            .map(|v| turn.saturating_sub(v.start_turn) as i32)
                    };
                    let confusion_ticks = m
                        .vols
                        .iter()
                        .find(|v| v.key == "confusion")
                        .map(|v| v.counter.unwrap_or(0));
                    crate::belief::HiddenTimerMonObs {
                        slept: m.slept,
                        rest: m.rest,
                        confusion_ticks,
                        partiallytrapped_elapsed: elapsed_of("partiallytrapped"),
                        disable_elapsed: elapsed_of("disable"),
                        encore_elapsed: elapsed_of("encore"),
                    }
                })
                .collect()
        };
        belief.set_timer_observations(crate::belief::HiddenTimerObs {
            sides: [map_side(0), map_side(1)],
        });
    }
    pub fn new(side: usize) -> ProtocolTracker {
        ProtocolTracker {
            side,
            sides: [TrackSide::default(), TrackSide::default()],
            weather: None,
            turn: 0,
            upkeep_this_turn: false,
            cur_move: None,
        }
    }

    pub fn turn(&self) -> u16 {
        self.turn
    }

    /// Opponent preview facts as M10a `MonObs` (feeds `Observer::from_mons`).
    pub(crate) fn observer_mons(&self) -> Vec<MonObs> {
        self.sides[1 - self.side]
            .mons
            .iter()
            .map(|m| MonObs {
                species: m.species,
                level: m.level,
                gender: m.gender,
                name: m.name.clone(),
                preview_has_item: m.preview_item,
                revealed_moves: Vec::new(),
                hp_type_mask: u16::MAX,
                item: Default::default(),
                excluded_items: 0,
                max_spe_stat: u16::MAX,
                appeared: m.appeared,
            })
            .collect()
    }

    /// Nicknames currently known for `side`'s roster slots ("" = unknown).
    pub(crate) fn names(&self, side: usize) -> Vec<&str> {
        self.sides[side].mons.iter().map(|m| m.name.as_str()).collect()
    }

    pub fn opp_roster_len(&self) -> usize {
        self.sides[1 - self.side].mons.len()
    }

    /// Read-only public snapshot of `side`'s tracked state + active roster
    /// slot — for harnesses that fabricate requests from a public stream
    /// (M16b human-agreement corpus replay). Facts only; no imputation.
    pub fn snapshot(&self, side: usize) -> (Vec<MonSnapshot>, Option<usize>) {
        let ts = &self.sides[side];
        let mons = ts
            .mons
            .iter()
            .map(|m| MonSnapshot {
                species: m.species,
                level: m.level,
                gender: m.gender,
                name: m.name.clone(),
                appeared: m.appeared,
                active: m.active,
                fainted: m.fainted,
                hp_frac: (m.pixels as f64 / m.hp_den.max(1) as f64).clamp(0.0, 1.0),
                hp_num: m.pixels,
                hp_den: m.hp_den.max(1),
                status: m.status,
                rest: m.rest,
                boosts: m.boosts,
                vols: m.vols.iter().map(|v| (v.key.clone(), v.move_id)).collect(),
                uses: m.uses.clone(),
                choiceless: m.locked.is_some()
                    || m.charging.is_some()
                    || m.must_recharge
                    || m.transformed_into.is_some(),
            })
            .collect();
        (mons, ts.active)
    }

    // ------------------------------------------------------------ subjects

    /// "p2a: Nick" / "p2: Nick" → (side, roster slot) via known nicknames.
    fn subject(&self, s: &str) -> Option<(usize, usize)> {
        let b = s.as_bytes();
        if b.len() < 4 || b[0] != b'p' || (b[1] != b'1' && b[1] != b'2') {
            return None;
        }
        let side = (b[1] - b'1') as usize;
        let rest = &s[2..];
        let has_pos = rest.starts_with(|c: char| c.is_ascii_lowercase());
        let rest = rest.strip_prefix(|c: char| c.is_ascii_lowercase()).unwrap_or(rest);
        let name = rest.strip_prefix(": ")?;
        if has_pos {
            if let Some(act) = self.sides[side].active {
                if self.sides[side].mons.get(act).is_some_and(|m| m.name == name) {
                    return Some((side, act));
                }
            }
        }
        let slot = self.sides[side].mons.iter().position(|m| m.name == name)?;
        Some((side, slot))
    }

    /// "p1: P1" side-condition subject → side index.
    fn side_subject(s: &str) -> Option<usize> {
        let b = s.as_bytes();
        if b.len() >= 2 && b[0] == b'p' && (b[1] == b'1' || b[1] == b'2') {
            Some((b[1] - b'1') as usize)
        } else {
            None
        }
    }

    /// Parse "Species, L50, M" details.
    pub(crate) fn parse_details(dex: &Dex, details: &str) -> Option<(SpeciesId, u8, Gender)> {
        let mut species_name = details;
        let mut level = 100u8;
        let mut gender = Gender::N;
        for (i, part) in details.split(", ").enumerate() {
            if i == 0 {
                species_name = part;
                continue;
            }
            if let Some(l) = part.strip_prefix('L') {
                if let Ok(v) = l.parse() {
                    level = v;
                }
            } else if part == "M" {
                gender = Gender::M;
            } else if part == "F" {
                gender = Gender::F;
            }
        }
        let sid = dex.species.id(&toid(species_name))?;
        Some((sid, level, gender))
    }

    /// Parse an HP-status field: "137/211 par" / "31/48" / "0 fnt" →
    /// (current, max, status token or ""). Fainted → (0, 0, "fnt").
    fn parse_hp(s: &str) -> (i32, i32, &str) {
        let (hp_part, status) = match s.split_once(' ') {
            Some((h, st)) => (h, st),
            None => (s, ""),
        };
        if hp_part == "0" {
            return (0, 0, status);
        }
        match hp_part.split_once('/') {
            Some((c, m)) => (c.parse().unwrap_or(0), m.parse().unwrap_or(0), status),
            None => (hp_part.parse().unwrap_or(0), 0, status),
        }
    }

    fn apply_hp_status(&mut self, side: usize, slot: usize, field: &str) {
        let (cur, max, status) = Self::parse_hp(field);
        let mon = &mut self.sides[side].mons[slot];
        if status == "fnt" {
            mon.pixels = 0;
        } else {
            mon.pixels = cur; // own-side exact values are unused (request wins)
            if max > 0 {
                mon.hp_den = max;
            }
            let st = Status::from_str(status);
            if st != mon.status && !status.is_empty() {
                mon.status = st;
            } else if status.is_empty() && mon.status != Status::None {
                // HP strings always carry the status token when one exists;
                // its absence on a fresh announcement means it is gone.
                // (Curing is always separately announced too — harmless.)
                mon.status = Status::None;
            }
        }
    }

    // ------------------------------------------------------------ the feed

    /// Feed one player-visible protocol line.
    pub fn push_line(&mut self, dex: &Dex, line: &str) {
        let Some(body) = line.strip_prefix('|') else { return };
        let parts: Vec<&str> = body.split('|').collect();
        let cmd = *parts.first().unwrap_or(&"");
        let arg = |i: usize| parts.get(i).copied().unwrap_or("");
        match cmd {
            "poke" => {
                let Some(side) = Self::side_subject(arg(1)) else { return };
                if let Some((sp, lv, g)) = Self::parse_details(dex, arg(2)) {
                    self.sides[side].mons.push(TrackMon::new(sp, lv, g, arg(3) == "item"));
                }
            }
            "turn" => {
                self.turn = arg(1).parse().unwrap_or(self.turn + 1);
                self.upkeep_this_turn = false;
                self.cur_move = None;
                for s in self.sides.iter_mut() {
                    s.acted_this_turn = false;
                    s.pending_bp = false;
                    s.fainted_last_turn = s.fainted_this_turn.take();
                    if let Some(a) = s.active {
                        let m = &mut s.mons[a];
                        if m.protected_this_turn {
                            m.stall_streak += 1;
                            m.last_protect_turn = self.turn - 1;
                            m.protected_this_turn = false;
                        } else if m.last_protect_turn + 1 < self.turn.max(1) {
                            m.stall_streak = 0;
                        }
                    }
                }
            }
            "upkeep" => self.upkeep_this_turn = true,
            "switch" | "drag" => self.on_switch(dex, &parts, cmd == "drag"),
            "faint" => {
                if let Some((s, m)) = self.subject(arg(1)) {
                    let turn = self.turn;
                    let side = &mut self.sides[s];
                    side.pending_bp = false;
                    side.fainted_this_turn = Some(m);
                    let mon = &mut side.mons[m];
                    mon.fainted = true;
                    mon.pixels = 0;
                    mon.clear_on_exit();
                    let _ = turn;
                    self.clear_sourced_vols(s, m);
                }
            }
            "move" => self.on_move(dex, &parts),
            "cant" => {
                if let Some((s, m)) = self.subject(arg(1)) {
                    self.sides[s].acted_this_turn = true;
                    let mon = &mut self.sides[s].mons[m];
                    match arg(2) {
                        "slp" => {
                            mon.slept += 1;
                            mon.locked = None; // lockedmove drops during sleep
                            mon.charging = None;
                        }
                        "recharge" => {
                            mon.must_recharge = false;
                            mon.charging = None;
                        }
                        _ => {
                            mon.charging = None; // twoturnmove aborts on cant
                        }
                    }
                }
            }
            "-damage" | "-heal" | "-sethp" => {
                if let Some((s, m)) = self.subject(arg(1)) {
                    self.apply_hp_status(s, m, arg(2));
                    if parts.iter().any(|p| *p == "[from] psn" || *p == "[from] brn") {
                        let mon = &mut self.sides[s].mons[m];
                        if let Some(n) = mon.comp_res.as_mut() {
                            *n += 1;
                        }
                    }
                    // a confusion self-hit replaces the turn's move: a
                    // pending two-turn charge is lost (its release would
                    // otherwise stay imputed as the only legal choice), and
                    // the side's action for the turn is spent.
                    if parts.iter().any(|p| *p == "[from] confusion") {
                        self.sides[s].acted_this_turn = true;
                        self.sides[s].mons[m].charging = None;
                    }
                    if cmd == "-damage"
                        && !parts.iter().any(|p| p.starts_with("[from] "))
                    {
                        if let Some(cm) = &mut self.cur_move {
                            cm.3 = false;
                        }
                    }
                }
            }
            "-crit" => {
                if let Some(cm) = &mut self.cur_move {
                    cm.3 = true;
                }
            }
            "-status" => {
                if let Some((s, m)) = self.subject(arg(1)) {
                    let silent = parts.iter().any(|p| *p == "[silent]");
                    let mon = &mut self.sides[s].mons[m];
                    mon.status = Status::from_str(arg(2));
                    mon.slept = 0;
                    mon.rest = parts.iter().any(|p| *p == "[from] move: Rest");
                    if !silent {
                        // onStart companion creation (silent = the tox→psn
                        // switch-in rewrite, which runs no onStart)
                        match mon.status {
                            Status::Tox => mon.comp_res = Some(0),
                            Status::Brn => mon.comp_brn = true,
                            Status::Par => mon.comp_par = true,
                            _ => {}
                        }
                    }
                }
            }
            "-curestatus" => {
                if let Some((s, m)) = self.subject(arg(1)) {
                    cure(&mut self.sides[s].mons[m]);
                }
            }
            "-cureteam" => {
                let side_opt = Self::side_subject(arg(1))
                    .or_else(|| self.subject(arg(1)).map(|(s, _)| s));
                if let Some(s) = side_opt {
                    for mon in self.sides[s].mons.iter_mut() {
                        if !mon.fainted {
                            cure(mon);
                        }
                    }
                }
            }
            "-boost" | "-unboost" => {
                if let Some((s, m)) = self.subject(arg(1)) {
                    if let Some(i) = boost_index(arg(2)) {
                        let d: i8 = arg(3).parse().unwrap_or(0);
                        let d = if cmd == "-unboost" { -d } else { d };
                        let mon = &mut self.sides[s].mons[m];
                        let b = &mut mon.boosts[i];
                        *b = (*b + d).clamp(-6, 6);
                        // any spe change while par (atk while brn) publicly
                        // removes the Stadium stat-drop companion volatile
                        if d != 0 && i == 4 && mon.status == Status::Par {
                            mon.comp_par = false;
                        }
                        if d != 0 && i == 0 && mon.status == Status::Brn {
                            mon.comp_brn = false;
                        }
                    }
                }
            }
            "-copyboost" => {
                if let (Some((ts, tm)), Some((ss, sm))) =
                    (self.subject(arg(1)), self.subject(arg(2)))
                {
                    let src = self.sides[ss].mons[sm].boosts;
                    self.sides[ts].mons[tm].boosts = src;
                }
            }
            "-clearallboost" => {
                // Haze: boosts cleared AND the par/brn drop volatiles removed
                for s in self.sides.iter_mut() {
                    if let Some(a) = s.active {
                        s.mons[a].boosts = [0; 7];
                        s.mons[a].comp_par = false;
                        s.mons[a].comp_brn = false;
                    }
                }
            }
            "-start" => self.on_start(dex, &parts),
            "-end" => {
                if let Some((s, m)) = self.subject(arg(1)) {
                    let key = toid(arg(2).strip_prefix("move: ").unwrap_or(arg(2)));
                    // binding traps end under their MOVE's name
                    let key = if BIND_CLASS.contains(&key.as_str()) {
                        "partiallytrapped".to_string()
                    } else {
                        key
                    };
                    self.sides[s].mons[m].vols.retain(|v| v.key != key);
                }
            }
            "-activate" => self.on_activate(dex, &parts),
            "-singleturn" => {
                if let Some((s, m)) = self.subject(arg(1)) {
                    let what = arg(2).strip_prefix("move: ").unwrap_or(arg(2));
                    if matches!(toid(what).as_str(), "protect" | "endure" | "detect") {
                        self.sides[s].mons[m].protected_this_turn = true;
                    }
                }
            }
            "-singlemove" => {
                if let Some((s, m)) = self.subject(arg(1)) {
                    let key = toid(arg(2).strip_prefix("move: ").unwrap_or(arg(2)));
                    if matches!(key.as_str(), "rage" | "destinybond") {
                        let turn = self.turn;
                        let mon = &mut self.sides[s].mons[m];
                        if !mon.vols.iter().any(|v| v.key == key) {
                            mon.vols.push(TVol {
                                key,
                                start_turn: turn,
                                move_id: None,
                                source: None,
                                counter: None,
                            });
                        }
                    }
                }
            }
            "-sidestart" => {
                if let Some(s) = Self::side_subject(arg(1)) {
                    let key = toid(arg(2).strip_prefix("move: ").unwrap_or(arg(2)));
                    if !self.sides[s].conds.iter().any(|(k, _)| *k == key) {
                        let turn = self.turn;
                        self.sides[s].conds.push((key, turn));
                    }
                }
            }
            "-sideend" => {
                if let Some(s) = Self::side_subject(arg(1)) {
                    let key = toid(arg(2).strip_prefix("move: ").unwrap_or(arg(2)));
                    self.sides[s].conds.retain(|(k, _)| *k != key);
                }
            }
            "-weather" => {
                let w = arg(1);
                if w == "none" {
                    self.weather = None;
                } else if parts.iter().any(|p| *p == "[upkeep]") {
                    if let Some((_, upkeeps)) = self.weather.as_mut() {
                        *upkeeps += 1;
                    }
                } else {
                    self.weather = Some((toid(w), 0));
                }
            }
            "-prepare" => {
                // charge turn of a genuine two-turn move (the |move| line
                // with [still] cannot be trusted — failed moves get [still]
                // attributed retroactively too)
                if let Some((s, m)) = self.subject(arg(1)) {
                    if let Some(mid) = dex.moves.id(&toid(arg(2))) {
                        self.sides[s].mons[m].charging = Some(mid);
                    }
                }
            }
            "-anim" => {
                // an animation right after |-prepare| = the charge released
                // in the same turn (Solar Beam in sun): no lock next turn
                if let Some((s, m)) = self.subject(arg(1)) {
                    self.sides[s].mons[m].charging = None;
                }
            }
            "-transform" => {
                if let (Some((s, m)), Some(t)) = (self.subject(arg(1)), self.subject(arg(2))) {
                    // Transform copies the target's stat stages (public: both
                    // sides' boost lines are announced) — seed the tracked
                    // boosts with the copy; later lines mutate them normally.
                    let tb = self.sides[t.0].mons[t.1].boosts;
                    let mon = &mut self.sides[s].mons[m];
                    mon.transformed_into = Some(t);
                    mon.transformed_uses.clear();
                    mon.boosts = tb;
                }
            }
            "-mustrecharge" => {
                if let Some((s, m)) = self.subject(arg(1)) {
                    self.sides[s].mons[m].must_recharge = true;
                }
            }
            "-miss" | "-fail" => {
                // rollout's lock, fury cutter's streak, and a pending Baton
                // Pass switch all end on a whiff/failure
                if let Some((s, m)) = self.subject(arg(1)) {
                    self.sides[s].pending_bp = false;
                    let mon = &mut self.sides[s].mons[m];
                    if let Some((lm, _)) = mon.locked {
                        if dex.moves.key(lm) == "rollout" {
                            mon.locked = None;
                        }
                    }
                    mon.vols.retain(|v| v.key != "furycutter");
                }
            }
            "-immune" => {
                // subject is the TARGET; the attacker's rollout streak ends
                if let Some((s, _)) = self.subject(arg(1)) {
                    let atk = &mut self.sides[1 - s];
                    if let Some(a) = atk.active {
                        let mon = &mut atk.mons[a];
                        if let Some((lm, _)) = mon.locked {
                            if dex.moves.key(lm) == "rollout" {
                                mon.locked = None;
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn on_switch(&mut self, dex: &Dex, parts: &[&str], is_drag: bool) {
        let subj = parts.get(1).copied().unwrap_or("");
        let Some(side) = Self::side_subject(subj) else { return };
        let Some((sp, lv, _g)) = Self::parse_details(dex, parts.get(2).copied().unwrap_or(""))
        else {
            return;
        };
        // resolve incoming slot by species (+level; Species Clause ⇒ unique)
        let Some(slot) = self.sides[side]
            .mons
            .iter()
            .position(|m| m.species == sp && m.level == lv)
        else {
            return;
        };
        // learn nickname
        if let Some(name) = subj.split_once(": ").map(|(_, n)| n) {
            self.sides[side].mons[slot].name = name.to_string();
        }
        // outgoing
        let bp = self.sides[side].pending_bp && !is_drag;
        let mut inherit: Option<([i8; 7], Vec<TVol>, Option<i32>, bool, bool)> = None;
        if let Some(old) = self.sides[side].active {
            if old != slot {
                let om = &mut self.sides[side].mons[old];
                om.active = false;
                if bp {
                    let vols: Vec<TVol> = om
                        .vols
                        .iter()
                        .filter(|v| {
                            dex.conds_id(&v.key).map_or(false, |c| !dex.cond(c).no_copy)
                        })
                        .cloned()
                        .collect();
                    inherit = Some((om.boosts, vols, om.comp_res, om.comp_brn, om.comp_par));
                }
                om.clear_on_exit();
                self.clear_sourced_vols(side, old);
            }
        }
        self.sides[side].pending_bp = false;
        self.sides[side].acted_this_turn = true;
        // incoming
        let turn = self.turn;
        {
            let mon = &mut self.sides[side].mons[slot];
            mon.appeared = true;
            mon.appear_count += 1;
            mon.switch_in_turn = turn;
            mon.active = true;
            if let Some((boosts, vols, res, brn, par)) = inherit {
                mon.boosts = boosts;
                mon.vols = vols;
                mon.comp_res = res;
                mon.comp_brn = brn;
                mon.comp_par = par;
            }
        }
        self.sides[side].active = Some(slot);
        self.apply_hp_status(side, slot, parts.get(3).copied().unwrap_or(""));
        // brn/par onSwitchIn re-add their Stadium stat-drop volatiles
        {
            let mon = &mut self.sides[side].mons[slot];
            match mon.status {
                Status::Brn => mon.comp_brn = true,
                Status::Par => mon.comp_par = true,
                _ => {}
            }
        }
    }

    fn on_move(&mut self, dex: &Dex, parts: &[&str]) {
        let Some((s, m)) = self.subject(parts.get(1).copied().unwrap_or("")) else { return };
        self.sides[s].acted_this_turn = true;
        let name = parts.get(2).copied().unwrap_or("");
        let Some(mid) = dex.moves.id(&toid(name)) else { return };
        self.cur_move = Some((s, m, mid, false));
        let key = dex.moves.key(mid);
        // `[from] <other move>` = a called move (Sleep Talk / Mirror Move —
        // no PP of its own). `[from] <the same move>` = a special execution
        // of the mon's own choice (Pursuit intercepting a switch) — PS
        // deducts PP for those exactly like a normal use.
        let from_key = parts
            .iter()
            .find_map(|p| p.strip_prefix("[from] "))
            .map(|n| toid(n.strip_prefix("move: ").unwrap_or(n)));
        let called = match &from_key {
            Some(k) => k.as_str() != key,
            None => false,
        };
        if called {
            if key == "batonpass" {
                self.sides[s].pending_bp = true;
            }
            return; // Sleep Talk / Mirror Move calls: no last_move overwrite, no PP, no lock changes
        }
        {
            let side = &mut self.sides[s];
            side.side_last_move = Some(mid);
            side.pending_bp = key == "batonpass";
            let mon = &mut side.mons[m];
            mon.last_move = Some(mid);
            // single-move volatiles expire at the user's next move
            mon.vols.retain(|v| v.key != "destinybond");
            if key != "rage" {
                mon.vols.retain(|v| v.key != "rage");
            }
            if key != "furycutter" {
                mon.vols.retain(|v| v.key != "furycutter");
            }
        }
        if matches!(key, "struggle" | "recharge") {
            return;
        }
        let mon = &mut self.sides[s].mons[m];
        if mon.charging == Some(mid) {
            // release turn of a two-turn move (`-prepare` set `charging` on
            // the charge turn, whose |move| line already deducted): no PP
            mon.charging = None;
            return;
        }
        mon.charging = None;
        if let Some((lm, uses)) = mon.locked {
            if lm == mid {
                // lockedmove/rollout continuation: no PP deduction.
                // lockedmove runs 2-3 move actions; rollout up to 5 hits
                // (a miss ends it — handled at the |-miss| line).
                let cap = if key == "rollout" { 5 } else { 3 };
                let uses = uses + 1;
                mon.locked = if uses >= cap { None } else { Some((lm, uses)) };
                return;
            }
            mon.locked = None;
        }
        // A transformed mon is executing the copied set. Track its PP usage
        // on `transformed_uses` so the active transformed slots reflect exact
        // 5-PP depletion without polluting `uses` (base-set evidence).
        if mon.transformed_into.is_none() {
            mon.deduct(mid);
        } else {
            mon.deduct_transformed(mid);
        }
        if THRASH_CLASS.contains(&key) || key == "rollout" {
            mon.locked = Some((mid, 1));
        }
        let turn = self.turn;
        let mon = &mut self.sides[s].mons[m];
        match key {
            // moves whose lasting volatile is implied by the (public) use
            "defensecurl" | "minimize" => {
                if !mon.vols.iter().any(|v| v.key == key) {
                    mon.vols.push(TVol {
                        key: key.to_string(),
                        start_turn: turn,
                        move_id: None,
                        source: None,
                        counter: None,
                    });
                }
            }
            "furycutter" => {
                match mon.vols.iter_mut().find(|v| v.key == "furycutter") {
                    Some(v) => *v.counter.get_or_insert(1) += 1,
                    None => mon.vols.push(TVol {
                        key: "furycutter".to_string(),
                        start_turn: turn,
                        move_id: None,
                        source: None,
                        counter: Some(1),
                    }),
                }
            }
            _ => {}
        }
    }

    fn on_start(&mut self, dex: &Dex, parts: &[&str]) {
        let Some((s, m)) = self.subject(parts.get(1).copied().unwrap_or("")) else { return };
        let what = parts.get(2).copied().unwrap_or("");
        let turn = self.turn;
        // perish counter
        if let Some(n) = what.strip_prefix("perish").and_then(|n| n.parse::<i64>().ok()) {
            let mon = &mut self.sides[s].mons[m];
            match mon.vols.iter_mut().find(|v| v.key == "perishsong") {
                Some(v) => v.counter = Some(n),
                None => mon.vols.push(TVol {
                    key: "perishsong".to_string(),
                    start_turn: turn,
                    move_id: None,
                    source: None,
                    counter: Some(n),
                }),
            }
            return;
        }
        let key = toid(what.strip_prefix("move: ").unwrap_or(what));
        // fatigue confusion ends a thrash lock (indistinguishable from other
        // confusion sources — clearing is the conservative reading for PP)
        if key == "confusion" {
            self.sides[s].mons[m].locked = None;
        }
        if dex.conds_id(&key).is_none() {
            return; // not a runtime condition we can plant (e.g. typechange)
        }
        let move_id = match key.as_str() {
            // encore locks the subject's last move; disable names it in arg 3
            "encore" => self.sides[s].mons[m].last_move,
            "disable" => parts
                .get(3)
                .and_then(|n| dex.moves.id(&toid(n)))
                .or(self.sides[s].mons[m].last_move),
            _ => None,
        };
        let source = if key == "substitute" {
            Some((s, m))
        } else {
            self.sides[1 - s].active.map(|a| (1 - s, a))
        };
        let mon = &mut self.sides[s].mons[m];
        if key == "substitute" {
            mon.vols.retain(|v| v.key != "substitute");
        } else if mon.vols.iter().any(|v| v.key == key) {
            return;
        }
        mon.vols.push(TVol { key, start_turn: turn, move_id, source, counter: None });
    }

    fn on_activate(&mut self, dex: &Dex, parts: &[&str]) {
        let Some((s, m)) = self.subject(parts.get(1).copied().unwrap_or("")) else { return };
        let what = parts.get(2).copied().unwrap_or("");
        let turn = self.turn;
        if what == "confusion" {
            let mon = &mut self.sides[s].mons[m];
            if let Some(v) = mon.vols.iter_mut().find(|v| v.key == "confusion") {
                *v.counter.get_or_insert(0) += 1;
            }
            return;
        }
        if what == "Substitute" && parts.contains(&"[damage]") {
            let dmg = self.estimate_sub_damage(dex, s, m);
            let mon = &mut self.sides[s].mons[m];
            if let Some(v) = mon.vols.iter_mut().find(|v| v.key == "substitute") {
                *v.counter.get_or_insert(0) += dmg as i64;
            }
            if let Some(cm) = &mut self.cur_move {
                cm.3 = false;
            }
            return;
        }
        if what == "trapped" {
            let source = self.sides[1 - s].active.map(|a| (1 - s, a));
            let mon = &mut self.sides[s].mons[m];
            if !mon.vols.iter().any(|v| v.key == "trapped") {
                mon.vols.push(TVol {
                    key: "trapped".to_string(),
                    start_turn: turn,
                    move_id: None,
                    source,
                    counter: None,
                });
            }
            return;
        }
        if what == "Protect" || what == "Detect" {
            // the attacker was blocked: its rollout streak ends
            let atk = &mut self.sides[1 - s];
            if let Some(a) = atk.active {
                let mon = &mut atk.mons[a];
                if let Some((lm, _)) = mon.locked {
                    if dex.moves.key(lm) == "rollout" {
                        mon.locked = None;
                    }
                }
            }
            return;
        }
        // Mystery Berry PP restore (+5, capped — the cap is invisible from
        // the protocol but the restored slot was at 0, so uses -= 5 with a
        // floor of maxpp-consistency is exact up to the cap)
        if what == "item: Mystery Berry" {
            if let Some(mid) = parts.get(3).and_then(|n| dex.moves.id(&toid(n))) {
                let mon = &mut self.sides[s].mons[m];
                let list = if mon.transformed_into.is_some() {
                    &mut mon.transformed_uses
                } else {
                    &mut mon.uses
                };
                if let Some((_, n)) = list.iter_mut().find(|(id, _)| *id == mid) {
                    *n = (*n - 5).max(0);
                }
            }
            return;
        }
        if let Some(name) = what.strip_prefix("move: ") {
            let key = toid(name);
            if key == "spite" {
                // |-activate|target|move: Spite|<movekey>|<n>
                if let (Some(mid), Some(n)) = (
                    parts.get(3).and_then(|n| dex.moves.id(&toid(n))),
                    parts.get(4).and_then(|n| n.parse::<i32>().ok()),
                ) {
                    let mon = &mut self.sides[s].mons[m];
                    let list = if mon.transformed_into.is_some() {
                        &mut mon.transformed_uses
                    } else {
                        &mut mon.uses
                    };
                    match list.iter_mut().find(|(id, _)| *id == mid) {
                        Some((_, u)) => *u += n,
                        None => list.push((mid, n)),
                    }
                }
                return;
            }
            if key == "mimic" {
                if let Some(overlay) =
                    parts.get(3).and_then(|n| dex.moves.id(&toid(n)))
                {
                    self.sides[s].mons[m].mimic_overlay = Some(overlay);
                }
                return;
            }
            if BIND_CLASS.contains(&key.as_str()) {
                let mid = dex.moves.id(&key);
                let source = parts
                    .iter()
                    .find_map(|p| p.strip_prefix("[of] "))
                    .and_then(|of| self.subject(of));
                let mon = &mut self.sides[s].mons[m];
                if !mon.vols.iter().any(|v| v.key == "partiallytrapped") {
                    mon.vols.push(TVol {
                        key: "partiallytrapped".to_string(),
                        start_turn: turn,
                        move_id: mid,
                        source,
                        counter: None,
                    });
                }
            }
        }
    }

    /// Defensive: linked volatiles die with their source (mean look /
    /// binding) — remove them when the source leaves the field. Attract's
    /// end is always announced (`-end ... [silent]` is still a line), so it
    /// is NOT cleared here (gen 2 keeps it when the attractor leaves).
    fn clear_sourced_vols(&mut self, src_side: usize, src_slot: usize) {
        for s in 0..2 {
            for mon in self.sides[s].mons.iter_mut() {
                mon.vols.retain(|v| {
                    !(v.source == Some((src_side, src_slot))
                        && matches!(v.key.as_str(), "trapped" | "partiallytrapped"))
                });
            }
        }
    }

    /// Estimate damage dealt to a surviving Substitute on `(def_s, def_m)`
    /// from the most recent `|move|` line (`self.cur_move`).
    fn estimate_sub_damage(&self, dex: &Dex, def_s: usize, def_m: usize) -> i32 {
        let Some((atk_s, atk_m, mid, is_crit)) = self.cur_move else {
            return 1;
        };
        let atk = &self.sides[atk_s].mons[atk_m];
        let def = &self.sides[def_s].mons[def_m];
        let mkey = dex.moves.key(mid);
        match mkey {
            "dragonrage" => return 40,
            "sonicboom" => return 20,
            "seismictoss" | "nightshade" => return i32::from(atk.level).max(1),
            "psywave" => return (i32::from(atk.level) * 3 / 4).max(1),
            "superfang" => {
                let def_sp = dex.species.get(def.species);
                let maxhp = ((2 * i32::from(def_sp.base_stats.hp) + 30 + 63) * i32::from(def.level))
                    / 100
                    + i32::from(def.level)
                    + 10;
                let cur_hp = (def.pixels * maxhp / def.hp_den.max(1)).max(1);
                return (cur_hp / 2).max(1);
            }
            _ => {}
        }
        let ms = dex.move_static(mid);
        let bp = if mkey.starts_with("hiddenpower") {
            70
        } else if mkey == "present" {
            60
        } else if mkey == "magnitude" {
            70
        } else if mkey == "return" || mkey == "frustration" {
            102
        } else if ms.base_power > 0 {
            ms.base_power
        } else {
            return 1;
        };
        let is_phys = matches!(ms.category, nc2000_engine::dex::Category::Physical);
        let (atk_species, atk_level) = match atk.transformed_into {
            Some((ts, tslot)) => (self.sides[ts].mons[tslot].species, atk.level),
            None => (atk.species, atk.level),
        };
        let (def_species, def_level) = match def.transformed_into {
            Some((ts, tslot)) => (self.sides[ts].mons[tslot].species, def.level),
            None => (def.species, def.level),
        };
        let atk_bs = &dex.species.get(atk_species).base_stats;
        let def_bs = &dex.species.get(def_species).base_stats;
        let stat_at = |base: u16, lv: u8| -> i32 {
            ((2 * i32::from(base) + 30 + 63) * i32::from(lv)) / 100 + 5
        };
        let mut a = stat_at(if is_phys { atk_bs.atk } else { atk_bs.spa }, atk_level);
        let mut d = stat_at(if is_phys { def_bs.def } else { def_bs.spd }, def_level);
        if mkey == "explosion" || mkey == "selfdestruct" {
            d = (d / 2).max(1);
        }
        let atk_stage = atk.boosts[if is_phys { 0 } else { 2 }];
        let def_stage = def.boosts[if is_phys { 1 } else { 3 }];
        let apply_stage = |val: i32, st: i8| -> i32 {
            if st >= 0 {
                val * (2 + i32::from(st)) / 2
            } else {
                val * 2 / (2 - i32::from(st))
            }
        };
        let ignore_boosts = is_crit && atk_stage <= def_stage;
        if !ignore_boosts {
            a = apply_stage(a, atk_stage).clamp(1, 999);
            d = apply_stage(d, def_stage).clamp(1, 999);
            if is_phys && atk.comp_brn {
                a = (a / 2).max(1);
            }
            let screen_key = if is_phys { "reflect" } else { "lightscreen" };
            if self.sides[def_s].conds.iter().any(|(k, _)| k == screen_key) {
                d *= 2;
            }
        }
        if a > 255 || d > 255 {
            a = (a / 4).max(1);
            d = (d / 4).max(1);
        }
        let lv = i32::from(atk.level);
        let mut base = ((2 * lv / 5 + 2) * bp * a / d.max(1)) / 50;
        if is_crit {
            base *= 2;
        }
        base = base.min(997) + 2;
        if let Some((w, _)) = &self.weather {
            match (w.as_str(), ms.move_type) {
                ("raindance", t) if t == dex.known_types.water => base = base * 3 / 2,
                ("raindance", t) if t == dex.known_types.fire => base /= 2,
                ("sunnyday", t) if t == dex.known_types.fire => base = base * 3 / 2,
                ("sunnyday", t) if t == dex.known_types.water => base /= 2,
                _ => {}
            }
        }
        let atk_types = dex.species_types(atk_species);
        if atk_types.has(ms.move_type) {
            base += base / 2;
        }
        let def_types = dex.species_types(def_species);
        for dt in def_types.iter() {
            let e = dex.eff(ms.move_type, dt);
            if e > 0 {
                base *= 2;
            } else if e < 0 {
                base /= 2;
            }
        }
        (base * 236 / 255).max(1)
    }
}

/// Add/remove a Stadium companion volatile to match its tracked presence
/// (they carry no callbacks with side effects — see conditions.rs).
fn sync_companion(
    dex: &Dex,
    b: &mut Battle,
    id: PokeId,
    key: &str,
    present: bool,
) {
    let has = dex
        .conds_id(key)
        .map(|c| b.poke(id).has_volatile(c))
        .unwrap_or(false);
    if present && !has {
        b.add_volatile(dex, id, key, None, EffectHandle::None);
    } else if !present && has {
        b.remove_volatile(dex, id, key);
    }
}

fn cure(mon: &mut TrackMon) {
    // companion volatiles (residualdmg counter, stat drops) survive cures
    mon.status = Status::None;
    mon.rest = false;
    mon.slept = 0;
}

fn boost_index(name: &str) -> Option<usize> {
    ["atk", "def", "spa", "spd", "spe", "accuracy", "evasion"]
        .iter()
        .position(|&n| n == name)
}

// =================================================================== request

/// Parsed PS request JSON (the player-visible own-side truth).
#[derive(Clone, Debug, Default)]
pub struct Request {
    pub team_preview: bool,
    /// Public local format rules; absent on PS means the standard 155 cap.
    pub preview_level_caps: Option<[Option<u32>; 2]>,
    pub force_switch: bool,
    pub wait: bool,
    pub trapped: bool,
    pub maybe_trapped: bool,
    pub active_moves: Vec<ReqMove>,
    pub pokemon: Vec<ReqMon>,
}

#[derive(Clone, Debug)]
pub struct ReqMove {
    pub id: String,
    pub pp: Option<i32>,
    pub maxpp: Option<i32>,
    pub disabled: bool,
}

#[derive(Clone, Debug)]
pub struct ReqMon {
    pub species: SpeciesId,
    pub level: u8,
    pub gender: Gender,
    pub hp: i32,
    pub maxhp: i32,
    pub status: Status,
    pub fainted: bool,
    pub active: bool,
    pub item: String,
}

impl Request {
    pub fn parse(dex: &Dex, json: &str) -> Result<Request, String> {
        let v: serde_json::Value =
            serde_json::from_str(json).map_err(|e| format!("request parse: {e}"))?;
        let mut req = Request {
            team_preview: v["teamPreview"].as_bool().unwrap_or(false),
            wait: v["wait"].as_bool().unwrap_or(false),
            force_switch: v["forceSwitch"]
                .as_array()
                .map(|a| a.first().and_then(|x| x.as_bool()).unwrap_or(false))
                .unwrap_or(false),
            ..Default::default()
        };
        req.preview_level_caps = v["side"].get("previewLevelCaps")
            .map(|caps| serde_json::from_value(caps.clone()).map_err(|e|format!("preview caps: {e}")))
            .transpose()?;
        if let Some(active) = v["active"].as_array().and_then(|a| a.first()) {
            req.trapped = active["trapped"].as_bool().unwrap_or(false);
            req.maybe_trapped = active["maybeTrapped"].as_bool().unwrap_or(false);
            if let Some(moves) = active["moves"].as_array() {
                for m in moves {
                    req.active_moves.push(ReqMove {
                        id: m["id"].as_str().unwrap_or("").to_string(),
                        pp: m["pp"].as_i64().map(|x| x as i32),
                        maxpp: m["maxpp"].as_i64().map(|x| x as i32),
                        disabled: m["disabled"].as_bool().unwrap_or(false),
                    });
                }
            }
        }
        if let Some(mons) = v["side"]["pokemon"].as_array() {
            for p in mons {
                let details = p["details"].as_str().unwrap_or("");
                let (species, level, gender) = ProtocolTracker::parse_details(dex, details)
                    .ok_or_else(|| format!("request details: {details}"))?;
                let cond = p["condition"].as_str().unwrap_or("");
                let (hp, maxhp, status) = ProtocolTracker::parse_hp(cond);
                req.pokemon.push(ReqMon {
                    species,
                    level,
                    gender,
                    hp,
                    maxhp,
                    status: Status::from_str(status),
                    fainted: status == "fnt",
                    active: p["active"].as_bool().unwrap_or(false),
                    item: p["item"].as_str().unwrap_or("").to_string(),
                });
            }
        }
        Ok(req)
    }

    /// The PS-legal choice inputs this request admits (battle phase only;
    /// team preview legality is handled by the level mask).
    pub fn legal_inputs(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.team_preview || self.wait {
            return out;
        }
        if !self.force_switch {
            for m in &self.active_moves {
                if !m.disabled && m.pp != Some(0) {
                    out.push(format!("move {}", m.id));
                }
            }
        }
        let can_switch = self.force_switch || (!self.trapped && !self.active_moves.is_empty());
        if can_switch {
            for (pos, mon) in self.pokemon.iter().enumerate() {
                if !mon.active && !mon.fainted {
                    out.push(format!("switch {}", pos + 1));
                }
            }
        }
        if self.force_switch && out.is_empty() {
            out.push("pass".to_string());
        }
        out
    }
}

// ================================================================= synthesis

impl ProtocolTracker {
    /// Build the synthetic battle for the current decision point.
    ///
    /// `own_sets` = the exact team we submitted; `refs` = the belief's
    /// imputation roster for the opponent (aligned to `|poke|` order — the
    /// same alignment `Belief::determinize` uses); `obs` supplies public
    /// item knowledge; `req` is our request at this decision point.
    pub fn synthesize(
        &self,
        dex: &Dex,
        own_sets: &[PokemonSet],
        refs: &[nc2000_engine::state::Pokemon],
        obs: &Observer,
        req: &Request,
        rng: &mut SplitMix64,
    ) -> Result<Battle, String> {
        let me = self.side;
        let opp = 1 - me;
        let (p1, p2): (&[PokemonSet], &[PokemonSet]) = (own_sets, own_sets);
        let mut b = Battle::from_fixture(dex, &rng.battle_seed(), p1, p2)
            .map_err(|e| format!("from_fixture: {e:?}"))?;
        b.set_log_enabled(false);
        b.preview_level_caps=req.preview_level_caps.unwrap_or([Some(MAX_TOTAL_LEVEL as u32);2]);

        // opponent roster := belief refs (fresh preview-state mons aligned
        // to observed slots), with public preview facts stamped on
        if refs.len() != self.sides[opp].mons.len() {
            return Err("refs/public roster length mismatch".to_string());
        }
        b.sides[opp].roster = refs.to_vec();
        b.sides[opp].party=(0..refs.len() as u8).collect();
        b.sides[opp].pokemon_left=refs.len() as i32;
        for (slot, tm) in self.sides[opp].mons.iter().enumerate() {
            let p = &mut b.sides[opp].roster[slot];
            p.gender = tm.gender;
            if !tm.name.is_empty() {
                p.name = safe_name(&tm.name);
            }
        }
        for slot in 0..b.sides[opp].roster.len() {
            b.refresh_poke_mask(dex, PokeId { side: opp as u8, slot: slot as u8 });
        }
        // own genders are public (details) and PS sampled them with ITS rng
        for (slot, rm) in req.pokemon.iter().enumerate() {
            if req.team_preview {
                // preview request lists all 6 in roster order
                if let Some(p) = b.sides[me].roster.get_mut(slot) {
                    if p.species == rm.species {
                        p.gender = rm.gender;
                    }
                }
            }
        }

        if req.team_preview {
            // from_fixture state IS the preview decision point (party of 6,
            // TeamPreview request, queue holding the Start action).
            b.battle_mask = b.recompute_battle_mask(dex);
            return Ok(b);
        }

        // ------------------------------------------------- battle phase
        b.queue.clear(); // drop the preview Start action
        b.mid_turn = false;
        b.turn = self.turn;

        // ---- party arrangement
        // own: the request lists the picked mons in display order
        let mut own_party: Vec<u8> = Vec::new();
        for rm in &req.pokemon {
            let slot = b.sides[me]
                .roster
                .iter()
                .enumerate()
                .find(|(slot,p)| !own_party.contains(&(*slot as u8)) && p.species == rm.species && p.level == rm.level)
                .map(|(slot,_)|slot)
                .ok_or_else(|| "request mon not in own roster".to_string())?;
            own_party.push(slot as u8);
            let p = &mut b.sides[me].roster[slot];
            p.gender = rm.gender;
        }
        apply_party(&mut b, me, &own_party);
        // opponent: active first (display position 0 in singles), then the
        // other appeared picks, then imputed hidden picks (the determinizer
        // resamples those identities per iteration anyway)
        let ts = &self.sides[opp];
        let picked = self.sides[me].mons.len().min(req.pokemon.len()).max(1);
        let picked = picked.min(3); // NC2000: 3 picks
        let mut opp_party: Vec<u8> = Vec::new();
        if let Some(a) = ts.active {
            opp_party.push(a as u8);
        }
        for (i, m) in ts.mons.iter().enumerate() {
            if m.appeared && Some(i) != ts.active {
                opp_party.push(i as u8);
            }
        }
        for (i, m) in ts.mons.iter().enumerate() {
            if opp_party.len() >= picked {
                break;
            }
            if !m.appeared {
                let _ = m;
                opp_party.push(i as u8);
            }
        }
        apply_party(&mut b, opp, &opp_party);

        // ---- actives + flow
        for s in 0..2 {
            let active = self.sides[s].active;
            b.sides[s].active = active.map(|a| a as u8);
            b.sides[s].fainted_this_turn = self.sides[s].fainted_this_turn.map(|x| x as u8);
            b.sides[s].fainted_last_turn = self.sides[s].fainted_last_turn.map(|x| x as u8);
            b.sides[s].last_move = self.sides[s].side_last_move;
            for (slot, tm) in self.sides[s].mons.iter().enumerate() {
                let id = PokeId { side: s as u8, slot: slot as u8 };
                let p = b.poke_mut(id);
                p.is_active = tm.active && !tm.fainted;
                p.is_started = tm.active && !tm.fainted;
                p.previously_switched_in = tm.appear_count;
                p.active_turns =
                    (self.turn.saturating_sub(tm.switch_in_turn)) as i32;
                p.newly_switched = false;
            }
        }

        // ---- per-mon public state (status while HP is still full, then HP)
        for s in 0..2 {
            for slot in 0..b.sides[s].roster.len() {
                let id = PokeId { side: s as u8, slot: slot as u8 };
                let tm = &self.sides[s].mons[slot];
                self.apply_status(dex, &mut b, id, tm, rng);
            }
        }
        // own side authoritative overwrite from the request
        for (pos, rm) in req.pokemon.iter().enumerate() {
            let slot = b.sides[me].party[pos];
            let id = PokeId { side: me as u8, slot };
            if rm.fainted {
                let p = b.poke_mut(id);
                p.hp = 0;
                p.fainted = true;
                p.status = Status::None;
            } else {
                let p = b.poke_mut(id);
                p.hp = rm.hp.min(p.maxhp);
                p.status = rm.status;
            }
            // current item
            let item = if rm.item.is_empty() { None } else { dex.items.id(&toid(&rm.item)) };
            set_item_raw(&mut b, id, item);
            b.refresh_poke_mask(dex, id);
        }
        // opponent HP buckets + faints + items
        for (slot, tm) in self.sides[opp].mons.iter().enumerate() {
            let id = PokeId { side: opp as u8, slot: slot as u8 };
            if tm.fainted {
                let p = b.poke_mut(id);
                p.hp = 0;
                p.fainted = true;
                p.status = Status::None;
            } else {
                let maxhp = b.poke(id).maxhp;
                let hp = impute_hp(tm.pixels, tm.hp_den, maxhp);
                b.poke_mut(id).hp = hp;
            }
            // item: mirror impute_mon's contract (Observer knowledge wins)
            let mo = &obs.mons()[slot];
            match mo.item.current {
                Some(known) => set_item_raw(&mut b, id, known),
                None => {} // keep the ref's (candidate's) item
            }
            b.refresh_poke_mask(dex, id);
        }

        // ---- PP marks
        for s in 0..2 {
            for (slot, tm) in self.sides[s].mons.iter().enumerate() {
                let id = PokeId { side: s as u8, slot: slot as u8 };
                let p = b.poke_mut(id);
                for &(mid, n) in &tm.uses {
                    for ms in p.base_move_slots.iter_mut() {
                        // move_matches: PS reveals plain `hiddenpower`
                        if move_matches(dex, ms.id, mid) {
                            ms.pp = (ms.pp - n).max(0);
                            ms.used = true;
                        }
                    }
                }
                p.move_slots = p.base_move_slots;
                // Mimic overlay (announced): replace the Mimic slot
                if let Some(overlay) = tm.mimic_overlay {
                    if let Some(mimic) = dex.moves.id("mimic") {
                        if let Some(i) =
                            (0..p.move_slots.len()).find(|&i| p.move_slots[i].id == mimic)
                        {
                            let ms = dex.move_static(overlay);
                            let pp_ups = if ms.no_pp_boosts { 0 } else { 3 };
                            let mut maxpp = ms.pp * (5 + pp_ups) / 5;
                            if ms.pp == 40 {
                                maxpp -= pp_ups;
                            }
                            p.move_slots[i] = MoveSlot {
                                id: overlay,
                                pp: ms.pp.min(5),
                                maxpp,
                                disabled: false,
                                used: false,
                                shared: false,
                            };
                        }
                    }
                }
            }
        }

        // ---- transforms (needs both mons' base state final)
        for s in 0..2 {
            for (slot, tm) in self.sides[s].mons.iter().enumerate() {
                if let Some((ts_, tslot)) = tm.transformed_into {
                    let id = PokeId { side: s as u8, slot: slot as u8 };
                    let target = PokeId { side: ts_ as u8, slot: tslot as u8 };
                    // A live transform persists after the copy target later
                    // faints, but transform_into refuses fainted targets —
                    // lift the flag for the re-plant, then restore.
                    let (was_fainted, was_hp) = {
                        let t = b.poke(target);
                        (t.fainted, t.hp)
                    };
                    if was_fainted {
                        let t = b.poke_mut(target);
                        t.fainted = false;
                        if t.hp <= 0 {
                            t.hp = 1;
                        }
                    }
                    let _ = b.transform_into(dex, id, target);
                    if was_fainted {
                        let t = b.poke_mut(target);
                        t.fainted = true;
                        t.hp = was_hp;
                    }
                    let p = b.poke_mut(id);
                    for &(mid, n) in &tm.transformed_uses {
                        for ms in p.move_slots.iter_mut() {
                            if move_matches(dex, ms.id, mid) {
                                ms.pp = (ms.pp - n).max(0);
                                ms.used = true;
                            }
                        }
                    }
                }
            }
        }

        // ---- volatiles (engine-API planting)
        for s in 0..2 {
            for (slot, tm) in self.sides[s].mons.iter().enumerate() {
                let id = PokeId { side: s as u8, slot: slot as u8 };
                if b.poke(id).fainted {
                    continue;
                }
                self.plant_volatiles(dex, &mut b, id, tm, rng);
            }
        }

        // ---- boosts (after transform/plants: tracked values are the truth)
        for s in 0..2 {
            for (slot, tm) in self.sides[s].mons.iter().enumerate() {
                let id = PokeId { side: s as u8, slot: slot as u8 };
                b.poke_mut(id).boosts = tm.boosts;
            }
        }

        // ---- last-move flow (after encore/disable planting)
        for s in 0..2 {
            for (slot, tm) in self.sides[s].mons.iter().enumerate() {
                let id = PokeId { side: s as u8, slot: slot as u8 };
                let p = b.poke_mut(id);
                p.last_move = tm.last_move;
                p.last_move_encore = tm.last_move;
                p.last_move_used = tm.last_move;
            }
        }

        // ---- side conditions + weather
        for s in 0..2 {
            let Some(active) = b.active_id(s) else { continue };
            for (key, start) in &self.sides[s].conds {
                b.add_side_condition(dex, s as u8, key, Some(active), EffectHandle::None);
                if let Some(sc) = b.sides[s]
                    .side_conditions
                    .iter_mut()
                    .find(|(c, _)| dex.conds_key(*c) == key)
                {
                    if let Some(d) = sc.1.duration {
                        let elapsed = self.turn.saturating_sub(*start) as i32;
                        sc.1.duration = Some((d - elapsed).max(1));
                    }
                }
            }
        }
        if let Some((wkey, upkeeps)) = &self.weather {
            let src = b.active_id(me).or_else(|| b.active_id(opp));
            b.set_weather(dex, wkey, src, EffectHandle::None);
            if let Some(d) = b.field.weather_state.duration {
                b.field.weather_state.duration = Some((d - *upkeeps as i32).max(1));
            }
        }

        // ---- own active authoritative move state (move requests)
        if !req.force_switch && !req.active_moves.is_empty() {
            if let Some(active) = b.active_id(me) {
                let p = b.poke_mut(active);
                if p.transformed && req.active_moves.iter().all(|rm| rm.maxpp == Some(5)) {
                    let old_slots = p.move_slots;
                    let mut new_slots = nc2000_engine::state::MoveSlots::default();
                    for rm in &req.active_moves {
                        if let Some(mid) = dex.moves.id(&toid(&rm.id)) {
                            let resolved = old_slots
                                .iter()
                                .find(|ms| move_matches(dex, ms.id, mid))
                                .map(|ms| ms.id)
                                .unwrap_or(mid);
                            let pp = rm.pp.unwrap_or(5);
                            let maxpp = rm.maxpp.unwrap_or(5);
                            new_slots.push(MoveSlot {
                                id: resolved,
                                pp,
                                maxpp,
                                disabled: rm.disabled,
                                used: pp < maxpp,
                                shared: false,
                            });
                        }
                    }
                    if !new_slots.is_empty() {
                        p.move_slots = new_slots;
                    }
                } else {
                    for rm in &req.active_moves {
                        if let Some(mid) = dex.moves.id(&toid(&rm.id)) {
                            for ms in p.move_slots.iter_mut() {
                                if move_matches(dex, ms.id, mid) {
                                    if let Some(pp) = rm.pp {
                                        ms.pp = pp;
                                    }
                                    if let Some(maxpp) = rm.maxpp {
                                        ms.maxpp = maxpp;
                                    }
                                    ms.disabled = rm.disabled;
                                }
                            }
                            if !p.transformed {
                                // mirror into shared base slots (PS shares objects)
                                for ms in p.base_move_slots.iter_mut() {
                                    if move_matches(dex, ms.id, mid) {
                                        if let Some(pp) = rm.pp {
                                            ms.pp = pp;
                                        }
                                        if let Some(maxpp) = rm.maxpp {
                                            ms.maxpp = maxpp;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                p.trapped = req.trapped;
                p.maybe_trapped = req.maybe_trapped;
            }
        }

        // ---- pokemon_left: PS counts the picked, unfainted team. The
        // fixture constructor counted the full 6-mon roster and the faints
        // planted above never decremented it, so every terminal check
        // (win / Self-KO clause) was unreachable in searched games —
        // recount from the truncated party.
        for s in 0..2 {
            b.sides[s].pokemon_left = b.sides[s]
                .party
                .iter()
                .filter(|&&slot| !b.sides[s].roster[slot as usize].fainted)
                .count() as i32;
        }

        // ---- request bookkeeping + mid-turn queue
        let kind = if req.force_switch { RequestState::Switch } else { RequestState::Move };
        if kind == RequestState::Switch {
            for s in 0..2 {
                if let Some(active) = b.active_id(s) {
                    if b.poke(active).fainted {
                        // mirror check_fainted
                        b.poke_mut(active).status = Status::Fnt;
                        b.poke_mut(active).switch_flag = SwitchFlag::Yes;
                        b.refresh_poke_mask(dex, active);
                    }
                }
            }
            if let Some(active) = b.active_id(me) {
                if !b.poke(active).fainted {
                    // mid-turn self-switch (Baton Pass)
                    let bp = dex.moves.id("batonpass").ok_or("batonpass id")?;
                    b.poke_mut(active).switch_flag = SwitchFlag::Move(bp);
                }
            }
            b.mid_turn = true;
            // pending foe move (only when the foe still owes an action this
            // turn) — the determinizer's pending-move scrub resamples its id
            // per iteration, so any usable placeholder is fine
            let foe_pending = !self.sides[opp].acted_this_turn
                && b.active_id(opp).map(|a| !b.poke(a).fainted).unwrap_or(false)
                && b.active_id(me).map(|a| !b.poke(a).fainted).unwrap_or(false);
            if foe_pending {
                if let Some(fa) = b.active_id(opp) {
                    let usable: Vec<MoveId> = b
                        .poke(fa)
                        .move_slots
                        .iter()
                        .filter(|s| s.pp > 0 && !s.disabled)
                        .map(|s| s.id)
                        .collect();
                    let mid = if usable.is_empty() {
                        dex.moves.id("struggle").ok_or("struggle id")?
                    } else {
                        usable[rng.below(usable.len())]
                    };
                    let speed = b.get_pokemon_action_speed(dex, fa) as f64;
                    b.queue.push(Action {
                        choice: ActionKind::Move {
                            move_id: mid,
                            target_loc: 0,
                            original_target: None,
                            source_effect: None,
                        },
                        order: 200,
                        priority: dex.move_static(mid).priority as f64,
                        fractional_priority: 0.0,
                        speed,
                        pokemon: Some(fa),
                    });
                }
            }
            if !self.upkeep_this_turn {
                b.queue.push(Action {
                    choice: ActionKind::Residual,
                    order: 300,
                    priority: 0.0,
                    fractional_priority: 0.0,
                    speed: 1.0,
                    pokemon: None,
                });
            }
        }
        b.make_request(dex, kind);

        // ---- masks + speeds
        for s in 0..2u8 {
            for slot in 0..b.sides[s as usize].roster.len() {
                b.refresh_poke_mask(dex, PokeId { side: s, slot: slot as u8 });
            }
        }
        b.update_all_speeds(dex);
        b.battle_mask = b.recompute_battle_mask(dex);
        if kind == RequestState::Move {
            if let Some(active) = b.active_id(opp) {
                b.run_event(
                    dex, &ev::TrapPokemon, EvTarget::Poke(active), None,
                    EffectHandle::None, None, false, false,
                );
            }
        }
        Ok(b)
    }

    fn apply_status(
        &self,
        dex: &Dex,
        b: &mut Battle,
        id: PokeId,
        tm: &TrackMon,
        _rng: &mut SplitMix64,
    ) {
        if tm.fainted {
            return;
        }
        if tm.status != Status::None {
            self.plant_status(dex, b, id, tm);
        }
        // Stadium companion volatiles have public lifecycles of their own
        // (they survive cures and status replacement) — sync them exactly
        sync_companion(dex, b, id, "brnattackdrop", tm.comp_brn);
        sync_companion(dex, b, id, "parspeeddrop", tm.comp_par);
        sync_companion(dex, b, id, "residualdmg", tm.comp_res.is_some());
        if let Some(n) = tm.comp_res {
            if let Some(rd) = dex.conds_id("residualdmg") {
                if let Some(vs) = b.poke_mut(id).volatile_mut(rd) {
                    vs.set_int(DK::Counter, n as i64);
                }
            }
        }
    }

    fn plant_status(&self, dex: &Dex, b: &mut Battle, id: PokeId, tm: &TrackMon) {
        let self_inflicted = tm.status == Status::Slp && tm.rest;
        let source = Some(if self_inflicted { id } else { PokeId { side: 1 - id.side, slot: 0 } });
        b.restore_status(dex, id, tm.status, source);
        match tm.status {
            Status::Slp => {
                let p = b.poke_mut(id);
                if tm.rest {
                    // Rest sleep is a public 2-turn counter
                    p.status_state.set_int(DK::Time, (3 - tm.slept as i64).max(1));
                    p.status_state.set_int(DK::StartTime, 3);
                } else if let Some(t) = p.status_state.get(DK::Time).map(|v| v.as_int()) {
                    // clamp the rolled counter under the publicly slept turns
                    p.status_state.set_int(DK::Time, (t - tm.slept as i64).max(1));
                }
            }
            _ => {}
        }
    }

    fn plant_volatiles(
        &self,
        dex: &Dex,
        b: &mut Battle,
        id: PokeId,
        tm: &TrackMon,
        rng: &mut SplitMix64,
    ) {
        let src_id = |src: Option<(usize, usize)>| {
            src.map(|(s, m)| PokeId { side: s as u8, slot: m as u8 })
        };
        for v in &tm.vols {
            let elapsed = self.turn.saturating_sub(v.start_turn) as i32;
            match v.key.as_str() {
                "encore" => {
                    let Some(mid) = v.move_id else { continue };
                    b.poke_mut(id).last_move_encore = Some(mid);
                    b.add_volatile(dex, id, "encore", None, EffectHandle::None);
                }
                "disable" => {
                    let Some(mid) = v.move_id else { continue };
                    b.poke_mut(id).last_move = Some(mid);
                    b.add_volatile(dex, id, "disable", None, EffectHandle::None);
                }
                "trapped" => {
                    let Some(src) = src_id(v.source) else { continue };
                    if b.poke(src).is_active && !b.poke(src).fainted {
                        b.add_volatile_linked(
                            dex,
                            id,
                            "trapped",
                            Some(src),
                            EffectHandle::None,
                            "trapper",
                        );
                    }
                }
                "partiallytrapped" => {
                    let Some(mid) = v.move_id else { continue };
                    let src = src_id(v.source);
                    b.add_volatile(dex, id, "partiallytrapped", src, EffectHandle::MoveEff(mid));
                }
                "perishsong" => {
                    b.add_volatile(dex, id, "perishsong", None, EffectHandle::None);
                    if let Some(c) = dex.conds_id("perishsong") {
                        if let Some(vs) = b.poke_mut(id).volatile_mut(c) {
                            vs.duration = Some(v.counter.unwrap_or(3) as i32);
                        }
                    }
                }
                "furycutter" => {
                    b.add_volatile(dex, id, "furycutter", None, EffectHandle::None);
                    if let Some(c) = dex.conds_id("furycutter") {
                        if let Some(vs) = b.poke_mut(id).volatile_mut(c) {
                            let n = v.counter.unwrap_or(1).clamp(1, 5);
                            vs.set_int(DK::Multiplier, 1i64 << (n - 1).min(4));
                        }
                    }
                }
                "substitute" => {
                    let creator = src_id(v.source).unwrap_or(id);
                    let init_hp = (b.poke(creator).base_maxhp / 4).max(1);
                    let rem_hp = (init_hp - v.counter.unwrap_or(0) as i32).clamp(1, init_hp);
                    b.add_volatile(dex, id, "substitute", Some(creator), EffectHandle::None);
                    if let Some(c) = dex.conds_id("substitute") {
                        if let Some(vs) = b.poke_mut(id).volatile_mut(c) {
                            vs.set_int(DK::Hp, rem_hp as i64);
                        }
                    }
                }
                key => {
                    let src = src_id(v.source);
                    b.add_volatile(dex, id, key, src, EffectHandle::None);
                }
            }
            // hidden rolled durations: clamp under the publicly elapsed turns
            if let Some(c) = dex.conds_id(&v.key) {
                if let Some(vs) = b.poke_mut(id).volatile_mut(c) {
                    if v.key == "confusion" {
                        if let Some(t) = vs.get(DK::Time).map(|val| val.as_int()) {
                            let ticks = v.counter.unwrap_or(0);
                            vs.set_int(DK::Time, (t - ticks).max(1));
                        }
                    } else if v.key != "perishsong" {
                        if let Some(d) = vs.duration {
                            vs.duration = Some((d - elapsed).max(1));
                        }
                    }
                }
            }
        }
        // mechanics-inferred locks / charge state / recharge
        if tm.must_recharge {
            b.add_volatile(dex, id, "mustrecharge", None, EffectHandle::None);
        }
        if let Some(m) = tm.charging {
            b.add_volatile(dex, id, "twoturnmove", Some(id), EffectHandle::MoveEff(m));
        }
        if let Some((m, uses)) = tm.locked {
            if dex.moves.key(m) == "rollout" {
                b.add_volatile(dex, id, "rollout", Some(id), EffectHandle::MoveEff(m));
                if let Some(c) = dex.conds_id("rollout") {
                    if let Some(vs) = b.poke_mut(id).volatile_mut(c) {
                        vs.set_int(DK::HitCount, uses as i64);
                        vs.set_int(DK::ContactHitCount, uses as i64);
                    }
                }
            } else {
                b.add_volatile(dex, id, "lockedmove", Some(id), EffectHandle::MoveEff(m));
                if let Some(c) = dex.conds_id("lockedmove") {
                    if let Some(vs) = b.poke_mut(id).volatile_mut(c) {
                        // total roll is 2-3 actions; uses so far are public
                        vs.duration = Some((2 - uses).max(1));
                    }
                }
            }
        }
        if tm.stall_streak > 0 && tm.last_protect_turn + 1 >= self.turn && self.turn > 0 {
            b.add_volatile(dex, id, "stall", None, EffectHandle::None);
            if let Some(c) = dex.conds_id("stall") {
                if let Some(vs) = b.poke_mut(id).volatile_mut(c) {
                    let mut counter = 127.0f64;
                    for _ in 1..tm.stall_streak {
                        counter /= 2.0;
                    }
                    if counter.fract() == 0.0 {
                        vs.set(DK::Counter, nc2000_engine::state::Scalar::Int(counter as i64));
                    } else {
                        vs.set(DK::Counter, nc2000_engine::state::Scalar::Float(counter));
                    }
                    vs.duration = Some(if tm.protected_this_turn { 2 } else { 1 });
                }
            }
        }
        let _ = rng;
    }
}

/// party := `slots` (display order); positions canonical (party index for
/// members, then bench parked in roster order — the M10 determinizer scheme).
pub(crate) fn apply_party(b: &mut Battle, s: usize, slots: &[u8]) {
    let side = &mut b.sides[s];
    side.party.clear();
    side.party.extend_from_slice(slots);
    let party_len = side.party.len();
    for pos in 0..party_len {
        let slot = side.party[pos] as usize;
        side.roster[slot].position = pos as u8;
    }
    let mut bench_pos = party_len as u8;
    for slot in 0..side.roster.len() {
        if !side.party.contains(&(slot as u8)) {
            side.roster[slot].position = bench_pos;
            bench_pos += 1;
        }
    }
}

/// Set an item field directly (planting, not an in-battle transition).
fn set_item_raw(b: &mut Battle, id: PokeId, item: Option<nc2000_engine::dex::ItemId>) {
    let p = b.poke_mut(id);
    p.item = item;
    p.item_state = nc2000_engine::state::EffectState {
        id: item.map(nc2000_engine::state::EffId::Item).unwrap_or_default(),
        effect_order: p.item_state.effect_order,
        ..Default::default()
    };
}

/// Impute an HP amount inside the announced 1/48 pixel bucket:
/// px = floor(48·hp/maxhp), floored to 1 while alive.
/// Midpoint of the true-HP range consistent with an announced `cur/den`
/// HP string. The announcement rounding differs by mode: HP Percentage Mod
/// (den = 100) announces `ceil(100*hp/maxhp)` with a not-quite-full 100
/// knocked down to 99; the legacy pixel bar (den = 48) announces
/// `floor(48*hp/maxhp)` clamped to >= 1.
pub fn impute_hp(cur: i32, den: i32, maxhp: i32) -> i32 {
    let den = if den > 0 { den } else { 48 };
    if cur <= 0 {
        return 0;
    }
    if cur >= den {
        return maxhp;
    }
    let (lo, hi) = if den == 100 {
        let lo = ((cur - 1) * maxhp) / 100 + 1;
        let hi = if cur == 99 { maxhp - 1 } else { (cur * maxhp) / 100 };
        (lo, hi)
    } else {
        let lo = if cur == 1 { 1 } else { (cur * maxhp + den - 1) / den };
        let hi = ((cur + 1) * maxhp + den - 1) / den - 1;
        (lo, hi)
    };
    let hi = hi.min(maxhp - 1).max(lo);
    ((lo + hi + 1) / 2).clamp(1, maxhp)
}

/// Re-announce an HP amount the way the stream that produced `den` would —
/// the forward direction [`impute_hp`] inverts. HP Percentage Mod (den = 100)
/// prints `ceil(100*hp/maxhp)` with a not-quite-full value knocked down to 99;
/// the legacy pixel bar (den = 48) prints `floor(48*hp/maxhp)` floored to 1
/// while alive.
///
/// Exists so the `%`-HP invariant is checkable rather than argued: for every
/// announced `cur`, `announce_hp(impute_hp(cur, den, maxhp), maxhp, den)` must
/// be `cur` again. That round trip is exactly what the 2026-07-21 bug broke —
/// the importer assumed a /48 pixel bar on a /100 stream, inflating every foe
/// HP by ~2.08x — and it is the M18 class-A gate's second assertion.
pub fn announce_hp(hp: i32, maxhp: i32, den: i32) -> i32 {
    let den = if den > 0 { den } else { 48 };
    if hp <= 0 {
        return 0;
    }
    if maxhp <= 0 || hp >= maxhp {
        return den;
    }
    if den == 100 {
        ((100 * hp + maxhp - 1) / maxhp).clamp(1, 99)
    } else {
        (den * hp / maxhp).clamp(1, den - 1)
    }
}

fn safe_name(name: &str) -> PokeName {
    let mut s = String::new();
    for ch in name.chars().take(20) {
        if s.len() + ch.len_utf8() > 24 {
            break;
        }
        s.push(ch);
    }
    PokeName::new(&s)
}

// ==================================================================== agent

/// The M15 protocol-driven imperfect-info agent: `ProtocolTracker` +
/// `Observer`/`Belief` (M10a) + `BlindSearch` (M10c) over synthesized
/// battles. One instance per GAME; drive it with `push_line` for every
/// player-visible battle line and `on_request` at each `|request|`, then
/// pump `step` / read `best` exactly like the wasm `BlindSearcher`.
pub struct ProtocolAgent {
    side: usize,
    cfg: RmConfig,
    pool: MetaPool,
    tables: TableSet,
    own_sets: Vec<PokemonSet>,
    pinned_sets: Option<Vec<PokemonSet>>,
    /// M18 community belief prior for the hidden-team fallback imputation.
    /// `None` (the default) leaves it exactly as shipped.
    prior: Option<std::sync::Arc<crate::prior::BeliefPrior>>,
    fallback_policy: crate::belief::FallbackPolicy,
    pick_prior: Option<std::sync::Arc<crate::pick_prior::PickPrior>>,
    pick_preview: bool,
    tracker: ProtocolTracker,
    history: Vec<String>,
    observer: Option<Observer>,
    belief: Option<Belief>,
    rng: SplitMix64,
    battle: Option<Battle>,
    search: Option<BlindSearch>,
    baked: Option<SearchChoice>,
    forced: Option<String>,
    request: Option<Request>,
    /// Decision points where the synthesized own-side legal set differed
    /// from the request-derived one (target 0; the projection still keeps
    /// every submission PS-legal).
    pub legality_drift: u32,
    /// Times `best()` had to be projected onto the request-legal set.
    pub projections: u32,
}

impl ProtocolAgent {
    pub fn new(
        dex: &Dex,
        side: usize,
        pool: MetaPool,
        cfg: RmConfig,
        seed: u64,
    ) -> ProtocolAgent {
        let tables = TableSet::from_pool(dex, &pool);
        ProtocolAgent {
            side,
            cfg,
            pool,
            tables,
            own_sets: Vec::new(),
            pinned_sets: None,
            prior: None,
            fallback_policy: crate::belief::FallbackPolicy::default(),
            pick_prior: None,
            pick_preview: false,
            tracker: ProtocolTracker::new(side),
            history: Vec::new(),
            observer: None,
            belief: None,
            rng: SplitMix64::new(seed),
            battle: None,
            search: None,
            baked: None,
            forced: None,
            request: None,
            legality_drift: 0,
            projections: 0,
        }
    }

    pub fn set_own_team(&mut self, sets: Vec<PokemonSet>) {
        self.own_sets = sets;
    }

    pub fn set_fallback_policy(&mut self, policy: crate::belief::FallbackPolicy) {
        self.fallback_policy = policy;
    }

    /// Open-team-sheet mode: the opponent's true sets are public.
    pub fn pin_opponent(&mut self, sets: Vec<PokemonSet>) {
        self.pinned_sets = Some(sets);
    }

    /// M18: consume a community belief prior. Governs only the hidden-team
    /// fallback imputation; a pinned (open-sheet) belief ignores it.
    pub fn set_belief_prior(&mut self, prior: std::sync::Arc<crate::prior::BeliefPrior>) {
        self.prior = Some(prior);
    }

    pub fn set_pick_prior(&mut self, prior: std::sync::Arc<crate::pick_prior::PickPrior>) {
        self.pick_prior = Some(prior);
    }

    pub fn set_pick_preview(&mut self, enabled: bool) {
        self.pick_preview = enabled;
    }

    pub fn add_pair_json(&mut self, json: &str) -> Result<(), String> {
        self.tables.add_pair_json(json)
    }

    pub fn side(&self) -> usize {
        self.side
    }

    pub fn battle(&self) -> Option<&Battle> {
        self.battle.as_ref()
    }

    pub fn belief(&self) -> Option<&Belief> {
        self.belief.as_ref()
    }

    pub fn search(&self) -> Option<&BlindSearch> {
        self.search.as_ref()
    }

    pub fn observer(&self) -> Option<&Observer> {
        self.observer.as_ref()
    }

    /// Feed one player-visible protocol line (battle lines only, not
    /// `|request|`).
    pub fn push_line(&mut self, dex: &Dex, line: &str) {
        self.tracker.push_line(dex, line);
        match self.observer.as_mut() {
            Some(obs) => {
                sync_names(obs, &self.tracker);
                obs.ingest_line(line, dex);
            }
            None => self.history.push(line.to_string()),
        }
    }

    /// Feed the request JSON at a decision point. Returns false for `wait`
    /// requests (nothing owed).
    pub fn on_request(&mut self, dex: &Dex, request_json: &str) -> Result<bool, String> {
        let req = Request::parse(dex, request_json)?;
        if req.wait {
            self.search = None;
            self.baked = None;
            self.forced = None;
            self.request = None;
            return Ok(false);
        }
        if self.own_sets.is_empty() {
            return Err("own team not set".to_string());
        }
        // lazy observer/belief construction (preview |poke| lines are in by
        // the time the first request arrives)
        if self.observer.is_none() {
            if self.tracker.opp_roster_len() == 0 {
                return Err("no |poke| preview lines seen before first request".to_string());
            }
            let mut obs = Observer::from_mons(self.side, self.tracker.observer_mons());
            obs.set_mechanical_inference(self.pinned_sets.is_none());
            for line in &self.history {
                obs.ingest_line(line, dex);
            }
            sync_names(&mut obs, &self.tracker);
            let mut belief = match &self.pinned_sets {
                Some(sets) => Belief::pinned_checked(dex, "opponent", sets, &obs)?,
                None => Belief::with_fallback_policy(dex, &self.pool, &obs, self.fallback_policy),
            };
            if let (Some(prior), None) = (self.prior.clone(), &self.pinned_sets) {
                belief.set_prior(prior);
            }
            self.history.clear();
            self.observer = Some(obs);
            self.belief = Some(belief);
        }
        self.install(dex, req)
    }

    /// Everything a decision point needs once the observer and belief
    /// exist and the request is parsed: belief sync, synthesis, and the
    /// searcher plus its preview/forced shortcuts. Shared by the live
    /// protocol path (`on_request`) and the hand-entered one
    /// (`set_position`), so the two can never drift apart.
    fn install(&mut self, dex: &Dex, req: Request) -> Result<bool, String> {
        let obs = self.observer.as_ref().unwrap();
        let belief = self.belief.as_mut().unwrap();
        belief.sync_checked(dex, obs)?;
        if let Some(prior) = &self.pick_prior {
            let preview: Vec<_> = self.tracker.sides[self.side].mons.iter().map(|mon| {
                crate::pick_prior::PreviewMon {
                    species: dex.species.key(mon.species).to_string(),
                    level: mon.level,
                    gender: mon.gender.as_str().to_string(),
                    item: mon.preview_item,
                }
            }).collect();
            belief.condition_pick_prior(dex, prior, &preview, obs);
        }

        // Preserve player HP buckets across every candidate determinization.
        self.tracker.set_hp_observations(belief, 1-self.side);
        // synthesize
        let pick = belief.alive().first().copied();
        let battle = {
            let refs = belief.refs(pick);
            self.tracker.synthesize(dex, &self.own_sets, refs, obs, &req, &mut self.rng)?
        };

        // search + preview policy
        let mut search =
            BlindSearch::from_observation(&crate::player_state::ObservedPosition::reconstructed(battle.clone()), dex, self.cfg.clone(), self.side, self.rng.next());
        search.use_public_exclusions_observed(dex, obs, belief);
        search.set_opponent_preview_prior(self.pick_preview);
        self.baked = None;
        self.forced = None;
        if search.is_preview() {
            // PS enforces Max Total Level at preview; the engine's
            // enumeration doesn't — mask overweight picks out of the root
            let levels: Vec<i32> =
                battle.sides[self.side].party.iter().map(|&s| {
                    battle.sides[self.side].roster[s as usize].level as i32
                }).collect();
            let allowed: Vec<bool> = search
                .actions()
                .iter()
                .map(|c| match c {
                    SearchChoice::Team(slots) => battle.preview_level_caps[self.side]
                        .map_or(true, |cap|team_total_level(&levels, slots) <= cap as i32),
                    _ => true,
                })
                .collect();
            if allowed.iter().any(|&a| a) {
                search.mask_actions(&allowed);
            }
            search.prune_preview_aliases(dex);
            // baked pair tables, restricted to the legal picks
            let pick = if self.pinned_sets.is_some() {
                filtered_open_preview(&self.tables, &battle, self.side, &levels, &mut self.rng)
            } else {
                filtered_baked_preview(
                    &self.tables,
                    belief,
                    &battle,
                    self.side,
                    &levels,
                    &mut self.rng,
                )
            };
            if let Some(c) = pick {
                if search.actions().contains(&c) {
                    self.baked = Some(c);
                }
            }
        } else {
            // request-side legality metrics + single-choice bypass
            let legal = req.legal_inputs();
            let synth: Vec<String> = {
                let mut bb = battle.clone();
                bb.legal_choices(dex, self.side)
                    .iter()
                    .map(|&c| ps_input(dex, c))
                    .collect()
            };
            let mut a = legal.clone();
            let mut s2 = synth.clone();
            a.sort();
            s2.sort();
            if a != s2 {
                self.legality_drift += 1;
            }
            if legal.len() == 1 {
                self.forced = Some(legal[0].clone());
            }
            search.prune_rest_aliases(dex);
        }
        self.battle = Some(battle);
        self.search = Some(search);
        self.request = Some(req);
        Ok(true)
    }

    /// Baked preview pick when one applies (ready-to-submit input string).
    pub fn baked_preview(&self, dex: &Dex) -> Option<String> {
        self.baked.map(|c| c.to_input(dex))
    }

    pub fn step(&mut self, dex: &Dex, n: u32) -> Result<u32, String> {
        let search = self.search.as_mut().ok_or("step before on_request")?;
        if self.baked.is_some() || self.forced.is_some() {
            return Ok(search.iterations());
        }
        let belief = self.belief.as_ref().ok_or("no belief")?;
        let obs = self.observer.as_ref().ok_or("no observer")?;
        Ok(search.step(dex, belief, obs, n))
    }

    pub fn iterations(&self) -> u32 {
        self.search.as_ref().map_or(0, |s| s.iterations())
    }

    pub fn step_with_endgame(
        &mut self,
        dex: &Dex,
        n: u32,
        session: &mut crate::endgame::EndgameSession,
        clock: &mut impl FnMut() -> f64,
    ) -> Result<u32, String> {
        let search = self.search.as_mut().ok_or("step before on_request")?;
        if self.baked.is_some() || self.forced.is_some() { return Ok(search.iterations()); }
        let belief = self.belief.as_ref().ok_or("no belief")?;
        let obs = self.observer.as_ref().ok_or("no observer")?;
        Ok(search.step_with_endgame(dex, belief, obs, n, session, clock))
    }

    pub fn step_observed(
        &mut self,
        dex: &Dex,
        n: u32,
        trace: &mut impl FnMut(crate::smmcts::SearchTrace<'_>),
    ) -> Result<u32, String> {
        let search = self.search.as_mut().ok_or("step before on_request")?;
        if self.baked.is_some() || self.forced.is_some() {
            return Ok(search.iterations());
        }
        let belief = self.belief.as_ref().ok_or("no belief")?;
        let obs = self.observer.as_ref().ok_or("no observer")?;
        Ok(search.step_observed(dex, belief, obs, n, trace))
    }

    pub fn prune_root(&mut self) -> Result<(), String> {
        self.search.as_mut().ok_or("prune before on_request")?.prune_dominated();
        Ok(())
    }

    /// The callback returns a side-0 reward in [0, 1]; the flag marks a newly expanded leaf.
    pub fn step_with_leaf(
        &mut self,
        dex: &Dex,
        n: u32,
        leaf: &mut impl FnMut(&mut Battle, &mut SplitMix64, bool) -> f64,
    ) -> Result<u32, String> {
        let search = self.search.as_mut().ok_or("step before on_request")?;
        if self.baked.is_some() || self.forced.is_some() {
            return Ok(search.iterations());
        }
        let belief = self.belief.as_ref().ok_or("no belief")?;
        let obs = self.observer.as_ref().ok_or("no observer")?;
        Ok(search.step_with_leaf(dex, belief, obs, n, leaf))
    }

    fn best_legal_from_search(&self, dex: &Dex, legal: &[String]) -> Option<String> {
        let s = self.search.as_ref()?;
        let acts = s.actions();
        let visits = s.visits();
        let means = s.means();
        acts.iter()
            .zip(visits.iter().zip(means.iter()))
            .filter_map(|(&a, (&v, &m))| {
                let inp = ps_input(dex, a);
                legal.contains(&inp).then_some((inp, v, m))
            })
            .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.total_cmp(&b.2)))
            .map(|(inp, _, _)| inp)
    }

    /// Current best choice, projected onto the request-legal set (never
    /// submits something PS rejects).
    pub fn best(&mut self, dex: &Dex) -> Option<String> {
        if let Some(f) = &self.forced {
            return Some(f.clone());
        }
        if let Some(c) = self.baked {
            return Some(c.to_input(dex));
        }
        let best = ps_input(dex, self.search.as_ref()?.best()?);
        let req = self.request.as_ref()?;
        if req.team_preview {
            return Some(best); // level mask already applied at the root
        }
        let legal = req.legal_inputs();
        if legal.is_empty() || legal.contains(&best) {
            return Some(best);
        }
        self.projections += 1;
        self.best_legal_from_search(dex, &legal)
            .or_else(|| legal.first().cloned())
    }

    /// Execution distribution in request syntax. Project the whole policy
    /// before sampling, merging aliases and displaced mass; never discard
    /// probability after the draw. Preview tables have already sampled in
    /// the legacy lifecycle and are explicitly labelled as such.
    pub fn decision_policy(&self, dex: &Dex, mode: crate::decision::PolicyMode) -> Option<crate::decision::DecisionPolicy<String>> {
        use crate::decision::DecisionPolicy;
        if let Some(input) = &self.forced { return Some(DecisionPolicy::pure("forced", input.clone())); }
        if let Some(action) = self.baked { return Some(DecisionPolicy::pure("preview-sampled", action.to_input(dex))); }
        let policy = self.search.as_ref()?.decision_policy(mode)?.map(|&action| ps_input(dex, action));
        let req = self.request.as_ref()?;
        if req.team_preview { return Some(policy); }
        let legal = req.legal_inputs();
        if legal.is_empty() { return None; }
        let projected = policy.actions().iter().any(|row| !legal.contains(&row.input));
        if !projected {
            return Some(policy);
        }
        let legal_mass: f64 = policy
            .actions()
            .iter()
            .filter(|row| legal.contains(&row.input))
            .map(|row| row.probability)
            .sum();
        if legal_mass > 0.0 {
            DecisionPolicy::new(
                "request-projected",
                policy
                    .actions()
                    .iter()
                    .filter(|row| legal.contains(&row.input))
                    .map(|row| (row.input.clone(), row.probability)),
            )
            .ok()
        } else {
            let fallback = self
                .best_legal_from_search(dex, &legal)
                .unwrap_or_else(|| legal[0].clone());
            Some(DecisionPolicy::pure("request-projected", fallback))
        }
    }

    /// JSON belief info (mirrors the wasm `BlindSearcher.beliefInfo`).
    pub fn belief_info(&self) -> String {
        match &self.belief {
            Some(b) => {
                let candidates: Vec<&str> =
                    b.alive().iter().map(|&i| b.candidate_id(i)).collect();
                serde_json::json!({
                    "count": b.candidate_count(),
                    "fallback": b.is_fallback(),
                    "candidates": candidates,
                })
                .to_string()
            }
            None => r#"{"count":0,"fallback":false,"candidates":[]}"#.to_string(),
        }
    }

    /// Root policy rows (input/visits/frac), sorted by visits.
    pub fn root_policy(&self, dex: &Dex) -> String {
        let (iterations, rows) = match &self.search {
            Some(s) => {
                let acts = s.actions();
                let visits = s.visits();
                let means = s.means();
                let total: u32 = visits.iter().sum();
                let mut rows: Vec<serde_json::Value> = acts
                    .iter()
                    .zip(visits.iter().zip(means.iter()))
                    .map(|(&a, (&n, &m))| {
                        serde_json::json!({
                            "input": ps_input(dex, a),
                            "visits": n,
                            "mean": m,
                            "frac": if total > 0 { n as f64 / total as f64 } else { 0.0 },
                        })
                    })
                    .collect();
                rows.sort_by(|a, b| b["visits"].as_u64().cmp(&a["visits"].as_u64()));
                (s.iterations(), rows)
            }
            None => (0, Vec::new()),
        };
        serde_json::json!({
            "iterations": iterations,
            "baked": self.baked.is_some(),
            "forced": self.forced,
            "actions": rows,
        })
        .to_string()
    }
}

/// PS-canonical choice string: PS normalizes typed hidden powers to the
/// plain `hiddenpower` id (requests and choice parsing both), so a typed
/// engine id must be submitted in the plain form.
fn ps_input(dex: &Dex, c: SearchChoice) -> String {
    if let SearchChoice::Move(id) = c {
        let key = dex.moves.key(id);
        if key.starts_with("hiddenpower") && key != "hiddenpower" {
            return "move hiddenpower".to_string();
        }
    }
    c.to_input(dex)
}

fn sync_names(obs: &mut Observer, tracker: &ProtocolTracker) {
    let opp = 1 - tracker.side;
    obs.set_opp_active(tracker.sides[opp].active);
    for (slot, name) in tracker.names(opp).into_iter().enumerate() {
        if !name.is_empty() {
            obs.set_name(slot, name);
        }
    }
}

fn team_total_level(levels: &[i32], slots: &[u8; 3]) -> i32 {
    slots
        .iter()
        .filter(|&&s| s != 0)
        .map(|&s| levels.get(s as usize - 1).copied().unwrap_or(0))
        .sum()
}

/// `open_preview_pick` restricted to Max-Total-Level-legal actions.
fn filtered_open_preview(
    tables: &TableSet,
    battle: &Battle,
    side: usize,
    levels: &[i32],
    rng: &mut SplitMix64,
) -> Option<SearchChoice> {
    let (tab, i_am_a) = tables.lookup(battle, side)?;
    let p = if i_am_a { &tab.sol.p_a } else { &tab.sol.p_b };
    sample_filtered(tables, p, levels, rng)
}

/// `baked_preview_pick` restricted to Max-Total-Level-legal actions.
fn filtered_baked_preview(
    tables: &TableSet,
    belief: &Belief,
    battle: &Battle,
    side: usize,
    levels: &[i32],
    rng: &mut SplitMix64,
) -> Option<SearchChoice> {
    if belief.candidate_count() != 1 {
        return None;
    }
    let opp = belief.alive()[0];
    let me = tables.side_index(battle, side)?;
    let (tab, i_am_a) = tables.pair_by_idx(me, opp)?;
    let p = if i_am_a { &tab.sol.p_a } else { &tab.sol.p_b };
    sample_filtered(tables, p, levels, rng)
}

/// Sample a mixed preview policy conditioned on the PS-legal action subset
/// (renormalized; degenerate rows fall back to the legal argmax).
fn sample_filtered(
    tables: &TableSet,
    p: &[f64],
    levels: &[i32],
    rng: &mut SplitMix64,
) -> Option<SearchChoice> {
    let actions = tables.actions();
    let legal: Vec<usize> = (0..p.len())
        .filter(|&i| team_total_level(levels, &actions[i]) <= MAX_TOTAL_LEVEL)
        .collect();
    if legal.is_empty() {
        return None;
    }
    let mass: f64 = legal.iter().map(|&i| p[i]).sum();
    let pick = if mass <= 0.0 {
        legal
            .iter()
            .copied()
            .max_by(|&a, &b| p[a].total_cmp(&p[b]))
            .unwrap()
    } else {
        let u = rng.next_f64() * mass;
        let mut acc = 0.0;
        let mut pick = legal[legal.len() - 1];
        for &i in &legal {
            acc += p[i];
            if u < acc {
                pick = i;
                break;
            }
        }
        pick
    };
    Some(SearchChoice::Team(actions[pick]))
}

#[cfg(test)]
mod m17a_tests {
    use super::*;

    #[test]
    fn diagnostic_policy_uses_ps_canonical_hidden_power() {
        let dex = conformance::load_dex();
        let ice = dex.moves.id("hiddenpowerice").unwrap();
        assert_eq!(ps_input(&dex, SearchChoice::Move(ice)), "move hiddenpower");
    }

    #[test]
    fn transformed_moves_keep_charge_and_lock_lifecycle_without_base_pp_evidence() {
        let dex = conformance::load_dex();
        let mut tracker = ProtocolTracker::new(0);
        for line in [
            "|poke|p1|Ditto, L50|",
            "|poke|p2|Snorlax, L50, M|",
            "|switch|p1a: Ditto|Ditto, L50|100/100",
            "|switch|p2a: Snorlax|Snorlax, L50, M|100/100",
            "|move|p1a: Ditto|Transform|p2a: Snorlax",
            "|-transform|p1a: Ditto|p2a: Snorlax",
            "|move|p1a: Ditto|Solar Beam|p2a: Snorlax",
            "|-prepare|p1a: Ditto|Solar Beam|p2a: Snorlax",
        ] {
            tracker.push_line(&dex, line);
        }
        let solar_beam = dex.moves.id("solarbeam").unwrap();
        assert_eq!(tracker.sides[0].mons[0].charging, Some(solar_beam));

        tracker.push_line(&dex, "|move|p1a: Ditto|Solar Beam|p2a: Snorlax");
        assert_eq!(tracker.sides[0].mons[0].charging, None);

        let thrash = dex.moves.id("thrash").unwrap();
        tracker.push_line(&dex, "|move|p1a: Ditto|Thrash|p2a: Snorlax");
        assert_eq!(tracker.sides[0].mons[0].locked, Some((thrash, 1)));
        tracker.push_line(&dex, "|move|p1a: Ditto|Thrash|p2a: Snorlax");
        assert_eq!(tracker.sides[0].mons[0].locked, Some((thrash, 2)));
        tracker.push_line(&dex, "|move|p1a: Ditto|Thrash|p2a: Snorlax");
        assert_eq!(tracker.sides[0].mons[0].locked, None);

        let uses: Vec<_> = tracker.sides[0].mons[0]
            .uses
            .iter()
            .map(|(id, _)| dex.moves.key(*id))
            .collect();
        assert_eq!(uses, ["transform"]);
        let t_uses: Vec<_> = tracker.sides[0].mons[0]
            .transformed_uses
            .iter()
            .map(|(id, n)| (dex.moves.key(*id), *n))
            .collect();
        assert_eq!(t_uses, [("solarbeam", 1), ("thrash", 1)]);
    }

    #[test]
    fn confusion_self_hit_marks_side_acted_and_sleep_talk_preserves_last_move() {
        let dex = conformance::load_dex();
        let mut tracker = ProtocolTracker::new(0);
        for line in [
            "|poke|p1|Jolteon, L50, M|",
            "|poke|p2|Snorlax, L50, M|",
            "|switch|p1a: Jolteon|Jolteon, L50, M|100/100",
            "|switch|p2a: Snorlax|Snorlax, L50, M|100/100",
            "|turn|1",
            "|-activate|p2a: Snorlax|confusion",
            "|-damage|p2a: Snorlax|90/100|[from] confusion",
        ] {
            tracker.push_line(&dex, line);
        }
        assert!(tracker.sides[1].acted_this_turn);

        // Sleep Talk calling Earthquake must not overwrite `last_move` (sleep_talk)
        tracker.push_line(&dex, "|turn|2");
        tracker.push_line(&dex, "|move|p2a: Snorlax|Sleep Talk|p2a: Snorlax");
        tracker.push_line(
            &dex,
            "|move|p2a: Snorlax|Earthquake|p1a: Jolteon|[from] move: Sleep Talk",
        );
        let sleep_talk = dex.moves.id("sleeptalk").unwrap();
        assert_eq!(tracker.sides[1].mons[0].last_move, Some(sleep_talk));
        assert_eq!(tracker.sides[1].side_last_move, Some(sleep_talk));
    }

    #[test]
    fn failed_baton_pass_clears_pending_bp_and_sub_tracks_creator_and_damage() {
        let dex = conformance::load_dex();
        let mut tracker = ProtocolTracker::new(0);
        for line in [
            "|poke|p1|Jolteon, L55, M|",
            "|poke|p1|Marowak, L50, M|",
            "|poke|p2|Snorlax, L50, M|",
            "|switch|p1a: Jolteon|Jolteon, L55, M|100/100",
            "|switch|p2a: Snorlax|Snorlax, L50, M|100/100",
            "|turn|1",
            "|move|p1a: Jolteon|Baton Pass|p1a: Jolteon",
            "|-fail|p1a: Jolteon",
        ] {
            tracker.push_line(&dex, line);
        }
        assert!(!tracker.sides[0].pending_bp);

        // Create Substitute on Jolteon, absorb a Seismic Toss (50 dmg), Baton Pass to Marowak
        for line in [
            "|turn|2",
            "|move|p1a: Jolteon|Substitute|p1a: Jolteon",
            "|-start|p1a: Jolteon|Substitute",
            "|-damage|p1a: Jolteon|75/100",
            "|move|p2a: Snorlax|Seismic Toss|p1a: Jolteon",
            "|-activate|p1a: Jolteon|Substitute|[damage]",
            "|turn|3",
            "|move|p1a: Jolteon|Baton Pass|p1a: Jolteon",
            "|switch|p1a: Marowak|Marowak, L50, M|100/100",
        ] {
            tracker.push_line(&dex, line);
        }
        let sub = tracker.sides[0].mons[1]
            .vols
            .iter()
            .find(|v| v.key == "substitute")
            .expect("inherited substitute");
        assert_eq!(sub.source, Some((0, 0)));
        assert_eq!(sub.counter, Some(50));
    }

    #[test]
    fn duplicate_nicknames_resolve_active_mon_on_both_sides() {
        let dex = conformance::load_dex();
        let mut tracker = ProtocolTracker::new(0);
        for line in [
            "|poke|p1|Snorlax, L50, M|",
            "|poke|p1|Zapdos, L50|",
            "|poke|p2|Exeggutor, L50, M|",
            "|switch|p1a: Dup|Snorlax, L50, M|100/100",
            "|switch|p2a: Exeggutor|Exeggutor, L50, M|100/100",
            "|turn|1",
            "|switch|p1a: Dup|Zapdos, L50|100/100",
            "|move|p2a: Exeggutor|Psychic|p1a: Dup",
            "|-damage|p1a: Dup|60/100",
        ] {
            tracker.push_line(&dex, line);
        }
        assert_eq!(tracker.sides[0].mons[0].pixels, 100);
        assert_eq!(tracker.sides[0].mons[1].pixels, 60);
    }

    /// The M18 class-A `%`-HP invariant, as a pure property: whatever the
    /// stream announced, the imputed amount re-announces to the same string.
    /// This is the check the 2026-07-21 /48-on-a-/100-stream bug fails -- it
    /// inflated every foe HP by ~2.08x, and the bot then refused kills it had.
    #[test]
    fn imputed_hp_re_announces_to_the_number_the_stream_printed() {
        let mut checked = 0u32;
        for &den in &[48, 100] {
            for maxhp in [3, 17, 63, 100, 141, 189, 214, 363, 714] {
                for cur in 1..den {
                    let hp = impute_hp(cur, den, maxhp);
                    assert!(
                        (1..=maxhp).contains(&hp),
                        "den {den} maxhp {maxhp} cur {cur}: imputed {hp} out of range"
                    );
                    // Buckets can be empty when maxhp < den (a 63-HP mon has
                    // no HP at all for most of the 100 percentages); those
                    // announce as the nearest reachable value instead.
                    if maxhp >= den {
                        assert_eq!(
                            announce_hp(hp, maxhp, den),
                            cur,
                            "den {den} maxhp {maxhp}: {cur} -> {hp} -> {}",
                            announce_hp(hp, maxhp, den)
                        );
                        checked += 1;
                    }
                }
                assert_eq!(impute_hp(0, den, maxhp), 0);
                assert_eq!(impute_hp(den, den, maxhp), maxhp);
                assert_eq!(announce_hp(maxhp, maxhp, den), den);
                assert_eq!(announce_hp(0, maxhp, den), 0);
            }
        }
        assert!(checked > 900, "only {checked} buckets exercised");
    }
}

// ================================================== hand-entered positions

use crate::position::{
    CondSpec, ItemKnowledge, MonSpec, PositionSpec, PpSpec, SideSpec, UseSpec, VolSpec,
    WeatherSpec, SCHEMA,
};

fn gender_of(s: &str) -> Gender {
    match s {
        "M" => Gender::M,
        "F" => Gender::F,
        _ => Gender::N,
    }
}

fn move_key(dex: &Dex, id: MoveId) -> String {
    dex.moves.key(id).to_string()
}

fn move_id_of(dex: &Dex, key: &str) -> Result<MoveId, String> {
    dex.moves.id(&toid(key)).ok_or_else(|| format!("unknown move `{key}`"))
}

impl TrackMon {
    fn to_spec(&self, dex: &Dex) -> MonSpec {
        MonSpec {
            species: dex.species.key(self.species).to_string(),
            level: self.level,
            gender: self.gender.as_str().to_string(),
            name: self.name.clone(),
            item_flag: self.preview_item,
            appeared: self.appeared,
            appear_count: self.appear_count,
            switch_in_turn: self.switch_in_turn,
            active: self.active,
            fainted: self.fainted,
            hp_num: self.pixels,
            hp_den: self.hp_den.max(1),
            hp_exact: None,
            status: self.status.as_str().to_string(),
            rest: self.rest,
            slept: self.slept,
            tox_counter: self.comp_res,
            comp_brn: self.comp_brn,
            comp_par: self.comp_par,
            boosts: self.boosts,
            volatiles: self
                .vols
                .iter()
                .map(|v| VolSpec {
                    key: v.key.clone(),
                    start_turn: v.start_turn,
                    move_key: v.move_id.map(|m| move_key(dex, m)),
                    source: v.source.map(|(s, slot)| [s, slot]),
                    counter: v.counter,
                })
                .collect(),
            uses: self
                .uses
                .iter()
                .map(|&(m, n)| UseSpec { move_key: move_key(dex, m), n })
                .collect(),
            transformed_uses: self
                .transformed_uses
                .iter()
                .map(|&(m, n)| UseSpec { move_key: move_key(dex, m), n })
                .collect(),
            locked: self.locked.map(|(m, n)| UseSpec { move_key: move_key(dex, m), n }),
            charging: self.charging.map(|m| move_key(dex, m)),
            must_recharge: self.must_recharge,
            transformed_into: self.transformed_into.map(|(s, slot)| [s, slot]),
            mimic_overlay: self.mimic_overlay.map(|m| move_key(dex, m)),
            last_move: self.last_move.map(|m| move_key(dex, m)),
            stall_streak: self.stall_streak,
            last_protect_turn: self.last_protect_turn,
            protected_this_turn: self.protected_this_turn,
            ..MonSpec::default()
        }
    }

    fn from_spec(dex: &Dex, m: &MonSpec) -> Result<TrackMon, String> {
        let species = dex
            .species
            .id(&toid(&m.species))
            .ok_or_else(|| format!("unknown species `{}`", m.species))?;
        let mut tm = TrackMon::new(species, m.level, gender_of(&m.gender), m.item_flag);
        tm.name = m.name.clone();
        tm.appeared = m.appeared;
        // "It has appeared" and "it has switched in at least once" are the
        // same statement, and the tracker sets them together on every switch
        // line. A hand-written position states only the first — there is no
        // control for a switch-in COUNT and there should not be — so the
        // second is restored here.
        //
        // Not cosmetic: `Belief::determinize` decides which party slots hold
        // unseen picks from `previously_switched_in`, which this feeds. Left
        // at zero, an opponent the user has seen (and possibly watched
        // faint) reads as a slot never revealed, and the determinizer
        // reshuffles a live bench mon into it — the search then answers a
        // question about a team the opponent does not have.
        tm.appear_count = m.appear_count.max(i32::from(m.appeared));
        tm.switch_in_turn = m.switch_in_turn;
        tm.active = m.active;
        tm.fainted = m.fainted;
        tm.pixels = m.hp_num;
        tm.hp_den = m.hp_den.max(1);
        tm.status = m.status_enum();
        tm.rest = m.rest;
        tm.slept = m.slept;
        tm.comp_res = m.tox_counter;
        tm.comp_brn = m.comp_brn;
        tm.comp_par = m.comp_par;
        tm.boosts = m.boosts;
        for v in &m.volatiles {
            tm.vols.push(TVol {
                key: v.key.clone(),
                start_turn: v.start_turn,
                move_id: v.move_key.as_deref().map(|k| move_id_of(dex, k)).transpose()?,
                source: v.source.map(|[s, slot]| (s, slot)),
                counter: v.counter,
            });
        }
        for u in &m.uses {
            tm.uses.push((move_id_of(dex, &u.move_key)?, u.n));
        }
        for u in &m.transformed_uses {
            tm.transformed_uses.push((move_id_of(dex, &u.move_key)?, u.n));
        }
        tm.locked = match &m.locked {
            Some(u) => Some((move_id_of(dex, &u.move_key)?, u.n)),
            None => None,
        };
        tm.charging = m.charging.as_deref().map(|k| move_id_of(dex, k)).transpose()?;
        tm.must_recharge = m.must_recharge;
        tm.transformed_into = m.transformed_into.map(|[s, slot]| (s, slot));
        tm.mimic_overlay = m.mimic_overlay.as_deref().map(|k| move_id_of(dex, k)).transpose()?;
        tm.last_move = m.last_move.as_deref().map(|k| move_id_of(dex, k)).transpose()?;
        tm.stall_streak = m.stall_streak;
        tm.last_protect_turn = m.last_protect_turn;
        tm.protected_this_turn = m.protected_this_turn;
        Ok(tm)
    }
}

impl ProtocolTracker {
    /// Serialize the whole tracked information set as a hand-editable
    /// position. `obs` supplies the opponent reveal channel, which lives in
    /// the `Observer` rather than here (the tracker records *what happened*;
    /// the observer records *what that proves about the set*).
    ///
    /// `own_sets` are copied in verbatim — they are the analyzing side's
    /// private truth and have no protocol representation at all.
    pub fn to_spec(
        &self,
        dex: &Dex,
        own_sets: &[PokemonSet],
        obs: &Observer,
        req: &Request,
    ) -> PositionSpec {
        let me = self.side;
        let opp = 1 - me;
        let side_spec = |s: usize| -> SideSpec {
            let ts = &self.sides[s];
            SideSpec {
                mons: ts.mons.iter().map(|m| m.to_spec(dex)).collect(),
                active: ts.active,
                conditions: ts
                    .conds
                    .iter()
                    .map(|(key, start)| CondSpec { key: key.clone(), start_turn: *start })
                    .collect(),
                pending_bp: ts.pending_bp,
                acted_this_turn: ts.acted_this_turn,
                fainted_this_turn: ts.fainted_this_turn,
                fainted_last_turn: ts.fainted_last_turn,
                last_move: ts.side_last_move.map(|m| move_key(dex, m)),
                party: Vec::new(),
            }
        };
        let mut sides = [side_spec(0), side_spec(1)];
        // Own side: the REQUEST is the authority, not the protocol echo —
        // display order, exact HP, the item actually in hand, and the active's
        // per-move PP all come from it, and all four can differ from what the
        // public channel implies (a consumed berry, a Mystery Berry's PP, a
        // party the player reordered by switching).
        for (pos, rm) in req.pokemon.iter().enumerate() {
            let Some(slot) = self.sides[me]
                .mons
                .iter()
                .enumerate()
                .find(|(slot, m)| {
                    !sides[me].party.contains(slot)
                        && m.species == rm.species
                        && m.level == rm.level
                })
                .map(|(slot, _)| slot)
            else {
                continue;
            };
            sides[me].party.push(slot);
            let m = &mut sides[me].mons[slot];
            m.hp_exact = Some(rm.hp);
            m.item_now = Some(rm.item.clone());
            if rm.active && !req.active_moves.is_empty() {
                m.pp = req
                    .active_moves
                    .iter()
                    .map(|am| PpSpec {
                        move_key: am.id.clone(),
                        pp: am.pp.unwrap_or(0),
                        maxpp: am.maxpp.unwrap_or(0),
                        disabled: am.disabled,
                    })
                    .collect();
            }
            let _ = pos;
        }
        // Opponent: fold in the reveal channel, slot-aligned with the
        // observer's roster (both follow `|poke|` order).
        for (slot, mo) in obs.mons().iter().enumerate() {
            let Some(m) = sides[opp].mons.get_mut(slot) else { break };
            m.hp_type_mask = mo.hp_type_mask;
            m.revealed_moves = mo.revealed_moves.iter().map(|&x| move_key(dex, x)).collect();
            m.item_original =
                ItemKnowledge::from_obs(mo.item.original.map(|x| x.map(|i| dex.items.key(i))));
            m.item_current =
                ItemKnowledge::from_obs(mo.item.current.map(|x| x.map(|i| dex.items.key(i))));
            m.item_gained = mo.item.gained;
            m.excluded_items = mo.excluded_items;
        }
        PositionSpec {
            schema: SCHEMA.to_string(),
            side: me,
            turn: self.turn,
            upkeep_this_turn: self.upkeep_this_turn,
            own_sets: own_sets.to_vec(),
            sides,
            weather: self
                .weather
                .as_ref()
                .map(|(key, upkeeps)| WeatherSpec { key: key.clone(), upkeeps: *upkeeps }),
            team_preview: req.team_preview,
            force_switch: req.force_switch,
            trapped: req.trapped,
            maybe_trapped: req.maybe_trapped,
        }
    }

    /// Rebuild a tracker from a position — the exact inverse of [`to_spec`]
    /// over every tracked field. `crates/bot/tests/import.rs` proves the
    /// round trip at every decision point of the conformance corpus.
    pub fn from_spec(dex: &Dex, spec: &PositionSpec) -> Result<ProtocolTracker, String> {
        let mut tr = ProtocolTracker::new(spec.side);
        tr.turn = spec.turn;
        tr.upkeep_this_turn = spec.upkeep_this_turn;
        tr.weather = spec.weather.as_ref().map(|w| (w.key.clone(), w.upkeeps));
        for s in 0..2 {
            let src = &spec.sides[s];
            let dst = &mut tr.sides[s];
            for m in &src.mons {
                dst.mons.push(TrackMon::from_spec(dex, m)?);
            }
            dst.active = src.active;
            dst.conds = src.conditions.iter().map(|c| (c.key.clone(), c.start_turn)).collect();
            dst.pending_bp = src.pending_bp;
            dst.acted_this_turn = src.acted_this_turn;
            dst.fainted_this_turn = src.fainted_this_turn;
            dst.fainted_last_turn = src.fainted_last_turn;
            dst.side_last_move = src.last_move.as_deref().map(|k| move_id_of(dex, k)).transpose()?;
        }
        Ok(tr)
    }
}

impl ProtocolAgent {
    /// The accumulated public information set (solver export, harness
    /// inspection). Read-only: the tracker only ever advances through
    /// `push_line` or a whole-position install.
    pub fn tracker(&self) -> &ProtocolTracker {
        &self.tracker
    }

    /// Export the current decision point as a hand-editable position.
    /// `None` before the first request — there is no decision point yet, and
    /// a spec without one would not describe anything.
    pub fn to_position_spec(&self, dex: &Dex) -> Option<PositionSpec> {
        let obs = self.observer.as_ref()?;
        let req = self.request.as_ref()?;
        Some(self.tracker.to_spec(dex, &self.own_sets, obs, req))
    }

    /// Re-run this decision point's synthesis with an explicit seed. The
    /// battle `on_request` already built is the one being searched; this is
    /// the same construction made reproducible, so two information sets can
    /// be compared field-for-field without the rolled hidden durations
    /// (sleep counters, bind turns, substitute HP) drifting between them.
    pub fn resynthesize(&self, dex: &Dex, seed: u64) -> Result<Battle, String> {
        let obs = self.observer.as_ref().ok_or("resynthesize before on_request")?;
        let belief = self.belief.as_ref().ok_or("resynthesize before on_request")?;
        let req = self.request.as_ref().ok_or("resynthesize before on_request")?;
        let pick = belief.alive().first().copied();
        let mut rng = SplitMix64::new(seed);
        self.tracker.synthesize(dex, &self.own_sets, belief.refs(pick), obs, req, &mut rng)
    }

    /// Solver mode: analyze a hand-entered position. Replaces the tracked
    /// battle wholesale — own team, public facts, reveal channel and belief
    /// — and lands on the same searcher the live path builds.
    ///
    /// Deliberately NOT a variant of `push_line`: a position is a *state*,
    /// while the protocol is a *history*, and fabricating a history that
    /// happens to produce the desired state is both harder and weaker than
    /// stating the state (see `position.rs`).
    pub fn set_position(&mut self, dex: &Dex, spec: &PositionSpec) -> Result<(), String> {
        spec.check()?;
        if spec.side != self.side {
            return Err(format!(
                "position is for side {} but this agent plays side {}",
                spec.side, self.side
            ));
        }
        let request_json = spec.request_json(dex)?;
        let req = Request::parse(dex, &request_json)?;
        self.own_sets = spec.own_sets.clone();
        self.tracker = ProtocolTracker::from_spec(dex, spec)?;
        self.history.clear();
        let obs = Observer::from_position(dex, spec)?;
        let mut belief = match &self.pinned_sets {
            Some(sets) => Belief::pinned_checked(dex, "opponent", sets, &obs)?,
            None => Belief::new(dex, &self.pool, &obs),
        };
        if let (Some(prior), None) = (self.prior.clone(), &self.pinned_sets) {
            belief.set_prior(prior);
        }
        self.observer = Some(obs);
        self.belief = Some(belief);
        self.install(dex, req).map(|_| ())
    }
}
