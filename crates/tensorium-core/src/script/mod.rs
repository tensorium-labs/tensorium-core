pub mod standard;
pub mod vm;

// ── Data push (0x01–0x4b push the next N bytes) ───────────────────────────────
// Any byte 0x01..=0x4b encountered during execution pushes the next N bytes.

// ── Push false / zero ─────────────────────────────────────────────────────────
pub const OP_0: u8 = 0x00;

// ── Stack ─────────────────────────────────────────────────────────────────────
pub const OP_DUP: u8 = 0x76;
pub const OP_DROP: u8 = 0x75;
pub const OP_2DROP: u8 = 0x6f;
pub const OP_SWAP: u8 = 0x7c;

// ── Crypto ────────────────────────────────────────────────────────────────────
pub const OP_SHA256: u8 = 0xa8;
/// Tensorium-specific: SHA256(x)[0..20] — matches Address::from_public_key
pub const OP_HASH160: u8 = 0xa9;
pub const OP_CHECKSIG: u8 = 0xac;

// ── Multisig ──────────────────────────────────────────────────────────────────
pub const OP_CHECKMULTISIG: u8 = 0xae;
pub const OP_CHECKMULTISIGVERIFY: u8 = 0xaf;

// ── Small integers ────────────────────────────────────────────────────────────
// OP_1..OP_16 push the byte value [n] onto the stack.
pub const OP_1: u8 = 0x51;
pub const OP_2: u8 = 0x52;
pub const OP_3: u8 = 0x53;
pub const OP_4: u8 = 0x54;
pub const OP_5: u8 = 0x55;
pub const OP_6: u8 = 0x56;
pub const OP_7: u8 = 0x57;
pub const OP_8: u8 = 0x58;
pub const OP_9: u8 = 0x59;
pub const OP_10: u8 = 0x5a;
pub const OP_11: u8 = 0x5b;
pub const OP_12: u8 = 0x5c;
pub const OP_13: u8 = 0x5d;
pub const OP_14: u8 = 0x5e;
pub const OP_15: u8 = 0x5f;
pub const OP_16: u8 = 0x60;

// ── Comparison ────────────────────────────────────────────────────────────────
pub const OP_EQUAL: u8 = 0x87;
pub const OP_EQUALVERIFY: u8 = 0x88;
pub const OP_VERIFY: u8 = 0x69;

// ── Control ───────────────────────────────────────────────────────────────────
pub const OP_IF: u8 = 0x63;
pub const OP_ELSE: u8 = 0x67;
pub const OP_ENDIF: u8 = 0x68;
pub const OP_RETURN: u8 = 0x6a;

// ── Timelock ──────────────────────────────────────────────────────────────────
/// Absolute timelock: fails the script unless ctx.block_height >= top-of-stack value.
pub const OP_CHECKLOCKTIMEVERIFY: u8 = 0xb1;

// ── Limits ────────────────────────────────────────────────────────────────────
pub const MAX_STACK_DEPTH: usize = 100;
pub const MAX_SCRIPT_SIZE: usize = 10_000;
pub const MAX_ELEMENT_SIZE: usize = 520;

#[derive(Debug, PartialEq, Eq)]
pub enum ScriptError {
    StackOverflow,
    StackUnderflow,
    ScriptTooLarge,
    ElementTooLarge,
    InvalidOpcode(u8),
    InvalidSignature,
    InvalidKey,
    InvalidAddress,
    CheckSigFailed,
    VerifyFailed,
    UnexpectedEndOfScript,
    ScriptInSigContainsChecksig,
    LockTimeNotMet,
    P2shHashMismatch,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
