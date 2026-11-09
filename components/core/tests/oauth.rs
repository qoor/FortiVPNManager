//! Google OAuth formats: PKCE, URLs, token answers, client files and the loopback redirect
//! (SPEC F-GA-01, F-GA-02).

use std::collections::BTreeMap;
use std::time::Duration;

use fvm_contract::app_internal::SignInPages;
use fvm_contract::app_internal::section::OAuthClientFileProblem;
use fvm_core::oauth::{
    CallbackDecision, GMAIL_MODIFY_SCOPE, HeadParse, LoopbackRequest, Pkce, TokenError,
    authorization_code_body, authorization_url, form_encoded, loopback_redirect_uri,
    parse_client_file, parse_request_head, read_token_response, refresh_body, revocation_body,
    state_from_random,
};

fn request(text: &str) -> HeadParse {
    parse_request_head(text.as_bytes())
}

/// Splits an `application/x-www-form-urlencoded` body into decoded pairs.
fn form_pairs(body: &str) -> BTreeMap<String, String> {
    let HeadParse::Request(parsed) = request(&format!("GET /?{body} HTTP/1.1\r\n\r\n")) else {
        panic!("unparsable body {body}")
    };
    parsed.query.into_iter().collect()
}

fn pairs(items: &[(&str, &str)]) -> BTreeMap<String, String> {
    items
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn get(path: &str, query: &[(&str, &str)]) -> HeadParse {
    HeadParse::Request(LoopbackRequest {
        method: "GET".into(),
        path: path.into(),
        query: query
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    })
}

// MARK: PKCE and URLs

#[test]
fn the_rfc7636_vector() {
    let pkce = Pkce::from_verifier("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk").unwrap();
    assert_eq!(
        pkce.challenge(),
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
    );
}

#[test]
fn verifiers_from_random_bytes_are_long_and_unreserved() {
    let first = Pkce::from_random(&[7; 48]);
    let second = Pkce::from_random(&[8; 48]);
    assert_eq!(first.verifier().len(), 64);
    assert_ne!(first.verifier(), second.verifier());
    assert!(
        first
            .verifier()
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    );
    assert_eq!(first.challenge().len(), 43);
    assert!(!first.challenge().contains(['=', '+', '/']));
    assert_eq!(
        Pkce::from_verifier(first.verifier()).unwrap().challenge(),
        first.challenge()
    );
}

#[test]
fn verifier_length_and_alphabet() {
    for (verifier, valid) in [
        ("a".repeat(42), false),
        ("a".repeat(43), true),
        ("~".repeat(128), true),
        ("a".repeat(129), false),
        ("a".repeat(42) + "=", false),
        ("a".repeat(42) + " ", false),
        ("a".repeat(42) + "\u{E9}", false),
        ("A-._~".to_owned() + &"0".repeat(40), true),
    ] {
        assert_eq!(
            Pkce::from_verifier(&verifier).is_some(),
            valid,
            "{verifier}"
        );
    }
}

#[test]
fn states_are_base64url_of_the_random_bytes() {
    let state = state_from_random(&[0xFB; 24]);
    assert_eq!(state.len(), 32);
    assert_ne!(state, state_from_random(&[0xFA; 24]));
    assert!(
        state
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    );
}

#[test]
fn percent_encoding() {
    for (raw, encoded) in [
        ("abcXYZ019-._~", "abcXYZ019-._~"),
        ("a b+c/d@e:f?g&h=i", "a%20b%2Bc%2Fd%40e%3Af%3Fg%26h%3Di"),
        ("\u{D55C}", "%ED%95%9C"),
        ("\n", "%0A"),
    ] {
        assert_eq!(form_encoded(&[("k", raw)]), format!("k={encoded}"));
    }
}

#[test]
fn the_loopback_redirect_uri() {
    assert_eq!(loopback_redirect_uri(53122), "http://127.0.0.1:53122");
}

#[test]
fn the_authorization_url_carries_every_parameter() {
    let url = authorization_url(
        "123-abc.apps.googleusercontent.com",
        "http://127.0.0.1:53122",
        "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
        "st-ate_1",
        Some("me+otp@example.com"),
    );
    let query = url
        .strip_prefix("https://accounts.google.com/o/oauth2/v2/auth?")
        .unwrap();
    // "+" must be escaped or Google reads a space.
    assert!(query.contains("login_hint=me%2Botp%40example.com"));
    assert_eq!(
        form_pairs(query),
        pairs(&[
            ("client_id", "123-abc.apps.googleusercontent.com"),
            ("redirect_uri", "http://127.0.0.1:53122"),
            ("response_type", "code"),
            ("scope", "https://www.googleapis.com/auth/gmail.modify"),
            (
                "code_challenge",
                "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
            ),
            ("code_challenge_method", "S256"),
            ("state", "st-ate_1"),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("login_hint", "me+otp@example.com"),
        ])
    );
}

#[test]
fn the_login_hint_is_optional() {
    for hint in [None, Some("")] {
        let url = authorization_url("c", "http://127.0.0.1:1", "x", "s", hint);
        let names = form_pairs(url.split_once('?').unwrap().1);
        assert!(!names.contains_key("login_hint"));
        assert_eq!(names.len(), 9);
    }
}

#[test]
fn token_request_bodies() {
    let exchange = authorization_code_body(
        "4/0Ab_cd+ef",
        "cid",
        "s3cr&t",
        "http://127.0.0.1:9",
        "verifier",
    );
    assert_eq!(
        form_pairs(&exchange.0),
        pairs(&[
            ("grant_type", "authorization_code"),
            ("code", "4/0Ab_cd+ef"),
            ("client_id", "cid"),
            ("client_secret", "s3cr&t"),
            ("redirect_uri", "http://127.0.0.1:9"),
            ("code_verifier", "verifier"),
        ])
    );
    let refresh = refresh_body("1//rt", "cid", "sec");
    assert_eq!(
        form_pairs(&refresh.0),
        pairs(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", "1//rt"),
            ("client_id", "cid"),
            ("client_secret", "sec"),
        ])
    );
    assert_eq!(revocation_body("1//rt").0, "token=1%2F%2Frt");
}

// MARK: Token answers

#[test]
fn an_authorization_code_grant() {
    let body = br#"{"access_token": "ya29.a0Af", "expires_in": 3599, "refresh_token": "1//0g-rt",
        "scope": "https://www.googleapis.com/auth/gmail.modify openid", "token_type": "Bearer"}"#;
    let grant = read_token_response(200, body).unwrap();
    assert_eq!(grant.access_token.0, "ya29.a0Af");
    assert_eq!(grant.expires_in, Duration::from_secs(3599));
    assert_eq!(
        grant.refresh_token.as_ref().map(|t| t.0.as_str()),
        Some("1//0g-rt")
    );
    assert_eq!(
        grant.scopes.iter().map(String::as_str).collect::<Vec<_>>(),
        ["https://www.googleapis.com/auth/gmail.modify", "openid"]
    );
    assert!(grant.grants(GMAIL_MODIFY_SCOPE));
    let debug = format!("{grant:?}");
    assert!(!debug.contains("ya29") && !debug.contains("1//0g"));
}

#[test]
fn a_refresh_grant_has_no_refresh_token() {
    let grant = read_token_response(
        200,
        br#"{"access_token": "ya29.b", "expires_in": 3599, "token_type": "Bearer"}"#,
    )
    .unwrap();
    assert!(grant.refresh_token.is_none());
    assert!(grant.scopes.is_empty());
    let empty = read_token_response(
        200,
        br#"{"access_token": "a", "expires_in": 10, "refresh_token": ""}"#,
    )
    .unwrap();
    assert!(empty.refresh_token.is_none());
}

#[test]
fn token_error_answers() {
    let some = |text: &str| Some(text.to_owned());
    for (body, status, expected) in [
        (
            r#"{"error": "invalid_grant", "error_description": "Token has been expired or revoked."}"#,
            400,
            TokenError::InvalidGrant {
                description: some("Token has been expired or revoked."),
            },
        ),
        (
            r#"{"error": "invalid_grant"}"#,
            400,
            TokenError::InvalidGrant { description: None },
        ),
        (
            r#"{"error": "invalid_client", "error_description": "Unauthorized"}"#,
            401,
            TokenError::InvalidClient {
                description: some("Unauthorized"),
            },
        ),
        (
            r#"{"error": "unauthorized_client"}"#,
            400,
            TokenError::InvalidClient { description: None },
        ),
        (
            r#"{"error": "invalid_request", "error_description": "Missing code verifier."}"#,
            400,
            TokenError::OAuth {
                code: "invalid_request".into(),
                description: some("Missing code verifier."),
            },
        ),
        (
            "<html>Bad Gateway</html>",
            502,
            TokenError::Server { status: 502 },
        ),
        ("{}", 500, TokenError::Server { status: 500 }),
        ("{}", 200, TokenError::MalformedResponse),
        (
            r#"{"access_token": "", "expires_in": 3599}"#,
            200,
            TokenError::MalformedResponse,
        ),
        (
            r#"{"access_token": "a", "expires_in": 0}"#,
            200,
            TokenError::MalformedResponse,
        ),
        (
            r#"{"access_token": "a"}"#,
            200,
            TokenError::MalformedResponse,
        ),
        ("not json", 200, TokenError::MalformedResponse),
    ] {
        assert_eq!(
            read_token_response(status, body.as_bytes()).err(),
            Some(expected),
            "{body}"
        );
    }
}

// MARK: Client files

const CLIENT_ID: &str = "123456789012-abcdefghijklmnopqrstuvwxyz012345.apps.googleusercontent.com";
const SECRET: &str = "GOCSPX-Test_Only-NotARealSecret12345";

fn complete() -> serde_json::Map<String, serde_json::Value> {
    serde_json::json!({
        "client_id": CLIENT_ID,
        "project_id": "example-123",
        "auth_uri": "https://accounts.google.com/o/oauth2/auth",
        "token_uri": "https://oauth2.googleapis.com/token",
        "auth_provider_x509_cert_url": "https://www.googleapis.com/oauth2/v1/certs",
        "client_secret": SECRET,
        "redirect_uris": ["http://localhost"],
    })
    .as_object()
    .unwrap()
    .clone()
}

fn installed(fields: serde_json::Map<String, serde_json::Value>) -> Vec<u8> {
    serde_json::json!({ "installed": fields })
        .to_string()
        .into_bytes()
}

#[test]
fn a_desktop_client() {
    let client = parse_client_file(&installed(complete())).unwrap();
    assert_eq!(client.client_id, CLIENT_ID);
    assert_eq!(client.client_secret.0, SECRET);
    let debug = format!("{client:?}");
    assert!(!debug.contains(SECRET));
    assert!(debug.contains(CLIENT_ID));
}

#[test]
fn a_minimal_desktop_client() {
    let mut fields = serde_json::Map::new();
    fields.insert("client_id".into(), CLIENT_ID.into());
    fields.insert("client_secret".into(), SECRET.into());
    assert_eq!(
        parse_client_file(&installed(fields)).unwrap().client_id,
        CLIENT_ID
    );
}

#[test]
fn a_web_client_is_refused() {
    let web = serde_json::json!({ "web": complete() }).to_string();
    assert_eq!(
        parse_client_file(web.as_bytes()),
        Err(OAuthClientFileProblem::WebClient {})
    );
}

#[test]
fn not_a_client_file() {
    for text in ["", "[]", "\"installed\"", "{", "null"] {
        assert_eq!(
            parse_client_file(text.as_bytes()),
            Err(OAuthClientFileProblem::NotJson {}),
            "{text}"
        );
    }
}

#[test]
fn a_missing_section() {
    for text in [r#"{"other": {}}"#, r#"{"installed": "x"}"#] {
        assert_eq!(
            parse_client_file(text.as_bytes()),
            Err(OAuthClientFileProblem::NoDesktopClient {}),
            "{text}"
        );
    }
}

#[test]
fn field_problems() {
    let missing = |name: &str| OAuthClientFileProblem::MissingField { name: name.into() };
    let invalid = |name: &str| OAuthClientFileProblem::InvalidField { name: name.into() };
    for (key, value, expected) in [
        ("client_id", None, missing("client_id")),
        ("client_secret", None, missing("client_secret")),
        ("client_id", Some(""), invalid("client_id")),
        ("client_id", Some("123-abc"), invalid("client_id")),
        (
            "client_id",
            Some(".apps.googleusercontent.com"),
            invalid("client_id"),
        ),
        (
            "client_id",
            Some("123 abc.apps.googleusercontent.com"),
            invalid("client_id"),
        ),
        (
            "client_id",
            Some("123-abc.apps.googleusercontent.com\n"),
            invalid("client_id"),
        ),
        (
            "client_id",
            Some("1-a.apps.googleusercontent.com.example"),
            invalid("client_id"),
        ),
        ("client_secret", Some(""), invalid("client_secret")),
        ("client_secret", Some("has space"), invalid("client_secret")),
        ("client_secret", Some("tab\t"), invalid("client_secret")),
    ] {
        let mut fields = complete();
        match value {
            Some(value) => fields.insert(key.into(), value.into()),
            None => fields.remove(key),
        };
        assert_eq!(
            parse_client_file(&installed(fields)),
            Err(expected),
            "{key} = {value:?}"
        );
    }
    let mut fields = complete();
    fields.insert("client_secret".into(), 42.into());
    assert_eq!(
        parse_client_file(&installed(fields)),
        Err(invalid("client_secret"))
    );
    let mut fields = complete();
    fields.insert(
        "client_id".into(),
        ("a".repeat(250) + ".apps.googleusercontent.com").into(),
    );
    assert_eq!(
        parse_client_file(&installed(fields)),
        Err(invalid("client_id"))
    );
    let mut fields = complete();
    fields.insert("client_id".into(), "1-a.apps.googleusercontent.com".into());
    assert!(parse_client_file(&installed(fields)).is_ok());
}

// MARK: Loopback requests

#[test]
fn the_google_redirect() {
    let head = "GET /?state=abc_123&code=4%2F0AVG7fiQ-x_y&scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fgmail.modify HTTP/1.1\r\n\
                Host: 127.0.0.1:53122\r\nUser-Agent: Mozilla/5.0\r\nAccept: text/html\r\n\r\n";
    assert_eq!(
        request(head),
        get(
            "/",
            &[
                ("state", "abc_123"),
                ("code", "4/0AVG7fiQ-x_y"),
                ("scope", "https://www.googleapis.com/auth/gmail.modify"),
            ]
        )
    );
}

#[test]
fn incomplete_heads_wait_for_more_bytes() {
    for text in [
        "",
        "GET",
        "GET /?code=1 HTTP/1.1\r\n",
        "GET /?code=1 HTTP/1.1\r\nHost: x\r\n",
        "GET / HTTP/1.1\r\n\r",
    ] {
        assert_eq!(request(text), HeadParse::Incomplete, "{text:?}");
    }
}

#[test]
fn bare_line_feeds_and_trailing_body_bytes() {
    assert_eq!(
        request("GET /favicon.ico HTTP/1.0\n\nextra"),
        get("/favicon.ico", &[])
    );
}

#[test]
fn form_decoding_and_fragments() {
    assert_eq!(
        request("GET /?a=x+y&b=%E2%9C%93&c&=v&d=1=2#frag HTTP/1.1\r\n\r\n"),
        get(
            "/",
            &[
                ("a", "x y"),
                ("b", "\u{2713}"),
                ("c", ""),
                ("", "v"),
                ("d", "1=2#frag")
            ]
        )
    );
    assert_eq!(request("GET /#x HTTP/1.1\r\n\r\n"), get("/", &[]));
}

#[test]
fn malformed_request_lines() {
    for text in [
        "\r\n\r\n",
        "GET\r\n\r\n",
        "GET / \r\n\r\n",
        "GET /  HTTP/1.1\r\n\r\n",
        "GET http://127.0.0.1/ HTTP/1.1\r\n\r\n",
        "GET / HTTP/2\r\n\r\n",
        "GET /?code=%ZZ HTTP/1.1\r\n\r\n",
        "GET /?code=%E2%9C HTTP/1.1\r\n\r\n",
        "GET /?code=%4 HTTP/1.1\r\n\r\n",
    ] {
        assert_eq!(request(text), HeadParse::Malformed, "{text:?}");
    }
    let mut bytes = b"GET /?x=".to_vec();
    bytes.push(0xFF);
    bytes.extend(b" HTTP/1.1\r\n\r\n");
    assert_eq!(parse_request_head(&bytes), HeadParse::Malformed);
}

#[test]
fn the_head_size_limit() {
    let limit = 8 * 1024;
    assert_eq!(
        request(&format!("GET /?x={} HTTP/1.1\r\n", "a".repeat(limit))),
        HeadParse::Malformed
    );
    assert_eq!(
        request(&format!("GET /?x={}", "a".repeat(3000))),
        HeadParse::Incomplete
    );
    let fits = format!("GET / HTTP/1.1\r\nX: {}\r\n\r\n", "b".repeat(limit - 23));
    assert_eq!(fits.len(), limit);
    assert_eq!(request(&fits), get("/", &[]));
    assert_eq!(
        request(&format!(
            "GET / HTTP/1.1\r\nX: {}\r\n\r\n",
            "b".repeat(limit - 22)
        )),
        HeadParse::Malformed
    );
}

// MARK: Callback decisions

fn decide(target: &str, method: &str, state: &str) -> CallbackDecision {
    let HeadParse::Request(request) = request(&format!("{method} {target} HTTP/1.1\r\n\r\n"))
    else {
        panic!("unparsable {target}")
    };
    CallbackDecision::decide(&request, state)
}

fn decide_get(target: &str) -> CallbackDecision {
    decide(target, "GET", "expected-state")
}

fn authorized_code(decision: CallbackDecision) -> Option<String> {
    match decision {
        CallbackDecision::Authorized { code } => Some(code.0),
        _ => None,
    }
}

#[test]
fn authorized() {
    assert_eq!(
        authorized_code(decide_get(
            "/?state=expected-state&code=4%2F0Ab-cd_EF&scope=x"
        ))
        .as_deref(),
        Some("4/0Ab-cd_EF")
    );
    assert_eq!(
        authorized_code(decide_get("/?code=abc&state=expected-state")).as_deref(),
        Some("abc")
    );
}

#[test]
fn denied() {
    let denied = |error: &str| CallbackDecision::Denied {
        error: error.into(),
    };
    assert_eq!(
        decide_get("/?error=access_denied&state=expected-state"),
        denied("access_denied")
    );
    assert_eq!(
        decide_get("/?error=%3Cscript%3E&state=expected-state"),
        denied("unknown_error")
    );
    assert_eq!(
        decide_get("/?error=&state=expected-state"),
        denied("unknown_error")
    );
    // An error wins over a code.
    assert_eq!(
        decide_get("/?error=access_denied&code=abc&state=expected-state"),
        denied("access_denied")
    );
}

#[test]
fn a_wrong_or_missing_state_or_no_code_is_rejected() {
    for target in [
        "/?code=abc",
        "/?code=abc&state=other",
        "/?code=abc&state=expected-state-but-longer",
        "/?code=abc&state=",
        "/?error=access_denied&state=other",
        "/?state=expected-state",
        "/?state=expected-state&code=",
        "/?state=expected-state&code=a+b",
        "/?state=expected-state&code=%0A",
        "/",
        // The first state value counts.
        "/?state=other&state=expected-state&code=abc",
    ] {
        assert_eq!(decide_get(target), CallbackDecision::Rejected, "{target}");
    }
}

#[test]
fn other_paths_are_not_found() {
    assert_eq!(decide_get("/favicon.ico"), CallbackDecision::NotFound);
    assert_eq!(
        decide_get("/callback?state=expected-state&code=abc"),
        CallbackDecision::NotFound
    );
}

#[test]
fn only_get() {
    for method in ["POST", "HEAD"] {
        assert_eq!(
            decide("/?state=expected-state&code=abc", method, "expected-state"),
            CallbackDecision::Rejected
        );
    }
}

#[test]
fn an_empty_expected_state_never_matches() {
    assert_eq!(
        decide("/?state=&code=abc", "GET", ""),
        CallbackDecision::Rejected
    );
}

#[test]
fn an_overlong_code_is_rejected() {
    let long = "a".repeat(2049);
    assert_eq!(
        decide_get(&format!("/?state=expected-state&code={long}")),
        CallbackDecision::Rejected
    );
}

// MARK: Responses

fn pages() -> SignInPages {
    SignInPages {
        completed: "FortiVPNManager: you can close this window.".into(),
        not_completed: "Sign-in was not completed.".into(),
        invalid: "\u{D55C} <b>".into(),
    }
}

#[test]
fn an_html_response() {
    let text = String::from_utf8(decide_get("/?state=expected-state&code=abc").response(&pages()))
        .unwrap();
    let (head, body) = text.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(head.contains("Content-Type: text/html; charset=utf-8"));
    assert!(head.contains(&format!("Content-Length: {}", body.len())));
    assert!(head.contains("Connection: close"));
    assert!(head.contains("Cache-Control: no-store"));
    assert!(body.contains("<p>FortiVPNManager: you can close this window.</p>"));
}

#[test]
fn markup_is_escaped() {
    let text = String::from_utf8(decide_get("/?code=abc").response(&pages())).unwrap();
    assert!(text.starts_with("HTTP/1.1 400 Bad Request\r\n"));
    assert!(text.contains("<p>\u{D55C} &lt;b&gt;</p>"));
    let quoted = SignInPages {
        invalid: r#"<a href="x">'&'</a>"#.into(),
        ..pages()
    };
    let text = String::from_utf8(decide_get("/?code=abc").response(&quoted)).unwrap();
    assert!(text.contains("&lt;a href=&quot;x&quot;&gt;&#39;&amp;&#39;&lt;/a&gt;"));
}
