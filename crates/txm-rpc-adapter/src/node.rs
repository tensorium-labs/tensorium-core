//! Thin HTTP client to a running `tensorium-node` RPC.
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone)]
pub struct Node {
    base: String,
}

#[derive(Debug, Deserialize)]
pub struct Utxo {
    pub txid: String,
    pub txid_bytes: Vec<u8>,
    pub output_index: u32,
    pub value_atoms: u64,
    pub address: String,
    pub coinbase: bool,
    pub created_height: u64,
    pub mature: bool,
}

#[derive(Debug, Deserialize)]
struct UtxosResp {
    utxos: Vec<Utxo>,
    #[allow(dead_code)]
    tip_height: u64,
}

impl Node {
    pub fn new(base: &str) -> Self {
        // Accept "host:port", "http://…", "https://…".
        let b = base.trim().trim_end_matches('/');
        let base = if b.starts_with("http://") || b.starts_with("https://") {
            b.to_string()
        } else {
            format!("http://{b}")
        };
        Self { base }
    }

    fn get(&self, path: &str) -> Result<Value, String> {
        match ureq::get(&format!("{}{}", self.base, path)).call() {
            Ok(r) => {
                let s = r.into_string().map_err(|e| format!("node read: {e}"))?;
                serde_json::from_str(&s).map_err(|e| format!("node json: {e}"))
            }
            Err(ureq::Error::Status(c, r)) => {
                Err(format!("node {c}: {}", r.into_string().unwrap_or_default()))
            }
            Err(e) => Err(format!("node connect: {e}")),
        }
    }

    pub fn block_count(&self) -> Result<u64, String> {
        let v = self.get("/getblockcount")?;
        v.get("height").and_then(|h| h.as_u64()).ok_or("no height".into())
    }

    /// Returns `(block_hash_hex, block_json)` for a height.
    pub fn block_at(&self, height: u64) -> Result<(String, Value), String> {
        let v = self.get(&format!("/getblock/{height}"))?;
        // hash is a 32-byte array; render hex.
        let hash = match v.get("hash") {
            Some(Value::Array(a)) => a
                .iter()
                .filter_map(|x| x.as_u64())
                .map(|b| format!("{:02x}", b as u8))
                .collect::<String>(),
            Some(Value::String(s)) => s.clone(),
            _ => return Err("block missing hash".into()),
        };
        let block = v.get("block").cloned().ok_or("block missing body")?;
        Ok((hash, block))
    }

    pub fn utxos(&self, address: &str) -> Result<Vec<Utxo>, String> {
        let v = self.get(&format!("/getutxos/{address}"))?;
        let r: UtxosResp = serde_json::from_value(v).map_err(|e| format!("utxos parse: {e}"))?;
        Ok(r.utxos)
    }

    pub fn send_raw(&self, tx_json: &str) -> Result<String, String> {
        match ureq::post(&format!("{}/sendrawtransaction", self.base))
            .set("content-type", "application/json")
            .send_string(tx_json)
        {
            Ok(r) => {
                let s = r.into_string().map_err(|e| format!("sendraw read: {e}"))?;
                let v: Value = serde_json::from_str(&s).map_err(|e| format!("sendraw json: {e}"))?;
                if v.get("accepted").and_then(|a| a.as_bool()).unwrap_or(false) {
                    // txid may be an array of bytes; derive hex from our own computed id instead.
                    Ok(v.get("txid")
                        .map(txid_to_hex)
                        .unwrap_or_default())
                } else {
                    Err(format!("node rejected tx: {v}"))
                }
            }
            Err(ureq::Error::Status(c, r)) => {
                Err(format!("sendraw {c}: {}", r.into_string().unwrap_or_default()))
            }
            Err(e) => Err(format!("sendraw connect: {e}")),
        }
    }
}

pub fn txid_to_hex(v: &Value) -> String {
    match v {
        Value::Array(a) => a
            .iter()
            .filter_map(|x| x.as_u64())
            .map(|b| format!("{:02x}", b as u8))
            .collect(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}
