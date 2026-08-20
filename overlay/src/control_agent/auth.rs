use base64::{decode_config, URL_SAFE_NO_PAD};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde_derive::Deserialize;
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use super::ServiceJwtConfig;

const MAX_SERVICE_TOKEN_BYTES: usize = 8_192;
const MAX_SERVICE_TOKEN_LIFETIME_SECONDS: u64 = 300;
const CLOCK_SKEW_SECONDS: u64 = 30;

pub(super) struct ServiceJwtVerifier {
    issuer: String,
    audience: String,
    keys: HashMap<String, DecodingKey>,
}

#[derive(Clone, Debug)]
pub(super) struct ControlPrincipal {
    pub(super) service: String,
    pub(super) actor: String,
    pub(super) certificate_uri_san: String,
}

#[derive(Debug)]
pub(super) struct AuthFailure {
    pub(super) code: &'static str,
    pub(super) detail: &'static str,
    pub(super) status: u16,
}

#[derive(Debug, Deserialize)]
struct Claims {
    iss: String,
    aud: Audience,
    sub: String,
    azp: String,
    scope: Scope,
    #[serde(default)]
    act: Option<ActorClaim>,
    iat: u64,
    nbf: u64,
    exp: u64,
    jti: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ActorClaim {
    sub: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Scope {
    Text(String),
    Values(Vec<String>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JwksDocument {
    keys: Vec<Jwk>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Jwk {
    kty: String,
    crv: String,
    #[serde(rename = "use")]
    key_use: String,
    alg: String,
    kid: String,
    x: String,
    #[serde(default)]
    key_ops: Option<Vec<String>>,
}

impl ServiceJwtVerifier {
    pub(super) fn load(
        config: &ServiceJwtConfig,
        base: &Path,
        instance_id: &str,
    ) -> Result<Self, String> {
        if config.issuer.trim().is_empty() {
            return Err("service_jwt.issuer is required".to_owned());
        }
        let path = super::resolve_path(base, &config.jwks_file);
        let raw = fs::read(&path)
            .map_err(|err| format!("cannot read control service JWKS {}: {err}", path.display()))?;
        let keys = parse_jwks(&raw)?;
        let audience = format!("{}{}", config.audience_prefix, instance_id);
        Ok(Self {
            issuer: config.issuer.trim().to_owned(),
            audience,
            keys,
        })
    }

    pub(super) fn verify(
        &self,
        token: Option<&str>,
        certificate_uri_san: Option<&str>,
        required_scope: &str,
    ) -> Result<ControlPrincipal, AuthFailure> {
        let certificate_uri_san = certificate_uri_san.ok_or(AuthFailure {
            code: "CLIENT_CERT_DENIED",
            detail: "The client certificate identity is not allowed.",
            status: 403,
        })?;
        let token = token.ok_or(AuthFailure {
            code: "AUTH_REQUIRED",
            detail: "A service bearer token is required.",
            status: 401,
        })?;
        if token.is_empty() || token.len() > MAX_SERVICE_TOKEN_BYTES {
            return Err(invalid_token());
        }
        let header = decode_header(token).map_err(|_| invalid_token())?;
        if header.alg != Algorithm::EdDSA
            || header.typ.as_deref().is_some_and(|value| value != "JWT")
        {
            return Err(invalid_token());
        }
        let kid = header
            .kid
            .as_deref()
            .filter(|kid| !kid.is_empty())
            .ok_or_else(invalid_token)?;
        let key = self.keys.get(kid).ok_or(AuthFailure {
            code: "AUTH_KEY_UNAVAILABLE",
            detail: "The service token verification key is unavailable.",
            status: 401,
        })?;
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.validate_exp = false;
        validation.validate_nbf = false;
        validation.leeway = 0;
        validation.set_required_spec_claims(&["iss", "aud", "sub"]);
        validation.set_issuer(&[self.issuer.as_str()]);
        validation.set_audience(&[self.audience.as_str()]);
        let claims = decode::<Claims>(token, key, &validation)
            .map_err(|_| invalid_token())?
            .claims;
        let now = epoch_seconds();
        if claims.exp.saturating_add(CLOCK_SKEW_SECONDS) < now {
            return Err(AuthFailure {
                code: "TOKEN_EXPIRED",
                detail: "The service bearer token has expired.",
                status: 401,
            });
        }
        if claims.iss != self.issuer
            || !claims.aud.contains(&self.audience)
            || !bounded_identity(&claims.sub)
            || claims.azp != certificate_uri_san
            || claims.jti.is_empty()
            || claims.jti.len() > 256
            || claims.nbf > now.saturating_add(CLOCK_SKEW_SECONDS)
            || claims.iat > now.saturating_add(CLOCK_SKEW_SECONDS)
            || claims.exp <= claims.iat
            || claims.nbf > claims.exp
            || claims.exp.saturating_sub(claims.iat) > MAX_SERVICE_TOKEN_LIFETIME_SECONDS
        {
            return Err(invalid_token());
        }
        if !claims.scope.contains(required_scope) {
            return Err(AuthFailure {
                code: "SCOPE_DENIED",
                detail: "The service token does not grant the required scope.",
                status: 403,
            });
        }
        let actor = match claims.act {
            Some(actor) if bounded_identity(&actor.sub) => actor.sub,
            Some(_) => return Err(invalid_token()),
            None => claims.sub.clone(),
        };
        Ok(ControlPrincipal {
            service: claims.sub,
            actor,
            certificate_uri_san: certificate_uri_san.to_owned(),
        })
    }
}

impl Audience {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Self::One(value) => value == expected,
            Self::Many(values) => values.iter().any(|value| value == expected),
        }
    }
}

impl Scope {
    fn contains(&self, expected: &str) -> bool {
        match self {
            Self::Text(value) => value
                .split_ascii_whitespace()
                .any(|value| value == expected),
            Self::Values(values) => values.iter().any(|value| value == expected),
        }
    }
}

fn parse_jwks(raw: &[u8]) -> Result<HashMap<String, DecodingKey>, String> {
    let document: JwksDocument = serde_json::from_slice(raw)
        .map_err(|err| format!("invalid control service JWKS: {err}"))?;
    if document.keys.is_empty() {
        return Err("control service JWKS contains no keys".to_owned());
    }
    let mut keys = HashMap::new();
    let mut seen = HashSet::new();
    for key in document.keys {
        if key.kty != "OKP"
            || key.crv != "Ed25519"
            || key.key_use != "sig"
            || key.alg != "EdDSA"
            || key.kid.is_empty()
            || key.kid.len() > 128
            || !seen.insert(key.kid.clone())
        {
            return Err("control service JWKS violates the Ed25519 signing profile".to_owned());
        }
        if key
            .key_ops
            .as_ref()
            .is_some_and(|operations| operations.as_slice() != ["verify"])
        {
            return Err("control service JWK key_ops must contain only verify".to_owned());
        }
        let raw_key = decode_config(&key.x, URL_SAFE_NO_PAD)
            .map_err(|_| "control service JWK x is not valid base64url".to_owned())?;
        if raw_key.len() != 32 {
            return Err("control service Ed25519 key must contain 32 bytes".to_owned());
        }
        keys.insert(key.kid, DecodingKey::from_ed_der(&raw_key));
    }
    Ok(keys)
}

fn invalid_token() -> AuthFailure {
    AuthFailure {
        code: "TOKEN_INVALID",
        detail: "The service bearer token is invalid.",
        status: 401,
    }
}

fn bounded_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
