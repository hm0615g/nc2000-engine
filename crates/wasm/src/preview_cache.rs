//! Storage-independent identity for completed, blind team-preview decisions.
use std::cell::RefCell;
use std::rc::Rc;

use nc2000_bot::{Observer, RmConfig};
use nc2000_engine::state::Battle;
use serde_json::{json, Value};

thread_local! {
    static CACHED_CANONICAL_POOL: RefCell<Option<(String, Rc<str>)>> = const { RefCell::new(None) };
}

fn canonicalize_pool_json(pool_json: &str) -> Rc<str> {
    CACHED_CANONICAL_POOL.with(|slot| {
        if let Some((ref raw, ref canon)) = *slot.borrow() {
            if raw == pool_json {
                return Rc::clone(canon);
            }
        }
        let parsed: Value = serde_json::from_str(pool_json).expect("validated pool JSON");
        let canon: Rc<str> = Rc::from(parsed.to_string());
        *slot.borrow_mut() = Some((pool_json.to_owned(), Rc::clone(&canon)));
        canon
    })
}

fn opponent_public_json(observer: &Observer) -> String {
    let public: Vec<_> = observer
        .mons()
        .iter()
        .map(|m| {
            json!([
                m.species.0,
                m.level,
                format!("{:?}", m.gender),
                m.preview_has_item,
                m.hp_type_mask,
                m.revealed_moves.iter().map(|x| x.0).collect::<Vec<_>>(),
                format!("{:?}", m.item),
                m.appeared,
            ])
        })
        .collect();
    serde_json::to_string(&public).expect("valid opponentPublic JSON")
}

pub(crate) struct Identity {
    driver: &'static str,
    config: String,
    pool_canonical: Rc<str>,
    prior_canonical: String,
    pick_prior_canonical: Option<String>,
}

impl Identity {
    pub fn new(driver: &'static str, cfg: &RmConfig, pool_json: &str) -> Self {
        Self {
            driver,
            config: format!("{cfg:?}"),
            // The constructor has already parsed and validated this JSON.
            pool_canonical: canonicalize_pool_json(pool_json),
            prior_canonical: "null".into(),
            pick_prior_canonical: None,
        }
    }

    pub fn set_prior(&mut self, json: &str) {
        let v: Value = serde_json::from_str(json).expect("applied prior JSON");
        self.prior_canonical = v.to_string();
    }

    pub fn set_pick_prior(&mut self, json: &str) {
        let v: Value = serde_json::from_str(json).expect("applied pick prior JSON");
        self.pick_prior_canonical = Some(v.to_string());
    }

    pub fn clear_pick_prior(&mut self) {
        self.pick_prior_canonical = None;
    }

    /// Lightweight public-only roster payload (`{"opponentPublic": ...}`) for
    /// compiled-preview table lookup without serializing the 1.8 MB `pool` JSON.
    pub fn public_info(&self, battle: &Battle, observer: &Observer) -> Option<String> {
        let _team_key = nc2000_bot::compiled_preview::key(battle, observer.side())?;
        let mut out = String::with_capacity(256);
        out.push_str("{\"opponentPublic\":");
        out.push_str(&opponent_public_json(observer));
        out.push('}');
        Some(out)
    }

    pub fn info(&self, battle: &Battle, observer: &Observer) -> Option<String> {
        let team_key = nc2000_bot::compiled_preview::key(battle, observer.side())?;
        let public_json = opponent_public_json(observer);
        // Exact lexicographical key order matching `serde_json::Map` (`BTreeMap`):
        // build, config, driver, format, opponentPublic, [pickPrior], pool, previewLevelCaps, prior, teamKey.
        let mut out = String::with_capacity(self.pool_canonical.len() + 512);
        out.push_str("{\"build\":");
        out.push_str(
            &serde_json::to_string(nc2000_bot::compiled_preview::build_fingerprint()).unwrap(),
        );
        out.push_str(",\"config\":");
        out.push_str(&serde_json::to_string(&self.config).unwrap());
        out.push_str(",\"driver\":");
        out.push_str(&serde_json::to_string(self.driver).unwrap());
        out.push_str(",\"format\":\"nc2000-preview-cache-info-v1\",\"opponentPublic\":");
        out.push_str(&public_json);
        if let Some(ref pick_prior) = self.pick_prior_canonical {
            out.push_str(",\"pickPrior\":");
            out.push_str(pick_prior);
        }
        out.push_str(",\"pool\":");
        out.push_str(&self.pool_canonical);
        out.push_str(",\"previewLevelCaps\":");
        out.push_str(&serde_json::to_string(&battle.preview_level_caps).unwrap());
        out.push_str(",\"prior\":");
        out.push_str(&self.prior_canonical);
        out.push_str(",\"teamKey\":");
        out.push_str(&serde_json::to_string(&team_key).unwrap());
        out.push('}');
        Some(out)
    }
}
