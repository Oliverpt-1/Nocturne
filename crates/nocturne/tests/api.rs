use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    thread,
};

use nocturne::*;
use serde_json::json;

fn serve_once(status: u16, body: serde_json::Value) -> (String, mpsc::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        let header_end;
        loop {
            let read = stream.read(&mut buffer).unwrap();
            assert_ne!(read, 0);
            request.extend_from_slice(&buffer[..read]);
            if let Some(position) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                header_end = position + 4;
                break;
            }
        }
        let headers = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(str::trim)
                    .map(str::parse::<usize>)
            })
            .transpose()
            .unwrap()
            .unwrap_or(0);
        while request.len() < header_end + content_length {
            let read = stream.read(&mut buffer).unwrap();
            assert_ne!(read, 0);
            request.extend_from_slice(&buffer[..read]);
        }
        let _ = sender.send(String::from_utf8(request).unwrap());

        let body = serde_json::to_vec(&body).unwrap();
        let reason = if status == 200 { "OK" } else { "Error" };
        write!(stream, "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
        stream.write_all(&body).unwrap();
    });
    (format!("http://{address}/v0/midnight"), receiver)
}

fn hex<const N: usize>(value: &[u8; N]) -> String {
    format!("0x{}", ::hex::encode(value))
}

fn fixture_market() -> Market {
    Market {
        chain_id: U256::from(BASE_CHAIN_ID).to_be_bytes(),
        midnight: [0x10; 20],
        loan_token: [0x20; 20],
        collateral_params: vec![CollateralParams {
            token: [0x30; 20],
            lltv: U256::from(770_000_000_000_000_000u64).to_be_bytes(),
            liquidation_cursor: U256::from(250_000_000_000_000_000u64).to_be_bytes(),
            oracle: [0x40; 20],
        }],
        maturity: U256::from(54_000).to_be_bytes(),
        rcf_threshold: [0; 32],
        enter_gate: [0; 20],
        liquidator_gate: [0; 20],
    }
}

fn take_json(tick: u64, buy: bool) -> serde_json::Value {
    let market = fixture_market();
    json!({
        "market_id": hex(&market_id(&market)),
        "units": "50",
        "offer": {
            "market": {
                "chain_id": BASE_CHAIN_ID,
                "midnight": hex(&market.midnight),
                "loan_token": hex(&market.loan_token),
                "collaterals": [{
                    "token": hex(&market.collateral_params[0].token),
                    "lltv": "770000000000000000",
                    "liquidation_cursor": "250000000000000000",
                    "oracle": hex(&market.collateral_params[0].oracle)
                }],
                "maturity": 54000,
                "rcf_threshold": "0",
                "enter_gate": hex(&market.enter_gate),
                "liquidator_gate": hex(&market.liquidator_gate)
            },
            "buy": buy,
            "maker": hex(&[0x50; 20]),
            "start": 1000,
            "expiry": 3600,
            "tick": tick,
            "group": hex(&[0x60; 32]),
            "callback": hex(&[0; 20]),
            "callback_data": "0x",
            "receiver_if_maker_is_seller": if buy { hex(&[0; 20]) } else { hex(&[0x51; 20]) },
            "ratifier": hex(&[0x70; 20]),
            "reduce_only": false,
            "max_units": "100",
            "max_assets": "0",
            "continuous_fee_cap": "123"
        },
        "ratifier_data": "0xdeadbeef"
    })
}

#[tokio::test]
async fn validates_the_exact_payload_and_preserves_issues() {
    let (base, request) = serve_once(
        200,
        json!({
            "data": { "issues": [{ "rule": "future_rule", "details": { "new": true } }] }
        }),
    );
    let api = MidnightApi::new(base).unwrap();
    let result = api
        .validate_payload(BASE_CHAIN_ID, &[1, 2, 3], Some("2026-08-21T12:00:00Z"))
        .await
        .unwrap();
    assert!(!result.valid);
    assert_eq!(result.issues[0].rule, "future_rule");
    assert_eq!(result.issues[0].details, Some(json!({ "new": true })));

    let request = request.recv().unwrap();
    assert!(request.starts_with(
        "POST /v0/midnight/mempool/validate?timestamp=2026-08-21T12%3A00%3A00Z HTTP/1.1"
    ));
    assert!(request.to_ascii_lowercase().contains("sdk-version: 0.1.0"));
    assert!(request.ends_with(r#"{"chain_id":8453,"payload":"0x010203"}"#));
}

#[tokio::test]
async fn fetches_and_maps_filtered_books() {
    let id = [0x11; 32];
    let (base, request) = serve_once(
        200,
        json!({
            "cursor": "next",
            "data": [{
                "market_id": hex(&id), "chain_id": 8453,
                "midnight": hex(&[0x10; 20]), "loan_token": hex(&[0x20; 20]),
                "collaterals": [{ "token": hex(&[0x30; 20]), "lltv": "770", "liquidation_cursor": "250", "oracle": hex(&[0x40; 20]) }],
                "maturity": 54000, "rcf_threshold": "7", "enter_gate": hex(&[0; 20]), "liquidator_gate": hex(&[0; 20]),
                "asks": [{ "tick": 10, "price": "20", "units": "30", "assets": "40", "count": 2 }], "bids": []
            }]
        }),
    );
    let api = MidnightApi::new(base).unwrap();
    let page = api
        .fetch_books(&BooksQuery {
            chain_ids: vec![8453],
            market_ids: vec![id],
            limit: Some(10),
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page.cursor.as_deref(), Some("next"));
    assert_eq!(page.data[0].market_id, id);
    assert_eq!(page.data[0].asks[0].assets, U256::from(40));
    let request = request.recv().unwrap();
    assert!(request.starts_with(&format!(
        "GET /v0/midnight/books?chain_ids=8453&ids={}&limit=10 HTTP/1.1",
        hex(&id)
    )));
}

#[tokio::test]
async fn executable_offers_are_bound_and_sorted_for_the_requested_side() {
    let id = market_id(&fixture_market());
    let (base, _) = serve_once(
        200,
        json!({ "data": [take_json(200, false), take_json(100, false)] }),
    );
    let takes = MidnightApi::new(base)
        .unwrap()
        .fetch_book_takeable_offers(id, BookSide::Asks)
        .await
        .unwrap();
    assert_eq!(word_to_u256(&takes[0].offer.tick), U256::from(100));
    assert_eq!(takes[0].ratifier_data, [0xde, 0xad, 0xbe, 0xef]);

    let (base, _) = serve_once(200, json!({ "data": [take_json(100, true)] }));
    let error = MidnightApi::new(base)
        .unwrap()
        .fetch_book_takeable_offers(id, BookSide::Asks)
        .await
        .unwrap_err();
    assert!(matches!(error, ApiError::InvalidResponse(message) if message.contains("side")));
}

#[tokio::test]
async fn quote_price_guard_is_recomputed_from_returned_takes() {
    let id = market_id(&fixture_market());
    let (base, _) = serve_once(
        200,
        json!({ "data": {
            "average_best_price": "0", "average_worst_price": "0",
            "available_assets": "50", "available_units": "50",
            "takeable_offers": [take_json(100, false)]
        }}),
    );
    let error = MidnightApi::new(base)
        .unwrap()
        .fetch_quote(
            id,
            BookSide::Asks,
            QuoteTarget::Units(U256::from(10)),
            Some(&QuoteGuard::AverageWorstPrice(U256::ZERO)),
        )
        .await
        .unwrap_err();
    assert!(
        matches!(error, ApiError::InvalidResponse(message) if message.contains("outside average_worst_price"))
    );
}

#[tokio::test]
async fn structured_http_errors_are_not_discarded() {
    let (base, _) = serve_once(
        422,
        json!({ "error": {
            "code": "bad_payload", "message": "payload rejected", "details": { "field": "payload" }, "request_id": "req-1"
        }}),
    );
    let error = MidnightApi::new(base)
        .unwrap()
        .validate_payload(8453, &[1], None)
        .await
        .unwrap_err();
    assert!(
        matches!(error, ApiError::Http { status: 422, code: Some(code), request_id: Some(id), .. } if code == "bad_payload" && id == "req-1")
    );
}
