use previously_on::redaction::{redact_excerpt, redact_text, redact_value, REDACTED};
use serde_json::json;

#[test]
fn redacts_assignments_headers_tokens_and_sensitive_paths() {
    let input = concat!(
        "api_key=super-secret-value\n",
        "Authorization: Bearer actual-bearer-token\n",
        "github_pat_abcdefghijklmnopqrstuvwxyz\n",
        "read /Users/me/project/.env.production now\n",
        "postgres://alice:hunter2@localhost/database"
    );
    let output = redact_text(input);
    assert!(!output.contains("super-secret-value"));
    assert!(!output.contains("actual-bearer-token"));
    assert!(!output.contains("github_pat_abcdefghijklmnopqrstuvwxyz"));
    assert!(!output.contains(".env.production"));
    assert!(!output.contains("hunter2"));
    assert!(output.matches(REDACTED).count() >= 5);
}

#[test]
fn redacts_prefixed_environment_keys_cli_flags_and_private_material() {
    let corpus = [
        "OPENAI_API_KEY=sk-plain-prefixed-secret",
        "AWS_SECRET_ACCESS_KEY: aws-prefixed-secret",
        "NPM_TOKEN='npm-prefixed-secret'",
        "MY_SERVICE_CLIENT_SECRET=service-client-secret",
        "token=bare-token-secret",
        "command --api-key cli-api-secret --access-token=cli-access-secret --token cli-bare-token-secret",
        "Authorization: Basic authorization-secret",
        "https://alice:url-password@example.test/private",
        "-----BEGIN OPENSSH PRIVATE KEY-----\nprivate-key-body\n-----END OPENSSH PRIVATE KEY-----",
        "read .env.local id_ed25519 credentials.json",
    ]
    .join("\n");

    let output = redact_text(&corpus);
    for secret in [
        "sk-plain-prefixed-secret",
        "aws-prefixed-secret",
        "npm-prefixed-secret",
        "service-client-secret",
        "bare-token-secret",
        "cli-api-secret",
        "cli-access-secret",
        "cli-bare-token-secret",
        "authorization-secret",
        "url-password",
        "private-key-body",
        ".env.local",
        "id_ed25519",
        "credentials.json",
    ] {
        assert!(!output.contains(secret), "secret leaked: {secret}");
    }
    assert!(output.matches(REDACTED).count() >= 12);
}

#[test]
fn recursively_redacts_json_and_caps_unicode_excerpt() {
    let input = json!({
        "password": "do-not-store",
        "cookie": "session=do-not-store-cookie",
        "privateKey": "opaque-private-material",
        "AWS_SECRET_ACCESS_KEY": "aws-secret-material",
        "nestedToken": "generic-token-material",
        "nested": {"excerpt": format!("{} api_key=also-secret", "가".repeat(600))},
        "safe": "keep me"
    });
    let output = redact_value(&input);
    assert_eq!(output["password"], REDACTED);
    assert_eq!(output["cookie"], REDACTED);
    assert_eq!(output["privateKey"], REDACTED);
    assert_eq!(output["AWS_SECRET_ACCESS_KEY"], REDACTED);
    assert_eq!(output["nestedToken"], REDACTED);
    assert_eq!(output["safe"], "keep me");
    assert!(!output.to_string().contains("also-secret"));
    assert!(
        output["nested"]["excerpt"]
            .as_str()
            .unwrap()
            .chars()
            .count()
            <= 500
    );

    let excerpt = redact_excerpt(&"🙂".repeat(700));
    assert_eq!(excerpt.chars().count(), 500);
}
