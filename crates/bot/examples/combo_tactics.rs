use conformance::load_dex;
use nc2000_bot::{
    blind::BlindSearch,
    smmcts::{RmConfig, SelRule},
    Belief, Observer,
};
use serde_json::json;

mod combo_certify;

#[path = "../tests/support/perish_deadline.rs"]
mod perish_deadline;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let arg = |key: &str, default: &str| {
        args.iter()
            .position(|s| s == key)
            .map(|i| args[i + 1].as_str())
            .unwrap_or(default)
            .to_string()
    };
    let seed: u64 = arg("--seed", "102001").parse().unwrap();
    let seeds: u64 = arg("--seeds", "4").parse().unwrap();
    let iterations: u32 = arg("--iters", "30000").parse().unwrap();
    let dex = load_dex();
    let dex = &dex;
    let wait = if args.iter().any(|s| s == "--snore") {
        perish_deadline::WaitingMove::Snore
    } else {
        perish_deadline::WaitingMove::Return
    };
    let (preview, b) = perish_deadline::position(dex, wait);
    let mut obs = Observer::new(&preview, 0);
    let mut belief = Belief::pinned_from_battle(&preview, &obs);
    obs.observe(&b, dex);
    belief.sync(dex, &obs);
    let states:Vec<_>=(0..2).map(|s|{
        let p=b.poke(b.active_id(s).unwrap());
        json!({"hp":p.hp,"species":dex.species.key(p.species),"trapped":p.trapped,
            "perish":dex.conds_id("perishsong").and_then(|c|p.volatile(c)).and_then(|v|v.duration),
            "moves":p.move_slots.iter().map(|m|json!({"move":dex.moves.key(m.id),"pp":m.pp})).collect::<Vec<_>>()})
    }).collect();
    println!("{}", json!({"case":wait.key(),"state":states,"log":b.log}));
    if args.iter().any(|s| s == "--proof") {
        println!(
            "{}",
            json!({"proof":combo_certify::certify(dex,&b,0,100000,4096)})
        );
    }
    for seed in seed..seed + seeds {
        let cfg = RmConfig {
            iterations,
            rule: SelRule::Ucb,
            c: 1.0,
            ..RmConfig::default()
        };
        let mut search = BlindSearch::new(&b, dex, cfg, 0, seed);
        search.step(dex, &belief, &obs, iterations);
        println!(
            "{}",
            json!({"case":wait.key(),"seed":seed,"best":search.best().unwrap().to_input(dex),
            "actions":search.actions().iter().enumerate().map(|(i,a)|json!({"action":a.to_input(dex),"visits":search.visits()[i],"mean":search.means()[i],"dominated":search.dominated()[i]})).collect::<Vec<_>>()})
        );
    }
}
