//! Leaf settlement modes for MCTS playout cutoffs and leaf evaluation.

/// How MCTS tree leaves settle rollout playouts and cutoff board evaluation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LeafSettlementMode {
    /// Legacy 8-turn `eps = 0.2` rollout + `eval::compact_eval_leaf`.
    #[default]
    LegacyRollout,
    /// Shipped default: Setup-Relay simultaneous-move Bellman leaf lift + equilibrium-gated
    /// rollout continuation on top of `eval::compact_eval_leaf_blindspot`.
    BlindSpotLeaf,
    /// Ablation baseline without the Setup-Relay Bellman solver (`-relay`): legacy 8-turn
    /// `eps = 0.2` rollout + static `eval::compact_eval_leaf_blindspot_norelay`.
    BlindSpotLeafNoRelay,
    /// Surgical sign-error-guarded 8-turn `eps = 0.2` rollout + `eval::compact_eval_leaf`.
    BlindSpotRollout,
    /// Best-of-both-worlds ("いいとこ取り"): surgical sign-error-guarded 8-turn `CompactBattle`
    /// rollout + `eval::compact_eval_leaf_blindspot_norelay`.
    BlindSpotBoth,
}

impl LeafSettlementMode {
    #[inline]
    pub const fn uses_blindspot_leaf(self) -> bool {
        matches!(
            self,
            Self::BlindSpotLeaf | Self::BlindSpotLeafNoRelay | Self::BlindSpotBoth
        )
    }

    #[inline]
    pub const fn uses_relay_bellman(self) -> bool {
        matches!(self, Self::BlindSpotLeaf)
    }

    #[inline]
    pub const fn uses_blindspot_rollout(self) -> bool {
        matches!(self, Self::BlindSpotRollout | Self::BlindSpotBoth)
    }
}
