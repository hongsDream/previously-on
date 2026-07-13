use crate::domain::MAX_EVIDENCE_EXCERPT_CHARS;
use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use serde_json::Value;

pub const REDACTED: &str = "[REDACTED]";

/// Returns whether a repository-relative path names a credential-bearing file that must never
/// cross a capture boundary.
///
/// This is deliberately component based rather than substring based: `src/credentials.rs` is
/// ordinary source code, while `.env.production`, `credentials.json`, and SSH private-key names
/// are sensitive regardless of their containing directory. Callers must omit matching paths
/// entirely instead of replacing them with a marker, because even the path itself may disclose a
/// secret's location.
pub fn is_sensitive_path(path: &str) -> bool {
    path.replace('\\', "/").split('/').any(|component| {
        let component = component.to_ascii_lowercase();
        component.starts_with(".env")
            || matches!(
                component.as_str(),
                "credentials"
                    | "credentials.json"
                    | "credentials.yaml"
                    | "credentials.yml"
                    | "credentials.toml"
                    | "credentials.ini"
                    | "credential"
                    | "credential.json"
                    | "credential.yaml"
                    | "credential.yml"
                    | "credential.toml"
                    | "credential.ini"
                    | "secrets"
                    | "secrets.json"
                    | "secrets.yaml"
                    | "secrets.yml"
                    | "secrets.toml"
                    | "secrets.ini"
                    | "secret"
                    | "secret.json"
                    | "secret.yaml"
                    | "secret.yml"
                    | "secret.toml"
                    | "secret.ini"
            )
            || matches!(
                component.as_str(),
                "id_rsa" | "id_rsa.pub" | "id_dsa" | "id_dsa.pub" | "id_ed25519" | "id_ed25519.pub"
            )
    })
}

static AUTHORIZATION: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?im)(authorization\s*:\s*)(?:bearer|basic|token)?\s*[^\s,;]+")
        .expect("authorization regex")
});
static ASSIGNMENT: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?im)\b((?:[a-z0-9]+[_-])*(?:api[_-]?key|access[_-]?token|refresh[_-]?token|auth[_-]?token|password|passwd|client[_-]?secret|secret[_-]?(?:key|access[_-]?key)|private[_-]?key|session[_-]?token|npm[_-]?token|token))\b(\s*(?:=|:)\s*)('[^'\r\n]*'|"[^"\r\n]*"|[^\s,;\r\n}]+)"#,
    )
    .expect("assignment regex")
});
static CLI_SECRET_FLAG: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?im)(--(?:[a-z0-9]+[-_])*(?:api[-_]?key|access[-_]?token|refresh[-_]?token|auth[-_]?token|password|passwd|client[-_]?secret|secret(?:[-_]?(?:key|access[-_]?key))?|private[-_]?key|session[-_]?token|npm[-_]?token|token)(?:\s*=\s*|\s+))('[^'\r\n]*'|"[^"\r\n]*"|[^\s,;\r\n}]+)"#,
    )
    .expect("CLI secret flag regex")
});
static WELL_KNOWN_TOKEN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(?:sk-(?:proj-)?[a-z0-9_-]{12,}|github_pat_[a-z0-9_]{12,}|gh[pousr]_[a-z0-9]{12,}|xox[baprs]-[a-z0-9-]{12,}|akia[0-9a-z]{12,})\b",
    )
    .expect("well-known token regex")
});
static PRIVATE_KEY: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?s)-----BEGIN (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----.*?-----END (?:RSA |EC |OPENSSH |DSA )?PRIVATE KEY-----",
    )
    .expect("private key regex")
});
static URL_CREDENTIALS: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"(?i)([a-z][a-z0-9+.-]*://[^\s/@:]+:)[^\s/@]+(@)").expect("url credential regex")
});
static SENSITIVE_PATH: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r#"(?i)(?:[a-z]:)?(?:[^\s"'<>|]*[/\\])?(?:\.env(?:\.[a-z0-9_.-]+)?|credentials?(?:\.[a-z0-9_.-]+)?|secrets?(?:\.[a-z0-9_.-]+)?|id_(?:rsa|dsa|ed25519)(?:\.pub)?)\b(?:[/\\][^\s"'<>|]*)?"#,
    )
    .expect("sensitive path regex")
});
#[derive(Debug, Clone, Copy, Default)]
pub struct Redactor;

impl Redactor {
    pub fn redact_text(&self, input: &str) -> String {
        redact_text(input)
    }

    pub fn redact_excerpt(&self, input: &str) -> String {
        cap_chars(&redact_text(input), MAX_EVIDENCE_EXCERPT_CHARS)
    }

    pub fn redact_value(&self, value: &Value) -> Value {
        redact_value(value)
    }
}

pub fn redact_text(input: &str) -> String {
    let redacted = PRIVATE_KEY.replace_all(input, REDACTED);
    let redacted = AUTHORIZATION.replace_all(&redacted, |captures: &Captures<'_>| {
        format!("{}{}", &captures[1], REDACTED)
    });
    let redacted = ASSIGNMENT.replace_all(&redacted, |captures: &Captures<'_>| {
        format!("{}{}{}", &captures[1], &captures[2], REDACTED)
    });
    let redacted = CLI_SECRET_FLAG.replace_all(&redacted, |captures: &Captures<'_>| {
        format!("{}{}", &captures[1], REDACTED)
    });
    let redacted = URL_CREDENTIALS.replace_all(&redacted, |captures: &Captures<'_>| {
        format!("{}{}{}", &captures[1], REDACTED, &captures[2])
    });
    let redacted = WELL_KNOWN_TOKEN.replace_all(&redacted, REDACTED);
    SENSITIVE_PATH.replace_all(&redacted, REDACTED).into_owned()
}

pub fn redact_excerpt(input: &str) -> String {
    cap_chars(&redact_text(input), MAX_EVIDENCE_EXCERPT_CHARS)
}

pub fn redact_value(value: &Value) -> Value {
    match value {
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
        Value::String(text) => Value::String(redact_text(text)),
        Value::Array(items) => Value::Array(items.iter().map(redact_value).collect()),
        Value::Object(object) => {
            let mut output = serde_json::Map::new();
            let mut redacted_key_index = 0_u32;
            for (key, value) in object {
                let value = if is_sensitive_key(key) {
                    Value::String(REDACTED.to_string())
                } else if key.eq_ignore_ascii_case("excerpt") {
                    Value::String(redact_excerpt(value.as_str().unwrap_or_default()))
                } else {
                    redact_value(value)
                };
                let output_key = if redact_text(key) == *key {
                    key.clone()
                } else {
                    loop {
                        let candidate = format!("[REDACTED_KEY_{redacted_key_index}]");
                        redacted_key_index = redacted_key_index.saturating_add(1);
                        if !object.contains_key(&candidate) && !output.contains_key(&candidate) {
                            break candidate;
                        }
                    }
                };
                output.insert(output_key, value);
            }
            Value::Object(output)
        }
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized = key
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
    matches!(
        normalized.as_str(),
        "authorization"
            | "cookie"
            | "setcookie"
            | "credential"
            | "credentials"
            | "password"
            | "passwd"
            | "privatekey"
            | "token"
    ) || [
        "token",
        "cookie",
        "credential",
        "credentials",
        "privatekey",
        "apikey",
        "accesstoken",
        "refreshtoken",
        "authtoken",
        "idtoken",
        "sessiontoken",
        "clientsecret",
        "secretkey",
        "secretaccesskey",
        "signingkey",
    ]
    .iter()
    .any(|suffix| normalized.ends_with(suffix))
}

pub fn cap_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    input.chars().take(max_chars).collect()
}
