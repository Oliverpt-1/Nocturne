//! Tests for OfferBuilder / MarketBuilder.

use nocturne::*;

const MIDNIGHT: Address = [0x11; 20];
const LOAN: Address = [0x22; 20];
const RATIFIER: Address = [0xbb; 20];

fn wad_86() -> U256 {
    U256::from(860_000_000_000_000_000u64)
}

fn a_market() -> Market {
    MarketBuilder::new(1, MIDNIGHT, LOAN)
        .collateral([0x33; 20], wad_86(), U256::from(1u64), [0x44; 20])
        .maturity(1_800_000_000)
        .rcf_threshold(U256::from(1000u64))
        .build()
}

fn ctx() -> ValidateCtx {
    ValidateCtx {
        chain_id: Some(1),
        midnight: Some(MIDNIGHT),
        now: Some(1_700_000_000),
        market: Some(MarketSnapshot {
            tick_spacing: DEFAULT_TICK_SPACING,
            loss_factor_maxed: false,
            continuous_fee: 100,
        }),
    }
}

#[test]
fn builder_produces_expected_offer() {
    let offer = OfferBuilder::new(a_market(), [0x55; 20])
        .buy()
        .tick(8)
        .start(1_000)
        .expiry(2_000_000_000)
        .group_u64(7)
        .ratifier(RATIFIER)
        .max_units(1_000_000)
        .continuous_fee_cap(U256::from(100u64))
        .build();

    assert!(offer.buy);
    assert_eq!(offer.maker, [0x55; 20]);
    assert_eq!(word_to_u128(&offer.tick), Some(8));
    assert_eq!(word_to_u128(&offer.start), Some(1_000));
    assert_eq!(word_to_u128(&offer.expiry), Some(2_000_000_000));
    assert_eq!(word_to_u128(&offer.group), Some(7));
    assert_eq!(offer.ratifier, RATIFIER);
    assert_eq!(offer.max_units, 1_000_000);
    assert_eq!(offer.max_assets, 0);
    assert_eq!(word_to_u256(&offer.continuous_fee_cap), U256::from(100u64));
}

#[test]
fn defaults_are_sane() {
    let offer = OfferBuilder::new(a_market(), [0x55; 20])
        .tick(4)
        .ratifier(RATIFIER)
        .max_units(1)
        .build();
    // Raw build() skips the side/tick enforcement: side defaults to buy, start to 0, expiry to
    // max, no reduce_only/callback/receiver.
    assert!(offer.buy);
    assert_eq!(word_to_u128(&offer.start), Some(0));
    assert_eq!(word_to_u256(&offer.expiry), U256::MAX);
    assert!(!offer.reduce_only);
    assert!(offer.callback_data.is_empty());
    assert_eq!(offer.receiver_if_maker_is_seller, [0u8; 20]);
}

#[test]
fn caps_are_mutually_exclusive() {
    // Setting units then assets leaves only assets.
    let offer = OfferBuilder::new(a_market(), [0x55; 20])
        .tick(4)
        .ratifier(RATIFIER)
        .max_units(1_000)
        .max_assets(500)
        .build();
    assert_eq!(offer.max_units, 0);
    assert_eq!(offer.max_assets, 500);
    assert_eq!(active_cap(&offer), Some(Cap::Assets(500)));
}

#[test]
fn try_build_ok_for_valid_offer() {
    let res = OfferBuilder::new(a_market(), [0x55; 20])
        .buy()
        .tick(8) // multiple of DEFAULT_TICK_SPACING
        .start(1_000)
        .expiry(2_000_000_000)
        .ratifier(RATIFIER)
        .max_units(1_000_000)
        .continuous_fee_cap(U256::from(100u64))
        .try_build(&ctx());
    assert!(res.is_ok(), "expected Ok, got {res:?}");
}

#[test]
fn try_build_reports_errors() {
    // No cap set, tick not aligned to spacing (5 % 4 != 0).
    let res = OfferBuilder::new(a_market(), [0x55; 20])
        .buy()
        .tick(5)
        .start(1_000)
        .expiry(2_000_000_000)
        .ratifier(RATIFIER)
        .try_build(&ctx());
    let errs = res.expect_err("expected validation errors");
    assert!(errs.contains(&OfferError::InvalidOfferCaps));
    assert!(errs.contains(&OfferError::TickNotAccessible));
}

#[test]
fn try_build_rejects_unset_side() {
    // Everything else is valid, so the defaulted side is the only complaint.
    let res = OfferBuilder::new(a_market(), [0x55; 20])
        .tick(8)
        .ratifier(RATIFIER)
        .max_units(1_000_000)
        .continuous_fee_cap(U256::from(100u64))
        .try_build(&ctx());
    let errs = res.expect_err("expected SideNotSet");
    assert_eq!(errs, vec![OfferError::SideNotSet]);
}

#[test]
fn try_build_rejects_unset_tick() {
    // The tick-0 default passes every on-chain check (0 % spacing == 0), which is exactly why the
    // builder must flag it: its price rounds to zero.
    let res = OfferBuilder::new(a_market(), [0x55; 20])
        .buy()
        .ratifier(RATIFIER)
        .max_units(1_000_000)
        .continuous_fee_cap(U256::from(100u64))
        .try_build(&ctx());
    let errs = res.expect_err("expected TickNotSet");
    assert_eq!(errs, vec![OfferError::TickNotSet]);
}

#[test]
fn try_build_accepts_tick_set_via_apr() {
    // apr() counts as setting the tick (it snaps to the accessible grid itself).
    let res = OfferBuilder::new(a_market(), [0x55; 20])
        .lend()
        .apr(5.0, 1_700_000_000)
        .ratifier(RATIFIER)
        .max_units(1_000_000)
        .continuous_fee_cap(U256::from(100u64))
        .try_build(&ctx());
    assert!(res.is_ok(), "expected Ok, got {res:?}");
}

#[test]
fn build_checked_rejects_unset_side_then_tick() {
    let res = OfferBuilder::new(a_market(), [0x55; 20])
        .tick(8)
        .ratifier(RATIFIER)
        .max_units(1)
        .build_checked();
    assert_eq!(res.unwrap_err(), BuildError::SideNotSet);

    let res = OfferBuilder::new(a_market(), [0x55; 20])
        .buy()
        .ratifier(RATIFIER)
        .max_units(1)
        .build_checked();
    assert_eq!(res.unwrap_err(), BuildError::TickNotSet);
}

#[test]
fn built_offer_hashes_and_signs() {
    // The builder output flows through the existing hashing/signing pipeline unchanged.
    use k256::ecdsa::SigningKey;
    let sk = SigningKey::from_bytes(&[0x42u8; 32].into()).unwrap();
    let maker = signer_address(&sk);

    let offer = OfferBuilder::new(a_market(), maker)
        .buy()
        .tick(8)
        .ratifier(RATIFIER)
        .max_units(1_000_000)
        .build();

    let tree = OfferTree::build(vec![hash_offer(&offer)]).unwrap();
    let chain_id = word_from_u64(1);
    let digest = tree_digest(tree.root(), tree.height(), chain_id, &RATIFIER);
    let sig = sign_digest(&sk, &digest);
    assert!(verify(
        &offer,
        &tree.root(),
        0,
        &tree.proof(0),
        &sig,
        chain_id,
        &RATIFIER,
        &maker
    ));
}
