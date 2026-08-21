//! Async client for the Midnight order-book and mempool-validation API.

use std::fmt::Write as _;

use reqwest::{Client, StatusCode, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{
    market_id, Address, CollateralParams, Market, Offer, Payload, PayloadError, PayloadItem, Word,
    U256,
};

/// Default public Midnight API root.
pub const DEFAULT_MIDNIGHT_API_URL: &str = "https://api.morpho.org/v0/midnight/";

/// One side of an order book.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BookSide {
    Asks,
    Bids,
}

impl BookSide {
    fn as_str(self) -> &'static str {
        match self {
            Self::Asks => "asks",
            Self::Bids => "bids",
        }
    }
}

/// Collateral metadata returned with a book.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiCollateral {
    pub token: Address,
    pub lltv: U256,
    pub liquidation_cursor: U256,
    pub oracle: Address,
}

/// Aggregated liquidity at one tick.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceLevel {
    pub tick: u64,
    pub price: U256,
    pub units: U256,
    pub assets: U256,
    pub count: u64,
}

/// Market metadata and both top-of-book sides.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookMarket {
    pub market_id: Word,
    pub chain_id: u64,
    pub midnight: Address,
    pub loan_token: Address,
    pub collaterals: Vec<ApiCollateral>,
    pub maturity: u64,
    pub rcf_threshold: U256,
    pub enter_gate: Address,
    pub liquidator_gate: Address,
    pub asks: Vec<PriceLevel>,
    pub bids: Vec<PriceLevel>,
}

/// One executable, ratified take instruction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TakeableOffer {
    pub market_id: Word,
    pub units: U256,
    pub offer: Offer,
    pub ratifier_data: Vec<u8>,
}

/// Bundle-ready quote and signed take caps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BookQuote {
    pub average_best_price: U256,
    pub average_worst_price: U256,
    pub available_assets: U256,
    pub available_units: U256,
    pub takeable_offers: Vec<TakeableOffer>,
}

/// API policy issue for a maker payload.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidationIssue {
    pub rule: String,
    pub details: Option<serde_json::Value>,
}

/// Result of validating a complete encoded maker payload.
#[derive(Clone, Debug, PartialEq)]
pub struct ValidationResult {
    pub valid: bool,
    pub issues: Vec<ValidationIssue>,
}

/// Pagination wrapper used by book and maker-offer lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page<T> {
    pub cursor: Option<String>,
    pub data: Vec<T>,
}

/// Filters accepted by `GET /books`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BooksQuery {
    pub sort: Option<String>,
    pub maturities: Vec<u64>,
    pub collateral_tokens: Vec<Address>,
    pub loan_tokens: Vec<Address>,
    pub chain_ids: Vec<u64>,
    pub market_ids: Vec<Word>,
    pub limit: Option<u64>,
    pub cursor: Option<String>,
}

/// Target for a quote request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteTarget {
    Assets(U256),
    Units(U256),
}

/// Optional execution-price guard for a quote request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuoteGuard {
    AverageWorstPrice(U256),
    Slippage(String),
}

/// Filters accepted by the maker `GET /takeable-offers` route.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TakeableOffersQuery {
    pub market_ids: Vec<Word>,
    pub groups: Vec<Word>,
    pub limit: Option<u64>,
    pub cursor: Option<String>,
}

/// HTTP, response-shape, and payload errors from [`MidnightApi`].
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("invalid Midnight API base URL: {0}")]
    InvalidBaseUrl(String),
    #[error("Midnight API transport error: {0}")]
    Transport(#[from] reqwest::Error),
    #[error("Midnight API returned HTTP {status}: {message}")]
    Http {
        status: u16,
        code: Option<String>,
        message: String,
        details: Option<serde_json::Value>,
        request_id: Option<String>,
    },
    #[error("invalid Midnight API response: {0}")]
    InvalidResponse(String),
    #[error(transparent)]
    Payload(#[from] PayloadError),
}

/// Reusable async client for order books, quotes, executable offers, and payload validation.
#[derive(Clone, Debug)]
pub struct MidnightApi {
    base_url: Url,
    client: Client,
}

impl Default for MidnightApi {
    fn default() -> Self {
        Self::new(DEFAULT_MIDNIGHT_API_URL).expect("default Midnight API URL is valid")
    }
}

impl MidnightApi {
    /// Create a client rooted at `base_url`.
    pub fn new(base_url: impl AsRef<str>) -> Result<Self, ApiError> {
        let mut base_url = Url::parse(base_url.as_ref())
            .map_err(|error| ApiError::InvalidBaseUrl(error.to_string()))?;
        base_url.set_query(None);
        base_url.set_fragment(None);
        if !base_url.path().ends_with('/') {
            let path = format!("{}/", base_url.path());
            base_url.set_path(&path);
        }
        Ok(Self {
            base_url,
            client: Client::new(),
        })
    }

    /// Fetch active books with optional filters and pagination.
    pub async fn fetch_books(&self, query: &BooksQuery) -> Result<Page<BookMarket>, ApiError> {
        let mut params = Vec::new();
        push_opt(&mut params, "sort", query.sort.clone());
        push_csv(
            &mut params,
            "maturities",
            query.maturities.iter().map(ToString::to_string),
        );
        push_csv(
            &mut params,
            "collateral_tokens",
            query.collateral_tokens.iter().map(hex_address),
        );
        push_csv(
            &mut params,
            "loan_tokens",
            query.loan_tokens.iter().map(hex_address),
        );
        push_csv(
            &mut params,
            "chain_ids",
            query.chain_ids.iter().map(ToString::to_string),
        );
        push_csv(&mut params, "ids", query.market_ids.iter().map(hex_word));
        push_opt(
            &mut params,
            "limit",
            query.limit.map(|value| value.to_string()),
        );
        push_opt(&mut params, "cursor", query.cursor.clone());
        let response: PageWire<BookMarketWire> = self.get("books", &params).await?;
        Ok(Page {
            cursor: response.cursor,
            data: response
                .data
                .into_iter()
                .map(parse_book)
                .collect::<Result<_, _>>()?,
        })
    }

    /// Fetch one market and both sides of its book.
    pub async fn fetch_book(&self, id: Word, depth: Option<u64>) -> Result<BookMarket, ApiError> {
        let mut params = Vec::new();
        push_opt(&mut params, "depth", depth.map(|value| value.to_string()));
        let response: DataWire<BookMarketWire> = self
            .get(&format!("books/{}", hex_word(&id)), &params)
            .await?;
        parse_book(response.data)
    }

    /// Fetch aggregated levels for one book side.
    pub async fn fetch_price_levels(
        &self,
        id: Word,
        side: BookSide,
        depth: Option<u64>,
    ) -> Result<Vec<PriceLevel>, ApiError> {
        let mut params = Vec::new();
        push_opt(&mut params, "depth", depth.map(|value| value.to_string()));
        let response: DataWire<Vec<PriceLevelWire>> = self
            .get(
                &format!("books/{}/{}", hex_word(&id), side.as_str()),
                &params,
            )
            .await?;
        response.data.into_iter().map(parse_price_level).collect()
    }

    /// Fetch executable offers for one market side.
    pub async fn fetch_book_takeable_offers(
        &self,
        id: Word,
        side: BookSide,
    ) -> Result<Vec<TakeableOffer>, ApiError> {
        let response: DataWire<Vec<TakeableOfferWire>> = self
            .get(
                &format!("books/{}/{}/takeable-offers", hex_word(&id), side.as_str()),
                &[],
            )
            .await?;
        let mut takes = parse_takes(response.data)?;
        bind_takes(&takes, Some(id), Some(side), None, &[], &[])?;
        sort_takes(&mut takes, side);
        Ok(takes)
    }

    /// Fetch a target-aware, bundle-ready quote.
    pub async fn fetch_quote(
        &self,
        id: Word,
        side: BookSide,
        target: QuoteTarget,
        guard: Option<&QuoteGuard>,
    ) -> Result<BookQuote, ApiError> {
        let mut params = Vec::new();
        match target {
            QuoteTarget::Assets(value) => params.push(("assets".into(), value.to_string())),
            QuoteTarget::Units(value) => params.push(("units".into(), value.to_string())),
        }
        match guard {
            Some(QuoteGuard::AverageWorstPrice(value)) => {
                params.push(("average_worst_price".into(), value.to_string()))
            }
            Some(QuoteGuard::Slippage(value)) => params.push(("slippage".into(), value.clone())),
            None => {}
        }
        let response: DataWire<QuoteWire> = self
            .get(
                &format!("books/{}/{}/quote", hex_word(&id), side.as_str()),
                &params,
            )
            .await?;
        let average_best_price = decimal(&response.data.average_best_price, "average_best_price")?;
        let average_worst_price =
            decimal(&response.data.average_worst_price, "average_worst_price")?;
        let available_assets = decimal(&response.data.available_assets, "available_assets")?;
        let available_units = decimal(&response.data.available_units, "available_units")?;
        let mut takes = parse_takes(response.data.takeable_offers)?;
        bind_takes(&takes, Some(id), Some(side), None, &[], &[])?;
        sort_takes(&mut takes, side);
        let effective_guard = match guard {
            Some(QuoteGuard::AverageWorstPrice(value)) => Some(*value),
            Some(QuoteGuard::Slippage(_)) => Some(average_worst_price),
            None => None,
        };
        if let Some(guard) = effective_guard {
            verify_quote_guard(&takes, side, target, guard)?;
        }
        Ok(BookQuote {
            average_best_price,
            average_worst_price,
            available_assets,
            available_units,
            takeable_offers: takes,
        })
    }

    /// Fetch one maker's active executable offers.
    pub async fn fetch_takeable_offers(
        &self,
        maker: Address,
        query: &TakeableOffersQuery,
    ) -> Result<Page<TakeableOffer>, ApiError> {
        let mut params = vec![("maker".into(), hex_address(&maker))];
        push_csv(
            &mut params,
            "market_ids",
            query.market_ids.iter().map(hex_word),
        );
        push_csv(&mut params, "groups", query.groups.iter().map(hex_word));
        push_opt(
            &mut params,
            "limit",
            query.limit.map(|value| value.to_string()),
        );
        push_opt(&mut params, "cursor", query.cursor.clone());
        let response: PageWire<TakeableOfferWire> = self.get("takeable-offers", &params).await?;
        let takes = parse_takes(response.data)?;
        bind_takes(
            &takes,
            None,
            None,
            Some(maker),
            &query.market_ids,
            &query.groups,
        )?;
        Ok(Page {
            cursor: response.cursor,
            data: takes,
        })
    }

    /// Validate already encoded payload bytes against current API policy.
    pub async fn validate_payload(
        &self,
        chain_id: u64,
        payload: &[u8],
        timestamp: Option<&str>,
    ) -> Result<ValidationResult, ApiError> {
        let mut params = Vec::new();
        push_opt(&mut params, "timestamp", timestamp.map(str::to_owned));
        let response: serde_json::Value = self
            .post(
                "mempool/validate",
                &params,
                &ValidationRequest {
                    chain_id,
                    payload: format!("0x{}", hex::encode(payload)),
                },
            )
            .await?;
        parse_validation(response)
    }

    /// Encode payload-ready items and validate the exact bytes that would be published.
    pub async fn validate_items(
        &self,
        chain_id: u64,
        items: &[PayloadItem],
        timestamp: Option<&str>,
    ) -> Result<ValidationResult, ApiError> {
        let payload = Payload::encode(items)?;
        self.validate_payload(chain_id, &payload, timestamp).await
    }

    async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &[(String, String)],
    ) -> Result<T, ApiError> {
        self.request(self.client.get(self.url(path, query)?)).await
    }

    async fn post<T: DeserializeOwned, B: Serialize>(
        &self,
        path: &str,
        query: &[(String, String)],
        body: &B,
    ) -> Result<T, ApiError> {
        self.request(self.client.post(self.url(path, query)?).json(body))
            .await
    }

    fn url(&self, path: &str, query: &[(String, String)]) -> Result<Url, ApiError> {
        let mut url = self
            .base_url
            .join(path.trim_start_matches('/'))
            .map_err(|error| ApiError::InvalidBaseUrl(error.to_string()))?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        Ok(url)
    }

    async fn request<T: DeserializeOwned>(
        &self,
        request: reqwest::RequestBuilder,
    ) -> Result<T, ApiError> {
        let response = request
            .header("sdk-version", env!("CARGO_PKG_VERSION"))
            .send()
            .await?;
        let status = response.status();
        let bytes = response.bytes().await?;
        if !status.is_success() {
            return Err(parse_http_error(status, &bytes));
        }
        serde_json::from_slice(&bytes).map_err(|error| ApiError::InvalidResponse(error.to_string()))
    }
}

#[derive(Deserialize)]
struct DataWire<T> {
    data: T,
}

#[derive(Deserialize)]
struct PageWire<T> {
    cursor: Option<String>,
    data: Vec<T>,
}

#[derive(Deserialize)]
struct CollateralWire {
    token: String,
    lltv: String,
    liquidation_cursor: String,
    oracle: String,
}

#[derive(Deserialize)]
struct PriceLevelWire {
    tick: u64,
    price: String,
    units: String,
    assets: String,
    count: u64,
}

#[derive(Deserialize)]
struct BookMarketWire {
    market_id: String,
    chain_id: u64,
    midnight: String,
    loan_token: String,
    collaterals: Vec<CollateralWire>,
    maturity: u64,
    rcf_threshold: String,
    enter_gate: String,
    liquidator_gate: String,
    asks: Vec<PriceLevelWire>,
    bids: Vec<PriceLevelWire>,
}

#[derive(Deserialize)]
struct OfferMarketWire {
    chain_id: u64,
    midnight: String,
    loan_token: String,
    collaterals: Vec<CollateralWire>,
    maturity: u64,
    rcf_threshold: String,
    enter_gate: String,
    liquidator_gate: String,
}

#[derive(Deserialize)]
struct OfferWire {
    market: OfferMarketWire,
    buy: bool,
    maker: String,
    start: u64,
    expiry: u64,
    tick: u64,
    group: String,
    callback: String,
    callback_data: String,
    receiver_if_maker_is_seller: String,
    ratifier: String,
    reduce_only: bool,
    max_units: String,
    max_assets: String,
    continuous_fee_cap: String,
}

#[derive(Deserialize)]
struct TakeableOfferWire {
    market_id: String,
    units: String,
    offer: OfferWire,
    ratifier_data: String,
}

#[derive(Deserialize)]
struct QuoteWire {
    average_best_price: String,
    average_worst_price: String,
    available_assets: String,
    available_units: String,
    takeable_offers: Vec<TakeableOfferWire>,
}

#[derive(Serialize)]
struct ValidationRequest {
    chain_id: u64,
    payload: String,
}

fn parse_book(book: BookMarketWire) -> Result<BookMarket, ApiError> {
    Ok(BookMarket {
        market_id: fixed_hex(&book.market_id, "market_id")?,
        chain_id: book.chain_id,
        midnight: fixed_hex(&book.midnight, "midnight")?,
        loan_token: fixed_hex(&book.loan_token, "loan_token")?,
        collaterals: book
            .collaterals
            .into_iter()
            .map(parse_collateral)
            .collect::<Result<_, _>>()?,
        maturity: book.maturity,
        rcf_threshold: decimal(&book.rcf_threshold, "rcf_threshold")?,
        enter_gate: fixed_hex(&book.enter_gate, "enter_gate")?,
        liquidator_gate: fixed_hex(&book.liquidator_gate, "liquidator_gate")?,
        asks: book
            .asks
            .into_iter()
            .map(parse_price_level)
            .collect::<Result<_, _>>()?,
        bids: book
            .bids
            .into_iter()
            .map(parse_price_level)
            .collect::<Result<_, _>>()?,
    })
}

fn parse_collateral(value: CollateralWire) -> Result<ApiCollateral, ApiError> {
    Ok(ApiCollateral {
        token: fixed_hex(&value.token, "collateral.token")?,
        lltv: decimal(&value.lltv, "collateral.lltv")?,
        liquidation_cursor: decimal(&value.liquidation_cursor, "collateral.liquidation_cursor")?,
        oracle: fixed_hex(&value.oracle, "collateral.oracle")?,
    })
}

fn parse_price_level(value: PriceLevelWire) -> Result<PriceLevel, ApiError> {
    Ok(PriceLevel {
        tick: value.tick,
        price: decimal(&value.price, "price")?,
        units: decimal(&value.units, "units")?,
        assets: decimal(&value.assets, "assets")?,
        count: value.count,
    })
}

fn parse_takes(values: Vec<TakeableOfferWire>) -> Result<Vec<TakeableOffer>, ApiError> {
    values.into_iter().map(parse_take).collect()
}

fn parse_take(value: TakeableOfferWire) -> Result<TakeableOffer, ApiError> {
    let market = Market {
        chain_id: U256::from(value.offer.market.chain_id).to_be_bytes(),
        midnight: fixed_hex(&value.offer.market.midnight, "offer.market.midnight")?,
        loan_token: fixed_hex(&value.offer.market.loan_token, "offer.market.loan_token")?,
        collateral_params: value
            .offer
            .market
            .collaterals
            .into_iter()
            .map(|item| {
                Ok(CollateralParams {
                    token: fixed_hex(&item.token, "offer.market.collateral.token")?,
                    lltv: decimal(&item.lltv, "offer.market.collateral.lltv")?.to_be_bytes(),
                    liquidation_cursor: decimal(
                        &item.liquidation_cursor,
                        "offer.market.collateral.liquidation_cursor",
                    )?
                    .to_be_bytes(),
                    oracle: fixed_hex(&item.oracle, "offer.market.collateral.oracle")?,
                })
            })
            .collect::<Result<_, ApiError>>()?,
        maturity: U256::from(value.offer.market.maturity).to_be_bytes(),
        rcf_threshold: decimal(
            &value.offer.market.rcf_threshold,
            "offer.market.rcf_threshold",
        )?
        .to_be_bytes(),
        enter_gate: fixed_hex(&value.offer.market.enter_gate, "offer.market.enter_gate")?,
        liquidator_gate: fixed_hex(
            &value.offer.market.liquidator_gate,
            "offer.market.liquidator_gate",
        )?,
    };
    let max_units = decimal(&value.offer.max_units, "offer.max_units")?
        .try_into()
        .map_err(|_| ApiError::InvalidResponse("offer.max_units exceeds uint128".into()))?;
    let max_assets = decimal(&value.offer.max_assets, "offer.max_assets")?
        .try_into()
        .map_err(|_| ApiError::InvalidResponse("offer.max_assets exceeds uint128".into()))?;
    Ok(TakeableOffer {
        market_id: fixed_hex(&value.market_id, "market_id")?,
        units: decimal(&value.units, "units")?,
        offer: Offer {
            market,
            buy: value.offer.buy,
            maker: fixed_hex(&value.offer.maker, "offer.maker")?,
            start: U256::from(value.offer.start).to_be_bytes(),
            expiry: U256::from(value.offer.expiry).to_be_bytes(),
            tick: U256::from(value.offer.tick).to_be_bytes(),
            group: fixed_hex(&value.offer.group, "offer.group")?,
            callback: fixed_hex(&value.offer.callback, "offer.callback")?,
            callback_data: bytes_hex(&value.offer.callback_data, "offer.callback_data")?,
            receiver_if_maker_is_seller: fixed_hex(
                &value.offer.receiver_if_maker_is_seller,
                "offer.receiver_if_maker_is_seller",
            )?,
            ratifier: fixed_hex(&value.offer.ratifier, "offer.ratifier")?,
            reduce_only: value.offer.reduce_only,
            max_units,
            max_assets,
            continuous_fee_cap: decimal(
                &value.offer.continuous_fee_cap,
                "offer.continuous_fee_cap",
            )?
            .to_be_bytes(),
        },
        ratifier_data: bytes_hex(&value.ratifier_data, "ratifier_data")?,
    })
}

fn bind_takes(
    takes: &[TakeableOffer],
    requested_market: Option<Word>,
    side: Option<BookSide>,
    maker: Option<Address>,
    market_ids: &[Word],
    groups: &[Word],
) -> Result<(), ApiError> {
    for take in takes {
        if market_id(&take.offer.market) != take.market_id {
            return Err(ApiError::InvalidResponse(
                "takeable offer market_id does not match its embedded market".into(),
            ));
        }
        if requested_market.is_some_and(|id| id != take.market_id)
            || (!market_ids.is_empty() && !market_ids.contains(&take.market_id))
        {
            return Err(ApiError::InvalidResponse(
                "takeable offer is outside the requested market filter".into(),
            ));
        }
        if side.is_some_and(|side| take.offer.buy != matches!(side, BookSide::Bids)) {
            return Err(ApiError::InvalidResponse(
                "takeable offer side does not match the requested book side".into(),
            ));
        }
        if maker.is_some_and(|maker| maker != take.offer.maker) {
            return Err(ApiError::InvalidResponse(
                "takeable offer maker does not match the requested maker".into(),
            ));
        }
        if !groups.is_empty() && !groups.contains(&take.offer.group) {
            return Err(ApiError::InvalidResponse(
                "takeable offer is outside the requested group filter".into(),
            ));
        }
    }
    Ok(())
}

fn sort_takes(takes: &mut [TakeableOffer], side: BookSide) {
    takes.sort_by(|a, b| {
        let order = a.offer.tick.cmp(&b.offer.tick);
        match side {
            BookSide::Asks => order,
            BookSide::Bids => order.reverse(),
        }
    });
}

fn verify_quote_guard(
    takes: &[TakeableOffer],
    side: BookSide,
    target: QuoteTarget,
    guard: U256,
) -> Result<(), ApiError> {
    let wad = U256::from(1_000_000_000_000_000_000u128);
    let mut filled_units = U256::ZERO;
    let mut weighted_price = U256::ZERO;

    match target {
        QuoteTarget::Units(mut remaining) => {
            for take in takes {
                if remaining == U256::ZERO {
                    break;
                }
                let filled = take.units.min(remaining);
                let price = take_price(take)?;
                accumulate_quote(&mut filled_units, &mut weighted_price, filled, price)?;
                remaining -= filled;
            }
        }
        QuoteTarget::Assets(mut remaining) => {
            for take in takes {
                if remaining == U256::ZERO {
                    break;
                }
                let price = take_price(take)?;
                if price == U256::ZERO {
                    continue;
                }
                let product = take
                    .units
                    .checked_mul(price)
                    .ok_or_else(|| ApiError::InvalidResponse("quote amount overflow".into()))?;
                let take_assets = match side {
                    BookSide::Asks => div_up(product, wad),
                    BookSide::Bids => product / wad,
                };
                if take_assets == U256::ZERO {
                    continue;
                }
                let fills_entire_take = take_assets <= remaining;
                let filled = if fills_entire_take {
                    take.units
                } else {
                    let numerator = remaining
                        .checked_mul(wad)
                        .ok_or_else(|| ApiError::InvalidResponse("quote amount overflow".into()))?;
                    match side {
                        BookSide::Asks => numerator / price,
                        BookSide::Bids => div_up(numerator, price),
                    }
                };
                if filled == U256::ZERO {
                    continue;
                }
                accumulate_quote(&mut filled_units, &mut weighted_price, filled, price)?;
                remaining = if fills_entire_take {
                    remaining - take_assets
                } else {
                    U256::ZERO
                };
            }
        }
    }

    if filled_units != U256::ZERO {
        let average = weighted_price / filled_units;
        let violates = match side {
            BookSide::Asks => average > guard,
            BookSide::Bids => average < guard,
        };
        if violates {
            return Err(ApiError::InvalidResponse(format!(
                "quote takeable offers imply average price {average} outside average_worst_price guard {guard}"
            )));
        }
    }
    Ok(())
}

fn take_price(take: &TakeableOffer) -> Result<U256, ApiError> {
    let tick: u64 = crate::word_to_u256(&take.offer.tick)
        .try_into()
        .map_err(|_| ApiError::InvalidResponse("quote tick exceeds u64".into()))?;
    crate::tick_to_price(tick).map_err(|error| ApiError::InvalidResponse(error.to_string()))
}

fn accumulate_quote(
    filled_units: &mut U256,
    weighted_price: &mut U256,
    filled: U256,
    price: U256,
) -> Result<(), ApiError> {
    *filled_units = filled_units
        .checked_add(filled)
        .ok_or_else(|| ApiError::InvalidResponse("quote units overflow".into()))?;
    let weighted = filled
        .checked_mul(price)
        .ok_or_else(|| ApiError::InvalidResponse("quote weighted price overflow".into()))?;
    *weighted_price = weighted_price
        .checked_add(weighted)
        .ok_or_else(|| ApiError::InvalidResponse("quote weighted price overflow".into()))?;
    Ok(())
}

fn div_up(value: U256, divisor: U256) -> U256 {
    value / divisor + U256::from(value % divisor != U256::ZERO)
}

fn parse_validation(value: serde_json::Value) -> Result<ValidationResult, ApiError> {
    let issues = value
        .get("data")
        .and_then(|data| data.get("issues"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            ApiError::InvalidResponse("validation response is missing data.issues".into())
        })?;
    let issues = issues
        .iter()
        .map(|issue| {
            let rule = issue
                .get("rule")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    ApiError::InvalidResponse("validation issue is missing rule".into())
                })?;
            Ok(ValidationIssue {
                rule: rule.into(),
                details: issue
                    .get("details")
                    .filter(|value| !value.is_null())
                    .cloned(),
            })
        })
        .collect::<Result<Vec<_>, ApiError>>()?;
    Ok(ValidationResult {
        valid: issues.is_empty(),
        issues,
    })
}

fn parse_http_error(status: StatusCode, bytes: &[u8]) -> ApiError {
    let json: Option<serde_json::Value> = serde_json::from_slice(bytes).ok();
    let error = json.as_ref().and_then(|value| value.get("error"));
    ApiError::Http {
        status: status.as_u16(),
        code: error
            .and_then(|value| value.get("code"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        message: error
            .and_then(|value| value.get("message"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| status.canonical_reason().unwrap_or("request failed"))
            .to_owned(),
        details: error.and_then(|value| value.get("details")).cloned(),
        request_id: error
            .and_then(|value| value.get("request_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    }
}

fn decimal(value: &str, field: &str) -> Result<U256, ApiError> {
    value
        .parse()
        .map_err(|_| ApiError::InvalidResponse(format!("{field} is not a uint256 decimal")))
}

fn bytes_hex(value: &str, field: &str) -> Result<Vec<u8>, ApiError> {
    let value = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
        .ok_or_else(|| ApiError::InvalidResponse(format!("{field} is missing 0x prefix")))?;
    hex::decode(value)
        .map_err(|_| ApiError::InvalidResponse(format!("{field} is not byte-aligned hex")))
}

fn fixed_hex<const N: usize>(value: &str, field: &str) -> Result<[u8; N], ApiError> {
    let bytes = bytes_hex(value, field)?;
    bytes
        .try_into()
        .map_err(|_| ApiError::InvalidResponse(format!("{field} must contain {N} bytes")))
}

fn hex_address(value: &Address) -> String {
    format!("0x{}", hex::encode(value))
}
fn hex_word(value: &Word) -> String {
    format!("0x{}", hex::encode(value))
}

fn push_opt(params: &mut Vec<(String, String)>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        params.push((key.into(), value));
    }
}

fn push_csv(params: &mut Vec<(String, String)>, key: &str, values: impl Iterator<Item = String>) {
    let mut value = String::new();
    for item in values {
        if !value.is_empty() {
            value.push(',');
        }
        let _ = write!(value, "{item}");
    }
    if !value.is_empty() {
        params.push((key.into(), value));
    }
}
