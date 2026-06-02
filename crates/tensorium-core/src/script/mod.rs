pub mod standard;
pub mod vm;

// ── Data push (0x01–0x4b push the next N bytes) ───────────────────────────────
// Any byte 0x01..=0x4b encountered during execution pushes the next N bytes.

// ── Stack ─────────────────────────────────────────────────────────────────────
pub const OP_DUP:         u8 = 0x76;
pub const OP_DROP:        u8 = 0x75;
pub const OP_2DROP:       u8 = 0x6f;
pub const OP_SWAP:        u8 = 0x7c;

// ── Crypto ────────────────────────────────────────────────────────────────────
pub const OP_SHA256:      u8 = 0xa8;
/// Tensorium-specific: SHA256(x)[0..20] — matches Address::from_public_key
pub const OP_HASH160:     u8 = 0xa9;
pub const OP_CHECKSIG:    u8 = 0xac;

// ── Comparison ────────────────────────────────────────────────────────────────
pub const OP_EQUAL:       u8 = 0x87;
pub const OP_EQUALVERIFY: u8 = 0x88;
pub const OP_VERIFY:      u8 = 0x69;

// ── Control ───────────────────────────────────────────────────────────────────
pub const OP_IF:          u8 = 0x63;
pub const OP_ELSE:        u8 = 0x67;
pub const OP_ENDIF:       u8 = 0x68;
pub const OP_RETURN:      u8 = 0x6a;

// ── Limits ────────────────────────────────────────────────────────────────────
pub const MAX_STACK_DEPTH:   usize = 100;
pub const MAX_SCRIPT_SIZE:   usize = 10_000;
pub const MAX_ELEMENT_SIZE:  usize = 520;

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
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self)
    }
}
