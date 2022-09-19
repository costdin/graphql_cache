use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use warp::Filter;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub exp: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Key {
    pub kty: String,
    pub n: String,
    pub r#use: String,
    pub kid: String,
    pub e: String,
    pub alg: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct KeyDocument {
    pub keys: Vec<Key>,
}

pub enum AuthorizationType {
    Jwt(Vec<DecodingKey>),
    Simple,
}

#[derive(Debug)]
pub struct AuthHeader {
    pub sub: String,
    pub header: String,
}

pub struct AuthConfiguration {
    pub authorization_type: AuthorizationType,
    pub authorization_header: &'static str,
}

#[derive(Deserialize)]
pub struct OpenIdConfiguration {
    pub jwks_uri: String,
}

pub fn authorize_header(
    auth_configuration: Arc<AuthConfiguration>,
) -> impl Filter<Extract = (Option<AuthHeader>,), Error = warp::Rejection> + Clone {
    warp::header::optional::<String>(auth_configuration.authorization_header).map(
        move |auth: Option<String>| match (auth, &auth_configuration.authorization_type) {
            (Some(v), AuthorizationType::Simple) => Some(AuthHeader {
                sub: v.clone(),
                header: v,
            }),
            (Some(v), AuthorizationType::Jwt(keys)) => {
                let mut splitted = v.split_whitespace();
                match (splitted.next(), splitted.next()) {
                    (Some("Bearer"), Some(token)) => {
                        match decode::<Claims>(&token, &keys[0], &Validation::new(Algorithm::RS256)) {
                            Ok(token) => Some(AuthHeader {
                                sub: token.claims.sub,
                                header: v,
                            }),
                            Err(e) => {
                                println!("Error: {}", e);
                                None
                            },    
                        }
                    },
                    _ => None 
                }
            }
            (None, _) => None,
        },
    )
}

#[derive(Debug)]
pub struct Error {}

impl std::convert::From<reqwest::Error> for Error {
    fn from(_error: reqwest::Error) -> Self {
        Error {}
    }
}

impl std::convert::From<serde_json::Error> for Error {
    fn from(_error: serde_json::Error) -> Self {
        Error {}
    }
}

async fn get_jwks(discovery_document_url: &str) -> Result<KeyDocument, Error> {
    let client = reqwest::Client::new();
    let res = client.get(discovery_document_url).send().await?;

    let oidc_config = res.json::<OpenIdConfiguration>().await?;
    let jwks_res = client.get(oidc_config.jwks_uri).send().await?;

    return match jwks_res.json::<KeyDocument>().await {
        Ok(r) => Ok(r),
        Err(_) => Err(Error {}),
    };
}

pub async fn get_oidc_config(
    discovery_document_url: &str,
    header_name: &'static str,
) -> Result<AuthConfiguration, Error> {
    let key_document = get_jwks(discovery_document_url).await?;

    let keys = key_document
        .keys
        .iter()
        .filter(|k| k.r#use == "sig" && k.kty == "RSA")
        .filter_map(|k| DecodingKey::from_rsa_components(&k.n, &k.e).ok())
        .collect::<Vec<_>>();

    Ok(AuthConfiguration {
        authorization_type: AuthorizationType::Jwt(keys),
        authorization_header: header_name,
    })
}
