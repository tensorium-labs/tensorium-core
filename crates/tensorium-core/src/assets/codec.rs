use super::*;

// Scaffold stubs — replaced wholesale in Task 2 (codec) / Task 4 (extract).
pub fn encode_op(_op: &AssetOp) -> Vec<u8> {
    unimplemented!()
}
pub fn decode_op(_buf: &[u8]) -> Result<AssetOp, AssetError> {
    unimplemented!()
}
pub fn extract_asset_op(_tx: &crate::block::Transaction) -> Option<AssetOp> {
    unimplemented!()
}
