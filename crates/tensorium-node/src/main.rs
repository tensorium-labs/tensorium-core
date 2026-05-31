use std::time::{SystemTime, UNIX_EPOCH};

use tensorium_core::{
    block::{merkle_root, Block, BlockHeader, Transaction},
    chain::TESTNET,
    emission::reward_at_height,
    pow::mine_header,
    Hash256,
};

fn main() {
    let params = TESTNET;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before unix epoch")
        .as_secs();
    let height = 0;
    let reward = reward_at_height(&params, height);
    let tx = Transaction::coinbase(height, reward, "local-dev-miner");
    let header = BlockHeader {
        version: 1,
        chain_id: params.chain_id.to_owned(),
        height,
        previous_hash: Hash256::ZERO,
        merkle_root: merkle_root(core::slice::from_ref(&tx)),
        timestamp_seconds: now,
        leading_zero_bits: params.initial_leading_zero_bits,
        nonce: 0,
    };

    let Some(mined_header) = mine_header(header, 10_000_000) else {
        eprintln!("no valid nonce found in local search window");
        std::process::exit(1);
    };

    let block = Block::new(mined_header, vec![tx]);
    println!("{}", serde_json::to_string_pretty(&block).expect("block serializes"));
}
