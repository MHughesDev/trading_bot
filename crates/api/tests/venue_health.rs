//! P2-T05 acceptance tests: venue health endpoint returns structured status.
//!
//! These tests exercise the response serialization shape and unknown-venue
//! handling.  Network pings are not exercised here (they would require live
//! connectivity); the test for bad/missing credentials is structural.

use api::routes::venue_health::VenueHealthResponse;

#[test]
fn response_serializes_ok_shape() {
    let resp = VenueHealthResponse {
        ok: true,
        latency_ms: 42,
        message: "HTTP 200 OK".into(),
    };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("\"ok\":true"));
    assert!(json.contains("\"latency_ms\":42"));
    assert!(json.contains("\"message\""));
}

#[test]
fn response_serializes_error_shape() {
    let resp = VenueHealthResponse {
        ok: false,
        latency_ms: 0,
        message: "connection error: timeout".into(),
    };
    let json = serde_json::to_string(&resp).unwrap();
    assert!(json.contains("\"ok\":false"));
    assert!(json.contains("latency_ms"));
}

#[test]
fn response_never_leaks_credential_fields() {
    let resp = VenueHealthResponse {
        ok: false,
        latency_ms: 0,
        message: "HTTP 401".into(),
    };
    let json = serde_json::to_string(&resp).unwrap();
    // Must not contain credential-related field names.
    assert!(!json.contains("api_key"));
    assert!(!json.contains("secret"));
    assert!(!json.contains("token"));
    assert!(!json.contains("password"));
}

#[test]
fn unknown_venue_slug_produces_not_ok() {
    // Structural: unknown slug should result in ok=false.
    let slug = "bitfinex";
    let result: Result<domain::SupportedVenue, _> = slug.parse();
    assert!(result.is_err(), "unknown venue must not parse");
}
