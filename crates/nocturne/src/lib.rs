#![doc = include_str!("../README.md")]

use k256::ecdsa::{RecoveryId, Signature as EcdsaSig, SigningKey, VerifyingKey};
use tiny_keccak::{Hasher, Keccak};

mod convert;
pub use convert::*;

mod builder;
pub use builder::*;

mod validate;
pub use validate::*;

mod sim;
pub use sim::*;

mod decode;
pub use decode::*;

mod codec;
pub use codec::*;

mod signer;
pub use signer::*;

mod authorize;
pub use authorize::*;

mod sizing;
pub use sizing::*;

/// Errors from constructing a Merkle tree over offer leaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TreeError {
    /// The leaf set is empty or not a power of two.
    #[error("leaf count must be a nonzero power of two, got {0}")]
    NotPowerOfTwo(usize),
    /// The tree is taller than [`MAX_TREE_HEIGHT`], which `HashLib.offerTreeTypeHash` rejects
    /// on-chain with `TreeTooHigh`.
    #[error("tree height must be at most 20, got {0}")]
    TooHigh(usize),
}

pub type Word = [u8; 32];
pub type Address = [u8; 20];

// ---- EIP-712 type strings (order matches HashLib.sol comments) ----
pub const COLLATERAL_PARAMS_TYPE: &str =
    "CollateralParams(address token,uint256 lltv,uint256 liquidationCursor,address oracle)";
pub const MARKET_TYPE: &str = "Market(uint256 chainId,address midnight,address loanToken,CollateralParams[] collateralParams,uint256 maturity,uint256 rcfThreshold,address enterGate,address liquidatorGate)";
pub const OFFER_TYPE: &str = "Offer(Market market,bool buy,address maker,uint256 start,uint256 expiry,uint256 tick,bytes32 group,address callback,bytes callbackData,address receiverIfMakerIsSeller,address ratifier,bool reduceOnly,uint128 maxUnits,uint128 maxAssets,uint256 continuousFeeCap)";
pub const EIP712_DOMAIN_TYPE: &str = "EIP712Domain(uint256 chainId,address verifyingContract)";

pub fn keccak(bytes: &[u8]) -> Word {
    let mut h = Keccak::v256();
    let mut out = [0u8; 32];
    h.update(bytes);
    h.finalize(&mut out);
    out
}

#[inline]
fn addr_word(a: &Address) -> Word {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a);
    w
}
#[inline]
fn u128_word(x: u128) -> Word {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&x.to_be_bytes());
    w
}
#[inline]
fn bool_word(b: bool) -> Word {
    let mut w = [0u8; 32];
    w[31] = b as u8;
    w
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CollateralParams {
    pub token: Address,
    pub lltv: Word,
    pub liquidation_cursor: Word,
    pub oracle: Address,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Market {
    pub chain_id: Word,
    pub midnight: Address,
    pub loan_token: Address,
    pub collateral_params: Vec<CollateralParams>,
    pub maturity: Word,
    pub rcf_threshold: Word,
    pub enter_gate: Address,
    pub liquidator_gate: Address,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Offer {
    pub market: Market,
    pub buy: bool,
    pub maker: Address,
    pub start: Word,
    pub expiry: Word,
    pub tick: Word,
    pub group: Word,
    pub callback: Address,
    pub callback_data: Vec<u8>,
    pub receiver_if_maker_is_seller: Address,
    pub ratifier: Address,
    pub reduce_only: bool,
    pub max_units: u128,
    pub max_assets: u128,
    pub continuous_fee_cap: Word,
}

// Typehashes are computed from the type strings; tests assert they equal the on-chain constants.
pub fn collateral_params_typehash() -> Word {
    keccak(COLLATERAL_PARAMS_TYPE.as_bytes())
}
pub fn market_typehash() -> Word {
    keccak([MARKET_TYPE, COLLATERAL_PARAMS_TYPE].concat().as_bytes())
}
pub fn offer_typehash() -> Word {
    keccak(
        [OFFER_TYPE, COLLATERAL_PARAMS_TYPE, MARKET_TYPE]
            .concat()
            .as_bytes(),
    )
}
/// The tallest offer tree `HashLib.offerTreeTypeHash` has a constant for; above this the
/// contract reverts `TreeTooHigh`, so no taller tree can ever ratify.
pub const MAX_TREE_HEIGHT: usize = 20;

/// Panics if `height` exceeds [`MAX_TREE_HEIGHT`], where `HashLib.offerTreeTypeHash` reverts
/// `TreeTooHigh`.
pub fn offer_tree_typehash(height: usize) -> Word {
    assert!(
        height <= MAX_TREE_HEIGHT,
        "tree height {height} exceeds {MAX_TREE_HEIGHT} (HashLib.offerTreeTypeHash reverts TreeTooHigh)"
    );
    let mut field = String::from("OfferTree(Offer");
    for _ in 0..height {
        field.push_str("[2]");
    }
    field.push_str(" offerTree)");
    keccak(
        [
            field.as_str(),
            COLLATERAL_PARAMS_TYPE,
            MARKET_TYPE,
            OFFER_TYPE,
        ]
        .concat()
        .as_bytes(),
    )
}

fn encode(words: &[Word]) -> Vec<u8> {
    let mut out = Vec::with_capacity(words.len() * 32);
    for w in words {
        out.extend_from_slice(w);
    }
    out
}

pub fn hash_collateral_params(cp: &CollateralParams) -> Word {
    keccak(&encode(&[
        collateral_params_typehash(),
        addr_word(&cp.token),
        cp.lltv,
        cp.liquidation_cursor,
        addr_word(&cp.oracle),
    ]))
}

/// Return a market with collateral parameters in canonical ascending token order.
///
/// Midnight identifies and hashes markets independently of the caller's input order. Keeping
/// this normalization explicit is also useful before ABI encoding or persistence.
pub fn canonical_market(market: &Market) -> Market {
    let mut canonical = market.clone();
    canonical
        .collateral_params
        .sort_by(|left, right| left.token.cmp(&right.token));
    canonical
}

pub fn hash_market(m: &Market) -> Word {
    // collateralParamsHash = keccak256(abi.encodePacked(hashes))
    let mut packed = Vec::with_capacity(m.collateral_params.len() * 32);
    let mut collateral_params: Vec<&CollateralParams> = m.collateral_params.iter().collect();
    collateral_params.sort_by(|left, right| left.token.cmp(&right.token));
    for cp in collateral_params {
        packed.extend_from_slice(&hash_collateral_params(cp));
    }
    let cp_hash = keccak(&packed);
    keccak(&encode(&[
        market_typehash(),
        m.chain_id,
        addr_word(&m.midnight),
        addr_word(&m.loan_token),
        cp_hash,
        m.maturity,
        m.rcf_threshold,
        addr_word(&m.enter_gate),
        addr_word(&m.liquidator_gate),
    ]))
}

/// Compute the deterministic Midnight market id (`IdLib.toId`).
///
/// The id is the CREATE2 address word for the SSTORE2 pointer that contains the canonical ABI
/// encoding of the market parameters. It is returned as the protocol's 32-byte market id rather
/// than truncated to an Ethereum address.
pub fn market_id(market: &Market) -> Word {
    const SSTORE2_PREFIX: [u8; 11] = [
        0x60, 0x0b, 0x38, 0x03, 0x80, 0x60, 0x0b, 0x5f, 0x39, 0x5f, 0xf3,
    ];

    let canonical = canonical_market(market);
    let encoded = encode_market_params(&canonical);
    let mut init_code = Vec::with_capacity(SSTORE2_PREFIX.len() + encoded.len());
    init_code.extend_from_slice(&SSTORE2_PREFIX);
    init_code.extend_from_slice(&encoded);
    let creation_hash = keccak(&init_code);

    let mut create2 = Vec::with_capacity(1 + 20 + 32 + 32);
    create2.push(0xff);
    create2.extend_from_slice(&canonical.midnight);
    create2.extend_from_slice(&[0u8; 32]);
    create2.extend_from_slice(&creation_hash);
    keccak(&create2)
}

/// EIP-712 struct hash of an Offer - this is the Merkle leaf. Mirrors `HashLib.hashOffer`.
pub fn hash_offer(o: &Offer) -> Word {
    keccak(&encode(&[
        offer_typehash(),
        hash_market(&o.market),
        bool_word(o.buy),
        addr_word(&o.maker),
        o.start,
        o.expiry,
        o.tick,
        o.group,
        addr_word(&o.callback),
        keccak(&o.callback_data),
        addr_word(&o.receiver_if_maker_is_seller),
        addr_word(&o.ratifier),
        bool_word(o.reduce_only),
        u128_word(o.max_units),
        u128_word(o.max_assets),
        o.continuous_fee_cap,
    ]))
}

#[inline]
pub fn hash_node(left: &Word, right: &Word) -> Word {
    let mut buf = [0u8; 64];
    buf[..32].copy_from_slice(left);
    buf[32..].copy_from_slice(right);
    keccak(&buf)
}

/// A perfect binary Merkle tree over offer leaves. `height` = log2(leaves.len()).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OfferTree {
    /// `levels[0]` = leaves, `levels[height]` = `[root]`
    pub levels: Vec<Vec<Word>>,
}

impl OfferTree {
    /// Build a perfect binary tree over `leaves`. Errors unless the count is a nonzero power of
    /// two of at most `2^MAX_TREE_HEIGHT` - `HashLib.offerTreeTypeHash` reverts `TreeTooHigh`
    /// above height 20, so a taller tree could never ratify.
    pub fn build(leaves: Vec<Word>) -> Result<Self, TreeError> {
        if leaves.is_empty() || !leaves.len().is_power_of_two() {
            return Err(TreeError::NotPowerOfTwo(leaves.len()));
        }
        if leaves.len() > 1 << MAX_TREE_HEIGHT {
            return Err(TreeError::TooHigh(leaves.len().trailing_zeros() as usize));
        }
        let mut levels = vec![leaves];
        while levels.last().unwrap().len() > 1 {
            let prev = levels.last().unwrap();
            let next: Vec<Word> = prev
                .chunks_exact(2)
                .map(|p| hash_node(&p[0], &p[1]))
                .collect();
            levels.push(next);
        }
        Ok(OfferTree { levels })
    }

    pub fn height(&self) -> usize {
        self.levels.len() - 1
    }
    pub fn root(&self) -> Word {
        self.levels.last().unwrap()[0]
    }

    /// Merkle proof for the leaf at `index`, sibling per level (matches HashLib.isLeaf order).
    pub fn proof(&self, index: usize) -> Vec<Word> {
        let mut proof = Vec::with_capacity(self.height());
        let mut idx = index;
        for level in &self.levels[..self.height()] {
            let sib = idx ^ 1;
            proof.push(level[sib]);
            idx >>= 1;
        }
        proof
    }
}

/// Recompute the root from a leaf + proof, exactly as `HashLib.isLeaf` does on-chain -
/// including its leaf-index range check, whose `LeafIndexOutOfRange` revert is `false` here.
pub fn verify_leaf(root: &Word, leaf: &Word, leaf_index: usize, proof: &[Word]) -> bool {
    // HashLib.isLeaf requires leafIndex >> proof.length == 0: high bits beyond the proof are
    // never consumed by the fold, so an index outside [0, 2^len) could otherwise "verify"
    // off-chain and then revert on-chain. (A shift past usize::BITS - well-defined on the
    // contract's uint256 - always yields 0, i.e. in range.)
    if leaf_index.checked_shr(proof.len() as u32).unwrap_or(0) != 0 {
        return false;
    }
    let mut cur = *leaf;
    for (i, sib) in proof.iter().enumerate() {
        cur = if (leaf_index >> i) & 1 == 0 {
            hash_node(&cur, sib)
        } else {
            hash_node(sib, &cur)
        };
    }
    cur == *root
}

pub fn domain_separator(chain_id: Word, ratifier: &Address) -> Word {
    keccak(&encode(&[
        keccak(EIP712_DOMAIN_TYPE.as_bytes()),
        chain_id,
        addr_word(ratifier),
    ]))
}

/// The digest the maker signs for a whole tree (one signature covers every offer in it).
///
/// Panics if `height` exceeds [`MAX_TREE_HEIGHT`] (via [`offer_tree_typehash`]).
pub fn tree_digest(root: Word, height: usize, chain_id: Word, ratifier: &Address) -> Word {
    let struct_hash = keccak(&encode(&[offer_tree_typehash(height), root]));
    let mut buf = Vec::with_capacity(2 + 64);
    buf.extend_from_slice(&[0x19, 0x01]);
    buf.extend_from_slice(&domain_separator(chain_id, ratifier));
    buf.extend_from_slice(&struct_hash);
    keccak(&buf)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sig {
    pub r: Word,
    pub s: Word,
    pub v: u8,
}

/// Sign the tree digest with the maker's key (secp256k1, in-process - no wallet round-trip).
pub fn sign_digest(sk: &SigningKey, digest: &Word) -> Sig {
    let (sig, rec): (EcdsaSig, RecoveryId) = sk.sign_prehash_recoverable(digest).expect("sign");
    let b = sig.to_bytes();
    let mut r = [0u8; 32];
    let mut s = [0u8; 32];
    r.copy_from_slice(&b[..32]);
    s.copy_from_slice(&b[32..]);
    Sig {
        r,
        s,
        v: 27 + rec.to_byte(),
    }
}

/// The Ethereum address of a public key: last 20 bytes of keccak(uncompressed pubkey without the 0x04 tag).
fn address_of(vk: &VerifyingKey) -> Address {
    let point = vk.to_encoded_point(false);
    let h = keccak(&point.as_bytes()[1..]); // drop the 0x04 prefix
    let mut a = [0u8; 20];
    a.copy_from_slice(&h[12..]);
    a
}

/// The Ethereum address controlled by a signing key - the `maker` address to put in offers.
pub fn signer_address(sk: &SigningKey) -> Address {
    address_of(sk.verifying_key())
}

/// Recover the signer address from a digest and signature, exactly as the `ecrecover` in
/// `EcrecoverRatifier.isRatified`. Returns `None` for a malformed signature (the on-chain
/// equivalent of `ecrecover` yielding `address(0)`).
///
/// Like the precompile, this accepts a high-`s` (malleable) signature: `ecrecover` takes any
/// `s` in `[1, n-1]` with no low-`s` guard, so the malleated counterpart `(r, n - s, flipped v)`
/// of a valid signature recovers the same address. Detect such signatures with [`is_high_s`].
pub fn recover(digest: &Word, sig: &Sig) -> Option<Address> {
    // v is 27/28 on-chain (anything else makes ecrecover yield address(0)); RecoveryId wants 0/1.
    if sig.v != 27 && sig.v != 28 {
        return None;
    }
    let mut rec = RecoveryId::from_byte(sig.v - 27)?;
    let mut rs = [0u8; 64];
    rs[..32].copy_from_slice(&sig.r);
    rs[32..].copy_from_slice(&sig.s);
    let mut ecdsa = EcdsaSig::from_slice(&rs).ok()?;
    // `ecrecover` accepts any `s` in [1, n-1], but k256 rejects high-`s`. Substituting `n - s`
    // negates the signature, which flips the parity of the point `ecrecover` reconstructs from
    // `r`, so normalizing to low-`s` AND flipping the recovery id recovers exactly the address
    // the precompile returns for the original `(r, s, v)`.
    if let Some(low) = ecdsa.normalize_s() {
        ecdsa = low;
        rec = RecoveryId::from_byte(rec.to_byte() ^ 1)?;
    }
    let vk = VerifyingKey::recover_from_prehash(digest, &ecdsa, rec).ok()?;
    Some(address_of(&vk))
}

/// Full off-chain mirror of `EcrecoverRatifier.isRatified` (minus the `isAuthorized` /
/// `isRootCanceled` lookups, which need chain state). Recomputes the leaf, checks the Merkle
/// proof, rebuilds the digest, recovers the signer, and confirms it is `expected_maker`.
///
/// If this returns `true`, a `take` carrying `(sig, root, leaf_index, proof)` will pass the
/// ratifier as long as `expected_maker` is (or is authorized by) `offer.maker` on-chain.
#[allow(clippy::too_many_arguments)]
pub fn verify(
    offer: &Offer,
    root: &Word,
    leaf_index: usize,
    proof: &[Word],
    sig: &Sig,
    chain_id: Word,
    ratifier: &Address,
    expected_maker: &Address,
) -> bool {
    // The ratifier calls HashLib.offerTreeTypeHash(proof.length), which reverts TreeTooHigh
    // above MAX_TREE_HEIGHT - such a take can never pass (and there is no digest to rebuild).
    if proof.len() > MAX_TREE_HEIGHT {
        return false;
    }
    let leaf = hash_offer(offer);
    if !verify_leaf(root, &leaf, leaf_index, proof) {
        return false;
    }
    let digest = tree_digest(*root, proof.len(), chain_id, ratifier);
    recover(&digest, sig).as_ref() == Some(expected_maker)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn word_u64(x: u64) -> Word {
        let mut w = [0u8; 32];
        w[24..].copy_from_slice(&x.to_be_bytes());
        w
    }

    fn tiny_offer(maker: Address, i: u64) -> Offer {
        let market = Market {
            chain_id: word_u64(1),
            midnight: [0x11; 20],
            loan_token: [0x22; 20],
            collateral_params: vec![CollateralParams {
                token: [0x33; 20],
                lltv: word_u64(860_000_000_000_000_000),
                liquidation_cursor: word_u64(1),
                oracle: [0x44; 20],
            }],
            maturity: word_u64(1_800_000_000),
            rcf_threshold: word_u64(1000),
            enter_gate: [0u8; 20],
            liquidator_gate: [0u8; 20],
        };
        Offer {
            market,
            buy: i % 2 == 0,
            maker,
            start: word_u64(0),
            expiry: word_u64(2_000_000_000),
            tick: word_u64(i % 6744),
            group: word_u64(i),
            callback: [0u8; 20],
            callback_data: Vec::new(),
            receiver_if_maker_is_seller: [0u8; 20],
            ratifier: [0xbb; 20],
            reduce_only: false,
            max_units: 1_000_000 + i as u128,
            max_assets: 0,
            continuous_fee_cap: word_u64(0),
        }
    }

    #[test]
    fn recover_returns_the_signer() {
        let sk = SigningKey::from_bytes(&[0x42u8; 32].into()).unwrap();
        let maker = signer_address(&sk);
        let digest = keccak(b"any 32-byte digest goes here....");
        let sig = sign_digest(&sk, &digest);
        assert_eq!(recover(&digest, &sig), Some(maker));
    }

    #[test]
    fn verify_accepts_a_valid_offer_signature() {
        let sk = SigningKey::from_bytes(&[0x07u8; 32].into()).unwrap();
        let maker = signer_address(&sk);
        let ratifier = [0xbbu8; 20];
        let chain_id = word_u64(1);

        let offers: Vec<Offer> = (0..4).map(|i| tiny_offer(maker, i)).collect();
        let leaves: Vec<Word> = offers.iter().map(hash_offer).collect();
        let tree = OfferTree::build(leaves).unwrap();
        let digest = tree_digest(tree.root(), tree.height(), chain_id, &ratifier);
        let sig = sign_digest(&sk, &digest);

        // Every leaf verifies with its own proof.
        for (i, offer) in offers.iter().enumerate() {
            assert!(
                verify(
                    offer,
                    &tree.root(),
                    i,
                    &tree.proof(i),
                    &sig,
                    chain_id,
                    &ratifier,
                    &maker
                ),
                "leaf {i} should verify"
            );
        }
    }

    #[test]
    fn verify_rejects_wrong_maker() {
        let sk = SigningKey::from_bytes(&[0x07u8; 32].into()).unwrap();
        let maker = signer_address(&sk);
        let ratifier = [0xbbu8; 20];
        let chain_id = word_u64(1);

        let offers: Vec<Offer> = (0..2).map(|i| tiny_offer(maker, i)).collect();
        let tree = OfferTree::build(offers.iter().map(hash_offer).collect()).unwrap();
        let digest = tree_digest(tree.root(), tree.height(), chain_id, &ratifier);
        let sig = sign_digest(&sk, &digest);

        let not_maker = [0x99u8; 20];
        assert!(!verify(
            &offers[0],
            &tree.root(),
            0,
            &tree.proof(0),
            &sig,
            chain_id,
            &ratifier,
            &not_maker
        ));
    }

    #[test]
    fn verify_rejects_tampered_offer_and_proof() {
        let sk = SigningKey::from_bytes(&[0x07u8; 32].into()).unwrap();
        let maker = signer_address(&sk);
        let ratifier = [0xbbu8; 20];
        let chain_id = word_u64(1);

        let offers: Vec<Offer> = (0..4).map(|i| tiny_offer(maker, i)).collect();
        let tree = OfferTree::build(offers.iter().map(hash_offer).collect()).unwrap();
        let digest = tree_digest(tree.root(), tree.height(), chain_id, &ratifier);
        let sig = sign_digest(&sk, &digest);

        // Tampered offer (different tick) no longer hashes to the signed leaf -> proof fails.
        let mut tampered = offers[0].clone();
        tampered.tick = word_u64(999);
        assert!(!verify(
            &tampered,
            &tree.root(),
            0,
            &tree.proof(0),
            &sig,
            chain_id,
            &ratifier,
            &maker
        ));

        // Right offer, wrong leaf index -> proof fails.
        assert!(!verify(
            &offers[0],
            &tree.root(),
            1,
            &tree.proof(1),
            &sig,
            chain_id,
            &ratifier,
            &maker
        ));

        // Wrong chain id -> different digest -> recovers a different address.
        assert!(!verify(
            &offers[0],
            &tree.root(),
            0,
            &tree.proof(0),
            &sig,
            word_u64(999),
            &ratifier,
            &maker
        ));
    }

    #[test]
    #[should_panic(expected = "TreeTooHigh")]
    fn offer_tree_typehash_panics_above_max_height() {
        // HashLib.offerTreeTypeHash has constants for heights 0-20 only and reverts TreeTooHigh
        // above that; there is no typehash to compute.
        offer_tree_typehash(MAX_TREE_HEIGHT + 1);
    }

    #[test]
    fn verify_leaf_rejects_out_of_range_leaf_index() {
        // HashLib.isLeaf reverts LeafIndexOutOfRange unless leafIndex >> proof.length == 0.
        // Index 2 folds exactly like index 0 on a height-1 tree (bit 1 is never consumed), so
        // without the range check it would "verify" off-chain and then revert on-chain.
        let leaves = vec![keccak(b"a"), keccak(b"b")];
        let tree = OfferTree::build(leaves.clone()).unwrap();
        let proof = tree.proof(0);
        assert!(verify_leaf(&tree.root(), &leaves[0], 0, &proof));
        assert!(!verify_leaf(&tree.root(), &leaves[0], 2, &proof));

        // Same on the single-leaf tree (empty proof): any nonzero index is out of range.
        let one = OfferTree::build(vec![keccak(b"a")]).unwrap();
        assert!(verify_leaf(&one.root(), &keccak(b"a"), 0, &[]));
        assert!(!verify_leaf(&one.root(), &keccak(b"a"), 1, &[]));
    }

    #[test]
    fn verify_rejects_proofs_taller_than_max_height() {
        let sk = SigningKey::from_bytes(&[0x07u8; 32].into()).unwrap();
        let maker = signer_address(&sk);
        let ratifier = [0xbbu8; 20];
        let chain_id = word_u64(1);
        let offer = tiny_offer(maker, 0);
        let leaf = hash_offer(&offer);

        // Fold the leaf up under zero siblings: a consistent (root, proof) of any height
        // without materializing a 2^height tree.
        let fold = |height: usize| {
            let proof = vec![[0u8; 32]; height];
            let mut root = leaf;
            for sib in &proof {
                root = hash_node(&root, sib);
            }
            (root, proof)
        };

        // Height 20 is the on-chain cap and still verifies end to end.
        let (root, proof) = fold(MAX_TREE_HEIGHT);
        let sig = sign_digest(
            &sk,
            &tree_digest(root, MAX_TREE_HEIGHT, chain_id, &ratifier),
        );
        assert!(verify(
            &offer, &root, 0, &proof, &sig, chain_id, &ratifier, &maker
        ));

        // One level higher the ratifier reverts TreeTooHigh, so verify must say false
        // (whatever the signature - there is no digest a maker could even sign).
        let (root, proof) = fold(MAX_TREE_HEIGHT + 1);
        assert!(!verify(
            &offer, &root, 0, &proof, &sig, chain_id, &ratifier, &maker
        ));
    }

    #[test]
    fn recover_accepts_the_high_s_counterpart() {
        // ecrecover has no low-s guard: the malleated (r, n - s, flipped v) of a valid
        // signature recovers the same maker, and `recover` must mirror that.
        let sk = SigningKey::from_bytes(&[0x42u8; 32].into()).unwrap();
        let maker = signer_address(&sk);
        let digest = keccak(b"high-s malleability test digest.");
        let low = sign_digest(&sk, &digest);

        let high = Sig {
            r: low.r,
            s: high_s_counterpart(&low.s),
            v: if low.v == 27 { 28 } else { 27 },
        };
        assert!(is_high_s(&high.s), "counterpart must be high-s");
        assert_eq!(recover(&digest, &high), Some(maker));

        // The low-s original still recovers, unchanged.
        assert!(!is_high_s(&low.s));
        assert_eq!(recover(&digest, &low), Some(maker));
    }

    #[test]
    fn recover_rejects_malformed_v() {
        let sk = SigningKey::from_bytes(&[0x42u8; 32].into()).unwrap();
        let digest = keccak(b"another 32-byte test digest....");
        let good = sign_digest(&sk, &digest);
        // Anything outside 27/28 makes ecrecover yield address(0), i.e. None here.
        for v in [0u8, 1, 26, 29, 31, 255] {
            let mut sig = good;
            sig.v = v;
            assert_eq!(recover(&digest, &sig), None, "v = {v} must be rejected");
        }
    }
}
