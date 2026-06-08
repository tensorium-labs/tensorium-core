use super::*;

fn put_str(out: &mut Vec<u8>, s: &str, max: usize) {
    let b = s.as_bytes();
    let n = b.len().min(max);
    out.push(n as u8);
    out.extend_from_slice(&b[..n]);
}

fn take<'a>(buf: &'a [u8], i: &mut usize, n: usize) -> Result<&'a [u8], AssetError> {
    if *i + n > buf.len() {
        return Err(AssetError::Truncated);
    }
    let s = &buf[*i..*i + n];
    *i += n;
    Ok(s)
}

fn take_str(buf: &[u8], i: &mut usize) -> Result<String, AssetError> {
    let len = take(buf, i, 1)?[0] as usize;
    let bytes = take(buf, i, len)?;
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

fn take_u64(buf: &[u8], i: &mut usize) -> Result<u64, AssetError> {
    let b = take(buf, i, 8)?;
    Ok(u64::from_be_bytes(b.try_into().unwrap()))
}

/// Encode an asset op into the full OP_RETURN data payload (`TXMA` + version + op + body).
pub fn encode_op(op: &AssetOp) -> Vec<u8> {
    let mut out = Vec::with_capacity(64);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    match op {
        AssetOp::Issue(d) => {
            out.push(OP_ISSUE);
            put_str(&mut out, &d.ticker, 8);
            out.push(d.decimals);
            out.extend_from_slice(&d.supply.to_be_bytes());
            put_str(&mut out, &d.name, 32);
            out.push(d.flags);
        }
        AssetOp::NftMint(d) => {
            out.push(OP_NFT_MINT);
            out.extend_from_slice(&d.collection_id);
            out.extend_from_slice(&d.royalty_bps.to_be_bytes());
            put_str(&mut out, &d.royalty_addr, 90);
            put_str(&mut out, &d.uri, 200);
            out.extend_from_slice(&d.content_hash);
        }
        AssetOp::Transfer(d) => {
            out.push(OP_TRANSFER);
            out.extend_from_slice(&d.asset_id);
            out.extend_from_slice(&d.amount.to_be_bytes());
            out.push(d.dest_output_index);
        }
    }
    out
}

/// Decode an OP_RETURN data payload into an asset op.
pub fn decode_op(buf: &[u8]) -> Result<AssetOp, AssetError> {
    if buf.len() > MAX_PAYLOAD {
        return Err(AssetError::TooLarge);
    }
    let mut i = 0;
    if take(buf, &mut i, 4)? != MAGIC {
        return Err(AssetError::BadMagic);
    }
    if take(buf, &mut i, 1)?[0] != VERSION {
        return Err(AssetError::BadVersion);
    }
    let opcode = take(buf, &mut i, 1)?[0];
    match opcode {
        OP_ISSUE => {
            let ticker = take_str(buf, &mut i)?;
            let decimals = take(buf, &mut i, 1)?[0];
            let supply = take_u64(buf, &mut i)?;
            let name = take_str(buf, &mut i)?;
            let flags = take(buf, &mut i, 1)?[0];
            Ok(AssetOp::Issue(IssueData { ticker, decimals, supply, name, flags }))
        }
        OP_NFT_MINT => {
            let collection_id: [u8; 32] = take(buf, &mut i, 32)?.try_into().unwrap();
            let royalty_bps = u16::from_be_bytes(take(buf, &mut i, 2)?.try_into().unwrap());
            if royalty_bps > 10_000 {
                return Err(AssetError::BadRoyalty);
            }
            let royalty_addr = take_str(buf, &mut i)?;
            let uri = take_str(buf, &mut i)?;
            let content_hash: [u8; 32] = take(buf, &mut i, 32)?.try_into().unwrap();
            Ok(AssetOp::NftMint(NftMintData { collection_id, royalty_bps, royalty_addr, uri, content_hash }))
        }
        OP_TRANSFER => {
            let asset_id: [u8; 32] = take(buf, &mut i, 32)?.try_into().unwrap();
            let amount = take_u64(buf, &mut i)?;
            let dest_output_index = take(buf, &mut i, 1)?[0];
            Ok(AssetOp::Transfer(TransferData { asset_id, amount, dest_output_index }))
        }
        _ => Err(AssetError::UnknownOpcode),
    }
}

use crate::block::Transaction;
use crate::script::OP_RETURN;

/// Read the data bytes pushed after an `OP_RETURN`. Supports a direct
/// push (0x01..=0x4b) or `OP_PUSHDATA1` (0x4c). Returns None if the output
/// is not an OP_RETURN data carrier.
fn op_return_data(spk: &[u8]) -> Option<&[u8]> {
    if spk.first() != Some(&OP_RETURN) {
        return None;
    }
    let mut i = 1;
    let len = match spk.get(i)? {
        n @ 0x01..=0x4b => {
            i += 1;
            *n as usize
        }
        0x4c => {
            i += 1;
            *spk.get(i).map(|x| {
                i += 1;
                x
            })? as usize
        }
        _ => return None,
    };
    spk.get(i..i + len)
}

/// Find the first valid `TXMA` asset op in a transaction's outputs.
pub fn extract_asset_op(tx: &Transaction) -> Option<AssetOp> {
    for out in &tx.outputs {
        if let Some(data) = op_return_data(&out.script_pubkey) {
            if let Ok(op) = decode_op(data) {
                return Some(op);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_roundtrip() {
        let op = AssetOp::Issue(IssueData {
            ticker: "GOLD".into(),
            decimals: 8,
            supply: 21_000_000,
            name: "Gold Token".into(),
            flags: 0,
        });
        let bytes = encode_op(&op);
        assert_eq!(&bytes[0..4], MAGIC);
        assert_eq!(bytes[4], VERSION);
        assert_eq!(bytes[5], OP_ISSUE);
        assert_eq!(decode_op(&bytes).unwrap(), op);
    }

    #[test]
    fn nft_mint_roundtrip_with_royalty() {
        let op = AssetOp::NftMint(NftMintData {
            collection_id: [0u8; 32],
            royalty_bps: 500, // 5%
            royalty_addr: "txm1royaltyaddrexample00000000000000000".into(),
            uri: "ipfs://Qm123".into(),
            content_hash: [7u8; 32],
        });
        let bytes = encode_op(&op);
        assert_eq!(bytes[5], OP_NFT_MINT);
        assert_eq!(decode_op(&bytes).unwrap(), op);
    }

    #[test]
    fn transfer_roundtrip() {
        let op = AssetOp::Transfer(TransferData {
            asset_id: [9u8; 32],
            amount: 1234,
            dest_output_index: 2,
        });
        assert_eq!(decode_op(&encode_op(&op)).unwrap(), op);
    }

    #[test]
    fn decode_rejects_bad_inputs() {
        assert_eq!(decode_op(b"XXXX\x01\x01"), Err(AssetError::BadMagic));
        assert_eq!(decode_op(b"TXMA\x09\x01"), Err(AssetError::BadVersion));
        assert_eq!(decode_op(b"TXMA\x01\x99"), Err(AssetError::UnknownOpcode));
        assert_eq!(decode_op(b"TXMA\x01"), Err(AssetError::Truncated));
        // royalty > 10000 rejected
        let mut bad = vec![];
        bad.extend_from_slice(MAGIC);
        bad.push(VERSION);
        bad.push(OP_NFT_MINT);
        bad.extend_from_slice(&[0u8; 32]);          // collection
        bad.extend_from_slice(&10_001u16.to_be_bytes()); // royalty_bps
        bad.push(0);                                 // royalty_addr len 0
        bad.push(0);                                 // uri len 0
        bad.extend_from_slice(&[0u8; 32]);          // content_hash
        assert_eq!(decode_op(&bad), Err(AssetError::BadRoyalty));
        // oversize
        assert_eq!(decode_op(&vec![0u8; MAX_PAYLOAD + 1]), Err(AssetError::TooLarge));
    }

    use crate::block::{Transaction, TxOutput};
    use crate::script::OP_RETURN;

    fn op_return_output(data: &[u8]) -> TxOutput {
        // OP_RETURN <pushdata1 len> <data>
        let mut spk = vec![OP_RETURN, 0x4c, data.len() as u8];
        spk.extend_from_slice(data);
        TxOutput { value_atoms: 0, script_pubkey: spk }
    }

    #[test]
    fn extract_finds_first_txma_op_return() {
        let op = AssetOp::Transfer(TransferData { asset_id: [3u8; 32], amount: 5, dest_output_index: 0 });
        let tx = Transaction::payment(
            vec![],
            vec![
                TxOutput { value_atoms: 100, script_pubkey: vec![0x76, 0xa9] }, // non-OP_RETURN
                op_return_output(&encode_op(&op)),
            ],
        );
        assert_eq!(extract_asset_op(&tx), Some(op));
    }

    #[test]
    fn extract_ignores_non_txma_op_return() {
        let tx = Transaction::payment(
            vec![],
            vec![op_return_output(b"hello not an asset")],
        );
        assert_eq!(extract_asset_op(&tx), None);
    }
}
