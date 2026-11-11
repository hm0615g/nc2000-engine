//! Trusted local-player adapter. Search receives a reconstruction, never a
//! clone of the real game's hidden runtime state.
use crate::import::{ProtocolTracker, Request};
use crate::player::PlayerChannel;
use crate::{Belief, Observer, SplitMix64};
use nc2000_engine::{battle::PokemonSet, dex::Dex, state::Battle};

/// A root synthesized from a player's observations. There is deliberately no
/// public constructor taking a real Battle.
/// ```compile_fail
/// use nc2000_bot::player_state::ObservedPosition;
/// fn smuggle(b: nc2000_engine::state::Battle) -> ObservedPosition {
///     ObservedPosition { battle: b }
/// }
/// ```
pub struct ObservedPosition {
    battle: Battle,
}
impl ObservedPosition {
    pub(crate) fn reconstructed(battle: Battle) -> Self {
        Self { battle }
    }
    /// Read-only diagnostics of the synthesized world, not the real battle.
    pub fn model(&self) -> &Battle {
        &self.battle
    }

    /// Counterfactual world for policy-response research. The source is itself
    /// a player reconstruction, so no real hidden state can enter this path.
    pub fn sample(
        &self,
        dex: &Dex,
        belief: &Belief,
        observer: &Observer,
        rng: &mut SplitMix64,
    ) -> Self {
        Self::reconstructed(belief.determinize(dex, &self.battle, observer, rng))
    }
}

/// Trusted adapter for native callers and the legacy WASM facade. Live Web
/// workers instead use the same protocol directly, without receiving truth.
pub struct LocalPlayer {
    channel: PlayerChannel,
    tracker: ProtocolTracker,
    own_sets: Vec<PokemonSet>,
    request: Request,
}

impl LocalPlayer {
    pub fn new(battle: &Battle, dex: &Dex, side: usize) -> Result<(Self, Observer), String> {
        if !battle.log_enabled {
            return Err("blind play requires player-visible protocol history".into());
        }
        let own_sets = own_team(battle, dex, side);
        let mut channel = PlayerChannel::new(side);
        let frame = channel.frame(&mut battle.clone(), dex)?;
        let mut tracker = ProtocolTracker::new(side);
        for line in &frame.lines {
            tracker.push_line(dex, line);
        }
        let mut observer = Observer::from_mons(side, tracker.observer_mons());
        observer.set_mechanical_inference(true);
        for (slot, name) in tracker.names(1 - side).iter().enumerate() {
            if !name.is_empty() {
                observer.set_name(slot, name);
            }
        }
        for line in &frame.lines {
            observer.ingest_line(line, dex);
        }
        let request = Request::parse(dex, &frame.request.to_string())?;
        Ok((
            Self {
                channel,
                tracker,
                own_sets,
                request,
            },
            observer,
        ))
    }

    pub fn observe(
        &mut self,
        battle: &Battle,
        dex: &Dex,
        observer: &mut Observer,
    ) -> Result<(), String> {
        let frame = self.channel.frame(&mut battle.clone(), dex)?;
        for line in &frame.lines {
            self.tracker.push_line(dex, line);
            for (slot, name) in self.tracker.names(observer.opp()).iter().enumerate() {
                if !name.is_empty() {
                    observer.set_name(slot, name);
                }
            }
            observer.ingest_line(line, dex);
        }
        self.request = Request::parse(dex, &frame.request.to_string())?;
        Ok(())
    }

    pub fn position(
        &self,
        dex: &Dex,
        belief: &mut Belief,
        observer: &Observer,
        rng: &mut SplitMix64,
    ) -> Result<ObservedPosition, String> {
        self.condition_belief(belief, observer);
        let pick = belief.alive().first().copied();
        self.tracker
            .synthesize(
                dex,
                &self.own_sets,
                belief.refs(pick),
                observer,
                &self.request,
                rng,
            )
            .map(ObservedPosition::reconstructed)
    }

    /// Install only the observed opponent HP buckets; no own hidden set or
    /// runtime state is exported to an opponent-policy model.
    pub fn condition_belief(&self, belief: &mut Belief, observer: &Observer) {
        self.tracker.set_hp_observations(belief, observer.opp());
    }
}

/// Export only the observing player's canonical submitted set. In particular,
/// do not copy current stats/Transform overlays or hidden condition counters.
pub fn own_team(battle: &Battle, dex: &Dex, side: usize) -> Vec<PokemonSet> {
    battle.sides[side]
        .roster
        .iter()
        .map(|p| PokemonSet {
            name: p.name.as_str().to_string(),
            species: dex.species.key(p.base_species).into(),
            item: p.item.map(|i| dex.items.key(i)).unwrap_or("").into(),
            ability: "No Ability".into(),
            moves: p
                .base_move_slots
                .iter()
                .map(|m| dex.moves.key(m.id).to_string())
                .collect(),
            level: p.level,
            happiness: Some(p.happiness),
            gender: Some(p.gender.as_str().into()),
            ivs: Some(
                ["hp", "atk", "def", "spa", "spd", "spe"]
                    .iter()
                    .zip(p.set_ivs)
                    .map(|(k, v)| ((*k).into(), v as u8))
                    .collect(),
            ),
            evs: Some(
                ["hp", "atk", "def", "spa", "spd", "spe"]
                    .iter()
                    .zip(p.set_evs)
                    .map(|(k, v)| ((*k).into(), v as u16))
                    .collect(),
            ),
        })
        .collect()
}
