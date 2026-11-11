//! M10a observation tracker: what one player legitimately knows about the
//! opponent's team, accumulated over a battle.
//!
//! # Tap point
//!
//! Two channels, both driven by `Observer::observe(&Battle)` called at the
//! bot's real decision points (never inside search iterations):
//!
//! 1. **State diff** — reads opponent fields whose *every mutation path in
//!    this engine is publicly announced*, so the diff carries exactly the
//!    information a competent human extracts from the protocol:
//!    - `base_move_slots`: `pp < maxpp || used` ⇔ the move was executed at
//!      least once with a public `|move|` line. Audited mutation paths:
//!      `deduct_pp` is called only from `run_move` (public `|move|`) and
//!      Spite (public `-activate`, targets the already-revealed last move);
//!      Mystery Berry's PP restore is public and only reaches a slot that
//!      hit 0 PP (already revealed). Transform/Mimic overlay slots are
//!      `shared = false`, so foreign moves never touch the base slots.
//!    - `item`: transitions only. `Some(x) → None` (use_item / eat_item /
//!      take_item — all emit `-enditem` or the Thief `-item ... [of]` line),
//!      `None → Some(y)` (Thief steal, public `-item`). The standing value
//!      is stored privately for diffing but never exposed; only transitions
//!      (all public) produce knowledge.
//!    - preview-public facts read once at construction: species, level,
//!      gender (the `|poke|` details) and *item presence* (the `|poke|`
//!      line's item flag).
//!    This channel works with the protocol log disabled.
//!
//! 2. **Log scan** — incremental parse of `battle.log` when the observed
//!    battle happens to run log-on. Adds the reveals that leave no state
//!    trace: Leftovers residual heals (`-heal ... [from] item: Leftovers`),
//!    Focus Band procs (`-activate ... item: Focus Band`), and moves called
//!    by Sleep Talk (`|move| ... [from] Sleep Talk` — in gen 2 the called
//!    move is from the sleeper's own set). Degrades silently to channel 1
//!    when the log is off.
//!
//! Rejected taps: *pure log parsing* (dies in log-off arenas), *instrumenting
//! apply_choices* (would hand the tracker the opponent's submitted choice,
//! which is NOT public when the move is cancelled invisibly — sleep/para
//! `|cant|` hides the selection — i.e. a psychic tap by construction; it
//! would also touch engine code the bit-exactness constraint protects), and
//! a *tracker-owned log-on mirror battle* (needs the applied choices plumbed
//! in — the same psychic tap — and adds 2x real-game stepping for no
//! information beyond the two channels above).
//!
//! M10b should run the *outer* (real) battle log-on where possible — the
//! cost is one game's protocol log; search clones inside `SkuctSearch`
//! always disable the log themselves.
//!
//! # What is deliberately NOT tracked (conservative by design)
//!
//! Quick Claw (silent in this engine's log; humans infer it from impossible
//! ordering), Bright Powder (unattributed misses), King's Rock (unattributed
//! flinches), and stat-boost items (silent). Missing a reveal only leaves the belief a superset —
//! it can never false-filter the true team.

use nc2000_engine::dex::{toid, Dex, ItemId, MoveId, SpeciesId};
use nc2000_engine::state::{Battle, Gender};

pub const ITEM_LEFTOVERS: u32 = 1 << 0;
pub const ITEM_MIRACLE_BERRY: u32 = 1 << 1;
pub const ITEM_MINT_BERRY: u32 = 1 << 2;
pub const ITEM_PRZ_CURE_BERRY: u32 = 1 << 3;
pub const ITEM_PSN_CURE_BERRY: u32 = 1 << 4;
pub const ITEM_BURNT_BERRY: u32 = 1 << 5;
pub const ITEM_ICE_BERRY: u32 = 1 << 6;
pub const ITEM_BITTER_BERRY: u32 = 1 << 7;
pub const ITEM_BERSERK_GENE: u32 = 1 << 8;

pub fn excluded_item_bit(dex: &Dex, item: ItemId) -> u32 {
    match dex.items.key(item) {
        "leftovers" => ITEM_LEFTOVERS,
        "miracleberry" => ITEM_MIRACLE_BERRY,
        "mintberry" => ITEM_MINT_BERRY,
        "przcureberry" => ITEM_PRZ_CURE_BERRY,
        "psncureberry" => ITEM_PSN_CURE_BERRY,
        "burntberry" => ITEM_BURNT_BERRY,
        "iceberry" => ITEM_ICE_BERRY,
        "bitterberry" => ITEM_BITTER_BERRY,
        "berserkgene" => ITEM_BERSERK_GENE,
        _ => 0,
    }
}

pub fn move_matches(dex: &Dex, slot: MoveId, revealed: MoveId) -> bool {
    slot == revealed
        || (dex.moves.key(revealed) == "hiddenpower"
            && dex.moves.key(slot).starts_with("hiddenpower"))
}

/// Public knowledge about one opponent roster mon's held item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemObs {
    /// The item the mon *brought* (what set filtering matches against).
    /// `None` = identity unknown; `Some(None)` = known to have started
    /// empty-handed (its Thief succeeded).
    pub original: Option<Option<ItemId>>,
    /// The *currently held* item when publicly known (the determinizer keeps
    /// the true item field exactly when this is `Some`). `Some(None)` =
    /// known consumed/stolen away.
    pub current: Option<Option<ItemId>>,
    /// A gain event (Thief steal) was observed — later reveals no longer
    /// identify the original item. `pub(crate)` so a hand-entered position
    /// can round-trip the whole channel (`position.rs`), not just the two
    /// halves a reader would guess at.
    pub(crate) gained: bool,
}

impl ItemObs {
    /// `Some(x) → None` transition or `-enditem` line: x left the mon.
    fn lost(&mut self, x: ItemId) -> bool {
        let mut dirty = self.current != Some(None);
        self.current = Some(None);
        if !self.gained && self.original.is_none() {
            self.original = Some(Some(x));
            dirty = true;
        }
        dirty
    }

    /// `None → Some(y)` transition or `-item` line: y arrived (Thief).
    fn gained(&mut self, y: ItemId) -> bool {
        let dirty = self.current != Some(Some(y)) || !self.gained;
        self.current = Some(Some(y));
        if self.original.is_none() {
            // Thief only steals into an empty hand.
            self.original = Some(None);
        }
        self.gained = true;
        dirty
    }

    /// Passive reveal of a held item (Leftovers heal, Focus Band proc).
    /// Only upgrades unknown state — a stale `-activate` right after an
    /// `-enditem` (Mystery Berry) must not resurrect the item.
    fn revealed_held(&mut self, x: ItemId) -> bool {
        let mut dirty = false;
        if self.current.is_none() {
            self.current = Some(Some(x));
            dirty = true;
        }
        if !self.gained && self.original.is_none() {
            self.original = Some(Some(x));
            dirty = true;
        }
        dirty
    }
}

/// Everything legitimately known about one opponent roster mon.
#[derive(Clone, Debug)]
pub struct MonObs {
    // ---- public at team preview
    pub species: SpeciesId,
    pub level: u8,
    pub gender: Gender,
    /// Nickname (public whenever it appears in a log line; used only to
    /// resolve log subjects back to roster slots).
    pub name: String,
    /// The `|poke|` preview line's item flag: the mon brought *an* item.
    pub preview_has_item: bool,
    // ---- accumulated in battle
    /// Moves known to be in the mon's set (each publicly executed at least
    /// once, or named by a Sleep Talk call). Grows monotonically.
    pub revealed_moves: Vec<MoveId>,
    pub hp_type_mask: u16,
    pub item: ItemObs,
    /// Bitmask of items mechanically proved NOT to be held (`ITEM_*` flags).
    pub excluded_items: u32,
    /// Upper bound on base unboosted Spe stat proved by turn-order resolution.
    pub max_spe_stat: u16,
    /// The mon has switched in at least once (its pick is public).
    pub appeared: bool,
}

#[derive(Clone, Copy, Debug)]
struct PendingHpHit {
    opp_slot: usize,
    own_species: SpeciesId,
    saw_direct_damage: bool,
    sub_hit: bool,
    eff: Option<crate::hp_evidence::Effectiveness>,
}

#[derive(Clone, Debug, Default)]
struct MechTracker {
    own_active_species: Option<SpeciesId>,
    own_type_dirty: bool,
    foresight_on_own: bool,
    pending_hp: Option<PendingHpHit>,
    pending_status_berry: Option<(usize, u32)>,
    opp_active_damaged: bool,
    opp_active_switched_mid_turn: bool,
    opp_saw_leftovers_this_turn: bool,
}

/// Observation tracker for one side of one battle. Construct at battle
/// start (team preview — item *presence* is read then), call `observe`
/// at every real decision point.
#[derive(Clone)]
pub struct Observer {
    side: usize,
    mons: Vec<MonObs>,
    /// Private diff snapshot of the opponent's item fields — never exposed;
    /// only its (publicly-announced) transitions produce knowledge.
    prev_item: Vec<Option<ItemId>>,
    log_pos: usize,
    /// Per-mon log-channel suppression: while a Transform copy is active
    /// (tracked from the log lines themselves, in order), `|move|` lines may
    /// name any move of the copied set, so none of them is evidence.
    log_suppress: Vec<bool>,
    /// Per-mon: the single move Mimic overwrote its own slot with. Mimic
    /// replaces exactly one slot, and PS names the copied move on the
    /// `|-activate|<mon>|move: Mimic|<Move>` line, so only that move has to
    /// be dropped — the mon's other three slots are still its own and every
    /// use of them is a genuine reveal. Cleared with the mon's stay, because
    /// `clearVolatile` restores `baseMoveSlots` on switch-out.
    mimic_overlay: Vec<Option<MoveId>>,
    /// Per-mon: a two-turn move a CALLER (Metronome / Mirror Move) put into
    /// its charge turn. The release line is a plain `|move|` with no `[from]`,
    /// so without this the caller's victim move is credited to the mon's own
    /// set — an over-reveal, which is worse than a miss: five "revealed" moves
    /// for four slots is unsatisfiable and makes `Belief::sync_checked` fail
    /// closed on the pinned/open-sheet path. `corpus.rs::collect_set_evidence`
    /// has carried this rule offline since the corpus harness was built; this
    /// is the same rule on the live channel.
    called_charging: Vec<Option<MoveId>>,
    /// Per-mon: a multi-turn locked move (`Outrage`, `Petal Dance`, `Thrash`,
    /// `Rollout`, `Bide`) started by a CALLER (`Metronome` / `Mirror Move`).
    /// Continuation turns emit plain `|move|` lines with no `[from]` until the
    /// lock ends (mon switches/faints or executes a different move).
    called_locked: Vec<Option<MoveId>>,
    /// The last line was a called move announcing its charge (`[from]` +
    /// `[still]`), awaiting the `-prepare` that confirms which move it is.
    pending_call: Option<(usize, MoveId)>,
    /// Chronologically active opponent roster slot (disambiguates active
    /// `p2a:` subjects when multiple roster slots share a nickname).
    opp_active: Option<usize>,
    /// Opt-in mechanical inference tracker (HP type & passive item exclusions).
    mech: Option<MechTracker>,
    revision: u64,
}

impl ItemObs {
    /// Rebuild the channel from a serialized position (`position.rs`). The
    /// three parts are not independent — `gained` decides whether a later
    /// reveal may still identify `original` — so they are set together
    /// rather than through three public fields.
    pub fn from_parts(
        original: Option<Option<ItemId>>,
        current: Option<Option<ItemId>>,
        gained: bool,
    ) -> ItemObs {
        ItemObs { original, current, gained }
    }
}

impl Observer {
    /// Solver mode: construct the reveal channel directly from a
    /// hand-entered position instead of replaying protocol lines. Same
    /// contract as `from_mons` — the state-diff channel never runs, so
    /// `observe` must not be called on the result.
    pub fn from_position(
        dex: &Dex,
        spec: &crate::position::PositionSpec,
    ) -> Result<Observer, String> {
        let foe = spec.foe();
        let item = |k: &crate::position::ItemKnowledge| -> Result<Option<Option<ItemId>>, String> {
            match k.to_obs() {
                None => Ok(None),
                Some(None) => Ok(Some(None)),
                Some(Some(key)) => dex
                    .items
                    .id(&toid(key))
                    .map(|i| Some(Some(i)))
                    .ok_or_else(|| format!("unknown item `{key}`")),
            }
        };
        let mut mons = Vec::new();
        for m in &spec.sides[foe].mons {
            if m.hp_type_mask == 0 {
                return Err("Hidden Power type constraint is empty".into());
            }
            if m.hp_type_mask != u16::MAX && !m.revealed_moves.iter().any(|m| toid(m).starts_with("hiddenpower")) {
                return Err("Hidden Power type constraint requires a revealed move".into());
            }
            let species = dex
                .species
                .id(&toid(&m.species))
                .ok_or_else(|| format!("unknown species `{}`", m.species))?;
            if m.hp_type_mask != u16::MAX {
                let gender = match m.gender.as_str() { "M" => Gender::M, "F" => Gender::F, _ => Gender::N };
                if m.hp_type_mask & crate::hp_evidence::possible_types(dex, species, gender) == 0 {
                    return Err("Hidden Power type contradicts public gender".into());
                }
            }
            if m.hp_type_mask != u16::MAX && m.revealed_moves.iter().any(|key| {
                let key = toid(key);
                key.strip_prefix("hiddenpower").filter(|suffix| !suffix.is_empty()).and_then(|suffix| {
                    crate::hp_evidence::TYPES.iter().position(|t| t.eq_ignore_ascii_case(suffix))
                }).is_some_and(|index| m.hp_type_mask & (1 << index) == 0)
            }) {
                return Err("Hidden Power type constraint contradicts a revealed move".into());
            }
            let mut revealed = Vec::new();
            for key in &m.revealed_moves {
                let id = dex
                    .moves
                    .id(&toid(key))
                    .ok_or_else(|| format!("unknown move `{key}`"))?;
                if !revealed.contains(&id) {
                    revealed.push(id);
                }
            }
            mons.push(MonObs {
                species,
                level: m.level,
                gender: match m.gender.as_str() {
                    "M" => Gender::M,
                    "F" => Gender::F,
                    _ => Gender::N,
                },
                name: m.name.clone(),
                preview_has_item: m.item_flag,
                revealed_moves: revealed,
                hp_type_mask: m.hp_type_mask,
                item: ItemObs::from_parts(item(&m.item_original)?, item(&m.item_current)?, m.item_gained),
                excluded_items: m.excluded_items,
                max_spe_stat: u16::MAX,
                appeared: m.appeared,
            });
        }
        let mut obs = Observer::from_mons(spec.side, mons);
        obs.opp_active = spec.sides[foe].active;
        Ok(obs)
    }

    /// M15 protocol mode: construct from preview-public facts parsed out of
    /// the protocol (`|poke|` lines) instead of a battle. The state-diff
    /// channel never runs (there is no true battle to diff — `observe` must
    /// not be called); all knowledge arrives via `ingest_line`. Nicknames
    /// are learned from switch lines by the caller (`set_name`) — subject
    /// resolution needs them before the mon's first `|move|` line, and
    /// switch lines always precede them.
    pub(crate) fn from_mons(side: usize, mons: Vec<MonObs>) -> Observer {
        let n = mons.len();
        Observer {
            side,
            mons,
            prev_item: vec![None; n],
            log_pos: 0,
            log_suppress: vec![false; n],
            mimic_overlay: vec![None; n],
            called_charging: vec![None; n],
            called_locked: vec![None; n],
            pending_call: None,
            opp_active: None,
            mech: None,
            revision: 0,
        }
    }

    pub fn set_mechanical_inference(&mut self, enabled: bool) {
        self.mech = if enabled { Some(MechTracker::default()) } else { None };
    }

    /// Protocol mode: feed one player-visible protocol line through the log
    /// channel (the only channel — see `from_mons`).
    pub(crate) fn ingest_line(&mut self, line: &str, dex: &Dex) {
        if self.scan_line(line, dex) {
            self.revision += 1;
        }
    }

    /// Protocol mode: a mon appeared under `name` (resolves log subjects).
    pub(crate) fn set_name(&mut self, slot: usize, name: &str) {
        if self.mons[slot].name != name {
            self.mons[slot].name = name.to_string();
        }
        if !self.mons[slot].appeared {
            self.mons[slot].appeared = true;
            self.revision += 1;
        }
    }

    /// Protocol mode: sync the active opponent roster slot.
    pub(crate) fn set_opp_active(&mut self, slot: Option<usize>) {
        self.opp_active = slot;
    }

    pub fn new(battle: &Battle, side: usize) -> Observer {
        let opp = 1 - side;
        let mons = battle.sides[opp]
            .roster
            .iter()
            .map(|p| MonObs {
                species: p.species,
                level: p.level,
                gender: p.gender,
                name: p.name.as_str().to_string(),
                preview_has_item: p.item.is_some(),
                revealed_moves: Vec::new(),
                hp_type_mask: u16::MAX,
                item: ItemObs::default(),
                excluded_items: 0,
                max_spe_stat: u16::MAX,
                appeared: false,
            })
            .collect::<Vec<_>>();
        let prev_item = battle.sides[opp].roster.iter().map(|p| p.item).collect();
        let n = mons.len();
        Observer {
            side,
            mons,
            prev_item,
            log_pos: 0,
            log_suppress: vec![false; n],
            mimic_overlay: vec![None; n],
            called_charging: vec![None; n],
            called_locked: vec![None; n],
            pending_call: None,
            opp_active: battle.sides[opp].active.map(|a| a as usize),
            mech: None,
            revision: 0,
        }
    }

    pub fn side(&self) -> usize {
        self.side
    }

    pub fn opp(&self) -> usize {
        1 - self.side
    }

    /// Per opponent roster slot, aligned with `battle.sides[opp].roster`.
    pub fn mons(&self) -> &[MonObs] {
        &self.mons
    }

    /// Bumped whenever new information arrives (belief sync trigger).
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Ingest everything that became visible since the last call. Call at
    /// every decision point of the *real* battle (never on search clones).
    pub fn observe(&mut self, battle: &Battle, dex: &Dex) {
        let mut dirty = false;
        let opp = self.opp();

        // ---- channel 1: state diff over public-equivalent fields
        self.opp_active = battle.sides[opp].active.map(|a| a as usize);
        if let Some(mech) = self.mech.as_mut() {
            if let Some(own_act) = battle.active_id(self.side) {
                let p = battle.poke(own_act);
                let conversion = dex
                    .conds
                    .id("conversion")
                    .is_some_and(|c| p.has_volatile(c))
                    || dex
                        .conds
                        .id("conversion2")
                        .is_some_and(|c| p.has_volatile(c));
                let foresight = dex
                    .conds
                    .id("foresight")
                    .is_some_and(|c| p.has_volatile(c));
                mech.own_active_species = Some(p.species);
                mech.own_type_dirty = p.transformed || conversion;
                mech.foresight_on_own = foresight;
            }
            if let Some(opp_act) = battle.active_id(opp) {
                let p = battle.poke(opp_act);
                mech.opp_active_damaged = p.hp > 0 && p.hp < p.maxhp && !p.fainted;
            }
        }
        for (slot, p) in battle.sides[opp].roster.iter().enumerate() {
            let mo = &mut self.mons[slot];
            if !mo.appeared && (p.previously_switched_in > 0 || p.is_active) {
                mo.appeared = true;
                dirty = true;
            }
            // moves: base-slot usage marks (every path is a public |move|)
            for s in p.base_move_slots.iter() {
                if (s.pp < s.maxpp || s.used) && !mo.revealed_moves.contains(&s.id) {
                    mo.revealed_moves.push(s.id);
                    dirty = true;
                    if self.mech.is_some() && dex.moves.key(s.id) == "hiddenpower" {
                        let gender_mask =
                            crate::hp_evidence::possible_types(dex, mo.species, mo.gender);
                        let next = mo.hp_type_mask & gender_mask;
                        if next != 0 && next != mo.hp_type_mask {
                            mo.hp_type_mask = next;
                        }
                    }
                }
            }
            // item: transitions only (every path is publicly announced)
            let cur = p.item;
            let prev = self.prev_item[slot];
            if cur != prev {
                match (prev, cur) {
                    (Some(x), None) => dirty |= mo.item.lost(x),
                    (None, Some(y)) => dirty |= mo.item.gained(y),
                    (Some(x), Some(y)) => {
                        // unreachable in gen 2 (no in-place swap); defensive
                        if !mo.item.gained && mo.item.original.is_none() {
                            mo.item.original = Some(Some(x));
                        }
                        dirty |= mo.item.gained(y);
                    }
                    (None, None) => unreachable!(),
                }
                self.prev_item[slot] = cur;
            }
        }

        // ---- channel 2: incremental protocol-log scan (log-on battles)
        if battle.log.len() < self.log_pos {
            // log was cleared (set_log_enabled(false)); lose the tail
            self.log_pos = battle.log.len();
        }
        for i in self.log_pos..battle.log.len() {
            let line = battle.log[i].clone();
            dirty |= self.scan_line(&line, dex);
        }
        self.log_pos = battle.log.len();
        dirty |= self.flush_pending_mech(dex);

        if dirty {
            self.revision += 1;
        }
    }

    fn flush_pending_mech(&mut self, dex: &Dex) -> bool {
        let Some(mech) = self.mech.as_mut() else {
            return false;
        };
        let mut dirty = false;
        if let Some((slot, bit)) = mech.pending_status_berry.take() {
            if let Some(mo) = self.mons.get_mut(slot) {
                if !mo.item.gained && (mo.excluded_items & bit) == 0 {
                    mo.excluded_items |= bit;
                    dirty = true;
                }
            }
        }
        if let Some(hit) = mech.pending_hp.take() {
            let eff = match hit.eff {
                Some(e) => Some(e),
                None if !hit.sub_hit && hit.saw_direct_damage => {
                    Some(crate::hp_evidence::Effectiveness::Neutral)
                }
                None => None,
            };
            if let Some(eff) = eff {
                if let Some(mo) = self.mons.get_mut(hit.opp_slot) {
                    let eff_mask = crate::hp_evidence::types_with_effectiveness(
                        dex,
                        hit.own_species,
                        mech.foresight_on_own,
                        eff,
                    );
                    let gender_mask =
                        crate::hp_evidence::possible_types(dex, mo.species, mo.gender);
                    let next = mo.hp_type_mask & eff_mask & gender_mask;
                    if next != 0 && next != mo.hp_type_mask {
                        mo.hp_type_mask = next;
                        dirty = true;
                    }
                }
            }
        }
        dirty
    }

    fn opp_hp_is_damaged(cond: &str) -> Option<bool> {
        let hp_part = cond.split_whitespace().next().unwrap_or("");
        if hp_part == "0" || cond.ends_with("fnt") {
            return Some(false);
        }
        let (num_s, den_s) = hp_part.split_once('/')?;
        let num: i32 = num_s.parse().ok()?;
        let den: i32 = den_s.parse().ok()?;
        Some(num > 0 && num < den)
    }

    // ------------------------------------------------------------ log scan

    fn scan_line(&mut self, line: &str, dex: &Dex) -> bool {
        let Some(body) = line.strip_prefix('|') else { return false };
        let parts: Vec<&str> = body.split('|').collect();
        let cmd = parts.first().copied().unwrap_or("");
        let mut dirty = false;
        if matches!(
            cmd,
            "move" | "switch" | "drag" | "faint" | "cant" | "upkeep" | "turn"
        ) {
            dirty |= self.flush_pending_mech(dex);
        }
        // The charge announcement and its `-prepare` are adjacent; anything
        // in between means this was not a charging call after all.
        let pending = self.pending_call.take();
        match cmd {
            "-prepare" => {
                let subject = parts.get(1).copied().unwrap_or("");
                let named = parts.get(2).copied().unwrap_or("");
                if let (Some(m), Some(id)) = (self.opp_subject(subject), dex.moves.id(&toid(named)))
                {
                    if pending == Some((m, id)) {
                        self.called_charging[m] = Some(id);
                    }
                }
                dirty
            }
            "cant" | "-anim" => {
                if let Some(m) = self.opp_subject(parts.get(1).copied().unwrap_or("")) {
                    self.called_charging[m] = None;
                }
                dirty
            }
            "move" => dirty | self.scan_move(&parts, dex),
            "switch" | "drag" | "faint" => {
                // clear_volatile drops Transform/Mimic overlays on exit; a
                // switch line's subject is the incoming mon, so clear all
                // suppression for the side (only actives carry overlays).
                let subj = parts.get(1).copied().unwrap_or("");
                if self.subject_is_opp_side(subj) {
                    self.log_suppress.iter_mut().for_each(|s| *s = false);
                    // Mimic's overwritten slot goes back to Mimic on exit
                    self.mimic_overlay.iter_mut().for_each(|o| *o = None);
                    // a charge or multi-turn lock dies with the mon that was holding it
                    self.called_charging.iter_mut().for_each(|c| *c = None);
                    self.called_locked.iter_mut().for_each(|c| *c = None);
                    if cmd == "faint" {
                        if self.opp_subject(subj) == self.opp_active {
                            self.opp_active = None;
                        }
                        if let Some(mech) = self.mech.as_mut() {
                            mech.opp_active_damaged = false;
                        }
                    } else if let Some((sp, lv, _)) =
                        crate::import::ProtocolTracker::parse_details(
                            dex,
                            parts.get(2).copied().unwrap_or(""),
                        )
                    {
                        if let Some(slot) = self
                            .mons
                            .iter()
                            .position(|m| m.species == sp && m.level == lv)
                        {
                            self.opp_active = Some(slot);
                            if let Some((_, name)) = subj.split_once(": ") {
                                self.mons[slot].name = name.to_string();
                            }
                            if let Some(mech) = self.mech.as_mut() {
                                mech.opp_active_switched_mid_turn = true;
                                mech.opp_active_damaged =
                                    Self::opp_hp_is_damaged(parts.get(3).copied().unwrap_or(""))
                                        .unwrap_or(false);
                                // Berserk Gene activates immediately on switch-in; if no
                                // -enditem Berserk Gene arrives before the next major command,
                                // exclude Berserk Gene.
                                mech.pending_status_berry = Some((slot, ITEM_BERSERK_GENE));
                            }
                            if !self.mons[slot].appeared {
                                self.mons[slot].appeared = true;
                                dirty = true;
                            }
                        }
                    }
                } else if self.subject_is_own_side(subj) {
                    if let Some(mech) = self.mech.as_mut() {
                        mech.own_type_dirty = false;
                        mech.foresight_on_own = false;
                        if cmd == "faint" {
                            mech.own_active_species = None;
                        } else if let Some((sp, _, _)) =
                            crate::import::ProtocolTracker::parse_details(
                                dex,
                                parts.get(2).copied().unwrap_or(""),
                            )
                        {
                            mech.own_active_species = Some(sp);
                        }
                    }
                }
                dirty
            }
            "turn" => {
                if let Some(mech) = self.mech.as_mut() {
                    if mech.opp_active_damaged
                        && !mech.opp_active_switched_mid_turn
                        && !mech.opp_saw_leftovers_this_turn
                    {
                        if let Some(act) = self.opp_active {
                            if let Some(mo) = self.mons.get_mut(act) {
                                if !mo.item.gained && (mo.excluded_items & ITEM_LEFTOVERS) == 0 {
                                    mo.excluded_items |= ITEM_LEFTOVERS;
                                    dirty = true;
                                }
                            }
                        }
                    }
                    mech.opp_active_switched_mid_turn = false;
                    mech.opp_saw_leftovers_this_turn = false;
                }
                dirty
            }
            "-transform" => {
                let subj = parts.get(1).copied().unwrap_or("");
                if let Some(m) = self.opp_subject(subj) {
                    self.log_suppress[m] = true;
                } else if self.subject_is_own_side(subj) {
                    if let Some(mech) = self.mech.as_mut() {
                        mech.own_type_dirty = true;
                    }
                }
                dirty
            }
            "-start" => {
                let subj = parts.get(1).copied().unwrap_or("");
                let what = parts.get(2).copied().unwrap_or("");
                if self.subject_is_own_side(subj) {
                    if let Some(mech) = self.mech.as_mut() {
                        if what.starts_with("typechange") {
                            mech.own_type_dirty = true;
                        } else if what == "Foresight" || what == "move: Foresight" {
                            mech.foresight_on_own = true;
                        }
                    }
                } else if let Some(m) = self.opp_subject(subj) {
                    if what == "confusion" {
                        if let Some(mech) = self.mech.as_mut() {
                            mech.pending_status_berry =
                                Some((m, ITEM_BITTER_BERRY | ITEM_MIRACLE_BERRY));
                        }
                    }
                }
                dirty
            }
            "-status" => {
                let subj = parts.get(1).copied().unwrap_or("");
                let st = parts.get(2).copied().unwrap_or("");
                if let Some(m) = self.opp_subject(subj) {
                    if let Some(mech) = self.mech.as_mut() {
                        let mask = match st {
                            "slp" => ITEM_MINT_BERRY | ITEM_MIRACLE_BERRY,
                            "par" => ITEM_PRZ_CURE_BERRY | ITEM_MIRACLE_BERRY,
                            "psn" | "tox" => ITEM_PSN_CURE_BERRY | ITEM_MIRACLE_BERRY,
                            "brn" => ITEM_ICE_BERRY | ITEM_MIRACLE_BERRY,
                            "frz" => ITEM_BURNT_BERRY | ITEM_MIRACLE_BERRY,
                            _ => 0,
                        };
                        if mask != 0 {
                            mech.pending_status_berry = Some((m, mask));
                        }
                    }
                }
                dirty
            }
            "-damage" => {
                let subj = parts.get(1).copied().unwrap_or("");
                if self.subject_is_own_side(subj) {
                    let has_from = parts.iter().any(|p| p.starts_with("[from] "));
                    if !has_from {
                        if let Some(mech) = self.mech.as_mut() {
                            if let Some(hit) = mech.pending_hp.as_mut() {
                                hit.saw_direct_damage = true;
                            }
                        }
                    }
                } else if self.subject_is_opp_side(subj) {
                    if let Some(mech) = self.mech.as_mut() {
                        if let Some(dmg) = Self::opp_hp_is_damaged(parts.get(2).copied().unwrap_or("")) {
                            mech.opp_active_damaged = dmg;
                        }
                    }
                }
                dirty
            }
            "-end" => {
                let subj = parts.get(1).copied().unwrap_or("");
                let what = parts.get(2).copied().unwrap_or("");
                if self.subject_is_own_side(subj) && what == "Substitute" {
                    if let Some(mech) = self.mech.as_mut() {
                        if let Some(hit) = mech.pending_hp.as_mut() {
                            hit.sub_hit = true;
                        }
                    }
                }
                dirty
            }
            "-immune" | "-resisted" | "-supereffective" => {
                let subj = parts.get(1).copied().unwrap_or("");
                if self.subject_is_own_side(subj) {
                    if let Some(mech) = self.mech.as_mut() {
                        if let Some(hit) = mech.pending_hp.as_mut() {
                            hit.eff = Some(match cmd {
                                "-immune" => crate::hp_evidence::Effectiveness::Immune,
                                "-resisted" => crate::hp_evidence::Effectiveness::Resisted,
                                _ => crate::hp_evidence::Effectiveness::SuperEffective,
                            });
                        }
                    }
                }
                dirty
            }
            "-activate" => {
                let subj = parts.get(1).copied().unwrap_or("");
                let what = parts.get(2).copied().unwrap_or("");
                if self.subject_is_own_side(subj) && what == "Substitute" {
                    if let Some(mech) = self.mech.as_mut() {
                        if let Some(hit) = mech.pending_hp.as_mut() {
                            hit.sub_hit = true;
                        }
                    }
                    return dirty;
                }
                let Some(m) = self.opp_subject(subj) else {
                    return dirty;
                };
                if what == "move: Mimic" {
                    // PS names the copied move on this line, so exactly one
                    // slot becomes foreign. Suppressing the whole mon instead
                    // would drop every later use of its other three slots --
                    // the design's class-A example, "ignoring an already-
                    // revealed move". Fall back to whole-mon suppression only
                    // if the name does not resolve.
                    match parts.get(3).and_then(|n| dex.moves.id(&toid(n))) {
                        Some(copied) => self.mimic_overlay[m] = Some(copied),
                        None => self.log_suppress[m] = true,
                    }
                    return dirty;
                }
                if let Some(item_name) = what.strip_prefix("item: ") {
                    if let Some(id) = dex.items.id(&toid(item_name)) {
                        return dirty | self.mons[m].item.revealed_held(id);
                    }
                }
                dirty
            }
            "-enditem" => {
                let Some(m) = self.opp_subject(parts.get(1).copied().unwrap_or("")) else {
                    return dirty;
                };
                if let Some(mech) = self.mech.as_mut() {
                    if mech.pending_status_berry.is_some_and(|(s, _)| s == m) {
                        mech.pending_status_berry = None;
                    }
                }
                match dex.items.id(&toid(parts.get(2).copied().unwrap_or(""))) {
                    Some(id) => dirty | self.mons[m].item.lost(id),
                    None => dirty,
                }
            }
            "-item" => {
                let subject = parts.get(1).copied().unwrap_or("");
                let Some(id) = dex.items.id(&toid(parts.get(2).copied().unwrap_or(""))) else {
                    return dirty;
                };
                if let Some(m) = self.opp_subject(subject) {
                    // opponent gained an item (their Thief stole ours)
                    return dirty | self.mons[m].item.gained(id);
                }
                // our Thief stole theirs: `-item <us> <item> [from] move: Thief [of] <them>`
                if parts.iter().any(|p| *p == "[from] move: Thief") {
                    if let Some(of) = parts.iter().find_map(|p| p.strip_prefix("[of] ")) {
                        if let Some(v) = self.opp_subject(of) {
                            return dirty | self.mons[v].item.lost(id);
                        }
                    }
                }
                dirty
            }
            "-heal" => {
                let subj = parts.get(1).copied().unwrap_or("");
                let Some(m) = self.opp_subject(subj) else {
                    return dirty;
                };
                if let Some(mech) = self.mech.as_mut() {
                    if let Some(dmg) = Self::opp_hp_is_damaged(parts.get(2).copied().unwrap_or("")) {
                        mech.opp_active_damaged = dmg;
                    }
                }
                if let Some(item_name) =
                    parts.iter().find_map(|p| p.strip_prefix("[from] item: "))
                {
                    if let Some(id) = dex.items.id(&toid(item_name)) {
                        if dex.items.key(id) == "leftovers" {
                            if let Some(mech) = self.mech.as_mut() {
                                mech.opp_saw_leftovers_this_turn = true;
                            }
                        }
                        return dirty | self.mons[m].item.revealed_held(id);
                    }
                }
                dirty
            }
            _ => dirty,
        }
    }

    fn scan_move(&mut self, parts: &[&str], dex: &Dex) -> bool {
        let subj = parts.get(1).copied().unwrap_or("");
        if self.subject_is_own_side(subj) {
            if let Some(mech) = self.mech.as_mut() {
                let named = toid(parts.get(2).copied().unwrap_or(""));
                if matches!(named.as_str(), "conversion" | "conversion2" | "transform") {
                    mech.own_type_dirty = true;
                }
            }
            return false;
        }
        let Some(m) = self.opp_subject(subj) else {
            return false;
        };
        if self.log_suppress[m] {
            // Transform copy active: plain |move| lines (and even Sleep Talk
            // calls) may name any move of the copied set.
            return false;
        }
        let named = parts.get(2).copied().unwrap_or("");
        let Some(id) = dex.moves.id(&toid(named)) else { return false };
        if self.mimic_overlay[m] == Some(id) {
            // the one slot Mimic overwrote. Sleep Talk selects from the
            // CURRENT slots, so this must be checked before the `[from]`
            // rules, not only for plain lines.
            return false;
        }
        let from = parts
            .iter()
            .find_map(|p| p.strip_prefix("[from] "))
            .map(|f| f.strip_prefix("move: ").unwrap_or(f));
        if let Some(from) = from {
            let from_id = dex.moves.id(&toid(from));
            // `[from]` naming the executing move itself is not a call at all:
            // Pursuit's interception of a switching target logs
            // `|move|p2a: X|Pursuit|p1a: Y|[from] Pursuit`, and that is the mon
            // publicly using its own slot. Sleep Talk is the other exception —
            // it selects from the user's own set. Everything else (Metronome,
            // Mirror Move) executes a FOREIGN move.
            let self_named = from_id.is_some_and(|f| move_matches(dex, id, f));
            let sleep_talk = toid(from) == "sleeptalk";
            if !self_named && !sleep_talk {
                // A called TWO-TURN move announces its charge here and then
                // releases on a plain `|move|` line that carries no `[from]`.
                // Remember it so the release is not read as the mon's own.
                if parts.iter().any(|p| *p == "[still]") {
                    self.pending_call = Some((m, id));
                }
                if matches!(
                    dex.moves.key(id),
                    "outrage" | "petaldance" | "thrash" | "rollout" | "bide"
                ) {
                    self.called_locked[m] = Some(id);
                }
                return false;
            }
        }
        if from.is_none() && self.called_charging[m] == Some(id) {
            // the release of the charge a caller started: not a submitted slot
            self.called_charging[m] = None;
            return false;
        }
        if from.is_none() && self.called_locked[m] == Some(id) {
            // continuation turn of a caller-initiated multi-turn lock
            return false;
        }
        // synthetic non-set moves
        if matches!(dex.moves.key(id), "struggle" | "recharge") {
            return false;
        }
        if from.is_none() {
            self.called_locked[m] = None;
        }
        let mut dirty = false;
        if !self.mons[m].revealed_moves.contains(&id) {
            self.mons[m].revealed_moves.push(id);
            dirty = true;
        }
        if self.mech.is_some() && dex.moves.key(id) == "hiddenpower" {
            let mo = &mut self.mons[m];
            let gender_mask = crate::hp_evidence::possible_types(dex, mo.species, mo.gender);
            let next = mo.hp_type_mask & gender_mask;
            if next != 0 && next != mo.hp_type_mask {
                mo.hp_type_mask = next;
                dirty = true;
            }
            if let Some(mech) = self.mech.as_mut() {
                if let (Some(own_sp), false) = (mech.own_active_species, mech.own_type_dirty) {
                    mech.pending_hp = Some(PendingHpHit {
                        opp_slot: m,
                        own_species: own_sp,
                        saw_direct_damage: false,
                        sub_hit: false,
                        eff: None,
                    });
                }
            }
        }
        dirty
    }

    // -------------------------------------------------- subject resolution

    /// `"p2a: Nick"` / `"p2: Nick"` → opponent roster slot (active slot wins
    /// on `p2a:` subjects when multiple roster mons share a nickname;
    /// non-active ambiguity or foreign side → `None`, i.e. skip = conservative).
    fn opp_subject(&self, s: &str) -> Option<usize> {
        if !self.subject_is_opp_side(s) {
            return None;
        }
        let rest = &s[2..];
        let has_pos = rest.starts_with(|c: char| c.is_ascii_lowercase());
        let rest = rest.strip_prefix(|c: char| c.is_ascii_lowercase()).unwrap_or(rest);
        let name = rest.strip_prefix(": ")?;
        if has_pos {
            if let Some(act) = self.opp_active {
                if self.mons.get(act).is_some_and(|m| m.name == name) {
                    return Some(act);
                }
            }
        }
        let mut found = None;
        for (i, m) in self.mons.iter().enumerate() {
            if m.name == name {
                if found.is_some() {
                    return None;
                }
                found = Some(i);
            }
        }
        found
    }

    fn subject_is_opp_side(&self, s: &str) -> bool {
        let b = s.as_bytes();
        b.len() >= 4 && b[0] == b'p' && b[1] == b'1' + self.opp() as u8
    }

    fn subject_is_own_side(&self, s: &str) -> bool {
        let b = s.as_bytes();
        b.len() >= 4 && b[0] == b'p' && b[1] == b'1' + self.side as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use conformance::load_dex;

    fn one_mon_observer(dex: &Dex, species: &str, name: &str) -> Observer {
        let mons = vec![MonObs {
            species: dex.species.id(&toid(species)).expect("species"),
            level: 50,
            gender: Gender::N,
            name: name.to_string(),
            preview_has_item: false,
            revealed_moves: Vec::new(),
            hp_type_mask: u16::MAX,
            item: Default::default(),
            excluded_items: 0,
            max_spe_stat: u16::MAX,
            appeared: true,
        }];
        // side 0 observes side 1 (`p2a:` subjects).
        Observer::from_mons(0, mons)
    }

    fn revealed(obs: &Observer, dex: &Dex) -> Vec<String> {
        obs.mons[0]
            .revealed_moves
            .iter()
            .map(|&id| dex.moves.key(id).to_string())
            .collect()
    }

    #[test]
    fn hp_effectiveness_inference_is_disabled_by_default() {
        let dex = load_dex();
        let mut obs = one_mon_observer(&dex, "Aerodactyl", "A");
        for line in ["|switch|p1a: N|Snorlax, L50|100/100",
            "|move|p2a: A|Hidden Power|p1a: N", "|-supereffective|p1a: N"] {
            obs.ingest_line(line, &dex);
        }
        assert_eq!(obs.mons[0].hp_type_mask, u16::MAX);
    }

    /// A caller charging a two-turn move must not credit the victim move to
    /// the mon's own set. Corpus battle 215 is the observed case: p1 Snorlax
    /// ended with FIVE revealed moves for four slots (substitute, doubleteam,
    /// attract, metronome, razorwind) because Metronome's Razor Wind released
    /// on a plain `|move|` line. Five facts for four slots is unsatisfiable,
    /// so `Belief::sync_checked` fails closed on the pinned/open-sheet path —
    /// the shipped product's own configuration.
    #[test]
    fn called_two_turn_release_is_not_a_reveal() {
        let dex = load_dex();
        let mut obs = one_mon_observer(&dex, "Snorlax", "Snorlax");
        for line in [
            "|move|p2a: Snorlax|Metronome|p2a: Snorlax",
            "|move|p2a: Snorlax|Razor Wind||[from] Metronome|[still]",
            "|-prepare|p2a: Snorlax|Razor Wind",
            "|move|p2a: Snorlax|Razor Wind|p1a: Target",
        ] {
            obs.ingest_line(line, &dex);
        }
        assert_eq!(revealed(&obs, &dex), vec!["metronome".to_string()]);
    }

    /// `[from]` naming the executing move itself is a genuine reveal, not a
    /// call. Pursuit's interception of a switching target is the case that
    /// occurs in the corpus (4 lines, all Umbreon): dropping it means the bot
    /// never learns a move it publicly watched land, which is the design's own
    /// class-A example — "ignoring an already-revealed move".
    #[test]
    fn a_self_named_from_tag_is_a_reveal() {
        let dex = load_dex();
        let mut obs = one_mon_observer(&dex, "Umbreon", "Umbreon");
        obs.ingest_line("|move|p2a: Umbreon|Pursuit|p1a: Rapidash|[from] Pursuit", &dex);
        assert_eq!(revealed(&obs, &dex), vec!["pursuit".to_string()]);
    }

    /// The rule must not eat a genuine reveal: the same move used normally on
    /// a later turn is the mon's own.
    #[test]
    fn an_uncalled_two_turn_move_still_reveals() {
        let dex = load_dex();
        let mut obs = one_mon_observer(&dex, "Snorlax", "Snorlax");
        for line in [
            "|move|p2a: Snorlax|Razor Wind||[still]",
            "|-prepare|p2a: Snorlax|Razor Wind",
            "|move|p2a: Snorlax|Razor Wind|p1a: Target",
        ] {
            obs.ingest_line(line, &dex);
        }
        assert_eq!(revealed(&obs, &dex), vec!["razorwind".to_string()]);
    }

    /// Mimic overwrites exactly ONE slot, and PS names the copied move on
    /// the `-activate` line. Suppressing the whole mon instead loses every
    /// later use of its other three slots — an under-reveal of moves the bot
    /// publicly watched land. The copied move itself stays uncredited, and so
    /// does a Sleep Talk call that selects it (Sleep Talk picks from the
    /// CURRENT slots, which include the overwritten one).
    #[test]
    fn mimic_suppresses_only_the_copied_slot() {
        let dex = load_dex();
        let mut obs = one_mon_observer(&dex, "Zapdos", "Zapdos");
        for line in [
            "|move|p2a: Zapdos|Mimic|p1a: Snorlax",
            "|-activate|p2a: Zapdos|move: Mimic|Body Slam",
            // the copied slot: not the mon's own, by either route
            "|move|p2a: Zapdos|Body Slam|p1a: Snorlax",
            "|move|p2a: Zapdos|Body Slam|p1a: Snorlax|[from] Sleep Talk",
            // its own remaining slots stay reveals
            "|move|p2a: Zapdos|Thunder|p1a: Snorlax",
            "|move|p2a: Zapdos|Rest|p2a: Zapdos",
        ] {
            obs.ingest_line(line, &dex);
        }
        assert_eq!(
            revealed(&obs, &dex),
            vec!["mimic".to_string(), "thunder".to_string(), "rest".to_string()]
        );
    }

    /// `clearVolatile` puts the Mimic slot back on switch-out, so the copied
    /// move must stop being suppressed — otherwise a genuine later use by a
    /// DIFFERENT mon of the same name, or by this one after it really learns
    /// it, would be dropped forever.
    #[test]
    fn a_mimic_overlay_dies_with_the_mons_stay() {
        let dex = load_dex();
        let mut obs = one_mon_observer(&dex, "Zapdos", "Zapdos");
        for line in [
            "|move|p2a: Zapdos|Mimic|p1a: Snorlax",
            "|-activate|p2a: Zapdos|move: Mimic|Body Slam",
            "|switch|p2a: Zapdos|Zapdos, L50|100/100",
            "|move|p2a: Zapdos|Body Slam|p1a: Snorlax",
        ] {
            obs.ingest_line(line, &dex);
        }
        assert_eq!(
            revealed(&obs, &dex),
            vec!["mimic".to_string(), "bodyslam".to_string()]
        );
    }

    /// Transform stays whole-mon suppression: the copy replaces all four
    /// slots, so no `|move|` line from it is evidence about the base set.
    #[test]
    fn transform_still_suppresses_every_slot() {
        let dex = load_dex();
        let mut obs = one_mon_observer(&dex, "Ditto", "Ditto");
        for line in [
            "|move|p2a: Ditto|Transform|p1a: Snorlax",
            "|-transform|p2a: Ditto|p1a: Snorlax",
            "|move|p2a: Ditto|Body Slam|p1a: Snorlax",
            "|move|p2a: Ditto|Rest|p2a: Ditto",
        ] {
            obs.ingest_line(line, &dex);
        }
        assert_eq!(revealed(&obs, &dex), vec!["transform".to_string()]);
    }

    /// An interrupted charge releases the tracking, so a later genuine use of
    /// the same move is still credited.
    #[test]
    fn an_interrupted_charge_does_not_suppress_forever() {
        let dex = load_dex();
        let mut obs = one_mon_observer(&dex, "Snorlax", "Snorlax");
        for line in [
            "|move|p2a: Snorlax|Razor Wind||[from] Metronome|[still]",
            "|-prepare|p2a: Snorlax|Razor Wind",
            "|cant|p2a: Snorlax|par",
            "|move|p2a: Snorlax|Razor Wind|p1a: Target",
        ] {
            obs.ingest_line(line, &dex);
        }
        assert_eq!(revealed(&obs, &dex), vec!["razorwind".to_string()]);
    }

    #[test]
    fn a_called_multiturn_locked_move_does_not_reveal_slot() {
        let dex = load_dex();
        let mut obs = one_mon_observer(&dex, "Granbull", "Granbull");
        for line in [
            "|move|p2a: Granbull|Metronome|p2a: Granbull",
            "|move|p2a: Granbull|Rollout|p1a: Electrode|[from] move: Metronome",
            "|move|p2a: Granbull|Rollout|p1a: Electrode",
            "|move|p2a: Granbull|Rollout|p1a: Electrode",
            "|move|p2a: Granbull|Mud-Slap|p1a: Electrode",
        ] {
            obs.ingest_line(line, &dex);
        }
        assert_eq!(
            revealed(&obs, &dex),
            vec!["metronome".to_string(), "mudslap".to_string()]
        );
    }

    #[test]
    fn duplicate_nicknames_resolve_active_slot_chronologically() {
        let dex = load_dex();
        let mons = vec![
            MonObs {
                species: dex.species.id("snorlax").unwrap(),
                level: 50,
                gender: Gender::M,
                name: String::new(),
                preview_has_item: true,
                revealed_moves: Vec::new(),
                hp_type_mask: u16::MAX,
                item: Default::default(),
                excluded_items: 0,
                max_spe_stat: u16::MAX,
                appeared: false,
            },
            MonObs {
                species: dex.species.id("zapdos").unwrap(),
                level: 50,
                gender: Gender::N,
                name: String::new(),
                preview_has_item: true,
                revealed_moves: Vec::new(),
                hp_type_mask: u16::MAX,
                item: Default::default(),
                excluded_items: 0,
                max_spe_stat: u16::MAX,
                appeared: false,
            },
        ];
        let mut obs = Observer::from_mons(0, mons);
        for line in [
            "|switch|p2a: Ace|Snorlax, L50, M|100/100",
            "|move|p2a: Ace|Body Slam|p1a: Target",
            "|switch|p2a: Ace|Zapdos, L50|100/100",
            "|move|p2a: Ace|Thunder|p1a: Target",
        ] {
            obs.ingest_line(line, &dex);
        }
        assert_eq!(
            obs.mons[0]
                .revealed_moves
                .iter()
                .map(|&id| dex.moves.key(id))
                .collect::<Vec<_>>(),
            vec!["bodyslam"]
        );
        assert_eq!(
            obs.mons[1]
                .revealed_moves
                .iter()
                .map(|&id| dex.moves.key(id))
                .collect::<Vec<_>>(),
            vec!["thunder"]
        );
    }

    #[test]
    fn mechanical_inference_narrows_hp_type_and_excludes_passive_items() {
        let dex = load_dex();
        let mut obs = one_mon_observer(&dex, "Snorlax", "Snorlax");
        obs.mons[0].gender = Gender::F;
        obs.set_mechanical_inference(true);

        // Female Snorlax (12.5% F -> Atk DV in {0, 1}) switches in without Berserk Gene triggering
        for line in [
            "|switch|p1a: Skarmory|Skarmory, L50, M|100/100",
            "|switch|p2a: Snorlax|Snorlax, L50, F|100/100",
            "|turn|1",
            "|move|p2a: Snorlax|Hidden Power|p1a: Skarmory",
            "|-supereffective|p1a: Skarmory",
            "|-damage|p1a: Skarmory|60/100",
            "|-damage|p2a: Snorlax|80/100",
            "|-status|p2a: Snorlax|par",
            "|turn|2",
        ] {
            obs.ingest_line(line, &dex);
        }

        // Berserk Gene (on switch-in), PRZ Cure Berry & Miracle Berry (on par), and Leftovers (damaged at turn boundary) must be excluded
        assert_ne!(obs.mons[0].excluded_items & ITEM_BERSERK_GENE, 0);
        assert_ne!(obs.mons[0].excluded_items & ITEM_PRZ_CURE_BERRY, 0);
        assert_ne!(obs.mons[0].excluded_items & ITEM_MIRACLE_BERRY, 0);
        assert_ne!(obs.mons[0].excluded_items & ITEM_LEFTOVERS, 0);

        // Female Snorlax can only have Atk DV 0 or 1 -> HP types 0..8 (Fighting..Steel).
        // Super-effective against Skarmory (Steel/Flying) requires Fire or Electric (indices 8 or 11),
        // wait: female Snorlax cannot have Fire/Electric! So the intersection with gender_mask would be 0,
        // meaning fail-open preserves gender_mask (non-zero)!
        let gender_mask = crate::hp_evidence::possible_types(&dex, obs.mons[0].species, Gender::F);
        assert_eq!(obs.mons[0].hp_type_mask, gender_mask);

        // Now test male Zapdos using Hidden Power super-effective against Rhydon (Ground/Rock)
        let mut obs2 = one_mon_observer(&dex, "Zapdos", "Zapdos");
        obs2.set_mechanical_inference(true);
        for line in [
            "|switch|p1a: Rhydon|Rhydon, L50, M|100/100",
            "|switch|p2a: Zapdos|Zapdos, L50|100/100",
            "|move|p2a: Zapdos|Hidden Power|p1a: Rhydon",
            "|-supereffective|p1a: Rhydon",
            "|-damage|p1a: Rhydon|40/100",
            "|turn|2",
        ] {
            obs2.ingest_line(line, &dex);
        }
        // Super-effective vs Ground/Rock: Fighting (0), Ground (3), Water (9), Grass (10), Ice (13), Steel (7)
        let water_idx = crate::hp_evidence::TYPES.iter().position(|&t| t == "Water").unwrap();
        let grass_idx = crate::hp_evidence::TYPES.iter().position(|&t| t == "Grass").unwrap();
        let fire_idx = crate::hp_evidence::TYPES.iter().position(|&t| t == "Fire").unwrap();
        assert_ne!(obs2.mons[0].hp_type_mask & (1 << water_idx), 0);
        assert_ne!(obs2.mons[0].hp_type_mask & (1 << grass_idx), 0);
        assert_eq!(obs2.mons[0].hp_type_mask & (1 << fire_idx), 0);

        // Verify Substitute hit does NOT infer Neutral effectiveness
        let mut obs3 = one_mon_observer(&dex, "Raikou", "Raikou");
        obs3.set_mechanical_inference(true);
        for line in [
            "|switch|p1a: Exeggutor|Exeggutor, L50, M|100/100",
            "|switch|p2a: Raikou|Raikou, L50|100/100",
            "|move|p2a: Raikou|Hidden Power|p1a: Exeggutor",
            "|-activate|p1a: Exeggutor|Substitute|[damage]",
            "|turn|2",
        ] {
            obs3.ingest_line(line, &dex);
        }
        assert_eq!(obs3.mons[0].hp_type_mask, u16::MAX);
    }
}
