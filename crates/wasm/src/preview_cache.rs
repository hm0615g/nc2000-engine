//! Storage-independent identity for completed, blind team-preview decisions.
use nc2000_bot::{Observer, RmConfig};
use nc2000_engine::state::Battle;
use serde_json::{json, Value};

pub(crate) struct Identity {
    driver: &'static str,
    config: String,
    pool: Value,
    prior: Value,
    pick_prior: Option<Value>,
}

impl Identity {
    pub fn new(driver: &'static str, cfg: &RmConfig, pool_json: &str) -> Self {
        Self {
            driver,
            config: format!("{cfg:?}"),
            // The constructor has already parsed and validated this JSON.
            pool: serde_json::from_str(pool_json).expect("validated pool JSON"),
            prior: Value::Null,
            pick_prior: None,
        }
    }

    pub fn set_prior(&mut self, json: &str) {
        self.prior = serde_json::from_str(json).expect("applied prior JSON");
    }

    pub fn set_pick_prior(&mut self, json: &str) {
        self.pick_prior = Some(serde_json::from_str(json).expect("applied pick prior JSON"));
    }

    pub fn clear_pick_prior(&mut self) {
        self.pick_prior = None;
    }

    pub fn info(&self, battle: &Battle, observer: &Observer) -> Option<String> {
        let team_key = nc2000_bot::compiled_preview::key(battle, observer.side())?;
        let public: Vec<_> = observer.mons().iter().map(|m| json!([
            m.species.0, m.level, format!("{:?}", m.gender), m.preview_has_item,
            m.hp_type_mask, m.revealed_moves.iter().map(|x| x.0).collect::<Vec<_>>(),
            format!("{:?}", m.item), m.appeared,
        ])).collect();
        let mut obj = serde_json::Map::from_iter([
            ("format".into(), json!("nc2000-preview-cache-info-v1")),
            ("build".into(), json!(nc2000_bot::compiled_preview::build_fingerprint())),
            ("driver".into(), json!(self.driver)),
            ("config".into(), json!(self.config)),
            ("pool".into(), self.pool.clone()),
            ("prior".into(), self.prior.clone()),
            ("teamKey".into(), json!(team_key)),
            ("previewLevelCaps".into(), json!(battle.preview_level_caps)),
            ("opponentPublic".into(), json!(public)),
        ]);
        if let Some(ref pick_prior) = self.pick_prior {
            obj.insert("pickPrior".into(), pick_prior.clone());
        }
        Some(Value::Object(obj).to_string())
    }
}
